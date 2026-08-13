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
import contextlib
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
EPISTEMIC = ("FACT", "HYPOTHESIS", "QUESTION")         # §3.1 기본 열거값 전수

# G2 리뷰어 격리(§8.1) — 리뷰어 슬롯·어댑터 키는 **식별자**이므로 상수로 고정한다(마스터 헌장
#   §3 '슬롯명·어댑터 키는 바꾸지 않는다'). send/wait 포함 전 공용 진입점에서 차단한다.
REVIEWER_SLOTS = ("reviewer-gemini", "reviewer-codex")
REVIEWER_KEYS = ("gemini", "codex")                    # 어댑터 별칭(reviewer- 접두가 없어 우회로였다)

# G1 진위 게이트 — 빈/공백/무의미 스니펫은 '검증 불능'으로 접어 강등한다(자명 통과 봉쇄).
#   verified:true 의 의미는 축소됐다: '스니펫이 그 라인에 실재함'만 증명할 뿐 '주장을
#   뒷받침함'은 증명하지 못한다(주장 뒷받침 판정은 리뷰어 사후 감사 · RADIO_CONTRACT §3·§9).
EVIDENCE_MIN_SNIPPET = 6                                # strip 후 최소 유의미 길이

# G6 A5-R01 — 단일 레코드 본문 상한(= SURFACE_CAP_BYTES 의 절반 = 8KB). 16KB 표면화 캡을
#   단일 초대형 레코드가 통째로 우회하는 경로를 차단한다(초과분은 절단+read 지시자).
RECORD_BODY_CAP = 8 * 1024

# A3-R-02 — GAP 봉인 범위 상한(단일 GAP 레코드로 티켓 영구 브릭·전 노드 watcher 마비 차단).
GAP_MAX_SPAN = 100000

# A1-R7 능력 영수증 위조 난이도 상향용 솔트(암호 비밀 아님 — 하드락 회피·직접 위조 차단선).
_CAP_SALT = "javis-radio-capability-v4"
# §3.9 관리 레코드 — grade:=FYI 고정·도구 자동 부여·wake/델타 제외
MGMT_TYPES = ("ROTATED", "ROTATED_FROZEN", "GAP", "CLOSE")
MSG_TYPE = "MSG"
RETRACT_TYPE = "RETRACT"

# §2.5 스레드 2종 전수 — worklog(작업 발견 전용)·results(워커 결과).
#   'master 스레드' 는 존재하지 않는다(A10(c)) · 2종 수량 불변식도 불변이다.
THREADS = ("worklog", "results")
DEFAULT_THREAD = "worklog"

ROTATE_BYTES = 5 * 1024 * 1024      # §2.3(b)
SURFACE_CAP_BYTES = 16 * 1024       # §4.3 — 단일 표면화 폭 제한(총 토큰 절약 아님·A15)
READ_LIMIT_DEFAULT = 40             # §4.10
READ_LIMIT_MAX = 200                # §4.10 — 구제 경로가 캡 목적을 무효화하지 않게 하는 상한
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
FYI_DIGEST_SEC = 1800.0             # §5.2(d) — FYI 단독 보류 상한(30분 뒤 digest 1회)
CONFIRM_RENOTIFY_BU_SEC = 600.0     # A25(c) — B/U 포함 batch 재통지 주기(10분)
CONFIRM_RENOTIFY_NF_SEC = 1800.0    # A31(e) — N/F-only batch 저빈도 재통지 주기(30분)
PAUSE_RELEASE_LABEL = "pause 중 도착 — 착수 전 master 확인"   # §9.2 라벨
RECOVERY_LABEL = "cycle 복원 재배달"                          # AA22 §10.3③ 라벨

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


# ── 신원·경로 게이트 (G2 노드 신원 · G3 티켓 경로 순회) ──────────────────────
def is_reviewer(node):
    """§8.1 리뷰어 격리 — 리뷰어 슬롯(reviewer-*)·어댑터 키(gemini·codex)는 radio 피어가
    아니다. open 한정 검사를 공용 진입점으로 끌어올려 send/wait 등 전 명령에서 차단한다
    (A1-R4: 종전 open 만 접두 검사해 send·wait 로 라이브 개입 가능했다)."""
    n = (node or "").strip()
    return n.startswith("reviewer-") or n in REVIEWER_SLOTS or n in REVIEWER_KEYS


def participant_check(ticket, node):
    """(ok, exit_code, msg) — send/wait/ack/resolve/recover/done-check 공용 진입점.
    ①리뷰어 차단(exit 6) ②비참여자 차단(exit 2) — A1-R3: --node 는 무인증이라 회전으로
    쿨다운·차단기·resolve 게이트가 전면 우회됐다. master 는 §8.2 옵저버라 항상 허용한다."""
    if is_reviewer(node):
        return False, EXIT_REVIEWER, ("리뷰어 노드 %s 는 radio 피어가 아니다(§8.1 격리 · exit 6) — "
                                      "리뷰어는 freeze 된 스레드 로그를 사후 일괄 열람한다" % node)
    if node == "master":
        return True, EXIT_OK, ""
    parts = list(read_meta(ticket).get("participants") or [])
    if node in parts:
        return True, EXIT_OK, ""
    return False, EXIT_USAGE, ("비참여자 노드 %s — META participants=%s. 개통 시 등록되지 않은 "
                               "노드는 스레드에 주입할 수 없다(A1-R3 --node 회전 차단)" % (node, parts))


_TICKET_RE = re.compile(r"^[A-Za-z0-9._-]+$")


def valid_ticket(ticket):
    """(ok, msg) — G3 경로 순회 차단(A3-R04). ticket 은 노드명과 동형으로 정화 대상이다.
    화이트리스트 + realpath 가 radio_dir() 하위인지 확인(../ · / · 심링크 이탈 거부) —
    종전에는 ticket_dir 이 무정화 join 이라 open --ticket '../../x' 가 radio 밖에 파일을
    생성했다(write-anywhere)."""
    if not ticket or not _TICKET_RE.match(ticket):
        return False, "ticket 이름 위반(^[A-Za-z0-9._-]+$ · ../ 나 / 불가): %r" % (ticket or "")[:60]
    rd = os.path.realpath(radio_dir())
    cand = os.path.realpath(ticket_dir(ticket))
    if cand != rd and not cand.startswith(rd + os.sep):
        return False, "ticket 경로가 radio 디렉터리 밖: %s" % ticket
    return True, ""


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
    if text[:1] == "﻿":        # A3-R06 — 파일 선두 BOM 은 첫 레코드를 유실시킨다
        text = text[1:]
    lines = text.split("\n")
    # §2.7·A3-R07/A5-R04 — 말미 원소(개행종료 시 빈 문자열 / 미종료 시 부분줄='기록 진행
    #   중')는 **항상** 버린다. 종전엔 `lines[:-1] if trailing_partial else lines[:-1]` 로 두
    #   분기가 문자 그대로 동일한 죽은 삼항이라, 유지보수자가 한쪽을 `lines` 로 '고치면'
    #   빈 문자열/부분줄이 레코드로 유입돼 판독이 깨졌다. 의도를 코드로 단순화한다.
    body = lines[:-1]
    for i, ln in enumerate(body, start=1):
        s = ln.strip().lstrip("﻿")     # A3-R06 — 라인 내 잔여 BOM 제거
        if not s:
            continue
        try:
            obj = json.loads(s)
        except ValueError:
            # A3-R06 — 잔여 raw 제어/널 바이트로 파싱 실패한 **정상 레코드**를 1회 회생한다
            #   (엄격 json 은 문자열 값 내부 \x00·제어문자를 거부). 진짜 오염은 여전히 잡힌다.
            s2 = _CTRL_STRIP.sub("", s)
            try:
                obj = json.loads(s2) if s2 != s else None
            except ValueError:
                obj = None
            if obj is None:
                bad.append(i)
                continue
        if isinstance(obj, dict):
            recs.append(obj)
        else:
            bad.append(i)
    return recs, bad


# A3-R06 — 라인 내 제어/널 바이트(개행·탭·CR 제외) — 정상 레코드 회생 시에만 제거한다.
_CTRL_STRIP = re.compile(r"[\x00-\x08\x0b\x0c\x0e-\x1f]")


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
# A4-R03 — **왼쪽부터** 파일·라인 2필드만 분리하고 나머지 전체가 스니펫이다. 종전 greedy
#   `^(.+):(\d+):(.*)$` 는 스니펫 내부의 마지막 `:숫자:`(로그 타임스탬프 '12:30:', 슬라이스
#   'x:10:y')를 라인 구분자로 오인해 파일경로·라인번호를 스니펫에서 탈취했다. 비탐욕 `.+?`
#   는 파일경로 다음의 **첫** `:\d+:` 를 라인 구분자로 앵커링한다(Windows 'C:\x.py:12:snip'
#   도 정상 — `.+?` 가 `C:\x.py` 까지 확장한 뒤 첫 `:12:` 에서 멈춘다).
_EV_RE = re.compile(r"^(.+?):(\d+):(.*)$", re.S)


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


def _resolve_under_root(path):
    """(cand, reason) — G1② · G3. evidence·재검증 대상 파일을 **JAVIS_ROOT 하위로 강제**한다.
    realpath 로 심링크·../ escape 를 해소한 뒤 ROOT 접두를 확인한다 — 절대경로(예 /etc/hosts)·
    상대 ../ 이탈·심링크 탈출은 거부한다(A1-R1·A3-R05: 종전엔 임의 절대/상대 파일을 FACT
    근거로 검증·전재해 scrub 미탐 비밀이 피어 가시 스레드로 유출됐다). 거부는 검증 실패=강등
    으로 접힌다(fail-closed). 워크스페이스 안의 절대경로(임시 루트의 probe 등)는 허용된다."""
    root = os.path.realpath(ROOT())
    raw = path if os.path.isabs(path) else os.path.join(ROOT(), path)
    cand = os.path.realpath(raw)
    if cand != root and not cand.startswith(root + os.sep):
        return None, "대상 파일이 워크스페이스(JAVIS_ROOT) 밖 — 인용 불가: %s" % path
    return cand, ""


def verify_evidence(evidence):
    """(ok, reason, 보강된 evidence). 근본 강화된 진위 게이트(G1):
        ①스니펫 필수(strip 후 비공백·최소 EVIDENCE_MIN_SNIPPET 자) — 빈/공백 스니펫은
          '검증 불능'으로 거부한다.
        ②대상 파일이 JAVIS_ROOT 하위(_resolve_under_root — 절대/../·심링크 이탈 거부)
        ③그 라인 실존 ④해당 라인에 인용 스니펫 포함(grep 일치)

    ★verified:true 의 의미는 축소됐다(정직 기술): '인용 스니펫이 그 라인에 실재함'만
      증명할 뿐 '그 스니펫이 주장(text)을 뒷받침함'은 증명하지 못한다 — 주장 뒷받침 판정은
      리뷰어 사후 감사의 몫이다(RADIO_CONTRACT §3·§9). 종전 구현은 빈 스니펫(file:1:)·
      대상 라인의 아무 부분문자열·워크스페이스 밖 임의 파일로 자명 통과해, 주장과 무관한
      허위 FACT 가 verified:true 로, 허위 BLOCKER 가 stdin 최고특권으로 저장됐다(A1-R1·R3).

    ★AA34(b): 통과 시 line_hash(현재 라인 SHA-256)·snippet_hash·verified:true 를 저장한다.
      사후 재검증은 라인 해시 대조로 수행한다(마스킹 비의존 · 사후 변조·근거 소멸 탐지).
    """
    if not evidence:
        return False, "evidence 없음", []
    enriched = []
    for ev in evidence:
        path, lineno, snip = ev.get("file"), ev.get("line"), ev.get("snippet") or ""
        if not path or not isinstance(lineno, int) or lineno < 1:
            return False, "evidence 필드 불비(file/line)", []
        # ① 스니펫 필수 — 빈/공백/무의미 스니펫 거부(A1-R1·A3-R03 자명 통과 봉쇄)
        if len(snip.strip()) < EVIDENCE_MIN_SNIPPET:
            return False, ("스니펫 부재/과단(최소 %d자 비공백 필요) — 빈 스니펫으로는 "
                           "'그 라인에 실재함'조차 증명되지 않는다" % EVIDENCE_MIN_SNIPPET), []
        # ② 대상 파일을 워크스페이스 하위로 강제(A1-R1·A3-R05)
        cand, why = _resolve_under_root(path)
        if cand is None:
            return False, why, []
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
        if snip.strip() not in target:
            return False, "스니펫 불일치: %s:%d" % (path, lineno), []
        e2 = dict(ev)
        e2["line_hash"] = _sha256(target)
        e2["snippet_hash"] = _sha256(snip)
        e2["verified"] = True
        enriched.append(e2)
    return True, "", enriched


def recheck_evidence(evidence):
    """AA34(b) 재검증 — 저장 line_hash 와 대상 파일 **현재** 해당 라인의 해시를 대조한다.
    대상 파일은 재검증 시점에도 JAVIS_ROOT 하위여야 한다(G1② — 저장 후 심링크 갈아끼우기 차단)."""
    for ev in evidence or []:
        path, lineno, want = ev.get("file"), ev.get("line"), ev.get("line_hash")
        if not (path and isinstance(lineno, int) and want):
            return False, "재검증 입력 불비"
        cand, why = _resolve_under_root(path)
        if cand is None:
            return False, why
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


class _Sealed(object):
    """A3-R-02 — GAP 봉인 구간을 파이썬 set 으로 materialize 하지 않는다. 종전
    `out.update(range(a, b+1))` 는 거대 범위(정상 JSON GAP 레코드로 파일에 들어오면)
    10^12 원소 set 생성을 매 wait 폴에서 시도해 uninterruptible hang/OOM 을 냈다
    (C레벨 set.update 가 GIL 을 잡아 SIGALRM 도 못 뚫는다). 정렬 구간 리스트로 보관하고
    '연속 판정'을 구간 포함 검사(O(log n) 근사)로 수행한다."""

    __slots__ = ("intervals",)

    def __init__(self, intervals):
        self.intervals = intervals          # 정렬된 [(from, to), ...]

    def __contains__(self, s):
        if not isinstance(s, int):
            return False
        for a, b in self.intervals:
            if a <= s <= b:
                return True
            if a > s:
                break                        # 정렬돼 있으므로 조기 종료
        return False

    def __bool__(self):
        return bool(self.intervals)


def sealed_seqs(records):
    """GAP 봉인 구간(§2.7(c)) — 커서 '최대 연속' 판정에서 존재하는 seq 로 취급한다.
    반환은 `_Sealed`(구간 멤버십 객체)이며 `s in sealed` 로만 쓴다(materialize 금지)."""
    intervals = []
    for r in records:
        if r.get("type") == "GAP":
            a, b = r.get("gap_from"), r.get("gap_to")
            if isinstance(a, int) and isinstance(b, int) and a <= b:
                intervals.append((a, b))
    intervals.sort()
    return _Sealed(intervals)


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
def _append_in_lock(ticket, thread, rec, allow_closed=False):
    """★임계구역 **안**에서만 호출한다(`.seq-lock-<스레드>` 보유 전제).

    ★AA20 §2.3(a) 치명 수리: seq 는 로테이션 **뒤에** 확정한다. 종전 구현은 로테이션
      전에 `final_seq+1` 로 seq 를 잡아두고 tombstone 에도 같은 값을 줘, 로테이션이
      일어난 순간 tombstone 과 신규 메시지가 **동일 seq** 를 갖고 read_thread 의 seq
      dedup 이 파일 순서상 앞선 tombstone 만 남겨 메시지를 **영구 은닉**했다.
      이제 로테이션이 소비한 seq 수(`consumed`)를 돌려받아 그 다음 번호를 쓴다 —
      tombstone 도 임계구역 안에서 정식으로 seq 를 소비하는 레코드다.
    """
    meta = read_meta(ticket)
    if meta.get("closed") and not allow_closed:
        return None, EXIT_CLOSED
    recs, _ = read_thread(ticket, thread, meta)
    base = final_seq(recs)
    path = thread_path(ticket, thread, meta)
    consumed = _rotate_if_needed(ticket, thread, path, recs, meta)
    path = thread_path(ticket, thread, read_meta(ticket))
    _heal_trailing_newline(path)
    seq = base + consumed + 1
    rec = dict(rec)
    rec["seq"] = seq
    rec.setdefault("schema_version", SCHEMA_VERSION)
    rec.setdefault("ts", _now())
    rec["thread"] = thread          # §3.1 필드 — msg_id 파싱 없이 소속 스레드가 읽히게
    rec["msg_id"] = "%s:%s:%d" % (ticket, thread, seq)
    _jsonl_append(path, rec)
    return rec, EXIT_OK


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
            return _append_in_lock(ticket, thread, rec, allow_closed=allow_closed)
    except javis_lock.LockError as e:
        sys.stderr.write("seq 락 획득 실패(%s) — 재시도하지 말고 상위에 보고하라\n" % e.status)
        return None, EXIT_CONFLICT


def close_commit(ticket, rec):
    """AA37 TOCTOU 봉쇄(close 측): CLOSE append 와 META closed=true 갱신을 **동일**
    `.seq-lock` 임계구역에서 수행한다. 둘이 갈리면 그 창에서 send 가 closed=false 를
    읽고 CLOSE 이후 seq 에 exit 0 append 하는 유령 레코드가 생긴다.

    락 획득 순서는 THREADS 순서로 고정한다 — 다른 경로(send·retract)는 스레드 락을
    하나만 잡으므로 순환 대기가 성립하지 않는다."""
    os.makedirs(ticket_dir(ticket), exist_ok=True)
    locks = []
    try:
        for th in THREADS:
            lk = javis_lock.FileLock(_tp(ticket, ".seq-lock-%s" % th),
                                     owner="radio-close", blocking=True, timeout=15.0)
            if lk.acquire() != javis_lock.ACQUIRED:
                return EXIT_CONFLICT
            locks.append(lk)
        for th in THREADS:
            _r, code = _append_in_lock(ticket, th, dict(rec))
            if code != EXIT_OK:
                return code
        meta_update(ticket, lambda m: (m.__setitem__("closed", True),
                                       m.__setitem__("closed_at", _now())))
        return EXIT_OK
    except javis_lock.LockError as e:
        sys.stderr.write("close 락 획득 실패(%s)\n" % e.status)
        return EXIT_CONFLICT
    finally:
        for lk in reversed(locks):
            lk.release()


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
    구파일에 ROTATED_FROZEN 을 찍고 신규 세그먼트를 즉시 개시한다.

    반환 = **이 로테이션이 소비한 seq 수**(0=미발생 · 1=tombstone · 2=동결마커+tombstone).
    호출자는 이 값만큼 건너뛴 다음 번호를 신규 레코드에 준다 — 동일 seq 충돌로 인한
    메시지 영구 은닉(AA20 §2.3(a) 치명 결함)의 봉쇄 지점이다."""
    try:
        if not os.path.exists(path) or os.path.getsize(path) < ROTATE_BYTES:
            return 0
    except OSError:
        return 0
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
        return 2
    tomb["seq"] = fseq + 1
    tomb["msg_id"] = "%s:%s:%d" % (ticket, thread, fseq + 1)
    _jsonl_append(path, tomb)
    return 1


def verify_rotation(ticket, thread):
    """A20(b) 3중 대조 — tombstone 의 prev_final_seq·레코드 수·SHA-256. 파일명 단독
    대조는 금지(레코드 표류가 무탐지 통과한다). 반환: 불일치 사유 목록."""
    problems = []
    meta = read_meta(ticket)
    recs, _ = read_thread(ticket, thread, meta)
    # §2.3(c) 동결 폴백 — 동결 마커 **이후** 구파일에 레코드가 나타나면 URGENT 사유다.
    frozen = {r.get("seq"): r for r in recs if r.get("type") == "ROTATED_FROZEN"}
    for fseq_marker in sorted(x for x in frozen if isinstance(x, int)):
        for p in segment_paths(ticket, thread, meta):
            seg, _b = _jsonl_read(p)
            if not any(x.get("seq") == fseq_marker for x in seg):
                continue        # 이 세그먼트에는 동결 마커가 없다 — 대상 아님
            late = [x.get("seq") for x in seg
                    if isinstance(x.get("seq"), int) and x["seq"] > fseq_marker]
            if late:
                problems.append("동결 마커(seq=%d) 이후 구파일 레코드 발견: %s"
                                % (fseq_marker, sorted(late)[:5]))
    for r in recs:
        if r.get("type") != "ROTATED":
            continue
        if r.get("frozen_fallback"):
            # 폴백 경로에는 rename 이 없었다 — 아카이브 파일명 대조는 상시 오탐이 된다.
            # 대신 짝이 되는 동결 마커의 존재로 연속성을 확인한다(위 late 검사가 표류 담당).
            if (r.get("prev_final_seq") or 0) + 1 not in frozen:
                problems.append("동결 폴백 tombstone 의 짝 ROTATED_FROZEN 부재: prev=%s"
                                % r.get("prev_final_seq"))
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


def _cooldown_lock_path(ticket, sender):
    """A2-CONC-05 — 쿨다운 check→append→consume 를 원자화하는 발신자×티켓 락."""
    return _tp(ticket, ".cooldown-lock-%s" % _safe(sender))


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


# ★A2-RADIO-CONC-02 — seq 공간은 **스레드 단위**(worklog·results 각자 1부터 채번)인데
#   커서·ack·surfaced·resolve·watcher-lock 이 종전엔 **노드 단위**라 seq 값이 스레드 간
#   충돌했다: worklog 커서 5 가 results seq 1~4 를 <=5 로 취급해 표면화 0건으로 억제하고,
#   worklog resolve 가 results 동일 seq 를 덮어 미표면화 건이 done 을 false PASS 로 통과했다.
#   경로를 (노드×스레드)로 확장한다. thread 기본값은 DEFAULT_THREAD 라 종전 호출부(2-인자)는
#   worklog 스코프를 그대로 얻는다(회귀 0).
def surfaced_ledger(ticket, node, thread=DEFAULT_THREAD):
    return _tp(ticket, ".surfaced-%s-%s" % (_safe(node), _safe(thread)))


def resolve_ledger(ticket, node, thread=DEFAULT_THREAD):
    return _tp(ticket, ".resolve-%s-%s" % (_safe(node), _safe(thread)))


def cursor_path(ticket, node, thread=DEFAULT_THREAD):
    return _tp(ticket, ".cursor-%s-%s" % (_safe(node), _safe(thread)))


def ack_path(ticket, node, thread=DEFAULT_THREAD):
    return _tp(ticket, ".ack-%s-%s" % (_safe(node), _safe(thread)))


def hb_path(ticket, node):
    return _tp(ticket, ".hb-%s" % _safe(node))


def watcher_lock_path(ticket, node, thread=DEFAULT_THREAD):
    """A35(a)·A2-CONC-02 — 락 스코프는 '노드×티켓×스레드'. 한 노드가 worklog·results 를
    별도 watcher 로 **동시 감시**할 수 있어야 한다(종전 노드 단위 락은 한 노드가 한 스레드만
    감시 가능하게 만들어 반대 스레드 메시지가 영구 미표면화됐다). 겸무 워커는 참여 티켓 수만큼
    watcher 를 각자의 락으로 병행 보유한다."""
    return _tp(ticket, ".watcher-lock-%s-%s" % (_safe(node), _safe(thread)))


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
    for kind, p in (("fence", fence_ledger(ticket, node)), ("pause", pause_ledger(ticket, node))):
        recs, _ = _jsonl_read(p)
        for r in recs:
            s = r.get("seq")
            if not isinstance(s, int):
                continue
            cur = out.get(s)
            if cur is None:
                cur = {"seq": s, "grade": r.get("grade"), "from": r.get("from"),
                       "stdin_delivered": bool(r.get("stdin_delivered")),
                       "released": False, "kind": kind, "label": None, "batch": None}
                out[s] = cur
            if not r.get("stdin_delivered"):
                cur["stdin_delivered"] = False
            if r.get("released"):
                cur["released"] = True
            if r.get("label"):
                cur["label"] = r["label"]
            if r.get("batch"):
                cur["batch"] = r["batch"]
            if kind == "pause":
                cur["kind"] = "pause"       # pause 격리는 확인 레코드 요구가 붙는다
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


def surfaced_seqs(ticket, node, thread=DEFAULT_THREAD):
    recs, _ = _jsonl_read(surfaced_ledger(ticket, node, thread))
    return {r.get("seq") for r in recs if isinstance(r.get("seq"), int)}


def resolved_seqs(ticket, node, thread=DEFAULT_THREAD):
    recs, _ = _jsonl_read(resolve_ledger(ticket, node, thread))
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


def _ev_sources(rec):
    """evidence 의 (파일, 라인) 출처 집합 — 짝 증거의 '독립 재유도' 판정용."""
    return {(e.get("file"), e.get("line")) for e in (rec.get("evidence") or [])
            if e.get("file") is not None}


def find_pair_evidence(hypo_id, node, records, ticket=None):
    """§5.5 짝 증거 — 다음 7요건 전부 충족하는 레코드가 1건+ 인가.
      ①동일 티켓 스레드 ②수신자 자신이 발신 ③refs 에 해당 HYPOTHESIS msg-id 포함
      ④기계 진위 검증을 통과한 FACT(자동 강등분 제외 — demoted_from 부재)
      ⑤철회 집합에 비포함(AA28 §5.5 요건 5호)
      ⑥★AA34(b) 라인 해시 **재검증** 통과 — 저장 시점 verified 플래그만 보면 사후 파일
        변조·근거 소멸이 무탐지로 done 을 통과한다. 불일치 건은 짝 자격을 잃고 master 에
        통지된다(반환 (레코드, 사유)).
      ⑦★A1-R2 자작 재유도 차단 — 짝 FACT 의 evidence 출처(파일:라인)가 원 HYPOTHESIS 의
        evidence 출처와 **동일**하면 거부한다. '가설을 refs 로 인용하고 같은 파일:라인을
        다시 단 FACT'는 독립 재유도가 아니라 자기 인용이며, §5.5 오염 방어의 핵심(인용 전
        독립 재유도)이 형식 조건 만족으로 전락하는 경로다."""
    rset = retracted_ids(records)
    idx = _by_id(records)
    hypo = idx.get(hypo_id)
    hypo_src = _ev_sources(hypo) if hypo else set()
    stale = []
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
        # ⑦ 자작 재유도 차단 — 원 가설과 동일 출처의 evidence 는 독립 재유도가 아니다.
        pair_src = _ev_sources(r)
        if hypo_src and pair_src and pair_src <= hypo_src:
            stale.append((r.get("msg_id"), "원 HYPOTHESIS 와 동일 출처(자작 재유도 · A1-R2)"))
            continue
        ok, why = recheck_evidence(r.get("evidence"))
        if not ok:
            stale.append((r.get("msg_id"), why))
            if ticket:
                notify_master(ticket, "pair-evidence-stale",
                              "짝 증거 재검증 실패 — %s: %s (짝 자격 상실 · AA34(b))"
                              % (r.get("msg_id"), why),
                              idem="pair-stale-%s" % r.get("msg_id"))
            continue
        return r, ""
    if stale:
        return None, "재검증 실패로 짝 자격 상실: %s" % "; ".join("%s(%s)" % s for s in stale)
    return None, ""


# ── send ─────────────────────────────────────────────────────────────────────
def cmd_send(a):
    ticket, thread, node = a.ticket, a.thread, a.node
    if thread not in THREADS:
        sys.stderr.write("미지 스레드 %r — 계약상 %s 2종뿐이다\n" % (thread, list(THREADS)))
        return EXIT_USAGE
    if not os.path.isdir(ticket_dir(ticket)):
        sys.stderr.write("티켓 미개통: %s — 먼저 `javis_radio.py open` 하라\n" % ticket)
        return EXIT_USAGE

    # ★G2 신원 게이트(A1-R3·A1-R4) — 리뷰어·비참여자의 --node 회전·라이브 개입 차단.
    ok, code, msg = participant_check(ticket, node)
    if not ok:
        sys.stderr.write(msg + "\n")
        return code

    # ★차단기 먼저 — 거부될 시도도 계수해야 거부 재시도 루프가 무비용으로 폭주하지 않는다.
    ok, n = breaker_check_and_count(node)
    if not ok:
        sys.stderr.write("차단기: 분당 %d건 초과(관측 %s) — 일시 거부(exit 8)\n" % (BREAKER_MAX, n))
        return EXIT_THROTTLED

    evidence, everr = parse_evidence(a.evidence)
    if everr:
        sys.stderr.write("evidence 형식 위반: %s\n" % "; ".join(everr))
        return EXIT_USAGE

    # §3.4 BLOCKER 는 사유+evidence 필수 (W0-3 어휘 정렬: 최소 8자·exit 5)
    if a.grade == "BLOCKER":
        if len((a.reason or "").strip()) < 8:
            sys.stderr.write("blocker reason required(5): BLOCKER 는 --reason 필수 "
                             "(최소 8자 — 사유 없는 최고 배달 특권 금지)\n")
            return EXIT_NO_EVIDENCE
        if not evidence:
            sys.stderr.write("evidence required(5): BLOCKER 는 --evidence 필수 "
                             "(파일:라인:스니펫 — 미검증 주장의 stdin 직배달 차단)\n")
            record_blocker_rejection(ticket, node, a.text, evidence, "evidence 부재")
            return EXIT_NO_EVIDENCE

    # ★A2-CONC-05 TOCTOU 봉쇄: cooldown_check→append→cooldown_consume 를 발신자×티켓 쿨다운
    #   락 안에서 원자 수행한다(두 동시 발신이 consume 이전에 모두 check 를 통과해 쿨다운
    #   창을 우회하던 경합 제거 · 락 순서 cooldown→seq 로 역방향 경로 없음). NORMAL/FYI 는
    #   쿨다운이 없어 락을 잡지 않는다.
    cd_lock = (javis_lock.FileLock(_cooldown_lock_path(ticket, node),
                                   owner="radio-cd-%s" % node, blocking=True, timeout=10.0)
               if a.grade in ("BLOCKER", "URGENT") else contextlib.nullcontext())
    try:
        with cd_lock:
            return _cmd_send_locked(a, ticket, thread, node, evidence)
    except javis_lock.LockError as e:
        sys.stderr.write("쿨다운 락 획득 실패(%s) — 재시도하지 말고 상위 보고\n" % e.status)
        return EXIT_CONFLICT


def _cmd_send_locked(a, ticket, thread, node, evidence):
    """cmd_send 의 쿨다운 임계구역(check→...→consume). 쿨다운 락 보유 하에서만 호출한다."""
    grade, epistemic, text = a.grade, a.epistemic, a.text
    orig_text = a.text          # §3.10(b) 유사 판정용 — 강등 접두가 섞이지 않은 원문

    # §3.4/§3.10(a) 쿨다운 — 위반은 exit 거부 + 시계 미소모(무언 강등·큐잉 구현 금지)
    ok, left = cooldown_check(ticket, node, grade)
    if not ok:
        sys.stderr.write("쿨다운: %s 재발신까지 %.0f초 — 일시 거부(exit 8·시계 미소모)\n"
                         % (grade, left))
        if grade == "BLOCKER":
            record_blocker_rejection(ticket, node, orig_text, evidence, "쿨다운 위반")
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
                record_blocker_rejection(ticket, node, orig_text, evidence, "진위 검증 실패: %s" % vwhy)
                text = "[GRADE-DEMOTED:진위 검증 실패 — %s] %s" % (vwhy, text)
    elif epistemic == "FACT":
        # §3.2 문언: '자동 강등'은 **검증 실패** 시의 처치이고, evidence **자체 부재**는
        # exit 5 거부다. 무증거 FACT 를 강등 저장으로 받아주면 '근거 없는 사실 주장'이
        # 스레드에 상시 축적된다(§3.2 가 명시적으로 봉쇄한 경로).
        sys.stderr.write("evidence required(5): FACT 는 --evidence 필수 "
                         "(파일:라인:스니펫 — 자동 강등은 '검증 실패' 시의 처치이지 "
                         "무증거 주장의 수용 경로가 아니다 · §3.2)\n")
        if grade == "BLOCKER":
            record_blocker_rejection(ticket, node, orig_text, evidence, "FACT evidence 부재")
        return EXIT_NO_EVIDENCE

    # §3.10(b) URGENT 남용 방어 — BLOCKER 거부 후 10분 내 유사 URGENT 자동 NORMAL 강등.
    #   ★대조는 **원문 기준**이다(거부 이력에도 원문 text140 을 저장한다) — 한쪽에만
    #   [DEMOTED:...] 접두가 섞이면 같은 문장이 문자 불일치로 탐지를 빠져나간다.
    if grade == "URGENT" and not demoted_from:
        abuse, why = urgent_abuse_demotion(ticket, node, orig_text, evidence)
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
    # ★AA27: ⓪진위 강등으로 stdin 특권을 잃은 건도 **master 사본은 유지**한다(강등 사유
    #   동봉 — 남용 시도의 즉시 가시화). 종전 구현은 강등 시 leg 전체를 건너뛰어
    #   .master-copy-log·통지가 모두 사라졌고 남용 탐지 입력이 로컬 파일에만 남았다.
    if a.grade == "BLOCKER":
        _blocker_stdin_legs(ticket, node, rec, stdin_privileged=stdin_privileged,
                            demotion_reason=(None if stdin_privileged else demotion_reason))
    print(json.dumps({"sent": rec["msg_id"], "seq": rec["seq"], "grade": grade,
                      "epistemic": epistemic, "verified": verified,
                      "demoted_from": demoted_from, "masked": nmask},
                     ensure_ascii=False))
    return EXIT_OK


def _blocker_stdin_legs(ticket, sender, rec, stdin_privileged=True, demotion_reason=None):
    """§3.4 배달 규약 — 게이트 순서 고정·fail-closed: ⓪진위 ①pause ②펜스.
    두 게이트를 통과한 건만 stdin 직배달하고, 결과는 §3.11 배달 대장에 즉시 기록한다.

    ★AA27: ⓪진위 게이트에서 강등된 건(`stdin_privileged=False`)은 수신자 leg 를 수행하지
      않되 **master 사본은 강등 사유를 동봉해 유지**한다 — 최고 배달 특권 남용 시도가
      master 에게 즉시 보이게 하는 것이 이 조문의 목적이다."""
    seq = rec["seq"]
    header = "[radio %s seq=%d]" % (ticket, seq)     # A30(d) 수신측 중복·지연 식별 근거
    body = "%s %s [BLOCKER] %s" % (header, rec.get("reason") or "", rec.get("text") or "")
    if demotion_reason:
        body = "%s [GRADE-DEMOTED:%s · stdin 특권 박탈 — master 사본만 유지(AA27)] %s %s" % (
            header, demotion_reason, rec.get("reason") or "", rec.get("text") or "")
    meta = read_meta(ticket)
    recipients = [n for n in (rec.get("to") or []) if n] or \
                 [n for n in (meta.get("participants") or []) if n != sender]

    paused, hits = paused_now()
    for node in (recipients if stdin_privileged else []):
        if paused:
            # §9.1 후단 — append 는 수행했고 stdin leg 만 정지시켜 격리 대장에 편입한다.
            _jsonl_append(pause_ledger(ticket, node),
                          {"seq": seq, "grade": rec["grade"], "from": sender,
                           "stdin_delivered": False, "ts": _now(), "why": "pause: %s" % hits,
                           "label": PAUSE_RELEASE_LABEL})
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
        if ok:
            # §3.11(c) ENQUEUED→INJECTED 전이 — 관측 표면이 '큐 비었음'을 주면 주입 확정.
            #   조회 불능·미제공은 미주입(fail-closed) 유지 → 델타에 전문 동승한다.
            _probe_and_mark_injected(ticket, node, [seq])

    # (c) master 사본 — 수신자 배달·격리·강등 여부와 무관하게 즉시. 영속 로그가 단일 진실.
    _jsonl_append(_tp(ticket, ".master-copy-log"),
                  {"seq": seq, "from": sender, "grade": rec["grade"], "ts": _now(),
                   "reason": rec.get("reason"), "text": rec.get("text"),
                   "demotion_reason": demotion_reason,
                   "evidence": rec.get("evidence")})
    if paused:
        _jsonl_append(pause_ledger(ticket, "master"),
                      {"seq": seq, "grade": rec["grade"], "from": sender,
                       "stdin_delivered": False, "ts": _now(), "why": "pause(master 사본)",
                       "label": PAUSE_RELEASE_LABEL})
    else:
        ok, detail = cys_send_queued("master", body)
        _jsonl_append(delivery_ledger(ticket, "master"),
                      {"seq": seq, "leg": "master_copy", "state": "ENQUEUED" if ok else "FAILED",
                       "ts": _now(), "idem_key": "radio-%s-%d-master" % (ticket, seq),
                       "detail": detail})
        if ok:
            _probe_and_mark_injected(ticket, "master", [seq], leg="master_copy")


def _probe_and_mark_injected(ticket, node, seqs, leg="recipient"):
    """§3.11(c) — 주입 확인 성공 시에만 state=INJECTED 를 대장에 추가한다.
    실패·조회 불능은 아무것도 쓰지 않는다(ENQUEUED 유지 = 미주입 취급 = fail-closed)."""
    if not seqs:
        return
    if not probe_injected(node):
        return
    for s in seqs:
        _jsonl_append(delivery_ledger(ticket, node),
                      {"seq": s, "leg": leg, "state": "INJECTED", "ts": _now(),
                       "idem_key": "radio-%s-%d-%s" % (ticket, s, node)})


def promote_injected(ticket, node):
    """build_delta **직전** 호출 — 아직 ENQUEUED 인 leg 를 관측 표면으로 재확인해
    INJECTED 로 전이시킨다. 전이가 없으면 그 건은 델타에 전문 동승한다(소실 0회 우선)."""
    st = delivery_state(ticket, node)
    pend = [s for s, legs in st.items()
            if "INJECTED" not in legs.values() and "ENQUEUED" in legs.values()]
    if not pend:
        return
    legs_by_seq = {s: [lg for lg, v in st[s].items() if v == "ENQUEUED"] for s in pend}
    if not probe_injected(node):
        return
    for s, legs in legs_by_seq.items():
        for lg in legs:
            _jsonl_append(delivery_ledger(ticket, node),
                          {"seq": s, "leg": lg, "state": "INJECTED", "ts": _now(),
                           "idem_key": "radio-%s-%d-%s" % (ticket, s, node)})


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


def _quarantine_nonblocker(ticket, node, rec, why="fence"):
    """AA29 §6.3 — 비BLOCKER 격리 대장 기입 주체는 **수신자 watcher 단독**이다.
    (BLOCKER 는 송신 게이트가 이미 기입했다 — seq 멱등이라 중복 행이 생겨도 방류는 1회.)"""
    _jsonl_append(fence_ledger(ticket, node),
                  {"seq": rec.get("seq"), "grade": rec.get("grade"), "from": rec.get("from"),
                   "stdin_delivered": False, "ts": _now(), "why": why})


def fence_retro_notice(ticket, node, meta, records, thread=DEFAULT_THREAD):
    """AA29 §6.1 추가문 — '펜스 fence_seq 초과인데 이미 표면화된 건'은 소급 격리가 불가능
    하므로(표면화는 비가역) '펜스 후 표면화됨' 라벨로 master 에 **1회** 통지해 리뷰 오염
    판정 입력을 남긴다. 통지 이력은 사이드카로 멱등 보장한다."""
    if not _fenced(meta, node, 10 ** 12):        # 그 노드에 활성 펜스가 없으면 대상 없음
        return []
    shown = surfaced_seqs(ticket, node, thread)
    side = _tp(ticket, ".fence-retro-%s-%s" % (_safe(node), _safe(thread)))
    known = {r.get("seq") for r in _jsonl_read(side)[0]}
    fresh = []
    for r in records:
        s = r.get("seq")
        if not isinstance(s, int) or s in known or s not in shown:
            continue
        if _fenced(meta, node, s):
            fresh.append(s)
    if not fresh:
        return []
    for s in fresh:
        _jsonl_append(side, {"seq": s, "ts": _now()})
    notify_master(ticket, "fence-after-surface",
                  "'펜스 후 표면화됨' — %s 앞 seq %s (리뷰 오염 여부 판정 입력 · 소급 격리 불가)"
                  % (node, sorted(fresh)[:10]), idem="fence-retro-%s" % node)
    return fresh


def _cap_body(body, seq):
    """A5-R01 — 단일 레코드 본문이 RECORD_BODY_CAP(8KB)을 넘으면 바이트 경계로 절단하고
    '전문은 read' 지시자를 붙인다. 표면화 캡(16KB)을 단일 초대형 레코드가 통째로 우회하는
    경로 차단(errors='ignore' 로 멀티바이트 경계 절단의 깨진 꼬리를 버린다)."""
    raw = body.encode("utf-8")
    if len(raw) <= RECORD_BODY_CAP:
        return body
    head = raw[:RECORD_BODY_CAP].decode("utf-8", "ignore")
    return head + ("\n   … [초대형 레코드 절단 — 전문은 javis_radio.py read --from %s 로 열람]"
                   % seq)


def build_delta(ticket, node, thread, cursor, records, meta,
                recovery=False, pilot=True, enforce_fence=True):
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
    shown = surfaced_seqs(ticket, node, thread)      # A2-CONC-02 — 스레드별 표시 대장
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
        # ★§6.1 펜스 표면화 격리(치명 공백 수리): 종전 build_delta 는 meta 를 받고도
        #   펜스를 판정에 **전혀 쓰지 않아** 비BLOCKER 메시지가 펜스 중에도 그대로
        #   표면화됐다(freeze 오염 차단이라는 §6 의 존재 이유가 무력). 이제 표면화 결정
        #   직전 META(재독본)로 판정하고, 격리 편입을 대장에 **즉시** 기록한다(A12 §6.3).
        if q is None and enforce_fence and _fenced(meta, node, seq):
            _quarantine_nonblocker(ticket, node, rec)
            quar[seq] = {"seq": seq, "grade": rec.get("grade"), "from": rec.get("from"),
                         "stdin_delivered": False, "released": False,
                         "kind": "fence", "label": None, "batch": None}
            continue                            # 격리 — 표면화도 정착도 아니다
        if q is not None and not q.get("released"):
            # ★격리는 cycle 복원(recovery)에서도 유지한다 — 미방류분은 애초에 표면화된
            #   적이 없으므로 회수 대상이 아니며(대장이 보존한다), 회수가 격리를 우회하면
            #   §6 리뷰 격리에 구멍이 난다.
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
                body = "[%s] " % RECOVERY_LABEL + body
            qq = quar.get(rec.get("seq")) or {}
            if qq.get("label"):
                body = "[%s] " % qq["label"] + body      # §9.2 방류 라벨
            if rec.get("msg_id") in rset:
                body = "[RETRACTED] " + body
            # AA30(d): 미주입(ENQUEUED) stdin leg 는 전문을 동승시킨 **직후** 취소 요청을
            #   대장에 남긴다. 실배달이 `cys send --queued` 직발이라 트랜잭션 취소는
            #   불가능하므로, 취소 불능을 감사 가능한 기록으로 격하한다(불변식 위계:
            #   '전문 표면화 0회 금지 > 1회 초과 금지').
            if not recovery and rec.get("grade") == "BLOCKER":
                for lg, st in (deliv.get(rec.get("seq")) or {}).items():
                    if st == "ENQUEUED":
                        _jsonl_append(delivery_ledger(ticket, node),
                                      {"seq": rec.get("seq"), "leg": lg,
                                       "state": "CANCEL_REQUESTED", "ts": _now(),
                                       "detail": "전문 동승 직후 취소 요청 — 직발 큐는 "
                                                 "트랜잭션 취소 불가(헤더 식별로 폴백)"})
        # ★A5-R01 — 단일 초대형 레코드가 16KB 캡을 통째로 우회하는 경로 차단. `full_line`
        #   은 text 를 절대 절단하지 않고, 아래 캡 게이트의 `and lines` 는 첫 엔트리(=lines
        #   비어있음)를 크기 무관 통과시켜, 피어 한 명이 수백 KB 메시지 하나로 워커 컨텍스트를
        #   한 턴에 무제한 주입할 수 있었다. 레코드 본문 자체를 8KB 로 절단하고 read 지시자를
        #   붙여 '한 턴 과대 유입 방지'라는 캡의 계약 목적을 단일 입력에도 관철한다.
        body = _cap_body(body, rec.get("seq"))
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
    표면화 payload·read 출력 양쪽에 출력 **직전** 적용한다.
    반환 (scrub본, 성공여부) — A5-R05: 실패 시 호출자가 커서·surfaced 를 전진시키면 안 된다."""
    try:
        return javis_scrub.scrub(payload)[0], True
    except Exception:
        return "[scrub 불능 — 출력 보류(fail-closed)]", False


# ── wait (watcher) ───────────────────────────────────────────────────────────
def _generation():
    """§4.11 세대 토큰 — 팩 업데이트로 이 스크립트가 갈리면 값이 바뀐다. 폴마다 대조해
    불일치면 sentinel② 출력 후 자진 종료한다(구버전 코드의 TTL 8h 잔존 금지)."""
    try:
        st = os.stat(os.path.abspath(__file__))
        return "%d:%d" % (st.st_mtime_ns, st.st_size)
    except OSError:
        return "unknown"


def surfaceable_records(ticket, node, cursor, records, meta, quar=None):
    """표면화 후보 — build_delta 의 필터와 같은 판정(관리·자기발신·봉인·격리·펜스 제외).
    wake 판정(FYI digest)과 pause 보류가 같은 집합을 보게 하는 단일 정의다."""
    quar = quar if quar is not None else quarantined_seqs(ticket, node)
    sealed = sealed_seqs(records)
    out = []
    for r in records:
        s = r.get("seq")
        if not isinstance(s, int) or s <= cursor:
            continue
        if s in sealed or is_mgmt(r) or r.get("from") == node:
            continue
        q = quar.get(s)
        if q is not None and not q.get("released"):
            continue
        if q is None and _fenced(meta, node, s):
            continue
        out.append(r)
    return out


def _settled_only(node, records, cursor):
    """pause 보류 중에도 커서가 전진해도 되는 seq — 봉인·관리 레코드·자기 발신뿐이다."""
    sealed = sealed_seqs(records)
    out = set()
    for r in records:
        s = r.get("seq")
        if not isinstance(s, int) or s <= cursor:
            continue
        if s in sealed or is_mgmt(r) or r.get("from") == node:
            out.add(s)
    return out


def pause_hold(ticket, node, cursor, records, meta):
    """§9.2 — pause 중 표면화 **보류**. 관측(스레드 판독)·기록(격리 대장)·커서(정착분)·
    heartbeat 는 계속하고 표면화만 멈춘다(pause 허용 상한 = 관측·저장·보고).
    보류분은 pause 격리 대장에 편입되어 `resume-release` 로만 방류된다 — 종전 구현은
    pause 를 아예 확인하지 않아 kill-switch 중에도 델타가 워커 컨텍스트로 유입됐다."""
    quar = quarantined_seqs(ticket, node)
    held = []
    for r in surfaceable_records(ticket, node, cursor, records, meta, quar):
        s = r["seq"]
        if s in quar:
            continue                # 이미 격리(방류 대기) — 중복 편입 금지
        _jsonl_append(pause_ledger(ticket, node),
                      {"seq": s, "grade": r.get("grade"), "from": r.get("from"),
                       "stdin_delivered": False, "ts": _now(),
                       "why": "pause: 표면화 보류(§9.2)", "label": PAUSE_RELEASE_LABEL})
        held.append(s)
    return held


def fyi_hold(ticket, node, candidates):
    """§5.2(d) — FYI 단독 신규는 wake 를 발동하지 않는다(digest 코얼레싱).
    (surface_now, note). 비FYI 가 하나라도 있으면 즉시 표면화하며 FYI 는 동승한다.
    보류가 30분을 넘기면 단독 digest 1회를 내보낸다 — 무기한 미도달을 막는 상한이다."""
    hp = _tp(ticket, ".fyi-hold-%s.json" % _safe(node))
    if not candidates:
        return False, ""
    if any((r.get("grade") or "") != "FYI" for r in candidates):
        if os.path.exists(hp):
            try:
                os.unlink(hp)
            except OSError:
                pass
        return True, ""
    st = _read_json(hp, default=None) or {}
    first = st.get("first_ts")
    now = _now()
    if not isinstance(first, (int, float)):
        javis_lock.atomic_write_json(hp, {"first_ts": now,
                                          "seqs": [r.get("seq") for r in candidates]})
        return False, "FYI 단독 %d건 — 보류(digest 코얼레싱)" % len(candidates)
    if now - first < FYI_DIGEST_SEC:
        return False, "FYI 단독 %d건 — 보류 %.0fs" % (len(candidates), now - first)
    try:
        os.unlink(hp)
    except OSError:
        pass
    return True, "FYI digest(%.0f분 경과 — 단독 1회)" % (FYI_DIGEST_SEC / 60.0)


def _write_hb(ticket, node, last_surfaced_ts, generation):
    """§4.5 heartbeat 사이드카 — META 비접촉(경합 원천 제거) + A22 '최근 표면화 성공 ts'."""
    javis_lock.atomic_write_json(hb_path(ticket, node),
                                 {"ts": _now(), "pid": os.getpid(),
                                  "last_surfaced_ts": last_surfaced_ts,
                                  "generation": generation})


def _poll_signature(ticket, thread):
    """A5-R03 — watcher 폴 무변경 판정용 서명. 스레드 세그먼트(mtime/size)·META·pause 상태의
    스냅샷. 무변경이면 전 스레드 재통독·전 아카이브 재-SHA256(verify_rotation)을 건너뛴다 —
    종전엔 신규가 0건이어도 매 5초 전량 재판독+재해시가 노드×티켓마다 8시간 지속됐다."""
    parts = []
    for p in segment_paths(ticket, thread):
        try:
            st = os.stat(p)
            parts.append((os.path.basename(p), st.st_size, st.st_mtime_ns))
        except OSError:
            parts.append((os.path.basename(p), -1, -1))
    try:
        st = os.stat(meta_path(ticket))
        parts.append(("META", st.st_size, st.st_mtime_ns))
    except OSError:
        parts.append(("META", -1, -1))
    parts.append(("paused", paused_now()[0]))
    return tuple(parts)


def _notify_rotation_problems(ticket, thread, problems):
    """A5-R02 — 로테이션 연속성 불일치 통지를 master 에 **dedup** 한다(동일 서명 5분/1회).
    종전엔 verify_rotation 이 폴마다 동일 problems 를 반환하면 매 폴(5초) 멱등키 없는 긴급
    send 를 무제한 발사했다(_report_corruption 은 사이드카 dedup 하는데 로테이션만 누락된
    비대칭). 서명 사이드카로 대칭화한다."""
    if not problems:
        return
    sig = _sha256("|".join(sorted(problems)))
    side = _tp(ticket, ".rotation-notify-%s.json" % _safe(thread))
    prev = _read_json(side, default={}) or {}
    now = _now()
    if prev.get("sig") == sig and (now - (prev.get("ts") or 0)) < 300.0:
        return                              # 동일 불일치 5분 내 재통지 금지
    javis_lock.atomic_write_json(side, {"sig": sig, "ts": now})
    notify_master(ticket, "rotation-mismatch",
                  "URGENT 로테이션 연속성 불일치: %s" % "; ".join(problems[:3]),
                  urgent=True, idem="rotation-%s-%s" % (thread, sig[:12]))


def cmd_wait(a):
    ticket, node, thread = a.ticket, a.node, a.thread
    if not os.path.isdir(ticket_dir(ticket)):
        sys.stderr.write("티켓 미개통: %s\n" % ticket)
        return EXIT_USAGE

    # ★G2 신원 게이트 — 리뷰어(§8.1)·비참여자가 watcher 를 돌려 실시간 델타를 열람하는
    #   라이브 개입(A1-R4) 차단.
    ok, code, msg = participant_check(ticket, node)
    if not ok:
        sys.stderr.write(msg + "\n")
        return code

    # A36(c) 재기동 가드 — close/done 티켓에서는 아예 기동하지 않는다(좀비 루프 차단).
    meta = read_meta(ticket)
    if meta.get("closed"):
        print(SENTINEL_CLOSED)
        return EXIT_OK

    # §4.1 싱글턴 — 락 스코프는 노드×티켓×스레드(A35(a)·A2-CONC-02). 한 노드가 worklog·
    #   results 를 별도 watcher 로 동시 감시할 수 있다(노드 단위 락은 한 스레드만 감시 가능
    #   하게 만들어 반대 스레드 메시지를 영구 미표면화했다).
    lk = javis_lock.FileLock(watcher_lock_path(ticket, node, thread),
                             owner="radio-wait-%s" % node, soft=True)
    if lk.acquire() != javis_lock.ACQUIRED:
        sys.stderr.write("watcher 이미 보유 중(%s) — '정확히 1개' 불변식(§4.1)\n" % lk.status)
        return EXIT_CONFLICT
    gen0 = _generation()
    started = _now()
    last_hb = 0.0
    last_surfaced = None
    polls = 0
    last_sig = None
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

            # ★A5-R03 — 세그먼트·META·pause 무변경이면 재판독·재해시·표면화를 통째로 건너뛴다.
            sig = _poll_signature(ticket, thread)
            if sig != last_sig:
                last_sig = sig
                records, diag = read_thread(ticket, thread)
                _report_corruption(ticket, thread, diag)
                _notify_rotation_problems(ticket, thread, verify_rotation(ticket, thread))
                # ★AA29 §6.1 — META 는 '스레드 읽기 후·표면화 결정 직전'에 재독한다.
                meta = read_meta(ticket)
                fence_retro_notice(ticket, node, meta, records, thread)
                cursor = _read_cursor(cursor_path(ticket, node, thread))
                fseq = final_seq(records)
                # ★§9.1 '표면화 전' pause 게이트 — 배달 leg 뿐 아니라 watcher 도 pause 를 존중.
                paused, _hits = paused_now()

                if meta.get("closed"):
                    # §10.2 — '미표면화 델타 선표면화 → sentinel → 종료' 고정.
                    if paused:
                        pause_hold(ticket, node, cursor, records, meta)
                        sys.stderr.write("pause 중 close — 미표면화분은 pause 격리 대장으로 "
                                         "보류했다(방류는 resume-release · AA37(ii))\n")
                    else:
                        payload, disp, settled, _h = build_delta(ticket, node, thread, cursor,
                                                                 records, meta)
                        if payload and _emit(ticket, node, payload, disp, thread):
                            cursor = advance_cursor(cursor, records, settled)
                            _write_cursor(cursor_path(ticket, node, thread), cursor)
                    print(SENTINEL_CLOSED)
                    return EXIT_OK

                if paused:
                    pause_hold(ticket, node, cursor, records, meta)
                    new_cur = advance_cursor(cursor, records,
                                             _settled_only(node, records, cursor))
                    if new_cur != cursor:
                        _write_cursor(cursor_path(ticket, node, thread), new_cur)
                elif fseq > cursor:     # §4.2 wake 판정 = 단조 seq 비교 하나(카운트 기반 금지)
                    promote_injected(ticket, node)      # §3.11(c) ENQUEUED→INJECTED 재확인
                    cand = surfaceable_records(ticket, node, cursor, records, meta)
                    go, note = fyi_hold(ticket, node, cand)
                    if go or not cand:
                        payload, disp, settled, _h = build_delta(ticket, node, thread, cursor,
                                                                 records, meta)
                        emitted = True
                        if payload:
                            if note:
                                payload = "[%s]\n%s" % (note, payload)
                            emitted = _emit(ticket, node, payload, disp, thread)
                            if emitted:
                                last_surfaced = _now()
                        # ★A5-R05 — _emit 이 fail-closed(scrub 불능)면 커서를 전진시키지
                        #   않는다(전송 실패 콘텐츠가 '표시됨'으로 회계돼 스텁 축소되는 경로 봉쇄).
                        if emitted:
                            new_cur = advance_cursor(cursor, records, settled)
                            if new_cur != cursor:
                                _write_cursor(cursor_path(ticket, node, thread), new_cur)

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


def _emit(ticket, node, payload, displayed, thread=DEFAULT_THREAD):
    """표면화 — 출력 직전 2차 scrub 후 기표시 대장에 등재한다(중복 배제의 근거).
    반환 True=표면화 성공(호출자가 커서·surfaced 전진). ★A5-R05: _second_scrub 이 fail-closed
    플레이스홀더를 반환하면(실제 내용 미출력) surfaced 등재·커서 전진을 **하지 않는다** —
    종전엔 전송 실패 콘텐츠가 '표시됨'으로 회계돼 다음 wake 부터 스텁으로 영구 축소됐다."""
    scrubbed, ok = _second_scrub(payload)
    print(scrubbed)
    if not ok:
        return False                    # 다음 폴에 재시도 — 커서·surfaced 미기록
    for s in sorted(x for x in displayed if isinstance(x, int)):
        _jsonl_append(surfaced_ledger(ticket, node, thread), {"seq": s, "ts": _now()})
    return True


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
    # §4.10 — --limit 에 **상한**을 건다. 상한이 없으면 `--limit 999999` 로 단일 호출
    #   무캡 출력이 가능해, 캡의 구제 경로가 캡 목적을 무효화하는 바로 그 경로가 열린다.
    limit = max(1, min(int(a.limit or READ_LIMIT_DEFAULT), READ_LIMIT_MAX))
    capped = (a.limit or READ_LIMIT_DEFAULT) > READ_LIMIT_MAX
    sel = [r for r in records if isinstance(r.get("seq"), int) and r["seq"] > start]
    page, rest = sel[:limit], sel[limit:]
    out = []
    if capped:
        out.append("[--limit 상한 %d 적용 — 초과분은 페이지네이션으로만 열람한다(§4.10)]"
                   % READ_LIMIT_MAX)
    rset = retracted_ids(records)
    for r in page:
        line = full_line(r) if not is_mgmt(r) else "· [관리] seq=%s type=%s" % (r.get("seq"), r.get("type"))
        if r.get("msg_id") in rset:
            line = "[RETRACTED] " + line
        out.append(line)
    if rest:
        out.append("")
        out.append("[다음 페이지: --from %s · 잔여 %d건]" % (page[-1].get("seq"), len(rest)))
    print(_second_scrub("\n".join(out) if out else "(신규 없음)")[0])
    return EXIT_OK


# ── ack / resolve (§4.7(b) · §5.6) ───────────────────────────────────────────
def cmd_ack(a):
    ok, code, msg = participant_check(a.ticket, a.node)
    if not ok:
        sys.stderr.write(msg + "\n")
        return code
    # ★A2-CONC-03 — 수용 커서에 단조 가드. 동시/역순 ack(예: ack 10 후 ack 3)가 커서를
    #   후퇴시켜 수용분이 재배달되고 done-check 의 seq>ack 판정이 흔들리던 경로 차단.
    #   read-modify-write 를 ack 락으로 원자화한다(스코프는 노드×스레드 · A2-CONC-02).
    p = ack_path(a.ticket, a.node, a.thread)
    lp = _tp(a.ticket, ".ack-lock-%s-%s" % (_safe(a.node), _safe(a.thread)))
    try:
        with javis_lock.FileLock(lp, owner="radio-ack-%s" % a.node, blocking=True, timeout=10.0):
            newv = max(_read_cursor(p), a.seq)
            _write_cursor(p, newv)
    except javis_lock.LockError as e:
        sys.stderr.write("ack 락 획득 실패(%s)\n" % e.status)
        return EXIT_CONFLICT
    print(json.dumps({"ack": newv, "node": a.node}, ensure_ascii=False))
    return EXIT_OK


def cmd_resolve(a):
    """§5.6 — BLOCKER·URGENT 의 반영·기각을 기계 레코드로 남긴다. 트랜스크립트·자유형
    todo 문자열은 게이트 판정 입력으로 **불인정**(자유형 파싱 오거부와 자기보고 신뢰
    통과의 양극단을 동시에 배제)."""
    ok, code, msg = participant_check(a.ticket, a.node)
    if not ok:
        sys.stderr.write(msg + "\n")
        return code
    if len((a.note or "").strip()) < 8:
        sys.stderr.write("resolve 근거 부족(5): --note 최소 8자\n")
        return EXIT_NO_EVIDENCE
    _jsonl_append(resolve_ledger(a.ticket, a.node, a.thread),
                  {"seq": a.seq, "action": a.action, "note": javis_scrub.scrub(a.note)[0],
                   "ts": _now(), "node": a.node, "thread": a.thread})
    print(json.dumps({"resolved": a.seq, "action": a.action}, ensure_ascii=False))
    return EXIT_OK


# ── retract (§7.4 · §7.5 · §7.2) ─────────────────────────────────────────────
def cmd_retract(a):
    ticket, thread, node = a.ticket, a.thread, a.node
    ok, code, msg = participant_check(ticket, node)
    if not ok:
        sys.stderr.write(msg + "\n")
        return code
    records, _ = read_thread(ticket, thread)
    idx = _by_id(records)
    # ★A1-R5 — 철회 식별자 정규화. _by_id 는 msg_id 와 str(seq)를 모두 인덱싱하므로 seq 형태
    #   ('1')로도 대상을 찾을 수 있는데, done-check 폐쇄·read [RETRACTED] 표기는 msg_id 형태
    #   ('T:worklog:1')를 쓴다. 두 형태가 교집합을 못 이뤄 seq 로 철회한 레코드는 done 을 계속
    #   통과하고 [RETRACTED] 로도 안 찍혔다. 대상을 **정규 msg_id 로 정규화**해 저장한다.
    trec = idx.get(a.target)
    if trec is None:
        sys.stderr.write("철회 대상 미발견: %s\n" % a.target)
        return EXIT_USAGE
    target = trec.get("msg_id") or a.target
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
    idx = _by_id(records)
    # ★AA28 §7.4(b) 확장 — 철회 대상이 어떤 HYPOTHESIS 의 **짝 증거**로 쓰였다면, 그
    #   가설에 의존해 done 을 통과한 티켓도 재검토 대상이다. 이 의존은 역방향이라
    #   (티켓 refs → 가설 → …) forward closure 스캔으로는 절대 잡히지 않는다.
    via_pair = set()
    tgt_rec = idx.get(target)
    if tgt_rec and tgt_rec.get("epistemic") == "FACT":
        for rid in tgt_rec.get("refs") or []:
            r2 = idx.get(rid)
            if r2 and r2.get("epistemic") == "HYPOTHESIS":
                via_pair.add(rid)
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
        if target in closure or (closure & via_pair):
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
    A25(c): 확인 레코드 부재로 인한 대기는 **워커 귀책이 아님**을 사유에 기계 판별해 남긴다.

    ★G4/A2-CONC-01 — **THREADS 전수**(worklog·results)를 순회한다. 종전엔 단일 --thread 만
      읽어(훅은 --thread 미전달 → worklog 고정), results 스레드로 자기 앞에 온 BLOCKER/URGENT
      가 게이트 시야 밖으로 무조건 통과했다(status 의 unresolved 산출과 정면 모순).
    ★A2-CONC-02 — 커서·ack·resolve 를 **스레드별**로 판독한다(worklog 커서·resolve 가 results
      동일 seq 를 덮어 미표면화 건이 false PASS 로 통과하던 교차 오염 차단)."""
    ticket, node = a.ticket, a.node
    ok, code, msg = participant_check(ticket, node)
    if not ok:
        sys.stderr.write(msg + "\n")
        return code
    quar = quarantined_seqs(ticket, node)
    reasons = []

    all_records = []
    for th in THREADS:
        recs_th, _ = read_thread(ticket, th)
        all_records.extend(recs_th)
        cursor = _read_cursor(cursor_path(ticket, node, th))
        ack = _read_cursor(ack_path(ticket, node, th))
        resolved = resolved_seqs(ticket, node, th)
        for r in recs_th:
            if is_mgmt(r) or r.get("from") == node:
                continue
            # AA32(a) — 미지(전방호환) 레코드에는 반영 의무를 부과하지 않는다(미래 schema_version
            #   + BLOCKER 하나로 done 을 영구 봉쇄하던 경로 차단).
            if is_unknown(r):
                continue
            if r.get("grade") not in ("BLOCKER", "URGENT"):
                continue
            to = r.get("to") or []
            if to and node not in to:
                continue
            seq = r["seq"]
            q = quar.get(seq)
            if q is not None and not confirm_covers(ticket, node, seq, q):
                reasons.append("pause 방류 확인 대기 [%s] seq=%d(batch=%s) — master 확인 레코드 "
                               "부재로 인한 대기 · 워커 귀책 아님(A25(c))" % (th, seq, q.get("batch")))
            elif seq > cursor or seq > ack:
                reasons.append("미표면화/미수용 [%s] seq=%d(커서=%d ack=%d)" % (th, seq, cursor, ack))
            elif seq not in resolved:
                reasons.append("resolve 레코드 부재 [%s] seq=%d — 반영·기각 기록 필요(§5.6)"
                               % (th, seq))

    records = all_records
    rset = retracted_ids(records)
    refs = [x for x in (a.refs or "").split(",") if x.strip()]
    closure = transitive_refs(refs, records)
    hit = closure & rset
    if hit:
        reasons.append("철회된 근거 인용(이행적 폐쇄 교집합): %s" % ", ".join(sorted(hit)))
    idx = _by_id(records)
    # ★A19 §5.5: 짝 증거를 요구하는 범위는 '산출물이 refs 인용한' **직접 집합**이다.
    #   이행적 폐쇄 전체에 요구하면 타인의 짝 증거 FACT 가 인용한 간접 가설에까지
    #   수신자 본인의 독립 재유도를 요구해, A19 가 봉쇄하려던 오거부 경로가 재개방된다.
    #   (철회 대조는 폐쇄 전체 유지 — 다단 체인 오통과 차단이 목적이라 층위가 다르다.)
    for rid in refs:
        rec = idx.get(rid)
        if rec and rec.get("epistemic") == "HYPOTHESIS":
            pair, why = find_pair_evidence(rid, node, records, ticket=ticket)
            if not pair:
                reasons.append("HYPOTHESIS %s 의 짝 증거 0건(§5.5) — 독립 재유도 필요%s"
                               % (rid, (" · " + why) if why else ""))

    if reasons:
        for r in reasons:
            print("[DONE-BLOCK] %s" % r)
        print("복구 경로: 미표면화 건은 `javis_radio.py wait --ticket %s --node %s --once` "
              "또는 `javis_radio.py read --ticket %s --from <seq>` 로 **강제 표면화**한 뒤 "
              "ack·resolve 를 기록하고 재시도하라(AA38 ①)." % (ticket, node, ticket))
        print("done 게이트: BLOCK — exit %d" % EXIT_NO_EVIDENCE)
        return EXIT_NO_EVIDENCE
    print("done 게이트: PASS (미표면화 0 · resolve 완비 · 철회 인용 0 · 짝 증거 완비)")
    return EXIT_OK


# ── 펜스 설정·해제 (§6.1 · §6.2 · §8.2 master 옵저버 권한) ────────────────────
def cmd_fence(a):
    """§6.1 — master 가 리뷰 격리를 건다. 종전에는 펜스 **판독**만 있고 기입 도구가 없어
    master 가 META 를 수기 편집해야 했다(A11 락 규율 우회 위험)."""
    ticket, target = a.ticket, a.target
    if not os.path.isdir(ticket_dir(ticket)):
        sys.stderr.write("티켓 미개통: %s\n" % ticket)
        return EXIT_USAGE
    # ★A4-R02 — pause(kill-switch) 중에는 펜스 상태 변형을 금지한다(exit 4). pause 허용 범위는
    #   관측·저장·보고·자기종료 넷뿐이며 살아있는 타 노드의 격리 상태 변경은 포함되지 않는다.
    paused, hits = paused_now()
    if paused:
        sys.stderr.write("pause 중 fence 금지(exit 4·fail-closed) — 살아있는 타 노드의 격리 "
                         "상태 변경은 pause 허용 범위 밖이다(%s)\n" % hits)
        return EXIT_PAUSED
    fseq = a.seq
    if fseq is None:
        fseq = max(final_seq(read_thread(ticket, th)[0]) for th in THREADS)
    meta = meta_update(ticket, lambda m: m.setdefault("fences", {}).__setitem__(
        target, {"fence_seq": int(fseq), "at": _now(), "by": a.node,
                 "reason": a.reason or ""}))
    print(json.dumps({"fenced": target, "fence_seq": int(fseq),
                      "fences": meta.get("fences")}, ensure_ascii=False))
    return EXIT_OK


def release_quarantine(ticket, node, kind=None, label=None, batch=None, by="release"):
    """§6.2·AA29 — 격리 대장 잔존분을 **seq 멱등**으로 방류 기입한다(중복 행이 물리적으로
    있어도 방류 기입은 seq 당 1회). 반환: 방류된 seq 목록."""
    ledgers = []
    if kind in (None, "fence"):
        ledgers.append(("fence", fence_ledger(ticket, node)))
    if kind in (None, "pause"):
        ledgers.append(("pause", pause_ledger(ticket, node)))
    out = []
    for _k, ledger in ledgers:
        rows, _ = _jsonl_read(ledger)
        state = {}
        for r in rows:
            s = r.get("seq")
            if not isinstance(s, int):
                continue
            cur = state.setdefault(s, dict(r))
            if r.get("released"):
                cur["released"] = True
        for s, r in sorted(state.items()):
            if r.get("released"):
                continue
            row = dict(r, seq=s, released=True, released_at=_now(), released_by=by)
            if label:
                row["label"] = label
            if batch:
                row["batch"] = batch
            _jsonl_append(ledger, row)
            out.append(s)
    return out


def cmd_unfence(a):
    """§6.2 — verdict 도착 → 펜스 해제 → 격리분 일괄 방류(다음 wake 델타에 전문 동승).
    종전에는 released 기입 주체가 close 뿐이라 리뷰 종료 시점의 방류가 불가능했다."""
    ticket, target = a.ticket, a.target
    if not os.path.isdir(ticket_dir(ticket)):
        sys.stderr.write("티켓 미개통: %s\n" % ticket)
        return EXIT_USAGE
    # ★A4-R02 — pause 중 격리 방류 금지(exit 4). unfence 는 격리분을 워커 stdin/델타로
    #   방류하는 상태 변형이므로 pause 허용 범위 밖이다(resume-release 와 동일 규율).
    paused, hits = paused_now()
    if paused:
        sys.stderr.write("pause 중 unfence 금지(exit 4·fail-closed) — 격리 방류는 pause 허용 "
                         "범위(관측·저장·보고·자기종료) 밖이다(%s)\n" % hits)
        return EXIT_PAUSED
    meta_update(ticket, lambda m: (m.setdefault("fences", {}).pop(target, None), m))
    freed = release_quarantine(ticket, target, kind="fence", by="unfence")
    print(json.dumps({"unfenced": target, "released": freed,
                      "note": "방류 건의 §5.2 유예는 **방류 표면화 시점**부터 기산한다(A16 §6.2)"},
                     ensure_ascii=False))
    return EXIT_OK


# ── pause 방류·확인 (§9.2 · §9.3 · AA25 · AA31(e)) ───────────────────────────
def batch_ledger(ticket, node):
    return _tp(ticket, ".pause-release-batch-%s" % _safe(node))


def confirm_path(ticket, node):
    return _tp(ticket, ".pause-release-confirm-%s" % _safe(node))


def read_confirms(ticket, node):
    d = _read_json(confirm_path(ticket, node), default=None) or {}
    out = d.get("confirmations")
    return out if isinstance(out, list) else []


def confirm_covers(ticket, node, seq, q):
    """A25(a) — 확인은 **기계 판독 레코드로만** 성립한다. pause 방류 건만 확인 대상이며
    (펜스 방류는 §6.2 경로라 master 확인 요건이 없다).

    ★A4-R04 — BLOCKER·URGENT 는 `--seqs` 로 **개별 확인**해야 한다(계약 §5). batch 범위
      확인만으로는 불충분하다. 종전 confirm_covers 는 grade 를 보지 않고 batch 일치만으로
      True 를 반환해, B/U 가 개별 확인 없이 batch 일괄 확인만으로 done 게이트를 통과했다.
      N/F 만 batch 확인으로 갈음한다."""
    if (q or {}).get("kind") != "pause":
        return True
    is_bu = (q or {}).get("grade") in ("BLOCKER", "URGENT")
    batch = (q or {}).get("batch")
    for c in read_confirms(ticket, node):
        if seq in (c.get("seqs") or []):
            return True                         # 개별 seq 확인은 등급 불문 충분
        if not is_bu and c.get("scope") == "batch" and batch and c.get("batch") == batch:
            return True                         # N/F 만 batch 확인으로 갈음
    return False


def open_batches(ticket, node):
    """미확인 방류 요약(batch) 목록 — A25(c)·A31(e) 재통지 대상 판정 입력."""
    rows, _ = _jsonl_read(batch_ledger(ticket, node))
    confirmed = {c.get("batch") for c in read_confirms(ticket, node)
                 if c.get("scope") == "batch"}
    return [r for r in rows if r.get("batch") not in confirmed]


def cmd_resume_release(a):
    """§9.2·AA24(c) — resume 후 pause 격리분 **일괄 방류**. 각 건에 '착수 전 master 확인'
    라벨을 붙이고, 노드당 방류 요약 1건(batch)을 만들어 확인 폭풍을 막는다(A12 §9.3).
    종전에는 released 기입 주체가 close 뿐이라 resume 후에도 close 까지 표면화가 잠겼다."""
    ticket = a.ticket
    if not os.path.isdir(ticket_dir(ticket)):
        sys.stderr.write("티켓 미개통: %s\n" % ticket)
        return EXIT_USAGE
    paused, hits = paused_now()
    if paused and not a.force:
        sys.stderr.write("아직 pause 중이다(%s) — 방류는 resume 후에만(exit 4·fail-closed)\n"
                         % hits)
        return EXIT_PAUSED
    meta = read_meta(ticket)
    nodes = [a.node] if a.node else (list(meta.get("participants") or []) + ["master"])
    out = []
    for node in nodes:
        batch = "B-%s-%s-%d" % (_safe(ticket), _safe(node), int(_now()))
        freed = release_quarantine(ticket, node, kind="pause",
                                   label=PAUSE_RELEASE_LABEL, batch=batch, by="resume-release")
        if not freed:
            continue
        quar = quarantined_seqs(ticket, node)
        grades = {}
        bu = []
        for s in freed:
            g = (quar.get(s) or {}).get("grade") or "NORMAL"
            grades[g] = grades.get(g, 0) + 1
            if g in ("BLOCKER", "URGENT"):
                bu.append(s)
        rec = {"batch": batch, "node": node, "ts": _now(), "count": len(freed),
               "grades": grades, "bu": sorted(bu), "seqs": sorted(freed),
               "label": PAUSE_RELEASE_LABEL}
        _jsonl_append(batch_ledger(ticket, node), rec)
        notify_master(ticket, "pause-release",
                      "%s 방류 요약 %s — %d건(등급 %s · B/U %s) · 확인 필요: "
                      "`javis_radio.py confirm-release --ticket %s --node %s --batch %s`"
                      % (node, batch, len(freed), grades, sorted(bu), ticket, node, batch),
                      urgent=bool(bu), idem="pause-release-%s" % batch)
        out.append(rec)
    print(json.dumps({"released_batches": out,
                      "note": "확인 레코드 기록 전까지 §5.2 유예 시계는 기산되지 않는다(A25(b))"},
                     ensure_ascii=False))
    return EXIT_OK


def cmd_confirm_release(a):
    """AA25(a) — master 확인 레코드 기록(temp-write-rename · A11 패턴).
    구두·트랜스크립트상의 확인은 무효다 — 이 파일만이 게이트 판정 입력이다."""
    ticket, node = a.ticket, a.node
    seqs = sorted({int(x) for x in (a.seqs or "").replace(" ", "").split(",") if x})
    if not a.batch and not seqs:
        sys.stderr.write("확인 범위 필요: --batch <id> 또는 --seqs 1,2,3\n")
        return EXIT_USAGE
    entry = {"batch": a.batch, "ts": _now(), "by": a.by or "master",
             "scope": "batch" if a.batch and not seqs else "seqs", "seqs": seqs,
             "note": a.note or ""}
    # ★A2-CONC-04 — read-append-write 를 확인 락으로 원자화한다. atomic_write_json 자체는
    #   원자적이나 read-modify-write 구간에 락이 없어 동시 확인이 서로를 덮어 소실됐다
    #   (확인 레코드는 done 게이트·pause 유예 기산의 유일 입력이라 소실 = done 영구 봉쇄).
    lp = _tp(ticket, ".confirm-lock-%s" % _safe(node))
    try:
        with javis_lock.FileLock(lp, owner="radio-confirm-%s" % node, blocking=True, timeout=10.0):
            cur = read_confirms(ticket, node)
            cur.append(entry)
            javis_lock.atomic_write_json(confirm_path(ticket, node), {"confirmations": cur})
    except javis_lock.LockError as e:
        sys.stderr.write("confirm-release 락 획득 실패(%s)\n" % e.status)
        return EXIT_CONFLICT
    print(json.dumps({"confirmed": entry, "node": node}, ensure_ascii=False))
    return EXIT_OK


# ── cycle 복원 회수 (§10.3③ · AA22 · AA39(d)) ────────────────────────────────
def _task_status(ticket):
    t = _read_json(os.path.join(ROOT(), "_round", "tasks", "%s.json" % ticket), default=None)
    return (t or {}).get("status")


def cmd_recover(a):
    """§10.3③ — `.ack-<노드>` 기준 **미수용** 델타 회수. 표면화 커서만으로 회수하는 구현은
    금지다(표면화≠통합 — clear 로 트랜스크립트와 함께 소실된 건은 표면화 커서상 '완료'다).
    회수 델타는 압축·'기배달'·'기표시' 종결 규칙을 적용하지 않고 전 건 전문 재표기한다 —
    '전문 표면화 정확히 1회' 회계가 clear 시점에 그 노드에 한해 리셋되기 때문(A22).

    AA39(d) 가드: done/취소/close 티켓은 회수를 스킵하고 고아 상태를 master 에 보고한다
    (죽은 티켓의 스테일 URGENT 가 신선 컨텍스트에 되살아나는 경로 봉쇄)."""
    ticket, node = a.ticket, a.node
    if not os.path.isdir(ticket_dir(ticket)):
        sys.stderr.write("티켓 미개통: %s\n" % ticket)
        return EXIT_USAGE
    ok, code, msg = participant_check(ticket, node)
    if not ok:
        sys.stderr.write(msg + "\n")
        return code
    meta = read_meta(ticket)
    st = _task_status(ticket)
    if meta.get("closed") or st in ("done", "DONE", "cancelled", "canceled"):
        notify_master(ticket, "recover-dead-ticket",
                      "%s 의 회수 요청 — 티켓이 이미 close/done/취소(status=%s) 이므로 스킵"
                      % (node, st), idem="recover-dead-%s" % ticket)
        print(json.dumps({"skipped": ticket, "reason": "closed/done/cancelled",
                          "task_status": st, "closed": bool(meta.get("closed"))},
                         ensure_ascii=False))
        return EXIT_OK
    total = 0
    last_ack = None
    for th in ([a.thread] if a.thread else list(THREADS)):
        ack = _read_cursor(ack_path(ticket, node, th))    # A2-CONC-02 — 스레드별 ack
        last_ack = ack
        records, _ = read_thread(ticket, th, meta)
        payload, disp, _settled, _h = build_delta(ticket, node, th, ack, records, meta,
                                                  recovery=True)
        if payload:
            print("── [%s] thread=%s (ack=%d 이후 미수용분)" % (RECOVERY_LABEL, th, ack))
            _emit(ticket, node, payload, disp, th)
            total += len(disp)
    if not total:
        print("(회수 대상 없음 — ack=%s 이후 미수용 델타 0건)" % last_ack)
    return EXIT_OK


# ── status (§4.5 · §8.3 · AA31(d)(e) master 점검·복원 절차의 도구 표면) ───────
def cmd_status(a):
    """master 능동 점검·복원 절차(§8.3)의 기계 판독 표면. 사람 눈이 아니라 JSON 이다.
      · heartbeat 신선도(HB_STALE_SEC 180초) — 종전에는 상수만 있고 판독 주체가 없었다
      · 배선 단절형 난청 신호(표면화 성공 ts 정체 + 구독 스레드 신규 seq)  — A22 §4.5
      · 펜스·격리 잔존(미방류)·미확인 batch — AA31(d) ①②
      · 미인지 BLOCKER(master 사본 중 표면화 대장에 없는 건) — AA31(d) ③
      · 확인 재통지 due(B/U 10분 · N/F-only 30분) — AA31(e)"""
    now = _now()
    try:
        tickets = [a.ticket] if a.ticket else sorted(
            n for n in os.listdir(radio_dir())
            if os.path.isdir(os.path.join(radio_dir(), n)) and not n.startswith("_"))
    except OSError:
        tickets = []
    out = {"ts": now, "hb_stale_sec": HB_STALE_SEC, "tickets": []}
    for t in tickets:
        meta = read_meta(t)
        finals = {}
        for th in THREADS:
            finals[th] = final_seq(read_thread(t, th, meta)[0])
        info = {"ticket": t, "closed": bool(meta.get("closed")),
                "participants": list(meta.get("participants") or []),
                "fences": meta.get("fences") or {}, "final_seq": finals,
                "task_status": _task_status(t), "nodes": [],
                "notify_fallback": len(_jsonl_read(_tp(t, ".notify-fallback.jsonl"))[0])}
        master_copies = {r.get("seq") for r in _jsonl_read(_tp(t, ".master-copy-log"))[0]
                         if isinstance(r.get("seq"), int)}
        for node in list(meta.get("participants") or []) + ["master"]:
            hb = _read_json(hb_path(t, node), default=None) or {}
            hb_ts = hb.get("ts")
            age = (now - hb_ts) if isinstance(hb_ts, (int, float)) else None
            # A2-CONC-02 — 커서·ack 는 스레드별이다(표시는 worklog 대표값). backlog·deaf 는
            #   스레드별로 판정한다(worklog 커서가 results 신규 seq 를 가리지 않도록).
            cursor = _read_cursor(cursor_path(t, node))          # worklog 대표(표시용)
            ack = _read_cursor(ack_path(t, node))
            quar = quarantined_seqs(t, node)
            shown = surfaced_seqs(t, node)
            lst = hb.get("last_surfaced_ts")
            # A22 — '표면화 성공 ts 정체 + 신규 seq 존재' 조합이 배선 단절형 난청 신호다.
            backlog = any(finals.get(th, 0) > _read_cursor(cursor_path(t, node, th))
                          for th in THREADS)
            deaf = bool(backlog and (lst is None or (now - lst) > HB_STALE_SEC))
            batches = []
            for b in open_batches(t, node):
                period = CONFIRM_RENOTIFY_BU_SEC if b.get("bu") else CONFIRM_RENOTIFY_NF_SEC
                batches.append({"batch": b.get("batch"), "ts": b.get("ts"),
                                "count": b.get("count"), "bu": b.get("bu") or [],
                                "renotify_period_sec": period,
                                "renotify_due": (now - (b.get("ts") or now)) >= period})
            nd = {"node": node,
                  "hb": {"ts": hb_ts, "age_sec": age,
                         "stale": (age is None or age > HB_STALE_SEC),
                         "last_surfaced_ts": lst, "generation": hb.get("generation")},
                  "cursor": cursor, "ack": ack,
                  "deaf_signal": deaf,
                  "fence_unreleased": sorted(s for s, q in quar.items()
                                             if q.get("kind") == "fence" and not q.get("released")),
                  "pause_unreleased": sorted(s for s, q in quar.items()
                                             if q.get("kind") == "pause" and not q.get("released")),
                  "released_unconfirmed": sorted(
                      s for s, q in quar.items()
                      if q.get("released") and not confirm_covers(t, node, s, q)),
                  "open_batches": batches,
                  "unresolved_bu": sorted(_unresolved_bu(t, node))}
            if node == "master":
                nd["unseen_blocker_copies"] = sorted(master_copies - shown)
            info["nodes"].append(nd)
        out["tickets"].append(info)
    print(json.dumps(out, ensure_ascii=False, indent=1))
    return EXIT_OK


def _unresolved_bu(ticket, node):
    """자기 앞 BLOCKER·URGENT 중 resolve 레코드가 없는 seq(§5.2(e) 판정 입력과 동형).
    ★A2-CONC-02 — resolve 는 스레드별로 판독한다(worklog resolve 가 results 동일 seq 를
    덮어 미resolve 건을 놓치던 교차 오염 차단)."""
    out = set()
    for th in THREADS:
        resolved = resolved_seqs(ticket, node, th)
        for r in read_thread(ticket, th)[0]:
            if is_mgmt(r) or is_unknown(r) or r.get("from") == node:
                continue
            if r.get("grade") not in ("BLOCKER", "URGENT"):
                continue
            to = r.get("to") or []
            if to and node not in to:
                continue
            if r.get("seq") not in resolved:
                out.add(r.get("seq"))
    return out


# ── open (§1.2 · AA33(a)) ────────────────────────────────────────────────────
def capability_receipt(node):
    return os.path.join(radio_dir(), ".capability-%s.json" % _safe(node))


def _capability_sig(node, ok, ts, generation):
    """A1-R7 — 능력 영수증 무결성 서명(내용 해시 · 암호 비밀 아님). 직접 {"ok":true} 위조를
    막는 선까지 난이도를 올린다(하드락 회피): 위조하려면 코드를 읽어 해시를 재현해야 하고,
    코드가 있으면 그냥 --self-test 를 돌리는 게 빠르다(radio 불능 노드의 위조 등록 차단)."""
    return _sha256("|".join(["cap", str(node), "1" if ok else "0",
                             repr(ts), str(generation), _CAP_SALT]))


def write_capability_receipt(node, ok=True, generation=None, lock_backend=None):
    """능력 영수증을 무결성 서명과 함께 기록한다(A1-R7). self-test·복원·테스트 공용."""
    gen = generation if generation is not None else _generation()
    ts = _now()
    rec = {"ok": bool(ok), "node": node, "ts": ts, "generation": gen,
           "lock_backend": lock_backend or javis_lock.backend_name(),
           "sig": _capability_sig(node, bool(ok), ts, gen)}
    javis_lock.atomic_write_json(capability_receipt(node), rec)
    return rec


def capability_ok(node):
    """AA33(a)·A1-R7 — 영수증이 ok=True 이고 **무결성 서명이 일치**하며 node 가 자기 이름일
    때만 참. 서명 없는/위조된 평문 {"ok":true} 는 거부된다(fail-closed)."""
    rec = _read_json(capability_receipt(node), default=None)
    if not rec or rec.get("ok") is not True:
        return False
    if rec.get("node") != node:
        return False
    want = _capability_sig(node, True, rec.get("ts"), rec.get("generation"))
    return rec.get("sig") == want


def cmd_open(a):
    ticket = a.ticket
    parts = [p.strip() for p in (a.participants or "").split(",") if p.strip()]
    if not parts:
        sys.stderr.write("참여자 필요: --participants a,b,c\n")
        return EXIT_USAGE
    # §8.1 리뷰어는 워커가 아니다 — reviewer-* 접두 **및 어댑터 키(gemini·codex)**를 거부
    #   (exit 6). 종전엔 접두만 봐서 'gemini'/'codex' 별칭으로 참여자 등록이 가능했다(A1-R4).
    bad = [p for p in parts if is_reviewer(p)]
    if bad:
        sys.stderr.write("리뷰어 등록 거부(exit 6): %s — 리뷰어는 master 전용 검증·반박 "
                         "노드이지 radio 피어가 아니다\n" % ", ".join(bad))
        return EXIT_REVIEWER
    # AA33(a)·A1-R7 능력 게이트 — 참여자별 --self-test exit 0 영수증(무결성 서명 검증)이 있어야
    #   등록된다. 서명 없는 위조 {"ok":true} 는 통과하지 못한다.
    missing = [p for p in parts if not capability_ok(p)]
    if missing and not a.skip_capability_gate:
        sys.stderr.write(
            "능력 게이트 실패(exit 3): %s — 각 노드에서 "
            "`javis_radio.py --self-test --record-capability --node <노드>` 를 먼저 통과시켜라 "
            "(radio 불능 노드의 난청 등록·재기동 무한 루프를 개통 시점에 차단)\n" % ", ".join(missing))
        notify_master(ticket, "capability-gate", "능력 게이트 미통과 노드: %s" % ", ".join(missing))
        return EXIT_CAPABILITY
    if missing and a.skip_capability_gate:
        # 우회는 막지 않되 숨기지 않는다(close --force 와 동형 규율).
        if len((a.skip_reason or "").strip()) < 8:
            sys.stderr.write("skip reason required(5): --skip-capability-gate 는 "
                             "--skip-reason 필수(최소 8자) — fail-closed 약화는 기록 없이 "
                             "허용되지 않는다\n")
            return EXIT_NO_EVIDENCE
        _jsonl_append(_tp(ticket, ".gate-bypass.jsonl"),
                      {"ts": _now(), "ticket": ticket, "gate": "capability",
                       "nodes": missing, "reason": a.skip_reason.strip()})
        notify_master(ticket, "capability-gate-bypass",
                      "능력 게이트 우회 — 노드 %s · 사유: %s"
                      % (", ".join(missing), a.skip_reason.strip()), urgent=True)

    # §1.2 — radio 는 '책임 영역이 얽힌 다중 워커 병렬 티켓'에서만 쓴다. 단일 워커 티켓의
    #   개통은 순비용(watcher·유예·게이트)만 남기므로 개통 시점에 고지한다.
    if len(parts) < 2:
        sys.stderr.write("주의(§1.2): 참여자 %d명 — radio 는 다중 워커 병렬 티켓 전용이다. "
                         "단일 워커 티켓 개통은 순비용이며 계약상 미사용 대상이다.\n" % len(parts))

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
    # ★A4-R02 pause 게이트 — close 는 CLOSE 레코드로 타 노드 watcher 를 SENTINEL_CLOSED(재기동
    #   금지)로 강제 종료시킨다. 살아있는 타 노드의 종료는 pause 허용 범위(관측·저장·보고·
    #   자기종료) 밖이므로 pause 중에는 exit 4 로 거부한다(resume 후 재시도).
    paused, hits = paused_now()
    if paused:
        sys.stderr.write("pause 중 close 금지(exit 4·fail-closed) — close 는 타 노드 watcher 를 "
                         "강제 종료시킨다. resume 후 재시도하라(%s)\n" % hits)
        return EXIT_PAUSED
    # ★AA37 게이트 우회의 **비은닉화**: --force 는 '전문 표면화 0회 금지' 불변식의 기계
    #   집행을 무력화하는 행위다. 제거하지는 않되(고아 티켓 최후 수단) 사유를 강제하고
    #   우회 사실을 원장·master 통지로 가시화한다.
    if a.force and len((a.force_reason or "").strip()) < 8:
        sys.stderr.write("force reason required(5): --force 는 --force-reason 필수(최소 8자) — "
                         "드레인·잔존0 게이트 우회는 기록 없이 허용되지 않는다\n")
        return EXIT_NO_EVIDENCE
    parts = list(meta.get("participants") or [])
    report = {"released": 0, "cancel_requested": 0, "drain_short": [], "residual": []}

    # ── ★A4-R01: 게이트 판정을 **상태 변형 전에** 계산한다. 종전 구현은 (i) 펜스를 무조건
    #   {} 로 지우고 (ii) 격리 대장을 released:true 로 방류한 **뒤** 게이트를 평가해, 게이트가
    #   거부돼도 펜스·pause 격리가 이미 비가역 파괴됐다(펜스 걸린 티켓은 커서가 target 에
    #   영원히 못 미쳐 close 첫 시도에서 반드시 거부되므로 상시 경로였다). 거부 경로에서는
    #   어떤 상태도 건드리지 않는다(원자 롤백 = 애초에 변형 전 판정).
    # (iv) 드레인 게이트 — 전 참여 워커의 표면화 커서 == 스레드 최종 비관리 seq(스레드별).
    for th in THREADS:
        records, _ = read_thread(ticket, th)
        nonmgmt = [r.get("seq") for r in records
                   if isinstance(r.get("seq"), int) and not is_mgmt(r)]
        target = max(nonmgmt or [0])
        for node in parts:
            cur = _read_cursor(cursor_path(ticket, node, th))
            if cur < target:
                report["drain_short"].append("%s/%s cursor=%d < %d" % (node, th, cur, target))

    # (v) 잔존 0 게이트 — 미방류 격리 + FAILED 미표면화. release **전**에 판정하므로 '아직
    #     방류되지 않은 격리'가 곧 residual 이다 — master 가 unfence/resume-release 로 먼저
    #     방류·표면화해야 close 가 통과한다(펜스·pause 격리를 close 가 몰래 방류하지 않는다).
    master_copies = {r.get("seq") for r in _jsonl_read(_tp(ticket, ".master-copy-log"))[0]
                     if isinstance(r.get("seq"), int)}
    for node in parts + ["master"]:
        for s, r in sorted(quarantined_seqs(ticket, node).items()):
            if not r.get("released"):
                report["residual"].append("%s 격리 미방류 seq=%d" % (node, s))
        # A30(b): FAILED 는 stdin 배달이 없었던 건이므로 델타 전문 표기가 곧 복구 경로다.
        # 따라서 '미처리 FAILED' 는 아직 그 노드에 표면화되지 않은 건만을 뜻한다.
        shown = surfaced_seqs(ticket, node)
        for s, legs in delivery_state(ticket, node).items():
            for leg, st in legs.items():
                if st != "FAILED" or s in shown:
                    continue
                if leg == "master_copy" and s in master_copies:
                    continue
                report["residual"].append("%s %s seq=%d FAILED 미표면화" % (node, leg, s))

    if report["drain_short"] or report["residual"]:
        if not a.force:
            for x in report["drain_short"] + report["residual"]:
                print("[CLOSE-BLOCK] %s" % x)
            print("close 거부 — 드레인·잔존0 게이트 미통과(exit %d · 펜스·격리 무손상). 미표면화분은 "
                  "해당 워커의 `wait --once`/`read` 로 강제 표면화하고, 격리분은 `unfence`/"
                  "`resume-release` 로 방류한 뒤 재시도하라." % EXIT_NO_EVIDENCE)
            return EXIT_NO_EVIDENCE
        bypass = {"ts": _now(), "ticket": ticket, "by": a.node,
                  "reason": a.force_reason.strip(),
                  "drain_short": report["drain_short"], "residual": report["residual"]}
        _jsonl_append(_tp(ticket, ".gate-bypass.jsonl"), bypass)
        report["forced"] = bypass
        notify_master(ticket, "close-force-bypass",
                      "close 게이트 우회(--force) — 사유: %s · 드레인 미달 %d · 잔존 %d"
                      % (bypass["reason"], len(report["drain_short"]), len(report["residual"])),
                      urgent=True, idem="close-force-%s" % ticket)

    # ── ★게이트 통과(또는 --force) 이후에만 상태를 변형한다(A4-R01 거부 경로는 여기 미도달).
    # (i)(ii) 펜스 강제 해제 + 격리 대장 잔존분 방류 — verdict 종류·취소 여부와 무관.
    meta_update(ticket, lambda m: m.__setitem__("fences", {}))
    for node in parts + ["master"]:
        report["released"] += len(release_quarantine(ticket, node, by="close"))
    # (iii) stdin leg 정리 — 실배달은 `cys send --queued` 직발이라 트랜잭션 취소가 없다.
    #   취소 요청을 배달 대장에 남기고 수신측이 `[radio <티켓> seq=N]` 헤더로 '[closed-ticket
    #   지연 배달]'을 식별해 무시하게 한다(RADIO_CONTRACT §8.4).
    for node in parts + ["master"]:
        for s, legs in sorted(delivery_state(ticket, node).items()):
            for leg, stt in legs.items():
                if stt in ("ENQUEUED",):
                    _jsonl_append(delivery_ledger(ticket, node),
                                  {"seq": s, "leg": leg, "state": "CANCEL_REQUESTED",
                                   "ts": _now(), "detail": "close 정리 — 직발 큐 취소 불가 · "
                                                           "지연 배달은 헤더로 식별해 무시"})
                    report["cancel_requested"] += 1

    rec = {"schema_version": SCHEMA_VERSION, "type": "CLOSE", "grade": "FYI",
           "from": a.node, "ts": _now(), "report": report}
    # ★AA37 TOCTOU: CLOSE append 와 META closed=true 를 동일 임계구역에서 커밋한다.
    code = close_commit(ticket, rec)
    if code != EXIT_OK:
        return code
    print(json.dumps({"closed": ticket, **report}, ensure_ascii=False))
    return EXIT_OK


# ── GAP 봉인 (§2.7(c)) ───────────────────────────────────────────────────────
def cmd_seal_gap(a):
    if not a.confirm:
        sys.stderr.write("GAP 봉인은 master 승인 필요 — --confirm 없이는 거부(정상 데이터 봉인 차단)\n")
        return EXIT_USAGE
    # ★A3-R-02 범위 상한 — 종전엔 to-from 무검증이라 `--from 0 --to 10**12` 한 번(오타 포함)
    #   으로 sealed_seqs 가 10^12 원소 range 를 materialize 해 티켓을 영구 브릭·전 노드 watcher
    #   를 uninterruptible hang 시켰다. 봉인 범위는 [1, 스레드 final_seq] 안이어야 하고
    #   폭은 GAP_MAX_SPAN 이하여야 한다(정상 데이터·거대 범위 봉인 차단).
    if a.to_seq < a.from_seq or a.from_seq < 1:
        sys.stderr.write("GAP 범위 위반: from=%s to=%s (1 <= from <= to 필요)\n"
                         % (a.from_seq, a.to_seq))
        return EXIT_USAGE
    fs = final_seq(read_thread(a.ticket, a.thread)[0])
    span = a.to_seq - a.from_seq + 1
    if a.to_seq > fs or span > GAP_MAX_SPAN:
        sys.stderr.write("GAP 범위 상한 초과: to=%d(스레드 final_seq=%d) span=%d(상한 %d) — "
                         "존재하지 않는 seq·거대 범위 봉인 거부\n"
                         % (a.to_seq, fs, span, GAP_MAX_SPAN))
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

        # ② open: 능력 게이트가 fail-closed 인가 (A1-R7 무결성 서명 영수증)
        if run(["open", "--ticket", T, "--participants", "w1,w2"]) != EXIT_CAPABILITY:
            fails.append("open 능력 게이트가 fail-closed 가 아니다")
        # 무결성 서명 영수증 발급 후 통과(위조 {"ok":true} 는 통과하지 못한다 · A1-R7)
        for n in ("w1", "w2"):
            write_capability_receipt(n)
        if run(["open", "--ticket", T, "--participants", "w1,w2"]) != EXIT_OK:
            fails.append("open 실패")
        # ★A1-R7 — 서명 없는 위조 영수증은 능력 게이트를 통과하지 못한다
        javis_lock.atomic_write_json(capability_receipt("forge"), {"ok": True})
        if run(["open", "--ticket", "T-forge", "--participants", "forge"]) != EXIT_CAPABILITY:
            fails.append("위조 영수증({\"ok\":true})이 능력 게이트를 통과했다(A1-R7)")
        # ③ 리뷰어 거부 exit 6
        write_capability_receipt("reviewer-codex")
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

        # ⑧ 차단기 — 분당 12건 초과는 exit 8. burst 는 별도 티켓의 참여자로 등록한다(신원
        #    게이트 강화로 비참여자 send 는 거부되며, T-st 참여자에 넣으면 BLOCKER broadcast
        #    수신자가 되어 close 드레인이 막힌다 — 시험 격리).
        write_capability_receipt("burst")
        run(["open", "--ticket", "T-brk", "--participants", "burst,w1"])
        for _ in range(BREAKER_MAX + 2):
            rc = run(["send", "--ticket", "T-brk", "--node", "burst", "--grade", "FYI",
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

        # ⑭ close — **정상 경로**(드레인·잔존0 게이트 통과)로 닫는다. --force 는 게이트
        #    우회이므로 자기검증의 기본 경로가 되어서는 안 된다.
        for n in ("w1", "w2"):
            run(["wait", "--ticket", T, "--node", n, "--once", "--interval", "0"])
        rc = run(["close", "--ticket", T, "--node", "master"])
        if rc != EXIT_OK:
            fails.append("정상 경로 close 실패 rc=%s (드레인·잔존0 게이트)" % rc)
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

        # ⑱ ★로테이션 소실 회귀(AA20 §2.3(a) 최상위 불변식) — tombstone 이 seq 를
        #    소비하지 않으면 신규 메시지와 seq 가 겹쳐 read dedup 이 메시지를 영구 은닉한다.
        T2 = "T-rot"
        for n in ("w1", "w2", "rotw"):
            write_capability_receipt(n)      # rotw = 로테이션 시험 발신자(참여자 등록 필요)
        run(["open", "--ticket", T2, "--participants", "w1,w2,rotw"])
        _bak_rot = globals()["ROTATE_BYTES"]
        globals()["ROTATE_BYTES"] = 400
        try:
            # ★발신자는 신규 노드로 — 차단기(분당 12건·발신자 전역)가 앞선 케이스의 발신에
            #   섞이면 '소실'이 아니라 '거부'를 소실로 오판한다.
            for i in range(10):
                if run(["send", "--ticket", T2, "--node", "rotw", "--grade", "NORMAL",
                        "--epistemic", "HYPOTHESIS", "--confidence", "low",
                        "--text", "로테이션 본문 %d" % i]) != EXIT_OK:
                    fails.append("로테이션 시험 send %d 실패(차단기·쿨다운 오염)" % i)
        finally:
            globals()["ROTATE_BYTES"] = _bak_rot
        rrecs, _ = read_thread(T2, DEFAULT_THREAD)
        texts = [r.get("text") for r in rrecs if r.get("type") == MSG_TYPE]
        missing = [i for i in range(10) if ("로테이션 본문 %d" % i) not in texts]
        if missing:
            fails.append("로테이션 소실: 메시지 %s 가 은닉됐다(tombstone seq 충돌)" % missing)
        rseqs = [r.get("seq") for r in rrecs]
        if rseqs != list(range(1, len(rseqs) + 1)):
            fails.append("로테이션 관통 seq 연속 위반: %s" % rseqs[:20])

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
        # A1-R7 — 무결성 서명 포함 영수증. 실패(ok:false)도 기록한다(게이트가 '미실행'과
        #   '실행 후 실패'를 구분해 재기동 무한 루프를 진단할 수 있게).
        write_capability_receipt(record_capability, ok=(not fails))

    if fails:
        for f in fails:
            sys.stderr.write("FAIL %s\n" % f)
        return 1
    print("javis_radio self-test OK — 19 케이스(명명 대조·능력 게이트·리뷰어 거부·FACT 강등·"
          "verified 저장·BLOCKER evidence/사유 하한·쿨다운 시계 미소모·차단기·seq 연속·"
          "반줄 복구·wait 커서·ack/resolve·retract 폐쇄·close 정상경로 시퀀스·exit 7/RETRACT "
          "예외·pause·전방호환·로테이션 무손실) · lock=%s" % javis_lock.backend_name())
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
                   help="능력 게이트 우회(--skip-reason 필수 · .gate-bypass.jsonl 원장 기록 "
                        "+ master 통지 — 상시 사용 금지)")
    p.add_argument("--skip-reason", dest="skip_reason", default="",
                   help="--skip-capability-gate 동반 필수 사유(최소 8자)")

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
                   help="드레인·잔존0 게이트 미달을 감수하고 닫는다(--force-reason 필수 · "
                        ".gate-bypass.jsonl 원장 기록 + master 통지)")
    p.add_argument("--force-reason", dest="force_reason", default="",
                   help="--force 동반 필수 사유(최소 8자) — 우회의 비은닉화")

    p = sub.add_parser("fence", help="리뷰 격리 설정(master · §6.1)")
    p.add_argument("--ticket", required=True)
    p.add_argument("--node", default="master", help="집행 주체(기록용)")
    p.add_argument("--target", required=True, help="펜스 대상 노드")
    p.add_argument("--seq", type=int, default=None,
                   help="fence_seq(이 seq 초과분을 격리) · 기본=현재 최종 seq")
    p.add_argument("--reason", default="")

    p = sub.add_parser("unfence", help="verdict 후 펜스 해제 + 격리분 일괄 방류(§6.2)")
    p.add_argument("--ticket", required=True)
    p.add_argument("--node", default="master")
    p.add_argument("--target", required=True, help="펜스 해제 대상 노드")

    p = sub.add_parser("resume-release",
                       help="pause 격리분 일괄 방류 + 방류 요약(batch) 생성(§9.2·§9.3)")
    p.add_argument("--ticket", required=True)
    p.add_argument("--node", default=None, help="미지정이면 전 참여자 + master")
    p.add_argument("--force", action="store_true",
                   help="pause 지속 중에도 방류(기본은 exit 4 거부 · fail-closed)")

    p = sub.add_parser("confirm-release",
                       help="master 확인 레코드 기록(AA25(a) — 구두 확인은 무효)")
    p.add_argument("--ticket", required=True)
    p.add_argument("--node", required=True, help="확인 대상 노드")
    p.add_argument("--batch", default="", help="방류 요약 id(batch 단위 확인)")
    p.add_argument("--seqs", default="", help="개별 확인 seq 목록(쉼표 구분 · B/U 용)")
    p.add_argument("--by", default="master")
    p.add_argument("--note", default="")

    p = sub.add_parser("recover", help="clear/cycle 복원 — ack 기준 미수용 델타 회수(§10.3③)")
    p.add_argument("--ticket", required=True)
    p.add_argument("--node", required=True)
    p.add_argument("--thread", default=None, choices=list(THREADS),
                   help="미지정이면 2종 전수")

    p = sub.add_parser("status", help="master 점검·복원 절차의 기계 판독 상태(JSON · §8.3)")
    p.add_argument("--ticket", default=None, help="미지정이면 활성 티켓 전수")

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
    # ★G3 경로 순회 차단(A3-R04) — ticket 을 가진 모든 명령의 단일 초크 포인트. ticket 이
    #   ../ 나 / 를 담으면 ticket_dir 이 radio 밖에 파일을 생성하므로 진입 전에 거부한다
    #   (status --ticket 미지정=전수 스캔은 예외 · gc 는 ticket 인자 없음).
    tk = getattr(a, "ticket", None)
    if tk:
        ok, why = valid_ticket(tk)
        if not ok:
            sys.stderr.write(why + "\n")
            return EXIT_USAGE
    try:
        return {
            "open": cmd_open, "send": cmd_send, "wait": cmd_wait, "read": cmd_read,
            "ack": cmd_ack, "resolve": cmd_resolve, "retract": cmd_retract,
            "done-check": cmd_done_check, "close": cmd_close, "seal-gap": cmd_seal_gap,
            "gc": cmd_gc, "fence": cmd_fence, "unfence": cmd_unfence,
            "resume-release": cmd_resume_release, "confirm-release": cmd_confirm_release,
            "recover": cmd_recover, "status": cmd_status,
        }[a.cmd](a)
    except RadioConflict as e:
        sys.stderr.write("충돌: %s\n" % e)
        return EXIT_CONFLICT
    except BrokenPipeError:
        return EXIT_OK


if __name__ == "__main__":
    sys.exit(main())
