#!/usr/bin/env python3
"""javis_wakeup.py — P0-2 wakeup 큐: 코얼레싱 + 멱등키 + zombie 가드 (클린룸 파일 포트)

계약(출처: _research/Paperclip_박사급_연구보고서.md §4 P0-2):
- enqueue가 몰려도 같은 (target, task_key)의 pending은 1건으로 병합(coalesced_count 증가,
  reason은 최신으로 갱신, payload는 얕은 병합 — Paperclip mergeCoalescedContextSnapshot의 축소판).
- idempotency_key가 같은 요청은 중복 삽입하지 않는다(suppressed로 기록).
- zombie 가드: 배달(drain) 시 대상 생존을 확인하고, 죽은 대상에는 배달하지 않고 skipped 처리
  ("죽은 런에 병합하면 불멸화" 함정의 배달 측 방어).
- 원장은 append-only: 모든 사건(queued/coalesced/suppressed/delivered/skipped/digest)을
  queue.jsonl에 기록.

★T-0147-2 층1 I6 (설계 `_round/T-0147-2-DESIGN-wakeup-demotion.md` §2 층1) 가산:
- `enqueue --severity {info,warn,critical}`(기본 warn). 코얼레싱 시 severity 는 **상승만** 한다.
- `drain` digest: 같은 target 의 살아남은 pending 을 **1건으로 접어 cys send 를 1회만** 호출한다
  (종전: pending 1건당 1회 → master stdin 잠식의 직접 원인). 단건이면 종전 포맷 그대로(무회귀).
  digest 본문에는 각 항목의 W-id·severity·task_key·근거를 **줄 단위로 보존**한다(뭉개기 금지).

상태 저장: $JAVIS_ROOT/_round/wakeups/pending/<safe(target)>__<safe(task_key)>.json
원장:      $JAVIS_ROOT/_round/wakeups/queue.jsonl (append-only)
enqueue 직렬화: pending 파일별 read-modify-write를 .lock 디렉터리(mkdir 원자성)로 보호.

대상 생존 판정(zombie 가드) 우선순위:
  1) 테스트/강제 주입: JAVIS_WAKEUP_LIVENESS=alive|dead
  2) `cys list` **비종료 행의 role 열**을 공유 술어(javis_boot_node.requirement_satisfied)로 해소
     — exited=true 행의 role 토큰에 속아 죽은 대상에 배달하던 결함(재감사 G27) 수리.
     (cys 없으면 unknown→배달 보류 아닌 경고 배달)
배달은 기본 드라이런(cys send --queued 명령을 출력만). --deliver 시 실제 실행.

exit codes: 0 ok · 2 usage · **4 paused(kill-switch 로 배달 거부)** · 5 nothing-to-do

★운영계약 §9-7 항목 11(v0.4):
- `drain --deliver` 는 배달 **전에** `AUTOPILOT_PAUSED` 2경로를 확인하고, 어느 하나라도
  존재하면 배달하지 않고 exit 4 로 거부한다(fail-closed · 확인 불능도 거부).
- `cancel` 은 pending 을 실제로 취소·삭제한다(종전에는 취소 인터페이스 자체가 없었다).
"""
import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import time
import uuid

# ★번들 파이썬(Windows embeddable · python312._pth) 경로 가드 — 형제 모듈 import 보장.
#   ._pth 는 표준 경로 계산을 우회해 **스크립트 폴더를 sys.path 에 넣지 않는다**
#   (2026-07-29 Windows 0.14.4 실측: `ModuleNotFoundError: No module named 'javis_scrub'`).
#   unix/mac 은 스크립트 폴더가 이미 sys.path[0] 이라 이 블록은 무동작(멱등).
#   ★append 인 이유는 MAJ#1 과 동일 — **발견이 목적이지 기존 항목의 precedence 를 강등하지 않는다**
#   (bin/ 을 stdlib 앞에 놓지 않아 미래의 이름충돌 shadowing 을 원천 차단).
#   선례(append 형태): javis_report.py:33-34.
#   (hooks/inject_gate.py:22 는 insert(0) + CYS_PACK_DIR 기반 경로 — 형태가 다르므로 선례 아님)
_SELF_DIR = os.path.dirname(os.path.abspath(__file__))
if _SELF_DIR not in sys.path:
    sys.path.append(_SELF_DIR)

import javis_scrub  # ★G2: 원장 기록 직전 비밀 마스킹(같은 폴더 형제 모듈 — 부재 시 즉시 실패=fail-closed)

ROOT = os.environ.get("JAVIS_ROOT") or os.getcwd()  # 개인경로 하드코딩 금지(pack scan gate) — env 또는 CWD(워크스페이스 루트에서 호출)
WK_DIR = os.path.join(ROOT, "_round", "wakeups")
PENDING_DIR = os.path.join(WK_DIR, "pending")
LEDGER = os.path.join(WK_DIR, "queue.jsonl")

EXIT_OK, EXIT_USAGE, EXIT_EMPTY = 0, 2, 5
# ★운영계약 §9-7 항목 11(a) · §9-4 · §9-1 조건③ — kill-switch 배달 차단.
#   정지 중 `drain --deliver` 가 자율 루프를 wake 로 재점화시키는 경로를 닫는다.
#   exit 4 는 `cys gate-check` 의 paused 코드와 같은 의미로 맞춘다(호출부 분기 일관성).
EXIT_PAUSED = 4
PAUSED_BASENAME = "AUTOPILOT_PAUSED"
# 팩 경로 env 키 목록·순서는 Rust 정본 `src/pack.rs::PACK_DIR_ENV_KEYS` 와 동일하다
# (javis_orchestra·javis_report·javis_bootstrap 과 같은 목록 — 한 곳만 고치면 계약이 깨진다).
PACK_DIR_ENV_KEYS = ("CYS_PACK_DIR", "JAVIS_PACK_DIR", "AITERM_PACK_DIR", "AITERM_JARVIS_DIR")


def pack_dir():
    for key in PACK_DIR_ENV_KEYS:
        v = os.environ.get(key, "")
        if v:
            return v
    return os.path.join(os.path.expanduser("~"), ".cys", "pack")


def paused_paths():
    """kill-switch 파일 2경로(§9-4 2중 기록 규약과 동일 판정식).

    set 은 둘 다 기록하고, 확인은 **어느 하나라도 존재하면 정지**다(fail-closed).
    """
    return [os.path.join(pack_dir(), PAUSED_BASENAME),
            os.path.join(ROOT, "_round", PAUSED_BASENAME)]


def paused_evidence():
    """(paused: bool, evidence: list[str]) — 확인 불능도 거부(정지와 동일 취급).

    측정 불능을 '정지 아님'으로 읽으면 kill-switch 가 측정 실패 한 번으로
    무력화된다 — 이 계약에서 측정 불능은 통과가 아니다.
    """
    hits = []
    for p in paused_paths():
        try:
            if os.path.exists(p):
                hits.append(p)
        except OSError as e:
            hits.append("%s (확인 불능: %s)" % (p, e))
    return bool(hits), hits

# ★T-0147-2 층1 I6 — severity 등급. 순서가 곧 계약이다(코얼레싱은 **상승만**).
_SEV_DEFAULT = "warn"
_SEV_RANK = {"info": 0, "warn": 1, "critical": 2}
# digest 항목 근거의 표시 상한. 요약(뭉개기)이 아니라 **절단 + 절단 표시**다 — 설계 I6 는
# "task·key·근거 1줄씩 병기"를 계약으로 못 박았으므로 근거를 없애는 축약은 금지다.
_DIGEST_REASON_MAX = 300


def _now():
    return time.strftime("%Y-%m-%dT%H:%M:%S%z")


def _safe(s):
    return re.sub(r"[^A-Za-z0-9._-]", "_", s)[:80]


def _pending_path(target, task_key):
    return os.path.join(PENDING_DIR, f"{_safe(target)}__{_safe(task_key)}.json")


def _ledger_append(event):
    os.makedirs(WK_DIR, exist_ok=True)
    event["ts"] = _now()
    # ★G2(cokacdir 성찰 2026-07-04): 원장(queue.jsonl) 기록 직전 비밀 마스킹 — 값 단위 재귀.
    event = javis_scrub.scrub_obj(event)
    with open(LEDGER, "a", encoding="utf-8") as f:
        f.write(json.dumps(event, ensure_ascii=False) + "\n")


def _write_json_atomic(path, obj):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    tmp = f"{path}.tmp.{os.getpid()}.{uuid.uuid4().hex[:8]}"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(obj, f, ensure_ascii=False, indent=1)
        f.flush()
        os.fsync(f.fileno())
    os.replace(tmp, path)


class _FileLock:
    """mkdir 원자성 기반 락. stale(30초+)은 rename으로 원자적 회수."""

    def __init__(self, path, timeout=5.0, stale_sec=30.0):
        self.path, self.timeout, self.stale_sec = path, timeout, stale_sec

    def __enter__(self):
        deadline = time.time() + self.timeout
        os.makedirs(os.path.dirname(self.path), exist_ok=True)
        while True:
            try:
                os.mkdir(self.path)
                return self
            except FileExistsError:
                try:
                    if time.time() - os.stat(self.path).st_mtime > self.stale_sec:
                        os.rename(self.path, f"{self.path}.stale.{time.time_ns()}")
                        continue
                except OSError:
                    pass
                if time.time() > deadline:
                    raise TimeoutError(f"lock timeout: {self.path}")
                time.sleep(0.02)

    def __exit__(self, *exc):
        try:
            os.rmdir(self.path)
        except OSError:
            pass


def live_target_rows(list_output):
    """★순수 판정(G27): `cys list` 출력 → **비종료** 행의 role 목록.

    종전 가드는 `cys list` **출력 전문**에 target 문자열이 있으면 alive 로 봤다 — 그래서
    `exited=true` 행의 role 토큰(죽은 노드가 남긴 litter 행)에 속아 **죽은 대상에 배달**했다
    (재감사 G27). 행 단위로 파싱해 exited 행을 배제하고, role 열만 후보로 삼는다.
    파싱 형식은 javis_boot_node.cys_list_rows 와 동형(TAB 구분 key=value 열).
    """
    roles = []
    for ln in (list_output or "").splitlines():
        cols = ln.split("\t")
        if not cols or not cols[0].strip().startswith("surface:"):
            continue
        role, exited = None, None
        for c in cols[1:]:
            if "=" not in c:
                continue
            k, v = c.split("=", 1)
            k, v = k.strip(), v.strip()
            if k == "role":
                role = v
            elif k == "exited":
                exited = (v == "true")
        if role and exited is not True:
            roles.append(role)
    return roles


def _target_matches(target, live_roles):
    """대상 role 이 라이브 좌석으로 해소되는가 — **공유 술어 소비**(A1 클래스).

    javis_boot_node.role_matches_requirement 가 단일 정의다(정확일치 + worker 접두만).
    소비 불가(부서 팩 결손) 시에만 동형 폴백 — 조용한 접힘 금지(사유는 호출부가 남긴다).
    """
    try:
        import javis_boot_node as _bn
        return _bn.requirement_satisfied(target, set(live_roles))
    except Exception:
        return any(target == r or (target == "worker" and r.startswith("worker"))
                   for r in live_roles)


def _target_alive(target):
    """zombie 가드. 반환 'alive'|'dead'|'unknown'."""
    override = os.environ.get("JAVIS_WAKEUP_LIVENESS")
    if override in ("alive", "dead"):
        return override
    cys = shutil.which("cys")
    if not cys:
        return "unknown"
    try:
        out = subprocess.run([cys, "list"], capture_output=True, text=True, timeout=10).stdout
        # ★G27: exited 행 배제 + 공유 술어(정확일치·worker 접두)로 해소. 종전의 전문 정규식
        #   경계 매칭은 '부분일치'는 막았지만 '죽은 행'은 못 막았다 — 두 결함은 다른 축이다.
        rows = live_target_rows(out)
        # ★★파서 드리프트 fail-safe(자가치유 전멸 차단): `cys list` 포맷이 바뀌어 파싱이 0행을
        #   내면 **전 대상이 'dead'** 가 되어 wakeup 배달이 전부 skip 된다 — 주기 자가치유·자율
        #   착수 wake 가 조용히 전멸한다(가장 발견하기 어려운 종류의 사고다). 출력은 있는데 행을
        #   못 뽑았다면 그것은 '대상이 죽었다'가 아니라 **'우리가 못 읽었다'** = unknown 이고,
        #   unknown 은 배달 보류가 아니라 경고 배달이다(계약: 모듈 docstring).
        if not rows and (out or "").strip():
            sys.stderr.write("[wakeup] cys list 파싱 0행(포맷 드리프트?) — liveness=unknown 으로 "
                             "강등해 배달은 계속한다(자가치유 전멸 방지)\n")
            return "unknown"
        return "alive" if _target_matches(target, rows) else "dead"
    except (subprocess.SubprocessError, OSError):
        return "unknown"


def _load_pending(path):
    try:
        with open(path, encoding="utf-8") as f:
            return json.load(f)
    except (FileNotFoundError, json.JSONDecodeError):
        return None


def _severity_of(rec):
    """pending 레코드의 severity — **구버전 레코드(키 부재)는 warn** 으로 읽는다.

    ADR-2 스큐 안전: 팩이 부분 갱신돼 구버전 enqueue 가 만든 pending 이 신버전 drain 을
    만나도 종전과 같은 등급으로 처리돼야 한다(모르는 값도 warn 으로 접어 KeyError 를 만들지
    않는다 — 배달 경로에서 예외를 올리면 그 wakeup 은 영영 배달되지 않는다).
    """
    sev = (rec or {}).get("severity")
    return sev if sev in _SEV_RANK else _SEV_DEFAULT


def _max_severity(a, b):
    """★T-0147-2 I6 — severity 는 **상승만** 한다(절대 하강 금지).

    코얼레싱은 "같은 (target, task_key) 를 1건으로 접는" 연산인데, 나중 도착한 warn 이 앞선
    critical 을 덮으면 **치명 신호가 병합에 먹혀 영영 안 보인다**. 병합은 정보를 줄여도 되지만
    등급을 낮춰서는 안 된다.
    """
    return a if _SEV_RANK[a] >= _SEV_RANK[b] else b


def cmd_enqueue(a):
    payload = json.loads(a.payload) if a.payload else {}
    path = _pending_path(a.to, a.task)
    lock = _FileLock(path + ".lock")
    with lock:
        cur = _load_pending(path)
        if cur:
            # 멱등키: 같은 키의 요청은 중복 없이 억제
            if a.idempotency_key and a.idempotency_key in cur.get("idempotency_keys", []):
                _ledger_append({"event": "suppressed", "target": a.to, "task_key": a.task,
                                "idempotency_key": a.idempotency_key, "wakeup_id": cur["id"]})
                print(json.dumps({"result": "suppressed", "id": cur["id"]}, ensure_ascii=False))
                return EXIT_OK
            # 코얼레싱: 최신 reason으로 갱신, payload 얕은 병합, count 증가
            cur["coalesced_count"] = cur.get("coalesced_count", 0) + 1
            cur["reason"] = a.reason
            cur["payload"] = {**cur.get("payload", {}), **payload}
            # ★T-0147-2 I6 — severity 는 최대치로 **상승만**(`_max_severity` 주석 참조).
            cur["severity"] = _max_severity(_severity_of(cur), a.severity)
            if a.idempotency_key:
                cur.setdefault("idempotency_keys", []).append(a.idempotency_key)
            cur["updated_at"] = _now()
            _write_json_atomic(path, cur)
            _ledger_append({"event": "coalesced", "target": a.to, "task_key": a.task,
                            "wakeup_id": cur["id"], "coalesced_count": cur["coalesced_count"],
                            "severity": cur["severity"]})
            print(json.dumps({"result": "coalesced", "id": cur["id"],
                              "coalesced_count": cur["coalesced_count"],
                              "severity": cur["severity"]}, ensure_ascii=False))
            return EXIT_OK
        rec = {
            "id": f"W-{uuid.uuid4().hex[:10]}",
            "target": a.to,
            "task_key": a.task,
            "reason": a.reason,
            "payload": payload,
            "severity": a.severity,   # ★T-0147-2 I6 (기본 warn — 구 호출부는 무변경으로 종전 등급)
            "idempotency_keys": [a.idempotency_key] if a.idempotency_key else [],
            "coalesced_count": 0,
            "queued_at": _now(),
            "updated_at": _now(),
        }
        _write_json_atomic(path, rec)
        _ledger_append({"event": "queued", "target": a.to, "task_key": a.task,
                        "wakeup_id": rec["id"], "reason": a.reason, "severity": rec["severity"]})
        print(json.dumps({"result": "queued", "id": rec["id"],
                          "severity": rec["severity"]}, ensure_ascii=False))
        return EXIT_OK


# javis_snapshot 소비 — 리네임 시 동반 수정
def _iter_pending():
    if not os.path.isdir(PENDING_DIR):
        return []
    out = []
    for name in sorted(os.listdir(PENDING_DIR)):
        if name.endswith(".json"):
            rec = _load_pending(os.path.join(PENDING_DIR, name))
            if rec:
                out.append((os.path.join(PENDING_DIR, name), rec))
    return out


def cmd_list(a):
    rows = [rec for _, rec in _iter_pending()]
    print(json.dumps(rows, ensure_ascii=False, indent=1))
    return EXIT_OK


def _build_send_message(rec):
    n = rec.get("coalesced_count", 0)
    merged = f" (병합 {n}건)" if n else ""
    return (f"[wakeup {rec['id']}] task={rec['task_key']} reason={rec['reason']}{merged} "
            f"payload={json.dumps(rec.get('payload', {}), ensure_ascii=False)}")


def _fold_reason(reason):
    """digest 항목의 근거 **1줄** — 개행을 공백으로 접고 상한 초과분만 절단한다.

    요약하지 않는다(설계 I6 "뭉개기 금지"). 자를 때는 잘랐다는 사실을 `…` 로 남긴다 —
    조용한 절삭은 수신자가 "이게 전부"라고 오해하게 만든다.
    """
    s = " ".join(str(reason or "").split())
    return s if len(s) <= _DIGEST_REASON_MAX else s[:_DIGEST_REASON_MAX] + "…"


def _digest_sort_key(rec):
    """digest 항목 정렬 — severity 내림차순(critical 먼저) → task_key 오름차순.

    결정론이 목적이다: 같은 pending 집합은 언제 접어도 같은 본문을 내야 데몬 에코·원장 대조·
    회귀 테스트가 성립한다. 심각한 것을 먼저 읽히게 두는 것은 수신자(사람·LLM) 배려다.
    """
    return (-_SEV_RANK[_severity_of(rec)], rec.get("task_key") or "")


def _build_digest_message(target, recs):
    """★T-0147-2 층1 I6 — 같은 target 의 N건을 **1 push** 로 접는다.

    · 단건이면 `_build_send_message` 포맷 **그대로**다(무회귀 — 기존 테스트·운영 습관 보존).
    · 2건 이상이면 첫 줄 `[wakeup digest <N>건] target=<target>` + 항목 1줄씩.

    ★데몬 계약(설계 §2 층3 A3′ 상태머신): 데몬은 배달된 텍스트에서 `W-<hex>` 토큰을 **전부**
      뽑아 `queue.delivered` 이벤트로 에코한다. 그 이벤트가 critical 티어의 유일한 Inject-ack
      (= disarm 근거)이므로, **병합된 전 항목의 W-id 가 본문에 리터럴로 남아야 한다**.
      id 를 요약·생략하면 ack 가 영영 오지 않아 critical edge 가 disarm 되지 않는다.
    ★critical 필드 보존(설계 I6): 항목마다 task_key·severity·근거 1줄을 병기한다. payload 는
      비어있지 않을 때만 줄 끝에 붙여 단건 포맷과 정보량을 맞춘다.

    `recs` 는 호출부가 `_digest_sort_key` 로 정렬해 넘긴다(정렬 책임을 한 곳에 둔다).
    """
    if len(recs) == 1:
        return _build_send_message(recs[0])
    lines = [f"[wakeup digest {len(recs)}건] target={target}"]
    for rec in recs:
        line = (f"- [{rec['id']}] sev={_severity_of(rec)} task={rec.get('task_key')} "
                f"reason={_fold_reason(rec.get('reason'))}")
        payload = rec.get("payload") or {}
        if payload:
            line += " payload=%s" % json.dumps(payload, ensure_ascii=False)
        lines.append(line)
    return "\n".join(lines)


# ★G13(cokacdir 성찰 2026-07-04): 연속 배달실패 카운터 + fast-fail — hang/유령 대상 무한
#   재시도('idle 5분' 산문 규칙뿐이던 갭)를 결정론으로 차단. 임계 도달 시 pending 종결 +
#   카운터 리셋(이후 재큐잉은 깨끗한 재시도 — 자가 회복·수동 리셋 불요).
FAILCOUNT = os.path.join(WK_DIR, "failcount.json")


def _load_failcount():
    try:
        with open(FAILCOUNT, encoding="utf-8") as f:
            return json.load(f)
    except (FileNotFoundError, json.JSONDecodeError):
        return {}


def _bump_failcount(target, reset=False):
    with _FileLock(FAILCOUNT + ".lock"):
        fc = _load_failcount()
        fc[target] = 0 if reset else fc.get(target, 0) + 1
        _write_json_atomic(FAILCOUNT, fc)
        return fc[target]


def cmd_cancel(a):
    """★운영계약 §9-7 항목 11(b) — pending wake 를 **실제로** 취소·삭제한다.

    종전에는 취소 인터페이스가 없어(§11-18) "해제 전까지 drain --deliver 를 실행하지
    않는 것"이 취소의 집행이었다 — 사람의 규율뿐이라 누군가 한 번 실행하면 끝이었다.

    선택자(--target / --task / --idempotency-key) 가 하나도 없으면 거부한다 —
    전량 취소는 **--all 명시로만** 한다(오조작 전면 삭제 차단).
    취소는 관측·안전 조작이므로 **정지 중에도 허용**된다(재점화 위험을 줄이는 방향).
    """
    if not (a.target or a.task or a.idempotency_key or a.all):
        print("usage: cancel 은 --target / --task / --idempotency-key 중 하나 이상, 또는 "
              "--all 을 명시해야 한다(전량 오삭제 차단).", file=sys.stderr)
        return EXIT_USAGE
    hit = []
    for path, rec in _iter_pending():
        if a.target and rec.get("target") != a.target:
            continue
        if a.task and rec.get("task_key") != a.task:
            continue
        if a.idempotency_key and a.idempotency_key not in (rec.get("idempotency_keys") or []):
            continue
        hit.append((path, rec))
    if not hit:
        print("nothing to cancel")
        return EXIT_EMPTY
    cancelled = []
    for path, rec in hit:
        try:
            os.remove(path)
        except OSError as e:
            print("cancel failed: %s — %s" % (rec.get("id"), e), file=sys.stderr)
            continue
        _ledger_append({"event": "cancelled", "target": rec.get("target"),
                        "task_key": rec.get("task_key"), "wakeup_id": rec.get("id"),
                        "why": a.reason or "manual cancel"})
        cancelled.append(rec.get("id"))
        print("cancelled: %s → %s (task=%s)"
              % (rec.get("id"), rec.get("target"), rec.get("task_key")))
    print(json.dumps({"cancelled": len(cancelled), "ids": cancelled}, ensure_ascii=False))
    return EXIT_OK if cancelled else EXIT_EMPTY


def cmd_drain(a):
    """★T-0147-2 층1 I6 — **target 별 digest 배달**(종전: pending 1건당 cys send 1회).

    종전 루프는 pending N건이면 master stdin 에 N개의 메시지를 꽂았다. 그 자체가 wakeup
    홍수의 마지막 증폭단이었다(설계 §0-2: queued 1030 / coalesced 0). 여기서 N→1 로 접는다.

    무회귀 계약:
      · 단건이면 메시지 포맷·원장·stdout 이 종전과 동일하다(digest 원장 레코드도 남기지 않는다).
      · stdout JSON 의 `delivered` 는 **병합된 항목 수**다(digest 1건이 3항목이면 3).
        게이트(`javis_report_gate.Runner.drain`)가 이 숫자로 배달 성공을 판정하므로 의미를
        바꾸면 배달 성공이 실패로 뒤집힌다.
      · zombie 가드(G27)·fast-fail 임계(G13)·원장 마스킹(G2)·exit code 전부 유지.
    """
    # ★운영계약 §9-7 항목 11(a): 배달 **전에** kill-switch 2경로를 확인한다.
    #   드라이런(--deliver 없음)은 관측이므로 정지 중에도 허용한다(§9-4 허용 범위).
    if getattr(a, "deliver", False):
        paused, evidence = paused_evidence()
        if paused:
            _ledger_append({"event": "deliver_refused_paused",
                            "target": getattr(a, "target", None), "evidence": evidence})
            print("refused: AUTOPILOT_PAUSED — 정지 중에는 wake 를 배달하지 않는다"
                  "(자율 루프 재점화 차단 · 운영계약 §9-4·§9-7-11). 근거: %s"
                  % "; ".join(evidence), file=sys.stderr)
            print(json.dumps({"delivered": 0, "skipped": 0, "refused": "paused",
                              "evidence": evidence}, ensure_ascii=False))
            return EXIT_PAUSED
    fastfail_max = int(os.environ.get("JAVIS_FASTFAIL_MAX", "3"))
    pending = _iter_pending()
    if a.target:
        pending = [(p, r) for p, r in pending if r["target"] == a.target]
    if not pending:
        print("nothing pending")
        return EXIT_EMPTY
    # target 별 그룹. 삽입 순서 = `_iter_pending` 의 파일명 정렬 순서라 그룹 순회도 결정론이다.
    groups = {}
    for path, rec in pending:
        groups.setdefault(rec["target"], []).append((path, rec))
    delivered = skipped = 0
    for target, items in groups.items():
        # 생존 판정은 **target 당 1회**(종전 pending 당 1회) — 판정 기준·강등 규칙은 그대로이고
        # 같은 target 을 같은 순간에 두 번 묻지 않을 뿐이다(`cys list` 호출도 N→1).
        alive = _target_alive(target)
        if alive == "dead":
            # zombie 가드: 죽은 대상에 배달/병합 유지 금지 → skipped로 종결(pending 제거)
            for path, rec in items:
                os.remove(path)
                _ledger_append({"event": "skipped", "target": target, "wakeup_id": rec["id"],
                                "why": "target_dead(zombie guard)"})
                print(f"skipped (대상 사망): {rec['id']} → {target}")
                skipped += 1
            continue
        items.sort(key=lambda pr: _digest_sort_key(pr[1]))
        msg = _build_digest_message(target, [rec for _p, rec in items])
        cmd = ["cys", "send", "--queued", "--to", target, msg]
        if a.deliver:
            if alive == "unknown":
                print(f"warn: {target} 생존 미확인 상태로 배달 시도", file=sys.stderr)
            try:
                subprocess.run(cmd, check=True, timeout=15)
                if _load_failcount().get(target):
                    _bump_failcount(target, reset=True)  # 성공 = 연속실패 해소
            except (subprocess.SubprocessError, OSError, FileNotFoundError) as e:
                # ★digest 는 배달 **1회**다 — 실패도 1회로 센다(항목 수만큼 세면 pending 이 많은
                #   target 이 첫 실패에서 곧장 임계를 넘어 전 항목을 잃는다).
                streak = _bump_failcount(target)
                if streak >= fastfail_max:
                    # ★G13 fast-fail: 임계 도달 — pending 종결(무한 재시도 차단)·카운터 리셋
                    for path, rec in items:
                        os.remove(path)
                        _ledger_append({"event": "skipped", "target": target,
                                        "wakeup_id": rec["id"],
                                        "why": f"fast-fail(연속 {streak}회 배달실패)"})
                        print(f"skipped (fast-fail {streak}회): {rec['id']} → {target}",
                              file=sys.stderr)
                        skipped += 1
                    _bump_failcount(target, reset=True)
                    continue
                for _path, rec in items:
                    _ledger_append({"event": "deliver_failed", "target": target,
                                    "wakeup_id": rec["id"], "why": str(e), "fail_streak": streak})
                    print(f"deliver failed (pending 유지·연속 {streak}회): {rec['id']} — {e}",
                          file=sys.stderr)
                continue
        else:
            # digest 본문은 여러 줄이라 그대로 찍으면 DRYRUN 이 N 줄이 된다 — **표시만** 1줄로
            # 접는다(실제 배달 인자는 원문 그대로). W-id 는 전부 리터럴로 남는다.
            print("DRYRUN:", " ".join(cmd[:-1] + [cmd[-1].replace("\n", "\\n")]))
        for path, rec in items:
            os.remove(path)
            _ledger_append({"event": "delivered" if a.deliver else "delivered_dryrun",
                            "target": target, "wakeup_id": rec["id"]})
            delivered += 1
        if len(items) > 1:
            # 접힘 사실 자체를 원장에 남긴다(M1 계수의 근거 — "N건을 1 push 로 접었다").
            _ledger_append({"event": "digest", "target": target, "count": len(items),
                            "wakeup_ids": [rec["id"] for _p, rec in items]})
    print(json.dumps({"delivered": delivered, "skipped": skipped}, ensure_ascii=False))
    return EXIT_OK


def main(argv=None):
    p = argparse.ArgumentParser(description="wakeup 큐 — 코얼레싱·멱등·zombie 가드 (P0-2)")
    sub = p.add_subparsers(dest="cmd", required=True)

    c = sub.add_parser("enqueue")
    c.add_argument("--to", required=True, help="대상(역할/노드 이름)")
    c.add_argument("--task", required=True, help="task_key(병합 단위)")
    c.add_argument("--reason", required=True)
    c.add_argument("--payload", help="JSON 문자열")
    c.add_argument("--idempotency-key", dest="idempotency_key")
    # ★T-0147-2 I6 — 기본 warn(구 호출부는 무변경으로 종전 등급 유지). 코얼레싱 시 상승만.
    c.add_argument("--severity", choices=tuple(_SEV_RANK), default=_SEV_DEFAULT,
                   help="심각도(기본 warn) — 병합 시 최대치로 상승만 한다")
    c.set_defaults(fn=cmd_enqueue)

    c = sub.add_parser("list")
    c.set_defaults(fn=cmd_list)

    c = sub.add_parser("drain")
    c.add_argument("--target", help="특정 대상만")
    c.add_argument("--deliver", action="store_true",
                   help="실제 cys send 실행(기본 드라이런). "
                        "AUTOPILOT_PAUSED 2경로 중 하나라도 존재하면 배달을 거부한다(exit 4)")
    c.set_defaults(fn=cmd_drain)

    # ★운영계약 §9-7 항목 11(b) — pending 을 실제로 취소·삭제한다.
    c = sub.add_parser("cancel")
    c.add_argument("--target", help="대상(역할/노드 이름)만 취소")
    c.add_argument("--task", help="task_key 만 취소")
    c.add_argument("--idempotency-key", dest="idempotency_key",
                   help="해당 멱등키를 가진 pending 만 취소")
    c.add_argument("--all", action="store_true",
                   help="선택자 없이 전량 취소(명시 필수 — 오삭제 차단)")
    c.add_argument("--reason", help="취소 사유(원장 기록)")
    c.set_defaults(fn=cmd_cancel)

    a = p.parse_args(argv)
    return a.fn(a)


if __name__ == "__main__":
    sys.exit(main())
