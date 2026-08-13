#!/usr/bin/env python3
"""javis_task.py — P0-1 원자적 태스크 체크아웃 (Paperclip 계약의 클린룸 파일 포트)

계약(출처: _research/Paperclip_박사급_연구보고서.md §4 P0-1):
- 체크아웃은 원자적이다: 락 획득은 POSIX mkdir(원자적)로, 경쟁 시 정확히 1명만 승리.
- 충돌 = 409(exit 9): 살아있는 소유자가 있다는 뜻 — 재시도 금지(Never retry a 409).
- stale 판정은 시간 TTL이 아니라 run-liveness: 소유 pid가 죽었을 때만 락을 회수(adopt).
  adopt도 원자적: 기존 락 디렉터리 rename(원자적, 1명만 성공) 후 새로 획득.
- blocker는 "done"만 해소로 친다(cancelled는 미해소 잔존). 미해소 blocker가 있으면 체크아웃 거부(exit 4).
- 자동생성 중복 차단: 같은 (origin_kind, origin_fingerprint)의 열린 태스크가 있으면 create 거부(exit 8).
- 태스크가 done이 되면, 그 태스크를 기다리던 태스크들 중 blocker가 전부 해소된 것을 보고한다
  (wakeup 큐 연동은 javis_wakeup.py 몫 — 여기선 unblocked 목록 출력만).

상태 저장: $JAVIS_ROOT/_round/tasks/<id>.json (쓰기는 temp+os.replace 원자적)
락:        $JAVIS_ROOT/_round/tasks/<id>.lock/ (디렉터리 = mkdir 원자성) + owner.json

exit codes: 0 ok · 1 self-test 실패 · 2 usage · 3 not found · 4 blocked ·
            5 no evidence(W0-3·E1 artifact) · 6 verify-spec missing(T1 게이트) ·
            8 duplicate origin · 9 conflict(409)

T0a(2026-07-13 · attention-p0 승인): W0-3 evidence 게이트·W2-1 전이표·W1-2 handoff 리마인더를
omc-w2 라인에서 이식 복원 — 문서화된 운영 계약(CLAUDE.md)과 라이브 코드의 분기 봉합.
버전CAS·lease renew·서킷 등 W2 심층 기능 재통합은 T0b(후속 티켓) 범위.

T1(Phase 1 Wave A · DESIGN-DECISIONS §1): verify_spec checkout 게이트.
- exit 6 계약(조건 02①·16④): 'exit 6 = verify_spec 부재: **재시도 금지**, 워커는 push 금지,
  javis_wakeup enqueue --to master --task verify-spec-missing-<id> --idempotency-key <task_id>
  로만 통지(태스크당 1회 병합)' — 거부 통지는 이 코드가 same-run drain --deliver 짝과 함께
  자동 수행한다(실패 무해 통과 · pending 잔류 시 후속 drain 편승 배달). 병합=pending 창+
  <id>.verify-notified 마커 창(TTL 24h) 2중 — 배달 후 재거부는 마커 만료(또는 set-verify-spec
  성공 시 삭제) 후 재통지(A4). waiver 계열 거부는 stderr·task_key '-waiver' 분기(B1).
- 게이트 모드 env JAVIS_VERIFY_GATE=off|warn(기본)|strict (조건 32 — 2단 롤아웃: warn 기본 출하,
  strict 승격은 오탐 0 실측+오너 승인). strict 최초 집행 시 grandfather 마커
  _round/tasks/.verify-gate-activated 생성 — created_at < 마커 시각 태스크는 WARN 통과 +
  grandfathered:true 표식(조건 16① — fail-closed 는 활성 이후 신규 태스크에만).
- set-verify-spec <id> (--file|--json): 스키마 검증(schemas/verify_spec_schema.json SOT ·
  인라인 폴백) + _wlock 직렬화. risk 는 T1 에서 class="auto" 기록만(어휘 4종 검사·서명은 B2).
- 시스템 태스크 클래스 origin_kind=recovery|healing|watchdog → create 시 표준 읽기전용
  verify_spec 템플릿 자동 부여(조건 16② · prereg 선서명은 [OT]).
- checkout --brief <파일>: javis_brief_lint hard 경로(verify[] 형식 위반 = exit 6 거부).
  인세션 Agent/Task 경고 경로는 hooks/brief-lint-warn.sh(fail-open — 조건 05·21·28 비대칭).

B1(Phase 1 Wave B1 · DESIGN-DECISIONS §2-1): completion-guard 태스크 바인딩 파일 계약.
- checkout 성공 시 `--claim-surface <sid>`(명시 인자 우선 · F3) 또는 `CYS_SURFACE_ID`
  (cysd 주입 — 둘 다 부재 시 무동작)로 `_round/tasks/.guard-claim.<surface>` 를 기록
  (task_id·pid·ts) — guard 는 이 파일 stat 1개로 자기 바인딩을 판정한다(조건 23④ —
  전수 스캔 금지).
- ★이음매 자인(F3): 표준 플로우에서 checkout 은 master pane 에서 실행되므로 env 만으로는
  claim 이 master surface 에 기록돼 **워커 pane 의 guard 가 무발동**한다 — master 는
  `--claim-surface <워커 sid>` 로 위임 대상 워커 surface 를 지정하거나, 워커가 자기
  pane 에서 동일-owner 재checkout(멱등 재진입 지원)해야 한다.
- ★E2-1(BLOCKER R-02 교정): 위 위임 경로에서 **pid 는 기록하지 않는다**(`--claim-pid` 명시
  또는 `--pid` 명시가 없으면 `pid: null` + `pid_source` 박제 + stderr WARN 1줄). 종전에는
  호출자(master 셸) pid 를 채워 넣어, master `/clear`·재기동으로 그 pid 가 죽는 순간 워커
  guard 가 claim 을 stale 로 보고 **영구 exit 0**(침묵형 무발동)이 됐다. guard 측 짝은
  `javis_completion_guard._claim_liveness()` — pid 부재/사망 시 **태스크 레코드 상태**
  (in_progress|in_review ∧ 레코드 신선도)로 판정하고 pid 는 보조 신호로 강등한다.
- release·set-status 터미널 전이(done|cancelled) 성공 시 해당 task 를 가리키는
  guard-claim 을 삭제한다(교차 surface 호출 대비 — task_id 일치 항목 소거).
- 사이드카 네임스페이스 격리(F5): guard 산출 사이드카(`<id>.guard.json`·
  `<id>.esc-bundle.json`)는 태스크 레코드가 아니다 — list 등 판독 진입점은 예약 접미
  파일을 제외하고, G10 id 검증은 예약 접미(`.guard`·`.esc-bundle`) id 를 거부한다.

B2(Phase 1 Wave B2 · DESIGN-DECISIONS §3): 고위험 서명·done-deferred·calibrated 강등.
- set-verify-spec 위험 어휘 4종 검사(조건 07①·30③ — T1 이 예약한 훅 지점 실구현):
  cmd 가 서버기동(javis_resource_gate SERVER_PATTERNS/classify **import 재사용** — 복제 금지·
  조건 04③)·push생산(cys send|feed push|wakeup enqueue|approval) 이면 **서명 불문 고정
  거부(exit 2)** — verify 는 멱등·읽기전용이어야 한다. 삭제(rm|rmdir|unlink|shutil.rmtree|
  git clean|find -delete)·네트워크(curl|wget|nc|netcat|ssh|scp|rsync) 는 risk.class=high
  승격 — `--signed-by <approver>` 인간 서명 없으면 exit 2. 서명 전 리뷰어 ACCEPT 의무는
  계약 문서 조항(조건 30③ — 자기신고 한계 자인·도구는 서명 문자열만 집행 가능).
- set-verify-spec <id> --demote-calibrated: guard timeout_violations≥2 소비 경로(조건 07②)
  — wlock 안에서 verify_spec.calibrated=false 전이(멱등·calibrated=true 일 때만 변이) +
  spec.demoted_at 병기(G7d — 강등 근거의 영속화). checkout 은 강등 상태(calibrated=false ∧
  demoted_at 존재)를 stderr 경고로 표면화한다(비차단 — 재캘리브레이션 권고 · guard.json
  사이드카 소실과 무관. spec 존재=게이트 충족 계약은 불변).
- set-status done deferred 검사(조건 04①·20② 우선순위 통일 — 20② 채택): <id>.guard.json
  미해소 판정은 **내구 신호 합성**(G1 — deferred=true ∨ last_verdict=VERIFY_FAIL ∨
  block_count>0 ∨ last_sig≠None · 전부 guard PASS 로만 리셋되는 필드)이다 — last_verdict
  단독 판정은 SKIPPED_*/NO_BLOCK_PASS 1회로 잔존이 마스킹된다(b2-bugs HIGH). 미해소 시
  risk.class=high 면 exit 4 차단 / 일반이면 통과 + evidence 에 'verify-deferred' 라벨 자동
  병기 + stderr 고지(미검증 목록 다이제스트 보고 대상). guard.json 부재·손상 = 무간섭(회귀 0).
- set-verify-spec 재등록 게이트(G2): 미해소 guard 상태(G1 동일 기준) 또는 기존 spec
  risk.class=high 교체는 --force-respec 명시 없이 exit 2 거부 — 무해 cmd 재등록으로
  done-deferred 차단·서명 의무를 세탁하는 우회 봉쇄. 강제 시 task-audit 원장 1줄 기록.

E1-1(BLOCKER R-08 · 긴급 롤백 비대칭 수리): 위 done-deferred 검사는 `CYS_COMPLETION_GUARD`
와 **무관**하다 — guard 를 끄면 잔존 신호를 리셋할 PASS 가 오지 않아 고위험 태스크 done 이
exit 4 로 영구 차단됐다(안전밸브 부재). 밸브 2종을 신설:
- `CYS_TASK_GUARD_SIGNAL_GATE=strict|warn|off`(기본 strict · alias
  `CYS_TASK_GUARD_DEFERRED_GATE`) — evidence 게이트 env 3종과 동일 문법. warn=고위험도
  통과+라벨, off=검사 생략. 미지 값은 strict(오타로 게이트가 열리지 않는다).
- 자동 강등: `<id>.guard-disarmed`(회로차단기 개방 — 재무장은 수동 마커 삭제) 존재 시
  strict 라도 warn 강등. "자동 자가치유 없음 + done 영구 차단" 이중 잠금 해제.
두 경로 모두 stderr 1줄 고지 + `_round/evidence/guard_bypass_audit.jsonl` append-only 원장
+ task.evidence.guard_gate_bypass 병기(우회는 숨기지 않는다).

W0-3 radio 편승(RADIO_SPEC_v4 AA38 §5.2(e)): 티켓에 radio 가 개통돼 있으면(=
`_round/radio/<id>/META.json` 실존) done 전이가 `javis_radio.py done-check` 를 **호출**해
피어 BLOCKER·URGENT 의 미표면화·미resolve 잔존을 검사한다 — 게이트를 워커의 자발적 호출에
맡기면 "안 부르면 안 걸리는" 게이트가 되어 묵살 금지의 기계 집행이 성립하지 않는다.
밸브는 evidence 게이트 3종과 동일 문법 `CYS_TASK_RADIO_GATE=strict|warn|off`(기본 strict ·
미지 값은 strict). 판정 노드는 `--radio-node` > task.owner 순으로 정한다(둘 다 없으면
strict 에서 거부 — 측정 불능은 통과가 아니다). `--refs` 는 피어 radio 레코드 인용 기록
(AA33(b)① 킬 지표 계측 · §7.4(b) 철회 역추적의 입력)으로 task.refs 에 영속한다.
"""
import argparse
import contextlib
import getpass
import hashlib
import json
import os
import re
import socket
import sys
import time
import uuid

# ★Windows 즉사 차단(P0): top-level `import fcntl` 은 Windows 에 그 모듈이 없어
#   ModuleNotFoundError 로 이 스크립트 **전체**를 불능화한다 — javis_org.py:9-22 가 같은
#   사고를 먼저 겪고 msvcrt 폴백으로 지혈한 선례가 있다.
#   여기서는 **이름 `fcntl` 을 그대로 유지하는 shim** 을 바인딩한다. 아래 사용처의
#   `fcntl.flock(fd, fcntl.LOCK_EX)` 호출을 한 글자도 바꾸지 않으므로
#     · posix: 진짜 fcntl 모듈이 그대로 바인딩되어 동작이 **바이트 단위로 보존**된다.
#     · Windows: msvcrt 바이트락으로만 접힌다.
#   락 획득 실패를 삼키는 것도 javis_org.py:20-22 와 동일 판단이다 — 이 파일들의 append 는
#   O_APPEND 단일 os.write 라 기록 원자성은 락과 무관하게 이미 성립한다(락은 끝개행 보정
#   구간의 직렬화용). 크로스플랫폼 락 정본이 필요한 신규 코드는 javis_lock.py 를 쓴다.
try:
    import fcntl
except ImportError:  # Windows
    import msvcrt as _msvcrt

    class _FcntlShim(object):
        LOCK_EX = 2      # 값은 posix fcntl 과 동일 — 호출부가 이 상수만 쓰므로 자기정합
        LOCK_UN = 8

        @staticmethod
        def flock(fd, op):
            # msvcrt.locking 은 '현재 위치의 1바이트' 영역락이라 잠글 때와 풀 때의 위치가
            # 같아야 짝이 맞는다 → 위치를 0으로 고정해 걸고 원복한다(중간 lseek 와 무관).
            pos = os.lseek(fd, 0, os.SEEK_CUR)
            try:
                os.lseek(fd, 0, os.SEEK_SET)
                try:
                    _msvcrt.locking(
                        fd, _msvcrt.LK_LOCK if op == _FcntlShim.LOCK_EX else _msvcrt.LK_UNLCK, 1)
                except OSError:
                    pass          # best-effort — 락 실패가 기록 자체를 막지는 않는다
            finally:
                os.lseek(fd, pos, os.SEEK_SET)

    fcntl = _FcntlShim()

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

ROOT = os.environ.get("JAVIS_ROOT") or os.getcwd()  # 개인경로 하드코딩 금지(pack scan gate) — env 또는 CWD(워크스페이스 루트에서 호출)
TASKS_DIR = os.path.join(ROOT, "_round", "tasks")

STATUSES = ["backlog", "todo", "in_progress", "in_review", "done", "blocked", "cancelled"]
OPEN_STATUSES = ["backlog", "todo", "in_progress", "in_review", "blocked"]
TERMINAL_STATUSES = ["done", "cancelled"]
TERMINAL_FROM = ("in_progress", "in_review")  # W2-1 전이표: 터미널 전이 허용 출발 상태(T0a 이식)

EXIT_OK, EXIT_USAGE, EXIT_NOTFOUND, EXIT_BLOCKED, EXIT_DUP, EXIT_CONFLICT = 0, 2, 3, 4, 8, 9
EXIT_NO_EVIDENCE = 5  # W0-3: done 전이에 증거 부재(T0a 이식 · 기존 exit 체계와 무충돌)
EXIT_VERIFY = 6  # T1: verify_spec 게이트 거부(D9 — 6·7 빈 슬롯 실측 후 배정 · 조건 28)

# ─────────────────────────────────────────────────────────────────────────────
# T1(Phase 1 · DESIGN-DECISIONS §1-1): verify_spec 스키마 — schemas/verify_spec_schema.json
#   단일 머신 SOT 런타임 로드 + 인라인 폴백(javis_verdict._load_consts 관례). 판정 어휘는
#   신설하지 않는다(G6): 3치 = actprobe PASS/FAIL/INDET, 도구별 exit 는 pass_rule 로 흡수(D11).
# ─────────────────────────────────────────────────────────────────────────────
_VS_INLINE = {
    "MODE_ENUM": ["command", "procedural", "probe", "waiver"],
    "TOP_KEYS": ["schema_version", "mode", "cmd", "cwd", "pass_rule", "probe", "procedural",
                 "waiver_ref", "timeout_s", "n_of_m", "harness_ref", "calibrated", "risk",
                 "template", "template_origin", "demoted_at"],
    "REQUIRED_TOP": ["mode"],
    "MODE_REQUIRED": {"command": ["cmd", "pass_rule"], "procedural": ["procedural"],
                      "probe": ["probe"], "waiver": ["waiver_ref"]},
    "PASS_RULE_KEYS": ["kind", "pass_exits", "fail_exits", "indet_exits", "needle", "expr"],
    "PASS_RULE_KIND_ENUM": ["exit_map", "output_contains", "json_check"],
    "PROBE_KEYS": ["name", "args"],
    "PROCEDURAL_KEYS": ["tool", "artifact_path", "phase", "criteria", "report_path"],
    "PROCEDURAL_TOOL_ENUM": ["check-criteria", "factcheck"],
    "PROCEDURAL_TOOL_REQUIRED": {"check-criteria": ["artifact_path", "phase", "criteria"],
                                 "factcheck": ["report_path"]},
    "CRITERIA_KIND_ENUM": ["citation_present", "contradiction_flagged", "min_sources",
                           "section_present", "no_unsupported_claims", "json_valid",
                           "field_present", "min_items"],
    "CRITERIA_KEYS": ["kind", "value", "statement"],
    "N_OF_M_KEYS": ["n", "m"],
    "RISK_KEYS": ["class", "signed_by", "signed_at"],
}


def _vs_schema_path():
    """schemas/verify_spec_schema.json 위치 — bin/ 의 형제 schemas/, 폴백 CYS_PACK_DIR(verdict 관례)."""
    cand = os.path.join(os.path.dirname(_SELF_DIR), "schemas", "verify_spec_schema.json")
    if os.path.isfile(cand):
        return cand
    pd = os.environ.get("CYS_PACK_DIR") or os.environ.get("JAVIS_PACK_DIR")
    if pd:
        p = os.path.join(pd, "schemas", "verify_spec_schema.json")
        if os.path.isfile(p):
            return p
    return cand


def _vs_load_consts():
    """스키마 로드, 실패·키 누락 시 인라인 폴백(graceful degrade — 외부의존 0·항상 동작).
    B4: 키 타입(list/dict) 대조 — 불일치=손상 스키마 → 인라인 폴백 강등 + stderr 경고 1줄."""
    try:
        with open(_vs_schema_path(), encoding="utf-8") as f:
            s = json.load(f)
        out = {k: s[k] for k in _VS_INLINE}  # 모든 키 존재 필수 — 누락 시 KeyError→폴백
        for k, v in out.items():
            if type(v) is not type(_VS_INLINE[k]):  # B4: list/dict 타입 대조
                print("verify-spec 스키마 키 %r 타입 불일치(%s≠%s) — 인라인 폴백 강등"
                      % (k, type(v).__name__, type(_VS_INLINE[k]).__name__), file=sys.stderr)
                return dict(_VS_INLINE)
        return out
    except (OSError, ValueError, KeyError, TypeError):
        return dict(_VS_INLINE)


_VS = _vs_load_consts()
# 점수·비용 키 금지(javis_manifest BANNED_KEY_RE 관례 — reward-hack·비용 채널 차단)
_VS_BANNED_KEY_RE = re.compile(r"(?<![a-z])(score|grade|rating|usd|cost|price)(?![a-z])", re.I)

SYSTEM_ORIGIN_KINDS = ("recovery", "healing", "watchdog")  # D13 — 신설값·기존 레코드 충돌 0
GATE_MARKER = ".verify-gate-activated"  # §1-2 — strict 승격 집행 시 생성(warn 기간엔 미생성)
VERIFY_NOTIFY_TTL_SEC = 24 * 3600  # A4 — 거부 통지 태스크당 1회 창(<id>.verify-notified mtime)


def _vs_banned_keys(obj, path="$"):
    errs = []
    if isinstance(obj, dict):
        for k, v in obj.items():
            if _VS_BANNED_KEY_RE.search(str(k)):
                errs.append("금지된 점수·비용 키 %r (%s.%s)" % (k, path, k))
            errs += _vs_banned_keys(v, "%s.%s" % (path, k))
    elif isinstance(obj, list):
        for i, v in enumerate(obj):
            errs += _vs_banned_keys(v, "%s[%d]" % (path, i))
    return errs


def _is_pos_int(x):
    return isinstance(x, int) and not isinstance(x, bool) and x >= 1


def _nonempty_str(x):
    return isinstance(x, str) and bool(x.strip())


def validate_verify_spec(spec):
    """순수 CHECK — verify_spec 객체 → 오류 리스트(javis_manifest.validate_manifest house style).

    닫힌 키 집합 + mode 별 필수 + pass_rule/procedural/probe/waiver 세부. probe.name 은
    개방 집합(D11 — enum 핀 금지), procedural.criteria[].kind 는 manifest 8종 닫힌 enum 핀.
    """
    errs = []
    if not isinstance(spec, dict):
        return ["verify_spec 이 객체(dict)가 아님"]
    errs += _vs_banned_keys(spec)
    for k in spec:
        if k not in _VS["TOP_KEYS"]:
            errs.append("미지 최상위 키 %r — 계약 키는 %s" % (k, "|".join(_VS["TOP_KEYS"])))
    for k in _VS["REQUIRED_TOP"]:
        if k not in spec:
            errs.append("필수 키 누락: %s" % k)
    sv = spec.get("schema_version")
    if sv is not None and sv != 1:
        errs.append("schema_version 무효(%r) — 1 만 유효" % (sv,))
    mode = spec.get("mode")
    if mode is not None and mode not in _VS["MODE_ENUM"]:
        errs.append("mode 무효(%r) — %s" % (mode, "|".join(_VS["MODE_ENUM"])))
    if mode in _VS["MODE_REQUIRED"]:
        for k in _VS["MODE_REQUIRED"][mode]:
            if k not in spec:
                errs.append("mode=%s 는 %s 필수" % (mode, k))
    if "cmd" in spec and not _nonempty_str(spec.get("cmd")):
        errs.append("cmd 비어있지 않은 문자열 필요")
    if "cwd" in spec and not _nonempty_str(spec.get("cwd")):
        errs.append("cwd 비어있지 않은 문자열 필요")
    pr = spec.get("pass_rule")
    if pr is not None:
        if not isinstance(pr, dict):
            errs.append("pass_rule 객체 아님")
        else:
            for k in pr:
                if k not in _VS["PASS_RULE_KEYS"]:
                    errs.append("pass_rule 미지 키 %r — %s" % (k, "|".join(_VS["PASS_RULE_KEYS"])))
            kind = pr.get("kind")
            if kind not in _VS["PASS_RULE_KIND_ENUM"]:
                errs.append("pass_rule.kind 무효(%r) — %s"
                            % (kind, "|".join(_VS["PASS_RULE_KIND_ENUM"])))
            if kind == "exit_map":
                pe = pr.get("pass_exits")
                if not (isinstance(pe, list) and pe
                        and all(isinstance(x, int) and not isinstance(x, bool) for x in pe)):
                    errs.append("pass_rule.exit_map 은 pass_exits 정수 배열(≥1) 필수")
                for lk in ("fail_exits", "indet_exits"):
                    lv = pr.get(lk)
                    if lv is not None and not (isinstance(lv, list)
                                               and all(isinstance(x, int) and not isinstance(x, bool)
                                                       for x in lv)):
                        errs.append("pass_rule.%s 정수 배열 필요" % lk)
            if kind == "output_contains" and not _nonempty_str(pr.get("needle")):
                errs.append("pass_rule.output_contains 는 needle 필수")
            if kind == "json_check" and not _nonempty_str(pr.get("expr")):
                errs.append("pass_rule.json_check 는 expr 필수")
    pb = spec.get("probe")
    if pb is not None:
        if not isinstance(pb, dict):
            errs.append("probe 객체 아님")
        else:
            for k in pb:
                if k not in _VS["PROBE_KEYS"]:
                    errs.append("probe 미지 키 %r — %s" % (k, "|".join(_VS["PROBE_KEYS"])))
            if not _nonempty_str(pb.get("name")):
                errs.append("probe.name 비어있지 않은 문자열 필요(개방 집합 — enum 핀 없음·D11)")
            ar = pb.get("args")
            if ar is not None and not (isinstance(ar, list) and all(isinstance(x, str) for x in ar)):
                errs.append("probe.args 문자열 배열 필요")
    pc = spec.get("procedural")
    if pc is not None:
        if not isinstance(pc, dict):
            errs.append("procedural 객체 아님")
        else:
            for k in pc:
                if k not in _VS["PROCEDURAL_KEYS"]:
                    errs.append("procedural 미지 키 %r — %s" % (k, "|".join(_VS["PROCEDURAL_KEYS"])))
            tool = pc.get("tool")
            if tool not in _VS["PROCEDURAL_TOOL_ENUM"]:
                errs.append("procedural.tool 무효(%r) — %s"
                            % (tool, "|".join(_VS["PROCEDURAL_TOOL_ENUM"])))
            for k in _VS["PROCEDURAL_TOOL_REQUIRED"].get(tool, []):
                if not pc.get(k):
                    errs.append("procedural.tool=%s 는 %s 필수(실도구 입력 정합 — R1-contract)"
                                % (tool, k))
            cr = pc.get("criteria")
            if cr is not None:
                if not isinstance(cr, list) or not cr:
                    errs.append("procedural.criteria 비어있지 않은 배열 필요")
                else:
                    for i, c in enumerate(cr):
                        w = "procedural.criteria[%d]" % i
                        if not isinstance(c, dict):
                            errs.append("%s 객체 아님" % w)
                            continue
                        for k in c:
                            if k not in _VS["CRITERIA_KEYS"]:
                                errs.append("%s 미지 키 %r — %s"
                                            % (w, k, "|".join(_VS["CRITERIA_KEYS"])))
                        if c.get("kind") not in _VS["CRITERIA_KIND_ENUM"]:
                            errs.append("%s kind 무효(%r) — 기존 8종 닫힌 enum 핀(file_exists 없음)"
                                        % (w, c.get("kind")))
    if "waiver_ref" in spec and not _nonempty_str(spec.get("waiver_ref")):
        errs.append("waiver_ref 비어있지 않은 문자열 필요(javis_waiver 레코드 id)")
    ts = spec.get("timeout_s")
    if ts is not None and not ((isinstance(ts, (int, float)) and not isinstance(ts, bool)
                                and ts > 0)):
        errs.append("timeout_s 양수 필요(%r)" % (ts,))
    nm = spec.get("n_of_m")
    if nm is not None:
        if not isinstance(nm, dict):
            errs.append("n_of_m 객체 필요({n, m})")
        else:
            for k in nm:
                if k not in _VS["N_OF_M_KEYS"]:
                    errs.append("n_of_m 미지 키 %r — n|m" % k)
            if not _is_pos_int(nm.get("n")) or not _is_pos_int(nm.get("m")):
                errs.append("n_of_m.n/m 양의 정수 필요(%r)" % (nm,))
            elif nm["n"] > nm["m"]:
                errs.append("n_of_m.n(%s) > m(%s) — n ≤ m" % (nm["n"], nm["m"]))
    if "calibrated" in spec and not isinstance(spec.get("calibrated"), bool):
        errs.append("calibrated 는 bool")
    rk = spec.get("risk")
    if rk is not None:
        if not isinstance(rk, dict):
            errs.append("risk 객체 아님")
        else:
            for k in rk:
                if k not in _VS["RISK_KEYS"]:
                    errs.append("risk 미지 키 %r — %s" % (k, "|".join(_VS["RISK_KEYS"])))
    return errs


def _system_verify_spec_template(origin_kind):
    """시스템 태스크 클래스(D13·조건 16②) 표준 읽기전용 verify_spec 템플릿.
    치유·복원 티켓이 게이트에 걸리는 경로를 원천 제거. prereg 체인 선서명은 [OT](오너 1회)."""
    return {
        "schema_version": 1,
        "mode": "command",
        "cmd": "cys status --json",  # 읽기전용 한정(조건 16②·20① — cys status·preflight 계열)
        "pass_rule": {"kind": "exit_map", "pass_exits": [0]},
        "timeout_s": 10,
        "n_of_m": {"n": 1, "m": 1},
        "harness_ref": None,
        "calibrated": False,
        "risk": {"class": "auto", "signed_by": None, "signed_at": None},
        "template": "system-readonly-v1",
        "template_origin": origin_kind,
    }


# ─────────────────────────────────────────────────────────────────────────────
# T1(§1-2): checkout verify 게이트 — 모드·grandfather 마커·waiver 소비·거부 통지.
# ─────────────────────────────────────────────────────────────────────────────
def _gate_mode():
    """JAVIS_VERIFY_GATE=off|warn(기본)|strict — 미지 값은 warn(기본값)으로 접는다(보수측)."""
    m = (os.environ.get("JAVIS_VERIFY_GATE") or "warn").strip().lower()
    return m if m in ("off", "warn", "strict") else "warn"


def _gate_marker_path():
    return os.path.join(TASKS_DIR, GATE_MARKER)


def _ensure_gate_marker():
    """grandfather 활성 마커 — **strict 승격 집행 시 생성**(R1-contract). 최초 strict 게이트
    평가가 승격 집행 시점이다: 마커 내용 = 그 시각 ISO. warn 기간엔 생성하지 않는다(이행기 —
    신규 태스크는 spec 을 채운다·명문). 이미 있으면 기존 시각 유지(멱등).
    B3: 쓰기 실패(IsADirectoryError 등 OSError)는 suppress + 마커 없음 취급(보수측 reject)
    + stderr 경고 1줄 — traceback 으로 게이트가 깨지지 않는다."""
    p = _gate_marker_path()
    if os.path.isfile(p):
        return
    tmp = None
    try:
        os.makedirs(TASKS_DIR, exist_ok=True)
        tmp = "%s.tmp.%s.%s" % (p, os.getpid(), uuid.uuid4().hex[:8])
        with open(tmp, "w", encoding="utf-8") as f:
            f.write(_now() + "\n")
        os.replace(tmp, p)
    except OSError as e:
        print("verify-gate: 활성 마커 기록 실패(%s) — 마커 없음 취급(보수측 거부·B3)" % e,
              file=sys.stderr)
        if tmp:
            with contextlib.suppress(OSError):
                os.remove(tmp)


def _gate_marker_epoch():
    """마커 시각(epoch) — 내용 ISO 우선, 파싱 불가 시 mtime 폴백, 부재 시 None."""
    p = _gate_marker_path()
    try:
        with open(p, encoding="utf-8") as f:
            ep = _iso_to_epoch(f.read().strip())
        if ep is not None:
            return ep
        return os.stat(p).st_mtime
    except OSError:
        return None


def _waivers_path():
    return os.path.join(ROOT, ".vibecoding", "waivers.jsonl")  # javis_waiver 저장 계약과 동일


def _load_waiver(waiver_ref):
    """waivers.jsonl 직접 판독(§1-1 R1-contract) — waiver_id 일치 레코드(최신=마지막 줄).
    append-only 원장이라 같은 id 재기록은 없지만 방어적으로 마지막 일치를 취한다."""
    found = None
    try:
        with open(_waivers_path(), encoding="utf-8") as f:
            for line in f:
                line = line.strip()
                if not line:
                    continue
                try:
                    rec = json.loads(line)
                except ValueError:
                    continue  # 손상 줄은 유효 waiver 로 취급하지 않음(javis_waiver fail-closed 관례)
                if isinstance(rec, dict) and rec.get("waiver_id") == waiver_ref:
                    found = rec
    except OSError:
        return None
    return found


def _waiver_state(rec):
    """만료 판정 — javis_waiver 의 날짜 산술 import 재사용(중복 정의 금지 · §1-1).
    반환 (state, detail) · state ∈ valid|grace|expired|invalid|infra.
    48h 유예 = **만료일+2일 날짜 산술**(조건 16③ 조작화): today ≤ expiry+2일 이면 grace.
    A1: 모듈 부재(버전 찢김 의심)는 INFRA 취급 — traceback 대신 상태 보고
    (_notify_verify_missing 의 wakeup isfile 가드와 대칭)."""
    try:
        import javis_waiver as _wv  # 형제 모듈 — 경로 가드(sys.path append) 뒤 지역 import
    except ImportError:
        return "infra", "javis_waiver 모듈 부재(버전 찢김 의심)"
    ed = _wv._parse_date(rec.get("expiry", ""))
    if ed is None:
        return "invalid", "expiry 파싱 불가(%r)" % rec.get("expiry")
    today = _wv._today()
    if ed >= today:
        return "valid", "expiry=%s" % ed.isoformat()
    if today.toordinal() - ed.toordinal() <= 2:
        return "grace", "expiry=%s(+2일 유예 중)" % ed.isoformat()
    return "expired", "expiry=%s(유예 경과)" % ed.isoformat()


def _verify_gate_check(task):
    """checkout verify_spec 게이트(§1-2) → (verdict, why).
    verdict ∈ 'pass' | 'warn'(경고 후 통과) | 'grandfather'(WARN 통과+표식) | 'reject'
    | 'reject-waiver'(B1 — waiver 계열 거부 stderr·task_key 분기용·exit 6 동일)."""
    mode = _gate_mode()
    if mode == "off":
        return "pass", None
    spec = task.get("verify_spec")
    if isinstance(spec, dict) and spec:
        if spec.get("mode") == "waiver":
            ref = spec.get("waiver_ref") or ""
            rec = _load_waiver(ref)
            if rec is None:
                why = "waiver_ref %r 미해소(waivers.jsonl 레코드 없음)" % ref
                return ("reject-waiver" if mode == "strict" else "warn"), why
            state, detail = _waiver_state(rec)
            if state == "infra":
                return "warn", ("waiver %s 판정 불가(INFRA) — %s: strict 에서도 warn 강등"
                                "(traceback 금지·A1)" % (ref, detail))
            if state == "valid":
                return "pass", None
            if state == "grace":
                return "warn", ("waiver %s 만료 — 48h 유예 중(%s) · 재승인(신규 issue) 필요"
                                % (ref, detail))
            why = "waiver %s %s(%s) — 만료일+2일 유예 경과" % (ref, state, detail)
            return ("reject-waiver" if mode == "strict" else "warn"), why
        return "pass", None  # spec 존재 = 게이트 충족(실행·판정은 completion-guard(B1) 소관)
    # spec 부재
    if mode == "warn":
        return "warn", "verify_spec 부재 — 이행기(warn): master 가 set-verify-spec 으로 채울 것(strict 승격 대비)"
    _ensure_gate_marker()  # strict 승격 집행 = 최초 strict 평가 — 마커 시각 박제
    marker_ep = _gate_marker_epoch()
    created_ep = _iso_to_epoch(task.get("created_at"))
    if marker_ep is not None and created_ep is not None and created_ep < marker_ep:
        return "grandfather", ("created_at < 게이트 활성 시각(%s) — WARN 통과+grandfathered 표식"
                               % GATE_MARKER)
    return "reject", "verify_spec 부재(게이트 활성 이후 신규 태스크)"


def _verify_notified_marker(task_id):
    return os.path.join(TASKS_DIR, "%s.verify-notified" % task_id)  # A4 — 재발화 억제 마커


def _notify_verify_missing(task_id, why, waiver=False):
    """거부 통지(§1-2): wakeup enqueue(멱등키=task_id) + **same-run drain --deliver 짝**.
    autopilot :442 전례 — CYS_NO_AUTOSTART=1 + timeout(2~5s) + 실패 무해 통과(블로킹 대기 금지).
    drain 실패 시 pending 잔류 → 후속 drain 편승 배달(계약 자인 — BRIEF_CONTRACT). best-effort:
    어떤 실패도 exit 코드·게이트 판정에 불개입.
    A4: 병합=pending 창+마커 창 2중 — <id>.verify-notified(TTL 24h·mtime) 존재+미만료면
    enqueue 생략(배달 성공 후 재거부 재발화 억제 = 진짜 '태스크당 1회'). 마커는 enqueue 성공
    시 생성·set-verify-spec 성공 시 삭제(재위임 사이클 재개)·만료 후 재통지.
    B1: waiver 계열 거부는 task_key 에 '-waiver' 접미(분리 병합)."""
    import subprocess  # 지역 import — 모듈 상단 결합 회피(E1 관례)
    wk = os.path.join(_SELF_DIR, "javis_wakeup.py")
    if not os.path.isfile(wk):
        return
    mark = _verify_notified_marker(task_id)
    try:
        if time.time() - os.stat(mark).st_mtime < VERIFY_NOTIFY_TTL_SEC:
            return  # A4: 마커 창 내 재거부 — enqueue 생략(재발화 억제)
    except OSError:
        pass
    env = dict(os.environ)
    env["CYS_NO_AUTOSTART"] = "1"
    base = [sys.executable, wk]
    task_key = "verify-spec-missing-%s%s" % (task_id, "-waiver" if waiver else "")
    reason = ("checkout 거부(exit 6) — %s: %s"
              % ("verify-waiver 만료·미해소" if waiver else "verify_spec 부재", why or task_id))
    enq_ok = False
    with contextlib.suppress(Exception):
        r = subprocess.run(base + ["enqueue", "--to", "master", "--task", task_key,
                                   "--reason", reason,
                                   "--idempotency-key", task_id, "--severity", "warn"],
                           capture_output=True, text=True, timeout=5, env=env)
        enq_ok = (r.returncode == 0)
    if enq_ok:
        with contextlib.suppress(OSError):
            with open(mark, "w", encoding="utf-8") as f:
                f.write(_now() + "\n")
    with contextlib.suppress(Exception):
        subprocess.run(base + ["drain", "--deliver", "--target", "master"],
                       capture_output=True, text=True, timeout=5, env=env)


def _now():
    return time.strftime("%Y-%m-%dT%H:%M:%S%z")


# ─────────────────────────────────────────────────────────────────────────────
# B1(§2-1): completion-guard 태스크 바인딩 — .guard-claim.<surface> 파일 계약.
#   writer = checkout(여기) · 소거 = release/터미널 전이(여기) · reader = guard(stat 1개).
#   전부 best-effort(suppress) — claim 실패가 checkout 성공을 되돌리지 않는다.
# ─────────────────────────────────────────────────────────────────────────────
def _guard_surface_id():
    """cysd 주입 CYS_SURFACE_ID(구 AITERM_SURFACE_ID 수용 — _lib.sh A2 게이트 동일 규약)."""
    raw = (os.environ.get("CYS_SURFACE_ID", "")
           or os.environ.get("AITERM_SURFACE_ID", "")).strip()
    return re.sub(r"[^0-9A-Za-z_-]", "", raw)


def _write_guard_claim(task_id, pid_arg, surface=None, claim_pid=None):
    """F3: 명시 `--claim-surface` 인자 우선 — master 가 위임 대상 워커 surface 로 claim 을
    기록할 수 있게 한다(guard-claim 이음매 해소). 기본은 현행 env(CYS_SURFACE_ID).

    ★E2-1(BLOCKER R-02 · 침묵형 무발동 근절): 종전에는 `--claim-surface` 위임 경로에서도
    pid 를 **호출자(master 셸)** 것으로 채웠다(`os.getppid()`). 그 pid 는 워커와 무관하고
    master 가 `/clear`·재기동하면 죽는다 ⇒ 워커 guard 가 `_pid_alive` 실패로 claim 을 stale
    판정해 **영구 exit 0**(무장했는데 아무것도 검증 안 함 · 카나리아 1일차 통과·2일차 침묵).
    교정: **pid 는 소유자가 확실할 때만 기록한다.**
      · `--claim-pid <pid>` 명시 = 그 pid(위임 대상 워커 pane 의 pid — 소유자 확실)
      · `--pid <pid>` 명시 = 락 생존 판정 pid 를 그대로 재사용(종전 계약 유지)
      · 둘 다 없이 `--claim-surface` 로 **위임**한 경우 = pid 미기록(`None`)
        → guard 는 pid 대신 **태스크 레코드 상태**로 생존을 판정한다(pid 는 보조 신호로 강등).
      · `--claim-surface` 없이 자기 pane 에서 checkout = 종전대로 `os.getppid()`(소유자 일치)
    `pid_source` 필드로 어느 경로였는지 claim 에 박제한다(사후 진단 · 무기록 이전 금지).
    """
    if surface is not None:
        sid = re.sub(r"[^0-9A-Za-z_-]", "", str(surface).strip())
        if not sid:
            print("warn: --claim-surface 값이 정화 후 공백 — claim 미기록(fail-open)",
                  file=sys.stderr)
            return
    else:
        sid = _guard_surface_id()
    if not sid:
        return
    if claim_pid is not None:
        pid_val, pid_src = claim_pid, "claim-pid(명시 — 위임 대상 pane pid)"
    elif pid_arg is not None:
        pid_val, pid_src = pid_arg, "pid(명시 — 락 생존 pid 재사용)"
    elif surface is not None:
        # 위임 경로: 호출자 pid 는 위임 대상과 무관하다 — 거짓 생존 신호를 남기지 않는다.
        pid_val, pid_src = None, "delegated(미기록 — 태스크 상태 기반 판정)"
        print("warn: --claim-surface 위임 — claim pid 미기록(호출자 pid 는 워커와 무관). "
              "guard 는 태스크 레코드 상태로 claim 생존을 판정한다. 워커 pane pid 를 고정하려면 "
              "`--claim-pid <워커 pid>` 를 주라.", file=sys.stderr)
    else:
        pid_val, pid_src = os.getppid(), "caller(자기 pane checkout)"  # _acquire_lock 관례 동일
    rec = {"task_id": task_id, "pid": pid_val, "pid_source": pid_src, "ts": _now()}
    with contextlib.suppress(OSError):
        _write_json_atomic(os.path.join(TASKS_DIR, ".guard-claim.%s" % sid), rec)


def _remove_guard_claims(task_id):
    """이 task 를 가리키는 guard-claim 전부 소거 — release/set-status 가 타 surface 에서
    호출될 수 있어 자기 surface 파일만 지우면 고아 claim 이 남는다(stale 24h 규정의 보완)."""
    try:
        names = os.listdir(TASKS_DIR)
    except OSError:
        return
    for name in names:
        if not name.startswith(".guard-claim."):
            continue
        p = os.path.join(TASKS_DIR, name)
        with contextlib.suppress(OSError, ValueError):
            with open(p, encoding="utf-8") as f:
                rec = json.load(f)
            if isinstance(rec, dict) and rec.get("task_id") == task_id:
                os.remove(p)


# ★G10(cokacdir 성찰 2026-07-04): task id allowlist — 경로 조합 전 traversal 차단.
#   wakeup._safe(:45)와 동일 문자집합이나 조용한 치환 대신 '거부'(fail-loud) —
#   기존 유효 id의 경로 매핑을 바꾸지 않고, `--id ../../tmp/x` 류 임의 쓰기를 닫는다.
_ID_RE = re.compile(r"^[A-Za-z0-9._-]{1,80}$")
# ★F5(B1): guard 사이드카 예약 접미 — `<id>.guard.json`·`<id>.esc-bundle.json` 이 태스크
#   레코드(`<id>.json`)와 같은 `.json` 종결이라 판독 진입점에서 유령 태스크로 오파싱되고,
#   역으로 이 접미로 끝나는 id 는 사이드카와 파일명이 충돌한다 — 양방향 격리.
RESERVED_ID_SUFFIXES = (".guard", ".esc-bundle")
RESERVED_FILE_SUFFIXES = tuple(s + ".json" for s in RESERVED_ID_SUFFIXES)


def _task_path(task_id):
    return os.path.join(TASKS_DIR, f"{task_id}.json")


def _lock_dir(task_id):
    return os.path.join(TASKS_DIR, f"{task_id}.lock")


def _wlock_dir(task_id):
    return os.path.join(TASKS_DIR, f"{task_id}.wlock")


@contextlib.contextmanager
def _wlock(task_id, timeout=5.0, stale=30.0):
    """★WP-8(P-ORCH): 태스크 JSON read-modify-write 단일 직렬화 쓰기락(<id>.wlock).

    소유권 락(<id>.lock)과 별개의 '쓰기 직렬화' 뮤텍스다 — set-status·checkout·release·create가
    같은 <id>.json 을 동시에 read-modify-write 하며 갱신을 서로 덮어쓰던 경로(이중 락 비배제)를
    닫는다. 임계구역은 JSON 갱신뿐(수 ms)이라 stale(30s) 회수가 산 소유자를 탈취하지 않는다 —
    긴 완료-하한선 sleep 은 이 락 '밖'에 둔다. 과경합으로 획득 실패 시 하드 실패 대신 경고 후
    무락 진행(기존 exit-code 계약 불변·최악의 경우 현행 무락 동작으로 degrade)."""
    os.makedirs(TASKS_DIR, exist_ok=True)
    path = _wlock_dir(task_id)
    deadline = time.time() + timeout
    acquired = False
    while True:
        try:
            os.mkdir(path)  # ← 원자적: 경쟁 시 1명만 성공
            acquired = True
            break
        except FileExistsError:
            try:  # 죽은 소유자가 남긴 만료 락은 원자적으로 회수(rename→rmdir, 빈 디렉터리)
                if time.time() - os.stat(path).st_mtime > stale:
                    stale_name = f"{path}.stale.{time.time_ns()}"
                    os.rename(path, stale_name)
                    with contextlib.suppress(OSError):
                        os.rmdir(stale_name)
                    continue
            except OSError:
                pass
            if time.time() > deadline:
                print(f"warn: wlock 획득 실패(과경합) — 무락 진행: {task_id}", file=sys.stderr)
                break
            time.sleep(0.02)
    try:
        yield
    finally:
        if acquired:
            with contextlib.suppress(OSError):
                os.rmdir(path)


def _write_json_atomic(path, obj):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    tmp = f"{path}.tmp.{os.getpid()}.{uuid.uuid4().hex[:8]}"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(obj, f, ensure_ascii=False, indent=1)
        f.flush()
        os.fsync(f.fileno())
    os.replace(tmp, path)  # 원자적 교체


def _read_task(task_id):
    try:
        with open(_task_path(task_id), encoding="utf-8") as f:
            return json.load(f)
    except FileNotFoundError:
        return None


# javis_snapshot 소비 — 리네임 시 동반 수정
def _list_tasks():
    if not os.path.isdir(TASKS_DIR):
        return []
    out = []
    for name in sorted(os.listdir(TASKS_DIR)):
        if (name.endswith(".json") and not name.startswith(".")
                and not name.endswith(RESERVED_FILE_SUFFIXES)):  # F5: 사이드카 제외
            t = _read_task(name[:-5])
            if t:
                out.append(t)
    return out


def _pid_alive(pid):
    """run-liveness: pid 생존 확인. 테스트 주입용 JAVIS_TASK_LIVENESS=alive|dead 지원."""
    override = os.environ.get("JAVIS_TASK_LIVENESS")
    if override == "alive":
        return True
    if override == "dead":
        return False
    try:
        os.kill(int(pid), 0)
        return True
    except (ProcessLookupError, ValueError, TypeError):
        return False
    except PermissionError:
        return True  # 존재하지만 남의 프로세스


def _read_owner(task_id):
    try:
        with open(os.path.join(_lock_dir(task_id), "owner.json"), encoding="utf-8") as f:
            return json.load(f)
    except (FileNotFoundError, NotADirectoryError, json.JSONDecodeError):
        return None


# ─────────────────────────────────────────────────────────────────────────────
# E1(증거의 기계화 · 설계 DESIGN_LAZYCODEX_DISTILLATION §E1): evidence artifact 게이트.
#   done 전이 시 --evidence-artifact 경로를 기계 검사(실존·비어있지않음·신선도)한다.
#   신선도 앵커 = owner.json acquired_at (release 전 판독 — R2), 부재 시 created_at 폴백.
# ─────────────────────────────────────────────────────────────────────────────
def _iso_to_epoch(iso):
    """_now() 형식('%Y-%m-%dT%H:%M:%S%z')의 ISO 문자열 → epoch 초. 파싱 실패 시 None."""
    if not iso:
        return None
    from datetime import datetime
    try:
        return datetime.strptime(iso, "%Y-%m-%dT%H:%M:%S%z").timestamp()
    except (ValueError, TypeError):
        return None


def _sha256_8(path):
    """파일 내용 sha256의 앞 8 hex — 증거 지문(감사 대조용, 무결성 강제 아님)."""
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(65536), b""):
            h.update(chunk)
    return h.hexdigest()[:8]


def _skip_audit_path():
    return os.path.join(ROOT, "_round", "evidence", "skip_audit.jsonl")


def _append_jsonl(path, rec):
    """append-only(O_APPEND) JSONL 1줄 기록 — 감사 원장(수정·삭제 금지 · R3)."""
    os.makedirs(os.path.dirname(path), exist_ok=True)
    line = (json.dumps(rec, ensure_ascii=False) + "\n").encode("utf-8")
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o644)
    try:
        os.write(fd, line)
    finally:
        os.close(fd)


def _check_evidence_artifacts(task_id, task, paths):
    """--evidence-artifact 경로들을 기계 검사(실존·비어있지않음·신선도).
    신선도 앵커 = owner.json acquired_at(release 전 판독 · R2), 부재 시 created_at 폴백.
    반환 (True, records) | (False, 오류문자열). records엔 anchor 출처를 명시(감사 가시화)."""
    owner = _read_owner(task_id)
    if owner and owner.get("acquired_at"):
        anchor_iso, anchor_src = owner["acquired_at"], "acquired_at"
    else:
        anchor_iso, anchor_src = task.get("created_at"), "created_at-fallback"
    anchor_ts = _iso_to_epoch(anchor_iso)
    records = []
    for p in paths:
        if not os.path.isfile(p):                               # ① 실존
            return False, "부재 경로: %s" % p
        st = os.stat(p)
        if st.st_size <= 0:                                     # ② 비어있지않음
            return False, "빈 파일(0바이트): %s" % p
        if anchor_ts is not None and st.st_mtime < anchor_ts:   # ③ 신선도
            return False, ("신선도 실패(mtime<%s): %s — 태스크 시작 전 파일은 증거 아님"
                           % (anchor_src, p))
        records.append({"path": p, "size": st.st_size,
                        "mtime": round(st.st_mtime, 3),
                        "sha256_8": _sha256_8(p), "anchor": anchor_src})
    return True, records


# ★WP-8(P-ORCH · lease-adopt 강화): '산 소유자 락 탈취' 창을 닫는다. _acquire_lock 의 adopt 는
#   이미 run-liveness 기반이다 — 읽을 수 있는 owner.json + 살아있는 pid 는 아래 :159 에서 conflict
#   로 보호되고, 죽은 pid 만 즉시 adopt 된다(그 경로는 이 grace 를 '건너뛴다'). 유일한 탈취 경로는
#   owner.json 이 일시 부재/훼손(holder is None)인 상태에서 grace 경과 후 adopt 하는 경로뿐이다.
#   pid 확인 불가(owner.json 없음)라 옵션 (a)'pid 사망 확인 한정'을 순수 적용하면 크래시-복구가
#   영구 deadlock 이 되므로, 옵션 (b) 'LEASE 상향'을 택해 5s→30s 로 올린다 — dead-pid adopt 는
#   무영향, ownerless 창만 길어져 산 소유자 탈취를 실질 0 에 수렴시키되 크래시 복구는 30s 내 보장.
OWNER_WRITE_GRACE_SEC = 30.0  # 락 획득~owner.json 기록 사이의 정당한 lease(과거 5.0 → P-ORCH 강화)


def _write_owner(lock, rec):
    """owner.json을 락 디렉터리 '재생성 없이' 기록. 락이 탈취돼 사라졌으면 False(경쟁 패배)."""
    tmp = os.path.join(TASKS_DIR, f".owner.tmp.{os.getpid()}.{uuid.uuid4().hex[:8]}")
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(rec, f, ensure_ascii=False, indent=1)
        f.flush()
        os.fsync(f.fileno())
    try:
        os.replace(tmp, os.path.join(lock, "owner.json"))
        return True
    except (FileNotFoundError, NotADirectoryError, OSError):
        try:
            os.remove(tmp)
        except OSError:
            pass
        return False


def _acquire_lock(task_id, owner_id, pid=None):
    """mkdir 원자성 기반 CAS. 반환: ('acquired'|'conflict'|'adopted', owner_dict|None)

    pid 기본값은 호출자(부모 프로세스) — CLI 자신의 pid는 즉시 종료되어 liveness 근거가
    못 되기 때문. 장수 프로세스(워커 pane 등)에 묶으려면 --pid로 명시.
    """
    lock = _lock_dir(task_id)
    owner_rec = {
        "owner_id": owner_id,
        "pid": pid if pid is not None else os.getppid(),
        "host": socket.gethostname(),
        "user": getpass.getuser(),
        "acquired_at": _now(),
    }
    try:
        os.mkdir(lock)  # ← 원자적: 경쟁 시 1명만 성공
        if not _write_owner(lock, owner_rec):
            return "conflict", None  # 기록 창에서 락 탈취됨 — 경쟁 패배로 처리
        return "acquired", owner_rec
    except FileExistsError:
        pass
    holder = _read_owner(task_id)
    if holder and holder.get("owner_id") == owner_id:
        return "acquired", holder  # 자기 재진입(멱등)
    if holder and _pid_alive(holder.get("pid")):
        return "conflict", holder  # 살아있는 소유자 = 409
    if holder is None:
        # owner.json 미기록: 방금 획득한 경쟁자의 "기록 창"일 수 있음 — grace 이내면 409
        try:
            age = time.time() - os.stat(lock).st_mtime
        except OSError:
            age = OWNER_WRITE_GRACE_SEC  # 락이 사라짐 = 경쟁 진행 중
        if age < OWNER_WRITE_GRACE_SEC:
            return "conflict", None
    # stale(소유 pid 사망, 또는 grace 지난 owner.json 훼손) → 원자적 adopt: rename은 1명만 성공
    stale_name = f"{lock}.stale.{time.time_ns()}.{uuid.uuid4().hex[:6]}"
    try:
        os.rename(lock, stale_name)
    except (FileNotFoundError, OSError):
        return "conflict", _read_owner(task_id)  # 경쟁자가 먼저 adopt함 — 409로 취급
    try:
        os.mkdir(lock)
    except FileExistsError:
        return "conflict", _read_owner(task_id)
    if not _write_owner(lock, owner_rec):
        return "conflict", None
    # stale 잔해는 감사용으로 보존하지 않고 정리(내용은 owner.json 하나)
    try:
        p = os.path.join(stale_name, "owner.json")
        if os.path.exists(p):
            os.remove(p)
        os.rmdir(stale_name)
    except OSError:
        pass
    return "adopted", owner_rec


def _release_lock(task_id, owner_id, force=False):
    lock = _lock_dir(task_id)
    holder = _read_owner(task_id)
    if not os.path.isdir(lock):
        return True
    if not force and holder and holder.get("owner_id") != owner_id:
        return False
    try:
        p = os.path.join(lock, "owner.json")
        if os.path.exists(p):
            os.remove(p)
        os.rmdir(lock)
    except OSError:
        return False
    return True


def _unresolved_blockers(task):
    """blocker는 done만 해소(cancelled는 미해소 잔존 — Paperclip 계약 준수)."""
    out = []
    for bid in task.get("blocked_by", []):
        b = _read_task(bid)
        if b is None or b.get("status") != "done":
            out.append(bid)
    return out


def _find_unblocked_dependents(done_task_id):
    """방금 done이 된 태스크로 인해 blocker가 전부 해소된 열린 태스크 목록."""
    out = []
    for t in _list_tasks():
        if t.get("status") in OPEN_STATUSES and done_task_id in t.get("blocked_by", []):
            if not _unresolved_blockers(t):
                out.append(t["id"])
    return out


def cmd_create(a):
    task_id = a.id or f"T{time.strftime('%m%d')}-{uuid.uuid4().hex[:6]}"
    with _wlock(task_id):  # ★WP-8(P-ORCH): dup 검사~쓰기 직렬화 — 동시 create 경합의 lost-write 차단
        if _read_task(task_id):
            print(f"error: task exists: {task_id}", file=sys.stderr)
            return EXIT_DUP
        if a.origin_fingerprint:
            for t in _list_tasks():
                if (t.get("status") in OPEN_STATUSES
                        and t.get("origin_kind") == a.origin_kind
                        and t.get("origin_fingerprint") == a.origin_fingerprint):
                    print(f"duplicate-origin: open task {t['id']} has same "
                          f"({a.origin_kind},{a.origin_fingerprint}) — create 거부", file=sys.stderr)
                    return EXIT_DUP
        task = {
            "id": task_id,
            "title": a.title,
            "status": a.status,
            "why": a.why or "",           # goal 체인: 이 일이 왜 존재하는가
            "goal": a.goal or "",
            "blocked_by": a.blocked_by or [],
            "origin_kind": a.origin_kind,
            "origin_fingerprint": a.origin_fingerprint or "default",
            "owner": None,
            "created_at": _now(),
            "updated_at": _now(),
        }
        # T1(D13·조건 16②): 시스템 태스크 클래스는 표준 읽기전용 verify_spec 템플릿 자동 부여 —
        # 치유·복원 티켓이 checkout 게이트에 걸리는 경로 원천 제거(prereg 선서명은 [OT]).
        if a.origin_kind in SYSTEM_ORIGIN_KINDS:
            task["verify_spec"] = _system_verify_spec_template(a.origin_kind)
        _write_json_atomic(_task_path(task_id), task)
    print(task_id)
    return EXIT_OK


def cmd_checkout(a):
    task = _read_task(a.id)
    if not task:
        print(f"not found: {a.id}", file=sys.stderr)
        return EXIT_NOTFOUND
    if task["status"] in TERMINAL_STATUSES:
        print(f"conflict: task is terminal ({task['status']}) — 재시도 금지", file=sys.stderr)
        return EXIT_CONFLICT
    unresolved = _unresolved_blockers(task)
    if unresolved:
        print(f"blocked: 미해소 blocker {unresolved} — 체크아웃 거부", file=sys.stderr)
        return EXIT_BLOCKED
    # ── B6: 게이트 평가 '전' 생존 소유자 선확인 — 산 소유자 충돌은 exit 9 우선(무통지).
    #    자기 재진입(owner 동일)은 _acquire_lock 멱등 경로에 위임. TOCTOU 는 아래 CAS 가 최종심.
    holder = _read_owner(a.id)
    if holder and holder.get("owner_id") != a.owner and _pid_alive(holder.get("pid")):
        print(f"conflict(409): 살아있는 소유자 {holder.get('owner_id')} — 재시도 금지, "
              "다른 태스크로 이동", file=sys.stderr)
        return EXIT_CONFLICT
    # ── T1 brief 게이트(--brief · hard 경로 — 조건 05①·21①·28): javis_brief_lint 의
    #    hard(2)=verify[] 형식 위반만 exit 6 거부. 경고(1)는 stderr 로 흘리고 통과(fail-open).
    #    인세션 Agent/Task 경고 경로는 hooks/brief-lint-warn.sh 별도(여긴 티켓 연동 위임 전용).
    if getattr(a, "brief", None):
        try:
            import javis_brief_lint as _bl  # 형제 모듈 — 경로 가드(sys.path append) 뒤 지역 import
        except ImportError:
            print("error: javis_brief_lint 모듈 부재(버전 찢김 의심) — --brief 게이트 평가 불가: "
                  "팩 설치 정합 복구 후 재시도(A1 — wakeup isfile 가드와 대칭)", file=sys.stderr)
            return EXIT_USAGE
        bcode, bfind = _bl.lint_file(a.brief)
        for ln in bfind:
            print("brief-lint: %s" % ln, file=sys.stderr)
        if bcode == 2:
            print("brief verify[] missing(6): checkout --brief hard 거부 — "
                  "BRIEF_CONTRACT 5필드+실행형 verify[] 동봉 후 재위임(재시도 금지)",
                  file=sys.stderr)
            return EXIT_VERIFY
    # ── T1 verify_spec 게이트(§1-2): 락 획득 '전' 평가 — 거부가 소유권·레코드를 변이하지 않는다.
    gate, gwhy = _verify_gate_check(task)
    if gate in ("reject", "reject-waiver"):
        # wakeup enqueue+same-run drain(무해 통과·A4 마커 억제) — 통지 선행
        _notify_verify_missing(a.id, gwhy, waiver=(gate == "reject-waiver"))
        if gate == "reject-waiver":  # B1: waiver 계열 거부 stderr 분기(exit 6 유지)
            print("verify-waiver expired(6): 재승인(신규 issue) 후 재위임. "
                  "통지 큐 등재됨(태스크당 1회 병합·미배달 시 후속 drain 편승).", file=sys.stderr)
        else:
            print("verify-spec missing(6): 재시도 금지 — master가 verify_spec 작성 후 재위임. "
                  "통지 큐 등재됨(태스크당 1회 병합·미배달 시 후속 drain 편승).", file=sys.stderr)
        if gwhy:
            print("verify-gate: %s" % gwhy, file=sys.stderr)
        return EXIT_VERIFY
    if gate in ("warn", "grandfather"):
        print("verify-gate WARN: %s" % gwhy, file=sys.stderr)
    # ── B2(조건 07②): calibrated 강등 상태 표면화 — spec 존재 통과 계약은 불변(비차단).
    #    G7d: 판정 근거 = spec.demoted_at(--demote-calibrated 가 강등 시 병기하는 영속 필드) —
    #    guard.json(휘발 사이드카) 판독에 의존하지 않아 사이드카 소실과 무관하게 발화한다.
    #    수동 calibrated:false 등록(강등 전이 아님)은 demoted_at 부재 → 경고 없음(의도). ──
    _spec_b2 = task.get("verify_spec")
    if (isinstance(_spec_b2, dict) and _spec_b2 and _spec_b2.get("calibrated") is False
            and _spec_b2.get("demoted_at")):
        print("verify-gate WARN: calibrated 강등 상태(demoted_at=%s — 조건 07② quick 상한 "
              "위반 반복) — 재캘리브레이션 전 신뢰 강등(비차단·재게이트 권고)"
              % _spec_b2.get("demoted_at"), file=sys.stderr)
    verdict, holder = _acquire_lock(a.id, a.owner, pid=a.pid)
    if verdict == "conflict":
        who = (holder or {}).get("owner_id", "unknown")
        print(f"conflict(409): 살아있는 소유자 {who} — 재시도 금지, 다른 태스크로 이동", file=sys.stderr)
        return EXIT_CONFLICT
    with _wlock(a.id):  # ★WP-8(P-ORCH): JSON 갱신 직렬화 — 동시 set-status 와의 lost-update 차단
        task = _read_task(a.id)          # 락 안 재독 — 대기 중 갱신 반영
        if not task:
            print(f"not found: {a.id}", file=sys.stderr)
            return EXIT_NOTFOUND
        task["status"] = "in_progress"
        task["owner"] = a.owner
        if gate == "grandfather":
            task["grandfathered"] = True  # §1-2: WARN 통과 표식(strict 활성 이전 생성분)
        task["updated_at"] = _now()
        _write_json_atomic(_task_path(a.id), task)
    # B1(§2-1)·F3: completion-guard 바인딩 — 성공 경로에서만·명시 --claim-surface 우선
    # ★E1-4(BLOCKER F12/R-25 · 팩 배포 무게이트 발효 강등): **ambient `CYS_SURFACE_ID` 경로는
    #   기본 OFF**다. 이 파일은 이미 6프로필에서 도는 공용 도구라 팩 install 즉시 발효하는데,
    #   그 경로를 켠 채로 배포하면 cys pane 안의 **모든** checkout 이 종전에 없던 파일
    #   `_round/tasks/.guard-claim.<sid>` 를 라이브 저장소에 만들기 시작한다(A/B 실측 확인 ·
    #   회수 규칙도 없다 — R-15). 게다가 표준 플로우에서 checkout 은 master pane 이 실행하므로
    #   그 claim 은 **엉뚱한 surface** 에 박히는, R-02 이음매의 진원이기도 하다.
    #   따라서 자동 발효 대상에서 뺀다:
    #     · 명시 `--claim-surface`/`--claim-pid` = 항상 기록(의도된 opt-in · OT-2 가 쓰는 경로)
    #     · ambient env 경로 = `CYS_TASK_GUARD_CLAIM=1` 일 때만(OT-2 무장 스텝과 같은 창에서 켠다)
    #   기본값에서 이 도구는 종전과 **바이트 동일한 산출물**을 남긴다(파일 0개 추가).
    _claim_explicit = (getattr(a, "claim_surface", None) is not None
                       or getattr(a, "claim_pid", None) is not None)
    if _claim_explicit or os.environ.get("CYS_TASK_GUARD_CLAIM") == "1":
        _write_guard_claim(a.id, a.pid, surface=getattr(a, "claim_surface", None),
                           claim_pid=getattr(a, "claim_pid", None))
    print(json.dumps({"checkout": verdict, "id": a.id, "owner": a.owner}, ensure_ascii=False))
    return EXIT_OK


def cmd_release(a):
    task = _read_task(a.id)
    if not task:
        print(f"not found: {a.id}", file=sys.stderr)
        return EXIT_NOTFOUND
    # ★G11(cokacdir 성찰 2026-07-04): --force 무검증 폐기 — 사유 필수 + 태스크 JSON에
    #   force_history 감사 기록. (암호학 verifier는 다중노드/원격 확장 시 — per-node secret
    #   없는 로컬 단일 시스템에서 해시는 검증이 아니라 장식이다. 성찰 G11 'P2 조건부' 준수.)
    if a.force and not getattr(a, "force_reason", None):
        print("error: --force는 --force-reason '<사유>' 필수 — 무검증 탈취 금지(G11)",
              file=sys.stderr)
        return EXIT_USAGE
    if not _release_lock(a.id, a.owner, force=a.force):
        holder = _read_owner(a.id)
        print(f"conflict(409): 락 소유자 불일치 (holder={(holder or {}).get('owner_id')})", file=sys.stderr)
        return EXIT_CONFLICT
    with _wlock(a.id):  # ★WP-8(P-ORCH): JSON 갱신 직렬화
        task = _read_task(a.id)          # 락 안 재독
        if not task:
            print(f"not found: {a.id}", file=sys.stderr)
            return EXIT_NOTFOUND
        if a.force:
            task.setdefault("force_history", []).append(
                {"at": _now(), "by": a.owner, "reason": a.force_reason})
        if task.get("owner") == a.owner or a.force:
            task["owner"] = None
            if task["status"] == "in_progress":
                task["status"] = "todo"
            task["updated_at"] = _now()
            _write_json_atomic(_task_path(a.id), task)
    _remove_guard_claims(a.id)  # B1(§2-1): 바인딩 해제 — guard 무발동 복귀
    print(f"released: {a.id}")
    return EXIT_OK


_PROBE_IDX = 0


def _settle_probe(owner):
    """1회 관측 → (idle_secs|None, todo_mtime|None). JAVIS_SETTLE_PROBE로 결정론 주입 가능
    (형식 "idle:mtime;idle:mtime" — 호출마다 하나 소비 · wakeup JAVIS_WAKEUP_LIVENESS 관례)."""
    global _PROBE_IDX
    seq = os.environ.get("JAVIS_SETTLE_PROBE")
    if seq is not None:
        parts = seq.split(";")
        p = parts[min(_PROBE_IDX, len(parts) - 1)]
        _PROBE_IDX += 1
        try:
            i, m = p.split(":", 1)
            return (float(i) if i else None), (float(m) if m else None)
        except ValueError:
            return None, None
    try:
        import javis_boot_node as _bn  # 형제 모듈(orchestra:194 관례) — 부재 시 관측불가=fail-closed
        st = _bn.cys_status()
        row = _bn.status_surface(st, owner) if st else None
    except Exception:
        row = None
    idle = row.get("idle_secs") if row else None
    todo = os.path.join(ROOT, "_round", "%s_TODO.md" % owner.upper())
    try:
        mt = os.stat(todo).st_mtime
    except OSError:
        mt = None
    return idle, mt


def _completion_settled(owner):
    """★G6(cokacdir 성찰 2026-07-04): 완료 하한선 — 단일 스냅샷 완료 판정 금지.
    owner 노드가 2회 연속(settle 간격) idle 안착 + 그 사이 TODO 갱신(mtime) 정지일 때만
    True(boot_node POLL-IDLE·channel_watch 2-strike 프리미티브 이식). sub-agent 활동은
    같은 surface의 idle_secs에 반영된다는 전제(부정확하면 idle 미안착으로 안전측 거부).
    반환 (ok, 사유)."""
    settle = float(os.environ.get("JAVIS_SETTLE_SEC", "5"))
    idle_min = float(os.environ.get("JAVIS_SETTLE_IDLE_MIN", "4"))
    last_mt = None
    for strike in range(2):
        if strike:
            time.sleep(settle)
        idle, mt = _settle_probe(owner)
        if idle is None:
            return False, "owner '%s' 상태 관측 불가 — 완료 거부(fail-closed)" % owner
        if idle < idle_min:
            return False, ("owner '%s' idle 미안착(idle=%s<%s) — sub-agent/작업 진행 중 추정"
                           % (owner, idle, idle_min))
        if strike and mt != last_mt:
            return False, "owner '%s' TODO 갱신 진행 중(mtime 변동) — 완료 아님" % owner
        last_mt = mt
    return True, "settled(2-strike idle)"


# ─────────────────────────────────────────────────────────────────────────────
# P1b(설계 §2.2·§2.1b · R1 리플레이 해소): probe 실행 영수증 대조 게이트. E1(산출물 파일
#   검증)과 상보적 — E1=검증 산출물 파일의 실존/신선도, 여기=probe 실행이 남긴 append-only
#   영수증의 최근성·exit0·대상/task 일치. 둘 다 통과해야 done. 영수증 스키마·경로는 actprobe
#   와 공유 계약이며, ts 파싱은 E1 의 _iso_to_epoch 를 재사용한다(중복 정의 금지).
# ─────────────────────────────────────────────────────────────────────────────
SCHEMA_VERSION = 1
_PROBE_TOKEN_RE = re.compile(r"probe:([a-z-]+)")
# relaxed 3종: target 이 surface ref·pid 라 부분일치가 무의미 → 대조 축을 task 로 교체(§2.1b).
#   이 3종은 현재 태스크로 바인딩된(--task) 영수증만 인정(무-task·타-task = 리플레이 거부).
_RELAXED_TARGET_PROBES = {"submit", "ctx-compare", "kill-preflight"}


def _rec_task(rec):
    """영수증 선택 필드 task(§2.1b) — 문자열이고 비어있지 않으면 반환, 아니면 None."""
    v = rec.get("task")
    return v if isinstance(v, str) and v != "" else None


def _probe_runs_path():
    """영수증 경로: env CYS_PROBE_RUNS > <pack>/state/probe_runs.jsonl (actprobe 와 동일)."""
    env = os.environ.get("CYS_PROBE_RUNS")
    if env:
        return env
    pack = os.environ.get("CYS_PACK_DIR") or os.path.expanduser("~/.cys/pack")
    return os.path.join(pack, "state", "probe_runs.jsonl")


def _load_receipts(path):
    """probe_runs.jsonl 파싱 → (rows, file_exists). 깨진 행은 fail-soft로 건너뛴다."""
    if not os.path.exists(path):
        return [], False
    rows = []
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            for line in f:
                line = line.strip()
                if not line:
                    continue
                try:
                    rec = json.loads(line)
                except ValueError:
                    continue  # 깨진 행 fail-soft(§2.2-3)
                if isinstance(rec, dict):
                    rows.append(rec)
    except OSError:
        return [], False
    return rows, True


def _verify_probe_receipts(task, ev_text, tid):
    """evidence 의 probe:<name> 토큰을 영수증과 기계 대조 → (ok, reason).

    토큰이 없으면 (True, None) — 하위호환. 토큰이 있으면 각 probe:
      · 영수증 파일 부재 = probe 미실행 → 거부(§2.2-3)
      · (probe명 ∧ exit0 ∧ 최근 K분) 영수증 최소 1건 필요(없으면 태만/스테일 거부)
      · relaxed 3종(§2.1b): task 필드가 현재 태스크(tid)와 일치하는 영수증만 인정 —
        무-task·타-task 영수증은 리플레이로 거부(target 대조 무의미 → 축을 task 로 교체)
      · artifact·verdict-match: target 이 evidence/goal/why 에 부분일치 + (task 필드가 있으면
        현재 태스크와 일치 — 타 태스크 리플레이 차단, 무-task 는 target 만으로 인정)

    ★위협모델 한계(설계 §0 v3): 동일 OS 사용자의 가짜 영수증 수기 작성은 이 게이트가 차단
    못 한다 — 감사층(원장·mine) 탐지 대상이며 출처증명은 cysd attestation 로드맵. 본 게이트는
    실측 지배 실패 모드(태만·스테일·대상/task 불일치)를 결정론으로 닫는다.
    """
    tokens = _PROBE_TOKEN_RE.findall(ev_text)
    if not tokens:
        return True, None
    path = _probe_runs_path()
    rows, exists = _load_receipts(path)
    if not exists:
        return False, ("probe 토큰 %s 영수증 미대조: 영수증 파일 부재(%s) — probe 미실행 증거"
                       % (sorted(set(tokens)), path))
    window_min = float(os.environ.get("CYS_PROBE_RECEIPT_WINDOW_MIN", "240"))
    now = time.time()
    # target 부분일치 대상 텍스트(대소문자 무시): evidence + 태스크 goal/why.
    haystack = " ".join([ev_text, task.get("goal") or "", task.get("why") or ""]).lower()
    for name in dict.fromkeys(tokens):  # 순서 보존 dedup
        recent0 = []
        for rec in rows:
            if rec.get("probe") != name or rec.get("exit") != 0:
                continue
            ep = _iso_to_epoch(rec.get("ts"))  # E1 헬퍼 재사용(중복 정의 금지)
            if ep is None or (now - ep) > window_min * 60:
                continue
            recent0.append(rec)
        if not recent0:
            return False, ("probe:%s 미대조 — exit0·최근 %d분 이내 영수증 없음"
                           % (name, int(window_min)))
        if name in _RELAXED_TARGET_PROBES:
            # 대조 축=task(§2.1b): 현재 태스크로 바인딩된 영수증만 인정.
            if any(_rec_task(r) == tid for r in recent0):
                continue
            if any(_rec_task(r) is not None for r in recent0):
                return False, ("probe:%s task 바인딩 불일치 — 타 태스크 영수증(리플레이 의심)"
                               % name)
            return False, ("probe:%s 무-task 영수증만 존재 — relaxed probe 는 --task 동반 "
                           "영수증만 인정(리플레이 차단)" % name)
        # artifact·verdict-match: target 부분일치 필요.
        tmatch = [r for r in recent0
                  if (r.get("target") or "").strip()
                  and (r.get("target") or "").strip().lower() in haystack]
        if not tmatch:
            return False, ("probe:%s 영수증은 있으나 target 불일치 — 영수증 target 이 "
                           "evidence/goal/why 텍스트에 없음" % name)
        # task 필드가 있으면 현재 태스크와 일치해야 함(리플레이 차단). 무-task 는 target 만으로 인정.
        if not any(_rec_task(r) is None or _rec_task(r) == tid for r in tmatch):
            return False, ("probe:%s target 은 맞으나 task 바인딩 불일치 — 타 태스크 영수증"
                           "(리플레이 의심)" % name)
    return True, None


def _append_receipt_line(path, rec):
    """actprobe._append_receipt 규약 복제(append-only·끝개행 보정·flock 단일 writer).
    E1 _append_jsonl 은 별도 파일(skip_audit.jsonl)용이므로 여기선 actprobe 와 같은 flock
    규약을 쓴다(공유 계약 정합) — 코드 결합 없이 규약만 소형 복제."""
    line = json.dumps(rec, ensure_ascii=False, sort_keys=True) + "\n"
    d = os.path.dirname(path) or "."
    os.makedirs(d, exist_ok=True)
    fd = os.open(path, os.O_RDWR | os.O_CREAT | os.O_APPEND, 0o644)
    try:
        fcntl.flock(fd, fcntl.LOCK_EX)
        size = os.fstat(fd).st_size
        if size:
            os.lseek(fd, size - 1, os.SEEK_SET)
            if os.read(fd, 1) != b"\n":
                os.write(fd, b"\n")
        os.write(fd, line.encode("utf-8"))
    finally:
        try:
            fcntl.flock(fd, fcntl.LOCK_UN)
        finally:
            os.close(fd)


def _resolve_caller():
    """caller: env CYS_ACTPROBE_CALLER > cys identify surface_ref > OS 사용자."""
    env = os.environ.get("CYS_ACTPROBE_CALLER")
    if env:
        return env
    try:
        import subprocess  # E1 self-test 관례(지역 import) — 모듈 상단 결합 회피
        out = subprocess.run([os.environ.get("CYS_BIN", "cys"), "identify"],
                             capture_output=True, text=True, timeout=5)
        if out.returncode == 0:
            ref = ((json.loads(out.stdout) or {}).get("caller") or {}).get("surface_ref")
            if isinstance(ref, str) and ref:
                return ref
    except Exception:
        pass
    try:
        return getpass.getuser()
    except Exception:
        return "unknown"


def _append_audit(task_id, audit_kind):
    """우회(skip-reason·gate-off) 가시화 감사 행 → probe_runs.jsonl(§2.2-2). best-effort·비차단.
    E1 skip_audit.jsonl 과 상보적 — 이쪽은 probe 원장에 남겨 §3 mine 이 반복 우회를 포착한다."""
    rec = {
        "schema_version": SCHEMA_VERSION,
        "ts": _now(),
        "probe": "task-audit",
        "target": task_id,
        "exit": 0,
        "argv_digest": hashlib.sha256(
            "\x00".join(sys.argv[1:]).encode("utf-8")).hexdigest()[:16],
        "caller": _resolve_caller(),
        "audit": audit_kind,
    }
    try:
        _append_receipt_line(_probe_runs_path(), rec)
    except OSError as e:
        print("task: WARN audit receipt append failed: %s" % e, file=sys.stderr)


def _radio_gate_mode():
    """`CYS_TASK_RADIO_GATE=strict|warn|off` — evidence 게이트 env 3종과 동일 문법·문체.
    미지 값은 strict(오타로 게이트가 열리지 않는다)."""
    raw = (os.environ.get("CYS_TASK_RADIO_GATE") or "").strip().lower()
    return raw if raw in ("strict", "warn", "off") else "strict"


def _radio_meta_path(task_id):
    root = os.environ.get("JAVIS_ROOT") or os.getcwd()
    return os.path.join(root, "_round", "radio", task_id, "META.json")


def _radio_done_gate(task_id, task, radio_node, refs):
    """AA38 §5.2(e) 편승 — radio 개통 티켓의 done 전이에서 `javis_radio.py done-check` 를
    호출한다. 반환: None=통과(또는 비대상) · 정수=거부 exit code.

    ★왜 훅인가: 게이트가 수동 단독 명령으로만 존재하면 '워커가 부르지 않으면 걸리지 않는'
      게이트다 — 묵살 금지의 기계 집행이 성립하지 않는다. radio 미개통 티켓에는 아무
      부작용도 없다(META 부재 = 비대상)."""
    mode = _radio_gate_mode()
    if mode == "off":
        return None
    meta_path = _radio_meta_path(task_id)
    if not os.path.isfile(meta_path):
        return None                     # radio 미개통 — 비대상(회귀 0)
    node = (radio_node or task.get("owner") or "").strip()
    if not node:
        msg = ("radio done-check: 판정 노드 미상(--radio-node 또는 task.owner 필요) — "
               "측정 불능은 통과가 아니다")
        if mode == "strict":
            print(msg + " (밸브: CYS_TASK_RADIO_GATE=warn|off)", file=sys.stderr)
            return EXIT_NO_EVIDENCE
        print("radio warn: " + msg, file=sys.stderr)
        return None
    script = os.path.join(os.path.dirname(os.path.abspath(__file__)), "javis_radio.py")
    if not os.path.isfile(script):
        msg = "radio done-check: javis_radio.py 부재 — 게이트 실행 불능"
        if mode == "strict":
            print(msg + " (밸브: CYS_TASK_RADIO_GATE=warn|off)", file=sys.stderr)
            return EXIT_NO_EVIDENCE
        print("radio warn: " + msg, file=sys.stderr)
        return None
    import subprocess  # E1 self-test 관례(지역 import) — 모듈 상단 결합 회피
    argv = [sys.executable, script, "done-check", "--ticket", task_id, "--node", node]
    if refs:
        argv += ["--refs", refs]
    try:
        r = subprocess.run(argv, capture_output=True, timeout=120)
    except Exception as e:
        msg = "radio done-check 실행 불가: %s" % e
        if mode == "strict":
            print(msg + " (밸브: CYS_TASK_RADIO_GATE=warn|off)", file=sys.stderr)
            return EXIT_NO_EVIDENCE
        print("radio warn: " + msg, file=sys.stderr)
        return None
    out = ((r.stdout or b"") + (r.stderr or b"")).decode("utf-8", "replace").strip()
    if r.returncode == 0:
        return None
    if mode == "strict":
        print("radio done-check BLOCK(%d): %s" % (r.returncode, out[-1200:]), file=sys.stderr)
        print("  강제 표면화(`javis_radio.py wait --once` 또는 `read`) 후 ack·resolve 를 "
              "기록하고 재시도하라. 밸브: CYS_TASK_RADIO_GATE=warn|off", file=sys.stderr)
        return EXIT_NO_EVIDENCE
    print("radio warn: done-check 비0(%d) — %s" % (r.returncode, out[-600:]), file=sys.stderr)
    return None


def cmd_set_status(a):
    task = _read_task(a.id)
    if not task:
        print(f"not found: {a.id}", file=sys.stderr)
        return EXIT_NOTFOUND
    # ★G6: done은 완료 하한선 통과 후에만 — 오종결이 의존자 unblock(:308)을 조기 발화하는
    #   경로를 닫는다. owner 없는(체크아웃 이력 없는) 태스크는 안착 대상이 없어 게이트 생략.
    #   ★WP-8(P-ORCH): 최대 settle sleep 은 wlock '밖'에서 — 락 장기점유·타임아웃을 피한다.
    # W0-3 evidence 게이트(T0a 이식 — omc-w2 라인): done 전이는 --evidence 또는 --skip-reason 필수.
    # 인자만 보는 즉시 검사라 settle 프로브(sleep 동반)보다 먼저 — cheap-first(성찰 D3).
    # 기본 strict, CYS_TASK_EVIDENCE_GATE=warn|off 안전밸브.
    ev_text = (getattr(a, "evidence", None) or "").strip()
    skip_text = (getattr(a, "skip_reason", None) or "").strip()
    art_paths = list(getattr(a, "evidence_artifact", None) or [])
    artifact_records = None  # done 게이트 통과 시 채워져 wlock 안에서 task.evidence.artifacts로 저장
    skip_audit_pending = False  # skip 감사는 실제 done 진행(전이·settle 게이트 통과) 후에만 기록
    guard_deferred = False  # B2: guard 미검증 잔존 → evidence 'verify-deferred' 라벨 병기 예약
    guard_bypass_rec = None  # E1-1(R-08): 안전밸브·자동강등으로 고위험 보류를 우회한 기록(원장)
    if a.status == "done":
        # ── B2 guard deferred 검사(조건 04①·20② 우선순위 통일 — 20② 채택 · cheap-first):
        #    <id>.guard.json 미해소 판정 = **내구 신호 합성**(G1 — _guard_unresolved:
        #    deferred ∨ last_verdict=VERIFY_FAIL ∨ block_count>0 ∨ last_sig≠None · 전부
        #    PASS 로만 리셋) — last_verdict 단독 판정의 SKIPPED_*/NO_BLOCK_PASS 마스킹 창 차단.
        #    risk.class=high 는 done 보류(exit 4) / 일반은 통과+라벨 병기(다이제스트 보고 대상).
        #    guard.json 부재·손상 = 무간섭(회귀 0 — B1 이전 레코드·비무장 pane 동일 동작).
        #    ★E1-1(R-08 롤백 비대칭 수리): 이 검사는 `CYS_COMPLETION_GUARD` env 와 **무관**하다
        #    — guard 를 끄면 잔존 신호를 리셋할 PASS 가 영원히 오지 않으므로, 안전밸브 없이는
        #    고위험 태스크 done 이 exit 4 로 **영구 차단**된다. 그래서 밸브 2개를 둔다:
        #      ① 명시 밸브 `CYS_TASK_GUARD_SIGNAL_GATE=strict|warn|off`(기본 strict · evidence
        #         게이트 env 3종과 동일 문법·문체 · alias `CYS_TASK_GUARD_DEFERRED_GATE`)
        #      ② 자동 강등: `<id>.guard-disarmed`(회로차단기 개방 = 수동 재무장 전까지 PASS
        #         불가능) 존재 시 strict 라도 **warn 으로 강등**(차단 아님·라벨 유지).
        #    두 경로 모두 stderr 1줄 고지 + `_round/evidence/guard_bypass_audit.jsonl` 원장. ──
        _gobj = _load_guard_obj(a.id)
        _gmode = _guard_signal_gate_mode()
        if _gmode == "off":
            if _guard_unresolved(_gobj):
                guard_deferred = True
                guard_bypass_rec = {"reason": "env-off", "mode": _gmode,
                                    "signals": _guard_signals(_gobj), "risk": None}
                print("guard-signal-gate off: guard 미검증 잔존(%s) — done-deferred 검사 생략"
                      "(CYS_TASK_GUARD_SIGNAL_GATE=off). 'verify-deferred' 라벨과 우회 원장"
                      "(_round/evidence/guard_bypass_audit.jsonl)은 유지된다."
                      % _guard_signals(_gobj), file=sys.stderr)
        elif _guard_unresolved(_gobj):
            _vs = task.get("verify_spec")
            _rk = _vs.get("risk") if isinstance(_vs, dict) else None
            _risk_cls = _rk.get("class") if isinstance(_rk, dict) else None
            _disarmed = _guard_disarmed_reason(a.id)
            if _risk_cls == "high" and _gmode == "strict" and _disarmed is None:
                print("done blocked(4): verify 미검증 잔존(%s) + risk.class=high — 고위험 "
                      "태스크는 verify 통과(guard PASS) 후 done(조건 20② 고위험 보류 · "
                      "G1 내구 신호 합성)" % _guard_signals(_gobj), file=sys.stderr)
                print("  되돌리는 법(R-08 안전밸브): guard 를 껐다면 PASS 는 오지 않는다 — "
                      "①`CYS_TASK_GUARD_SIGNAL_GATE=warn`(고위험도 통과+라벨) 또는 `=off`"
                      "(검사 생략) ②또는 잔존 파일 제거 `rm $JAVIS_ROOT/_round/tasks/%s.guard.json`"
                      % a.id, file=sys.stderr)
                return EXIT_BLOCKED
            guard_deferred = True
            if _risk_cls == "high":
                _why = ("guard-disarmed(%s)" % _disarmed) if _disarmed is not None \
                    else "CYS_TASK_GUARD_SIGNAL_GATE=warn"
                guard_bypass_rec = {"reason": "auto-demote-disarmed" if _disarmed is not None
                                    else "env-warn", "mode": _gmode,
                                    "signals": _guard_signals(_gobj), "risk": _risk_cls}
                print("guard-signal-gate warn: 고위험 보류를 라벨 강등 — %s. 잔존(%s)"
                      % (_why, _guard_signals(_gobj)), file=sys.stderr)
            print("verify-deferred: guard 미검증 잔존(%s) — evidence 에 'verify-deferred' "
                  "라벨 자동 병기(미검증 목록 다이제스트 보고 대상 · 조건 04①·20②)"
                  % _guard_signals(_gobj), file=sys.stderr)
        # ── E1 evidence-artifact 게이트(증거의 기계화 · 설계 §E1) — cheap-first(settle sleep 전) ──
        #   신선도 앵커 = owner.json acquired_at을 release(:하단) '전'에 판독한다(R2).
        #   기계 검사 실패(실존·비어있지않음·신선도)는 우회 불가 — strict 거부/warn 경고.
        art_mode = os.environ.get("CYS_TASK_EVIDENCE_ARTIFACT_GATE", "strict")
        art_valid = False
        if art_mode != "off" and art_paths:
            ok, res = _check_evidence_artifacts(a.id, task, art_paths)
            if ok:
                artifact_records = res
                art_valid = True
            else:
                if art_mode == "strict":
                    print("evidence-artifact required(5): %s "
                          "(기계 검사 3종 — 실존·비어있지않음·신선도 mtime≥앵커)" % res, file=sys.stderr)
                    return EXIT_NO_EVIDENCE
                print("evidence-artifact warn: %s" % res, file=sys.stderr)
        # 기존 텍스트 evidence 게이트(W0-3) — 유효 artifact는 이를 충족으로 인정(증거 상위호환)
        mode = os.environ.get("CYS_TASK_EVIDENCE_GATE", "strict")
        has_evidence = len(ev_text) >= 8 or bool(skip_text) or art_valid  # 품질 하한: evidence 최소 8자
        if mode != "off" and not has_evidence:
            if mode == "strict":
                print('evidence required(5): done 전이는 --evidence·--evidence-artifact 또는 --skip-reason 필수 '
                      '(예: --evidence "pytest → 31/31 PASS" · evidence 최소 8자)', file=sys.stderr)
                return EXIT_NO_EVIDENCE
            print("evidence warn: done 전이에 검증 증거 없음 — --evidence·--evidence-artifact 또는 "
                  "--skip-reason 권장", file=sys.stderr)
        # E1 artifact 게이트(R1 day-1 strict): 유효 artifact ≥1 또는 --skip-reason
        if art_mode != "off" and not art_valid and not skip_text:
            if art_mode == "strict":
                print("evidence-artifact required(5): done 전이는 유효 --evidence-artifact ≥1 또는 "
                      "--skip-reason 필수 (증거의 기계화 — 텍스트 단정만으로는 불충분)", file=sys.stderr)
                return EXIT_NO_EVIDENCE
            print("evidence-artifact warn: 유효 --evidence-artifact 없음 — "
                  "--evidence-artifact 또는 --skip-reason 권장", file=sys.stderr)
        # ── P1b probe 영수증 대조(설계 §2.2) — E1 게이트 '뒤'·settle sleep '전'(cheap-first). evidence
        #    의 probe:<name> 토큰을 actprobe 영수증과 기계 대조(최근성·exit0·target/task 바인딩).
        #    E1(산출물 파일)과 상보적이며 둘 다 통과해야 done. W0-3 텍스트 게이트와 동일 안전밸브
        #    (mode=CYS_TASK_EVIDENCE_GATE) 공유 — strict 거부(5)/warn 경고/off 생략.
        if mode != "off":
            pok, pwhy = _verify_probe_receipts(task, ev_text, a.id)
            if not pok:
                if mode == "strict":
                    print("probe receipt mismatch(5): " + pwhy, file=sys.stderr)
                    return EXIT_NO_EVIDENCE
                print("probe receipt warn: " + pwhy, file=sys.stderr)
        # ── radio done 게이트 편승(AA38 §5.2(e)) — evidence 게이트 **뒤**·settle sleep 앞.
        #    radio 미개통 티켓에는 아무 일도 하지 않는다(회귀 0).
        rc_radio = _radio_done_gate(a.id, task, getattr(a, "radio_node", None),
                                    getattr(a, "refs", None))
        if rc_radio is not None:
            return rc_radio
        # skip 감사(R3): skip-reason이 done 통과의 부담 근거(유효 artifact 부재)이면 예약 — art_mode와
        #   무관하다(★어태커 결함1: off 밸브에서도 --skip-reason은 텍스트 게이트 통과 근거로 쓰이므로
        #   기록해야 원장이 완전하다). 실제 기록은 전이·settle 게이트를 통과해 done이 확정된 뒤
        #   (wlock 안)에만 — 조기 기록은 거부된 재진입에도 감사가 오염된다.
        skip_audit_pending = (bool(skip_text) and not art_valid)
    settle_override_note = None
    if a.status == "done" and task.get("owner"):
        if getattr(a, "settled_override", None):
            settle_override_note = a.settled_override
        else:
            ok, why = _completion_settled(task["owner"])
            if not ok:
                print(json.dumps({"id": a.id, "status_denied": "done", "reason": why,
                                  "hint": "안착 후 재시도 또는 --settled-override '<사유>'"},
                                 ensure_ascii=False), file=sys.stderr)
                return EXIT_BLOCKED
    with _wlock(a.id):  # ★WP-8(P-ORCH): JSON read-modify-write 직렬화 — 동시 mutator lost-update 차단
        task = _read_task(a.id)          # 락 안 재독 — 대기 중 갱신 반영
        if not task:
            print(f"not found: {a.id}", file=sys.stderr)
            return EXIT_NOTFOUND
        # W2-1 전이표(T0a 이식): 터미널 전이는 in_progress|in_review에서만. 락 안 재독 상태 기준 —
        # 어떤 변이보다 먼저 검사(거부 시 무변이 반환). CYS_TASK_TRANSITION_GATE=warn|off 안전밸브.
        if a.status in TERMINAL_STATUSES and task.get("status") not in TERMINAL_FROM:
            tmode = os.environ.get("CYS_TASK_TRANSITION_GATE", "strict")
            if tmode == "strict":
                print(f"transition denied(2): {task.get('status')}→{a.status} — "
                      f"터미널 전이는 {'|'.join(TERMINAL_FROM)}에서만", file=sys.stderr)
                return EXIT_USAGE
            if tmode != "off":
                print(f"transition warn: {task.get('status')}→{a.status} — "
                      "터미널 전이는 in_progress|in_review에서 권장(전이표 W2-1)", file=sys.stderr)
        if skip_audit_pending:  # E1 R3: 전이 게이트 통과·done 확정 시점에만 append(재진입 오염 차단)
            _append_jsonl(_skip_audit_path(), {"ts": _now(), "task": a.id, "reason": skip_text})
        if guard_bypass_rec is not None:  # E1-1(R-08): 안전밸브 사용 원장 — skip 감사와 동일 규약
            _append_jsonl(_guard_bypass_audit_path(),
                          dict(guard_bypass_rec, ts=_now(), task=a.id))
        # AA33(b)① 인용 기록 의무의 저장 표면 — 킬 지표 계수(②)와 §7.4(b) 철회 역추적이
        #   읽는 원천이다(radio 측 _retract_backtrace 가 task.refs 를 스캔한다).
        _refs = [x.strip() for x in (getattr(a, "refs", None) or "").split(",") if x.strip()]
        if _refs:
            merged = list(task.get("refs") or [])
            for x in _refs:
                if x not in merged:
                    merged.append(x)
            task["refs"] = merged
        if ev_text or skip_text:
            task["evidence"] = {"type": "evidence" if ev_text else "skip",
                                "text": ev_text or skip_text, "at": _now()}
        if artifact_records:  # E1: 비파괴 확장 — 기존 evidence(text/skip)와 공존
            task.setdefault("evidence", {})["artifacts"] = artifact_records
        if a.status == "done" and guard_deferred:
            # B2(조건 20②): 'verify-deferred' 라벨 자동 병기 — 미검증 done 의 원장 가시화.
            ev = task.setdefault("evidence", {})
            ev["verify_deferred"] = True
            if isinstance(ev.get("text"), str) and ev["text"]:
                if "[verify-deferred]" not in ev["text"]:
                    ev["text"] += " [verify-deferred]"
            else:
                ev["text"] = "[verify-deferred]"
                ev.setdefault("type", "evidence")
                ev.setdefault("at", _now())
            if guard_bypass_rec is not None:  # E1-1(R-08): 태스크 레코드에도 우회 사실 병기
                ev["guard_gate_bypass"] = dict(guard_bypass_rec, at=_now())
        if settle_override_note is not None:
            task.setdefault("settle_overrides", []).append(
                {"at": _now(), "reason": settle_override_note})
        task["status"] = a.status
        task["updated_at"] = _now()
        _write_json_atomic(_task_path(a.id), task)
        if a.status in TERMINAL_STATUSES:
            _release_lock(a.id, task.get("owner") or "", force=True)
            task["owner"] = None
            _write_json_atomic(_task_path(a.id), task)
        result = {"id": a.id, "status": a.status}
        if task.get("evidence"):
            result["evidence"] = task["evidence"]
        if a.status == "done":
            result["unblocked"] = _find_unblocked_dependents(a.id)  # → javis_wakeup enqueue 대상
            # W1-2 리마인더 + T2 전사통계 배선 — 비차단(exit 불변·연성 의존: 도구 부재 시 그냥 생략)
            print("handoff: _round/handoffs/%s-done.md 5필드 기록 권장(HANDOFF_CONTRACT)" % a.id,
                  file=sys.stderr)
            print("evidence에 전사 통계 첨부 권장: javis_transcript_stats.py --latest --oneline "
                  "(도구 부재 시 생략 — 비차단)", file=sys.stderr)
    if a.status in TERMINAL_STATUSES:
        _remove_guard_claims(a.id)  # B1(§2-1): 터미널 전이 = 바인딩 소거(wlock 밖·best-effort)
    # P1b 우회 가시화(§2.2-2): done 확정 후에만 probe 원장에 감사 행(skip-reason·gate-off) append.
    #   E1 skip_audit.jsonl(위)과 상보적 — 비차단·wlock 밖. mode 는 done 분기에서 정의된다.
    if a.status == "done":
        if skip_text:
            _append_audit(a.id, "skip-reason")
        if mode != "strict":
            _append_audit(a.id, "gate-off")
    print(json.dumps(result, ensure_ascii=False))
    return EXIT_OK


# ─────────────────────────────────────────────────────────────────────────────
# B2(§3 · 조건 07①·30③): verify cmd 위험 어휘 4종 — 등록 직전 게이트의 어휘 정의.
#   삭제·네트워크 = risk.class=high 승격 + 인간 서명(--signed-by) 의무.
#   서버기동·push생산 = 서명으로도 불허(고정 거부) — verify 의 멱등·읽기전용 계약 위반.
#   서버기동 판정은 javis_resource_gate SERVER_PATTERNS/_count_matching import 재사용
#   (복제 금지 — 조건 04③·07①의 "SERVER_PATTERNS 재사용" 문언 그대로).
#   ※\bapproval\b 는 브리프 어휘 문언 그대로 — 읽기전용 승인 '조회' cmd 오탐 가능성은
#   고정 거부의 보수측 비용으로 수용(등록 시점 게이트라 master 가 cmd 재작성으로 해소).
#   ★G4(b2-bugs MEDIUM): 대소문자 무시(re.IGNORECASE) — `RM -rf`·`CURL …` 류 케이스 변조
#   우회 차단(서버기동 판정은 javis_resource_gate SERVER_PATTERNS 소관 유지 — 여기서 불변).
#   ★미검출 잔여 경계 명문(G4 자인 — 이 게이트는 어휘 표면 검사일 뿐 의미 분석이 아니다):
#     `python -c "os.remove(...)"`·`python -c "shutil.move(...)"` 등 인터프리터 경유 삭제,
#     `git reset --hard`·`git checkout -- .` 등 워킹트리 파괴, `truncate`·`dd of=`·`> file`
#     리다이렉션 덮어쓰기, `find -exec rm` 변형, base64/eval 난독은 검출하지 않는다 —
#     잔여 위험은 인간 서명+리뷰어 ACCEPT(조건 30③)와 guard 실행격리(setsid·killpg)가 흡수.
# ─────────────────────────────────────────────────────────────────────────────
_RISK_DELETE_RE = re.compile(
    r"\brm\b|\brmdir\b|\bunlink\b|shutil\.rmtree|\bgit\s+clean\b|\bfind\b[^|;&]*\s-delete\b",
    re.IGNORECASE)
_RISK_NET_RE = re.compile(
    r"\bcurl\b|\bwget\b|\bnc\b|\bnetcat\b|\bssh\b|\bscp\b|\brsync\b",
    re.IGNORECASE)
_RISK_PUSH_RE = re.compile(
    r"\bcys\s+send\b|\bfeed\s+push\b|\bwakeup\b[^|;&]*\benqueue\b"
    r"|javis_wakeup(?:\.py)?\b[^|;&]*\benqueue\b|javis_approval\w*|\bcys\s+approve\b"
    r"|\bapproval\b",
    re.IGNORECASE)


def _load_guard_obj(task_id):
    """<id>.guard.json 판독 — 부재·손상·비객체 = None(무간섭 — 회귀 0)."""
    with contextlib.suppress(OSError, ValueError):
        with open(os.path.join(TASKS_DIR, "%s.guard.json" % task_id),
                  encoding="utf-8") as f:
            obj = json.load(f)
        if isinstance(obj, dict):
            return obj
    return None


def _guard_unresolved(gobj):
    """B2 G1(b2-bugs HIGH): 미해소 guard 판정 — **내구 신호 합성**.
    last_verdict 단독 판정은 VERIFY_FAIL 후 SKIPPED_*·NO_BLOCK_PASS 1회(clear 사이클마다
    발생하는 창)로 잔존이 마스킹돼 고위험 done 차단이 우회된다. deferred·block_count·
    last_sig 는 guard §2-4 계약상 **PASS 로만 리셋**되는 내구 필드 — 넷 중 하나라도 잔존이면
    미해소다."""
    if not isinstance(gobj, dict):
        return False
    bc = gobj.get("block_count")
    bc_pos = isinstance(bc, (int, float)) and not isinstance(bc, bool) and bc > 0
    return (gobj.get("deferred") is True
            or gobj.get("last_verdict") == "VERIFY_FAIL"
            or bc_pos
            or gobj.get("last_sig") is not None)


def _guard_signal_gate_mode():
    """E1-1(R-08): done-deferred 게이트 안전밸브 모드 — strict(기본)|warn|off.

    evidence 게이트 env 3종(`CYS_TASK_EVIDENCE_GATE`·`CYS_TASK_EVIDENCE_ARTIFACT_GATE`·
    `CYS_TASK_TRANSITION_GATE`)과 **같은 문법·문체**다. 미지 값은 기본(strict) — 오타가
    게이트를 조용히 여는 창을 만들지 않는다(안전측 고정).
      · strict = 종전 동작(고위험 잔존 = exit 4 차단)
      · warn   = 고위험도 통과 + 'verify-deferred' 라벨 + 우회 원장
      · off    = 잔존 검사 자체를 생략(라벨·원장은 유지)
    alias `CYS_TASK_GUARD_DEFERRED_GATE` — 대장(R-08 해소 조건) 문면과 OT 롤백 표가 이 이름을
    쓰므로 둘 다 받는다(주 이름이 우선 · 둘 다 있으면 주 이름 승).
    """
    raw = (os.environ.get("CYS_TASK_GUARD_SIGNAL_GATE")
           or os.environ.get("CYS_TASK_GUARD_DEFERRED_GATE") or "").strip().lower()
    return raw if raw in ("strict", "warn", "off") else "strict"


def _guard_disarmed_reason(task_id):
    """`<id>.guard-disarmed` 마커 판독 → 사유 1줄(부재 = None).

    회로차단기가 열린 태스크는 guard 가 **더 이상 발동하지 않으므로** PASS 로 잔존 신호를
    리셋할 방법이 없다(재무장 = master 의 수동 마커 삭제). 그 상태에서 고위험 done 을
    exit 4 로 계속 막는 것은 '자동 자가치유 없음 + done 영구 차단' 이중 잠금이다(R-08) —
    따라서 strict 라도 **warn 으로 자동 강등**하고 사유를 원장에 남긴다.
    """
    p = os.path.join(TASKS_DIR, "%s.guard-disarmed" % task_id)
    try:
        with open(p, encoding="utf-8") as f:
            head = (f.readline() or "").strip()
    except OSError:
        return None
    return head[:200] or "사유 미기재"


def _guard_bypass_audit_path():
    return os.path.join(ROOT, "_round", "evidence", "guard_bypass_audit.jsonl")


def _guard_signals(gobj):
    """미해소 판정 근거 문면(stderr 고지·차단 사유 공용)."""
    return ("deferred=%s·last_verdict=%s·block_count=%s·last_sig=%s"
            % (gobj.get("deferred"), gobj.get("last_verdict"),
               gobj.get("block_count"), gobj.get("last_sig")))


def _risk_scan_cmd(cmd_text):
    """(categories:set, err|None) — cmd 문자열의 위험 어휘 4종 분류.
    서버기동은 _count_matching 이 ps 형식('pid command') 줄을 기대하므로 합성 pid 를 접두해
    분류기(제외 패턴 포함)를 그대로 태운다. import 실패(버전 찢김)는 (None, 사유) —
    master-facing 등록 경로라 fail-closed(등록 거부)가 계약(조건 18② 비대칭 원칙의 반대편:
    fail-closed 는 master-facing 에만)."""
    cats = set()
    if _RISK_DELETE_RE.search(cmd_text):
        cats.add("삭제")
    if _RISK_NET_RE.search(cmd_text):
        cats.add("네트워크")
    if _RISK_PUSH_RE.search(cmd_text):
        cats.add("push생산")
    try:
        import javis_resource_gate as _rg  # 형제 모듈 — 경로 가드(sys.path append) 뒤 지역 import
        if _rg._count_matching(["99999 " + cmd_text.replace("\n", " ")],
                               _rg.SERVER_PATTERNS, _rg.SERVER_EXCLUDE_PATTERNS) > 0:
            cats.add("서버기동")
    except Exception as e:  # noqa: BLE001 — import·분류 실패 = 검사 불가(fail-closed 소비)
        return None, "javis_resource_gate import/분류 실패(버전 찢김 의심): %s" % e
    return cats, None


def cmd_set_verify_spec(a):
    """T1(§1-2): verify_spec 등록 — 스키마 검증 + `with _wlock(id)` **필수**(R1-contract:
    set-status·checkout·release·create 에 이은 5번째 mutator — lost-update 차단).
    B2(§3): T1 이 예약한 훅 지점 실구현 — 위험 어휘 4종 검사(서버기동·push생산=고정 거부 /
    삭제·네트워크=high 승격+--signed-by 서명 의무) + --demote-calibrated 내부 경로(조건 07②)."""
    task = _read_task(a.id)
    if not task:
        print(f"not found: {a.id}", file=sys.stderr)
        return EXIT_NOTFOUND
    # ── B2(조건 07②): --demote-calibrated — guard timeout_violations≥2 소비 경로.
    #    wlock 직렬화(6번째 mutator 진입점이 아니라 set-verify-spec 의 내부 경로) · 멱등:
    #    calibrated=true 일 때만 변이·이미 false/부재는 무변이 보고. ──
    if getattr(a, "demote_calibrated", False):
        with _wlock(a.id):
            task = _read_task(a.id)          # 락 안 재독 — 대기 중 갱신 반영
            if not task:
                print(f"not found: {a.id}", file=sys.stderr)
                return EXIT_NOTFOUND
            spec = task.get("verify_spec")
            if not isinstance(spec, dict) or not spec:
                print("error: verify_spec 부재 — calibrated 강등 대상 없음", file=sys.stderr)
                return EXIT_USAGE
            was_true = spec.get("calibrated") is True
            if was_true:
                spec["calibrated"] = False
                # G7d: 강등 사실을 spec 자체에 병기 — checkout 경고가 guard.json(휘발 사이드카)
                # 소실과 무관하게 이 필드로 발화한다(강등 근거의 영속화).
                spec["demoted_at"] = _now()
                task["updated_at"] = _now()
                _write_json_atomic(_task_path(a.id), task)
                print("calibrated 강등(조건 07②): quick wall-clock 상한 위반 반복(guard "
                      "timeout_violations≥2) — 재캘리브레이션(하네스 워커 calibrate + master "
                      "봉인) 전까지 신뢰 강등", file=sys.stderr)
        print(json.dumps({"id": a.id, "calibrated": False, "demoted": was_true},
                         ensure_ascii=False))
        return EXIT_OK
    if a.file:
        try:
            with open(a.file, encoding="utf-8") as f:
                text = f.read()
        except OSError as e:
            print("error: --file 읽기 실패: %s (%s)" % (a.file, e), file=sys.stderr)
            return EXIT_USAGE
    else:
        text = a.json
    try:
        spec = json.loads(text)
    except ValueError as e:
        print("error: verify_spec JSON 파싱 실패: %s" % e, file=sys.stderr)
        return EXIT_USAGE
    if not isinstance(spec, dict):
        print("error: verify_spec 은 JSON 객체여야 함", file=sys.stderr)
        return EXIT_USAGE
    spec.setdefault("schema_version", 1)
    # ── risk 필드 예약(T1): class="auto" 는 '부재 시에만' 주입 — B5: 비-dict 입력은 무언 치환
    #    없이 exit 2 거부(어휘 검사·signed_by 전이는 Wave B2 몫) ──
    risk = spec.get("risk")
    if risk is None:
        spec["risk"] = {"class": "auto", "signed_by": None, "signed_at": None}
    elif not isinstance(risk, dict):
        print("error: risk 는 객체({class, signed_by, signed_at})여야 함 — 비-dict 입력 거부"
              "(무언 치환 금지 · B5)", file=sys.stderr)
        return EXIT_USAGE
    else:
        risk.setdefault("class", "auto")
        risk.setdefault("signed_by", None)
        risk.setdefault("signed_at", None)
    # ── B2(조건 07①·30③): 위험 어휘 4종 검사 — 등록 직전 게이트(T1 예약 지점 실구현) ──
    cmd_text = spec.get("cmd") if isinstance(spec.get("cmd"), str) else ""
    if cmd_text.strip():
        cats, scan_err = _risk_scan_cmd(cmd_text)
        if scan_err:
            print("error: 위험 어휘 검사 불가 — %s: 등록 거부(master-facing fail-closed — "
                  "팩 설치 정합 복구 후 재시도)" % scan_err, file=sys.stderr)
            return EXIT_USAGE
        hard = cats & {"서버기동", "push생산"}
        if hard:
            print("verify-cmd rejected(2): %s 어휘 감지 — 서명 불문 고정 거부(조건 07①): "
                  "verify 는 멱등·읽기전용이어야 한다(서버기동=자원 거버넌스 침식·push생산="
                  "큐 남발 점화원). cmd 를 읽기전용 검증으로 재작성하라."
                  % "·".join(sorted(hard)), file=sys.stderr)
            return EXIT_USAGE
        soft = cats & {"삭제", "네트워크"}
        if soft:
            if not getattr(a, "signed_by", None):
                print("verify-cmd high-risk(2): %s 어휘 감지 — risk.class=high 승격 대상: "
                      "인간 서명+리뷰어 ACCEPT 필요. `--signed-by <approver>` 로 서명 재등록"
                      "하라(서명 전 리뷰어 ACCEPT 의무는 계약 문서 조항 · 조건 30③ — 도구는 "
                      "서명 문자열만 집행하며 자기신고 한계를 자인한다)."
                      % "·".join(sorted(soft)), file=sys.stderr)
                return EXIT_USAGE
            spec["risk"]["class"] = "high"
    if getattr(a, "signed_by", None):
        spec["risk"]["signed_by"] = a.signed_by
        spec["risk"]["signed_at"] = _now()
    # ── B2 G2(성찰 2 MEDIUM): 무서명 재등록 우회 차단 — 기존 상태 참조 게이트.
    #    세탁 시나리오: VERIFY_FAIL 잔존/high 서명 spec 을 무해 cmd 로 갈아끼워 done-deferred
    #    차단(G1)·서명 의무를 우회하는 경로. 트리거 = ①미해소 guard 상태(G1 내구 신호 합성
    #    동일 기준) ②기존 spec risk.class=high 교체. --force-respec 명시 없으면 exit 2 거부 /
    #    있으면 통과하되 task-audit 원장 1줄(probe_runs task-audit 확장 전례 — _append_audit
    #    재사용)로 감사 가시화. 어휘 게이트(위) '이후' 배치 — 새 spec 자체의 위험 판정이
    #    항상 선행한다(고정 거부·서명 의무는 force 로도 우회 불가). ──
    respec_forced = False
    _g2_reasons = []
    _gobj_pre = _load_guard_obj(a.id)
    if _guard_unresolved(_gobj_pre):
        _g2_reasons.append("미해소 guard 상태(%s)" % _guard_signals(_gobj_pre))
    _old_spec = task.get("verify_spec")
    if isinstance(_old_spec, dict):
        _old_risk = _old_spec.get("risk")
        if isinstance(_old_risk, dict) and _old_risk.get("class") == "high":
            _g2_reasons.append("기존 spec risk.class=high 교체")
    if _g2_reasons:
        if not getattr(a, "force_respec", False):
            print("respec blocked(2): %s — 재등록이 검증 잔존·서명 의무를 세탁하는 우회 경로일 "
                  "수 있다(G2): 해소(guard PASS) 후 재등록하거나, 의도된 교체면 "
                  "`--force-respec` 명시(task-audit 원장 기록 동반)로만 허용."
                  % " ∧ ".join(_g2_reasons), file=sys.stderr)
            return EXIT_USAGE
        respec_forced = True
    errs = validate_verify_spec(spec)
    if errs:
        for e in errs:
            print("[SCHEMA] %s" % e, file=sys.stderr)
        print("error: verify_spec 스키마 위반 %d건 — 등록 거부(schemas/verify_spec_schema.json)"
              % len(errs), file=sys.stderr)
        return EXIT_USAGE
    # F6(fail-open 경고): probe.args 는 actprobe 서브커맨드 **플래그 그대로** 전달된다
    #   (예: ["--path","<p>"] — 위치 인자 아님). 플래그형 아님 감지 시 stderr 경고만·등록 진행.
    if spec.get("mode") == "probe":
        _pargs = (spec.get("probe") or {}).get("args") or []
        if _pargs and not str(_pargs[0]).startswith("-"):
            print("warn: probe.args 형식 — args 는 actprobe 서브커맨드 플래그 그대로여야 함"
                  '(예: ["--path","<p>"]) · args[0]=%r 이 플래그형 아님(fail-open — 등록 진행)'
                  % str(_pargs[0]), file=sys.stderr)
    with _wlock(a.id):  # ★R1-contract: 5번째 mutator 직렬화 — 동시 set-status lost-update 차단
        task = _read_task(a.id)          # 락 안 재독 — 대기 중 갱신 반영
        if not task:
            print(f"not found: {a.id}", file=sys.stderr)
            return EXIT_NOTFOUND
        task["verify_spec"] = spec
        task["updated_at"] = _now()
        _write_json_atomic(_task_path(a.id), task)
    if respec_forced:
        _append_audit(a.id, "force-respec")  # G2: 강제 재등록 원장 1줄(우회 가시화 — §3 mine 소비)
    with contextlib.suppress(OSError):
        os.remove(_verify_notified_marker(a.id))  # A4: 재위임 사이클 재개 — 통지 마커 해제
    print(json.dumps({"id": a.id, "verify_spec_mode": spec.get("mode"),
                      "risk_class": spec["risk"]["class"]}, ensure_ascii=False))
    return EXIT_OK


def cmd_ready(a):
    task = _read_task(a.id)
    if not task:
        print(f"not found: {a.id}", file=sys.stderr)
        return EXIT_NOTFOUND
    unresolved = _unresolved_blockers(task)
    print(json.dumps({"id": a.id, "ready": not unresolved, "unresolved": unresolved},
                     ensure_ascii=False))
    return EXIT_OK if not unresolved else EXIT_BLOCKED


def cmd_show(a):
    task = _read_task(a.id)
    if not task:
        print(f"not found: {a.id}", file=sys.stderr)
        return EXIT_NOTFOUND
    task["_lock_holder"] = _read_owner(a.id)
    print(json.dumps(task, ensure_ascii=False, indent=1))
    return EXIT_OK


def cmd_list(a):
    rows = _list_tasks()
    if a.open_only:
        rows = [t for t in rows if t.get("status") in OPEN_STATUSES]
    print(json.dumps(rows, ensure_ascii=False, indent=1))
    return EXIT_OK


def cmd_self_test(args):
    """E1 evidence-artifact 게이트 밀폐 자기검증 — subprocess로 실제 CLI(exit code)를 구동한다.
    ★결함11호: 모든 호출은 tmpdir + JAVIS_ROOT 주입으로 밀폐 — 실장부(_round/tasks/) 접촉 금지.
    커버리지(설계 §E1 시험 목록): 부정 4·긍정 1·폴백 1·skip 감사 1·재진입·warn/off 경계·하위호환."""
    import subprocess
    import tempfile

    self_path = os.path.abspath(__file__)

    def run(root, argv, env_extra=None):
        env = dict(os.environ)
        env["JAVIS_ROOT"] = root
        # B1 밀폐: ambient surface id(cys pane 안에서 self-test 실행 시)가 guard-claim 을
        # 만들어 케이스 간 상태를 오염시키지 않게 기본 제거 — claim 케이스만 env_extra 로 주입.
        env.pop("CYS_SURFACE_ID", None)
        env.pop("AITERM_SURFACE_ID", None)
        env.setdefault("JAVIS_TASK_LIVENESS", "alive")  # 체크아웃 락 생존 결정론
        # P1b 밀폐: probe 원장 감사 행(skip-reason·gate-off)이 라이브 pack/state 로 새지 않도록
        #   영수증 경로·caller 를 tmpdir 로 고정(라이브 미접촉 · cys identify 서브프로세스 회피).
        env.setdefault("CYS_PROBE_RUNS", os.path.join(root, "probe_runs.jsonl"))
        env.setdefault("CYS_ACTPROBE_CALLER", "self-test")
        # T1 밀폐: 게이트 모드는 기본 warn 으로 '핀'(ambient JAVIS_VERIFY_GATE=strict 누출 차단),
        #   거부 통지의 drain 이 라이브 cys 를 부르지 않도록 LIVENESS=dead 핀(dead=zombie 가드가
        #   cys 호출 없이 pending 종결). 게이트 케이스는 env_extra 로 명시 오버라이드한다.
        env["JAVIS_VERIFY_GATE"] = "warn"
        env["JAVIS_WAKEUP_LIVENESS"] = "dead"
        env["CYS_NO_AUTOSTART"] = "1"
        if env_extra:
            env.update(env_extra)
        r = subprocess.run([sys.executable, self_path] + argv,
                           capture_output=True, text=True, env=env)
        return r.returncode, r.stdout, r.stderr

    def read_task(root, tid):
        with open(os.path.join(root, "_round", "tasks", tid + ".json"), encoding="utf-8") as f:
            return json.load(f)

    def mkfile(root, name, content="ok evidence\n"):
        p = os.path.join(root, name)
        with open(p, "w", encoding="utf-8") as f:
            f.write(content)
        return p

    def setup_checked_out(root, tid):
        """create + checkout → owner.json(acquired_at) 생성·status in_progress."""
        rc, _, e = run(root, ["create", tid, "--id", tid])
        assert rc == 0, "create 실패(%s): %s" % (tid, e)
        rc, _, e = run(root, ["checkout", tid, "--owner", "w1"])
        assert rc == 0, "checkout 실패(%s): %s" % (tid, e)

    OV = ["--settled-override", "self-test"]  # 체크아웃된 태스크의 완료 하한선(settle) 우회
    try:
        # ── A3 parity: 스키마 파일 SOT ↔ _VS_INLINE 드리프트 0 + manifest CHECK_KINDS 핀 ──
        with open(_vs_schema_path(), encoding="utf-8") as f:
            _schema_doc = json.load(f)
        for k in _VS_INLINE:
            assert _schema_doc.get(k) == _VS_INLINE[k], \
                "스키마 파일↔_VS_INLINE 드리프트(%s): %r != %r" % (k, _schema_doc.get(k),
                                                                  _VS_INLINE[k])
        import javis_manifest as _mf
        assert tuple(_VS_INLINE["CRITERIA_KIND_ENUM"]) == tuple(_mf.CHECK_KINDS), \
            "CRITERIA_KIND_ENUM ≠ javis_manifest.CHECK_KINDS: %r vs %r" % (
                _VS_INLINE["CRITERIA_KIND_ENUM"], _mf.CHECK_KINDS)

        with tempfile.TemporaryDirectory(prefix="javis-task-e1-") as root:
            # ── 부정 1: 부재 경로 → exit 5 ──
            setup_checked_out(root, "Tneg-missing")
            rc, _, _ = run(root, ["set-status", "Tneg-missing", "done",
                                  "--evidence-artifact", os.path.join(root, "nope.txt")] + OV)
            assert rc == EXIT_NO_EVIDENCE, "부재 경로가 5를 안 냄: %s" % rc

            # ── 부정 2: 빈 파일 → exit 5 ──
            setup_checked_out(root, "Tneg-empty")
            empty = mkfile(root, "empty.txt", "")
            rc, _, _ = run(root, ["set-status", "Tneg-empty", "done",
                                  "--evidence-artifact", empty] + OV)
            assert rc == EXIT_NO_EVIDENCE, "빈 파일이 5를 안 냄: %s" % rc

            # ── 부정 3: acquired_at 이전 mtime(신선도 실패) → exit 5 ──
            setup_checked_out(root, "Tneg-stale")
            stale = mkfile(root, "stale.txt", "old\n")
            os.utime(stale, (time.time() - 3600, time.time() - 3600))  # 체크아웃보다 1h 전
            rc, _, err = run(root, ["set-status", "Tneg-stale", "done",
                                    "--evidence-artifact", stale] + OV)
            assert rc == EXIT_NO_EVIDENCE, "신선도 실패가 5를 안 냄: %s (%s)" % (rc, err)
            assert "신선도" in err, "신선도 사유 메시지 누락: %s" % err

            # ── 부정 4: strict에서 artifact·skip 모두 부재(텍스트 evidence만) → exit 5 ──
            setup_checked_out(root, "Tneg-noart")
            rc, _, _ = run(root, ["set-status", "Tneg-noart", "done",
                                  "--evidence", "pytest 31/31 PASS"] + OV)
            assert rc == EXIT_NO_EVIDENCE, "텍스트만(artifact/skip 부재)이 5를 안 냄: %s" % rc

            # ── 긍정: 유효 artifact → exit 0 + artifacts 기록(sha·anchor) ──
            setup_checked_out(root, "Tpos")
            good = mkfile(root, "good.txt", "verified ok\n")
            rc, out, e = run(root, ["set-status", "Tpos", "done",
                                    "--evidence-artifact", good] + OV)
            assert rc == EXIT_OK, "유효 artifact가 0을 안 냄: %s (%s)" % (rc, e)
            arts = read_task(root, "Tpos")["evidence"]["artifacts"]
            assert len(arts) == 1 and arts[0]["size"] > 0, "artifacts 기록 누락: %s" % arts
            assert len(arts[0]["sha256_8"]) == 8, "sha256_8 형식 오류: %s" % arts[0]
            assert arts[0]["anchor"] == "acquired_at", "체크아웃 태스크 anchor≠acquired_at: %s" % arts[0]

            # ── 폴백: owner 없는(체크아웃 없는) 태스크 + created_at 이후 파일 → 0 + anchor 폴백 ──
            rc, _, e = run(root, ["create", "Tfb", "--id", "Tfb", "--status", "in_progress"])
            assert rc == 0, "폴백 create 실패: %s" % e
            fb = mkfile(root, "fb.txt", "fallback ok\n")
            rc, out, e = run(root, ["set-status", "Tfb", "done", "--evidence-artifact", fb])
            assert rc == EXIT_OK, "폴백 done이 0을 안 냄: %s (%s)" % (rc, e)
            fbart = read_task(root, "Tfb")["evidence"]["artifacts"][0]
            assert fbart["anchor"] == "created_at-fallback", "폴백 anchor 오기록: %s" % fbart

            # ── skip 감사: --skip-reason → 0 + skip_audit.jsonl 1줄 append ──
            rc, _, e = run(root, ["create", "Tskip", "--id", "Tskip", "--status", "in_progress"])
            assert rc == 0, "skip create 실패: %s" % e
            rc, _, e = run(root, ["set-status", "Tskip", "done",
                                  "--skip-reason", "CI에서 재현 불가"])
            assert rc == EXIT_OK, "skip done이 0을 안 냄: %s (%s)" % (rc, e)
            audit = os.path.join(root, "_round", "evidence", "skip_audit.jsonl")
            with open(audit, encoding="utf-8") as f:
                lines = [ln for ln in f.read().splitlines() if ln.strip()]
            assert len(lines) == 1, "skip_audit 줄 수≠1: %s" % lines
            rec = json.loads(lines[0])
            assert rec["task"] == "Tskip" and rec["reason"] == "CI에서 재현 불가", "skip 감사 내용 오류: %s" % rec

            # ── 재진입: done 성공 후 재시도는 터미널 전이 게이트로 안전 거부(exit 2)·기록 무중복 ──
            rc, _, _ = run(root, ["set-status", "Tskip", "done",
                                  "--skip-reason", "재시도"])
            assert rc == EXIT_USAGE, "터미널 재진입이 2를 안 냄: %s" % rc
            with open(audit, encoding="utf-8") as f:
                lines2 = [ln for ln in f.read().splitlines() if ln.strip()]
            assert len(lines2) == 1, "재진입이 skip 감사에 중복 기록: %s" % lines2

            # ── 경계 warn: 증거 전무라도 warn 모드는 통과(exit 0) + 경고 출력 ──
            rc, _, e = run(root, ["create", "Twarn", "--id", "Twarn", "--status", "in_progress"])
            assert rc == 0
            rc, _, err = run(root, ["set-status", "Twarn", "done"],
                             {"CYS_TASK_EVIDENCE_ARTIFACT_GATE": "warn",
                              "CYS_TASK_EVIDENCE_GATE": "warn"})
            assert rc == EXIT_OK, "warn 모드가 0을 안 냄: %s (%s)" % (rc, err)
            assert "warn" in err, "warn 경고 메시지 누락: %s" % err

            # ── 경계 off: 게이트 완전 생략(exit 0)·artifacts 미기록 ──
            rc, _, e = run(root, ["create", "Toff", "--id", "Toff", "--status", "in_progress"])
            assert rc == 0
            rc, _, e = run(root, ["set-status", "Toff", "done"],
                           {"CYS_TASK_EVIDENCE_ARTIFACT_GATE": "off",
                            "CYS_TASK_EVIDENCE_GATE": "off"})
            assert rc == EXIT_OK, "off 모드가 0을 안 냄: %s (%s)" % (rc, e)
            assert "artifacts" not in read_task(root, "Toff").get("evidence", {}), "off인데 artifacts 기록됨"

            # ── off 밸브 skip 감사(어태커 결함1): artifact 게이트 off라도 --skip-reason이 done 통과
            #    근거이면 skip_audit.jsonl에 기록돼 원장이 완전해야 한다 ──
            rc, _, e = run(root, ["create", "Toffskip", "--id", "Toffskip", "--status", "in_progress"])
            assert rc == 0
            rc, _, e = run(root, ["set-status", "Toffskip", "done", "--skip-reason", "off 밸브 skip"],
                           {"CYS_TASK_EVIDENCE_ARTIFACT_GATE": "off"})
            assert rc == EXIT_OK, "off+skip done이 0을 안 냄: %s (%s)" % (rc, e)
            with open(audit, encoding="utf-8") as f:
                offlines = [json.loads(ln) for ln in f.read().splitlines() if ln.strip()]
            assert any(r["task"] == "Toffskip" and r["reason"] == "off 밸브 skip" for r in offlines), \
                "off 밸브에서 skip 감사가 누락됨(원장 불완전): %s" % offlines

            # ── 하위호환 A: --evidence 텍스트만(artifact 게이트 off) → 무파손(evidence.text 보존) ──
            rc, _, e = run(root, ["create", "Tbc1", "--id", "Tbc1", "--status", "in_progress"])
            assert rc == 0
            rc, _, e = run(root, ["set-status", "Tbc1", "done", "--evidence", "구형 텍스트 증거 12345"],
                           {"CYS_TASK_EVIDENCE_ARTIFACT_GATE": "off"})
            assert rc == EXIT_OK, "하위호환(텍스트만) 무파손 실패: %s (%s)" % (rc, e)
            ev = read_task(root, "Tbc1")["evidence"]
            assert ev["text"] == "구형 텍스트 증거 12345" and "artifacts" not in ev, "구형 evidence 필드 손상: %s" % ev

            # ── 하위호환 B: --evidence 텍스트 + --evidence-artifact 공존(strict) → 둘 다 기록 ──
            setup_checked_out(root, "Tbc2")
            g2 = mkfile(root, "bc2.txt", "both ok\n")
            rc, _, e = run(root, ["set-status", "Tbc2", "done",
                                  "--evidence", "pytest 12/12 PASS",
                                  "--evidence-artifact", g2] + OV)
            assert rc == EXIT_OK, "텍스트+artifact 공존 실패: %s (%s)" % (rc, e)
            ev2 = read_task(root, "Tbc2")["evidence"]
            assert ev2.get("text") == "pytest 12/12 PASS" and len(ev2.get("artifacts", [])) == 1, \
                "텍스트+artifact 비파괴 공존 실패: %s" % ev2

        # ══ T1 verify_spec 게이트 배터리(§1-2 — 별도 밀폐 root) ══
        with tempfile.TemporaryDirectory(prefix="javis-task-t1-") as groot:
            tasks_dir = os.path.join(groot, "_round", "tasks")
            marker = os.path.join(tasks_dir, GATE_MARKER)

            def write_marker(epoch):
                os.makedirs(tasks_dir, exist_ok=True)
                with open(marker, "w", encoding="utf-8") as f:
                    f.write(time.strftime("%Y-%m-%dT%H:%M:%S%z", time.localtime(epoch)) + "\n")

            def backdate_created(tid, epoch):
                p = os.path.join(tasks_dir, tid + ".json")
                with open(p, encoding="utf-8") as f:
                    t = json.load(f)
                t["created_at"] = time.strftime("%Y-%m-%dT%H:%M:%S%z", time.localtime(epoch))
                with open(p, "w", encoding="utf-8") as f:
                    json.dump(t, f, ensure_ascii=False)

            # ── 게이트 warn(기본): 무spec checkout → exit 0 + WARN 경고 ──
            rc, _, e = run(groot, ["create", "Tg-warn", "--id", "Tg-warn"])
            assert rc == 0, "게이트 create 실패: %s" % e
            rc, _, err = run(groot, ["checkout", "Tg-warn", "--owner", "w1"])
            assert rc == EXIT_OK, "warn 모드 checkout 이 0 을 안 냄: %s (%s)" % (rc, err)
            assert "verify-gate WARN" in err, "warn 모드 경고 누락: %s" % err

            # ── 게이트 off: 무간섭(경고도 없음) ──
            rc, _, e = run(groot, ["create", "Tg-off", "--id", "Tg-off"])
            assert rc == 0
            rc, _, err = run(groot, ["checkout", "Tg-off", "--owner", "w1"],
                             {"JAVIS_VERIFY_GATE": "off"})
            assert rc == EXIT_OK and "verify-gate" not in err, \
                "off 모드 무간섭 위반: rc=%s err=%s" % (rc, err)

            # ── 게이트 strict: 마커(과거)보다 늦게 생성된 무spec 태스크 → exit 6 + 문면 ──
            write_marker(time.time() - 3600)  # strict 승격이 1시간 전에 집행된 상태를 조성
            rc, _, e = run(groot, ["create", "Tg-strict", "--id", "Tg-strict"])
            assert rc == 0
            rc, _, err = run(groot, ["checkout", "Tg-strict", "--owner", "w1"],
                             {"JAVIS_VERIFY_GATE": "strict"})
            assert rc == EXIT_VERIFY, "strict 무spec 이 6 을 안 냄: %s (%s)" % (rc, err)
            assert "verify-spec missing(6): 재시도 금지 — master가 verify_spec 작성 후 재위임" in err, \
                "strict 거부 문면 불일치: %s" % err

            # ── 거부 통지(A4): 1회차만 enqueue+마커 생성 — 반복 거부는 마커 창 억제(진짜
            #    '태스크당 1회'). pending 정확 1건(D17 — drain 전 스냅샷) + same-run drain
            #    배달 시도 흔적(cys 셔임 로그 · exit 1 → 배달실패 → pending 잔류). 이후
            #    set-verify-spec 성공이 마커를 삭제(재위임 사이클 재개).
            shim_dir = os.path.join(groot, "shim")
            os.makedirs(shim_dir, exist_ok=True)
            shim = os.path.join(shim_dir, "cys")
            with open(shim, "w", encoding="utf-8") as f:
                f.write('#!/bin/sh\necho "$@" >> "$0.log"\nexit 1\n')
            os.chmod(shim, 0o755)
            gate_env = {"JAVIS_VERIFY_GATE": "strict", "JAVIS_WAKEUP_LIVENESS": "alive",
                        "JAVIS_FASTFAIL_MAX": "99",
                        "PATH": shim_dir + os.pathsep + os.environ.get("PATH", "")}
            good_spec = {"mode": "command", "cmd": "python3 -c 0",
                         "pass_rule": {"kind": "exit_map", "pass_exits": [0]}}
            rc, _, e = run(groot, ["create", "Tg-notif", "--id", "Tg-notif"])
            assert rc == 0
            for _i in range(3):
                rc, _, _e = run(groot, ["checkout", "Tg-notif", "--owner", "w1"], gate_env)
                assert rc == EXIT_VERIFY
            pend_dir = os.path.join(groot, "_round", "wakeups", "pending")
            pend = [n for n in os.listdir(pend_dir)] if os.path.isdir(pend_dir) else []
            pend = [n for n in pend if n.endswith(".json")]
            assert len(pend) == 1, "동일 거부 반복 후 pending ≠ 1건: %s" % pend
            assert os.path.isfile(shim + ".log"), "same-run drain 배달 시도 흔적(셔임 로그) 없음"
            with open(shim + ".log", encoding="utf-8") as f:
                shim_lines = [ln for ln in f.read().splitlines() if ln.strip()]
            assert any("send" in ln for ln in shim_lines), "cys send 시도 미기록: %s" % shim_lines
            notif_mark = os.path.join(tasks_dir, "Tg-notif.verify-notified")
            assert os.path.isfile(notif_mark), "A4 통지 마커 미생성"
            with open(os.path.join(groot, "_round", "wakeups", "queue.jsonl"),
                      encoding="utf-8") as f:
                _ledger = f.read()
            _queued = [ln for ln in _ledger.splitlines()
                       if '"queued"' in ln and "verify-spec-missing-Tg-notif" in ln]
            assert len(_queued) == 1, "A4 마커 억제 실패 — enqueue %d회: %s" % (len(_queued), _queued)
            rc, _, e = run(groot, ["set-verify-spec", "Tg-notif", "--json",
                                   json.dumps(good_spec)])
            assert rc == EXIT_OK, "Tg-notif spec 등록 실패: %s" % e
            assert not os.path.isfile(notif_mark), "A4 set-verify-spec 성공 후 통지 마커 미삭제"

            # ── grandfather: created_at < 마커 시각 → WARN 통과 + grandfathered:true ──
            rc, _, e = run(groot, ["create", "Tg-gf", "--id", "Tg-gf"])
            assert rc == 0
            backdate_created("Tg-gf", time.time() - 7200)  # 마커(1h 전)보다 이른 2h 전
            rc, _, err = run(groot, ["checkout", "Tg-gf", "--owner", "w1"],
                             {"JAVIS_VERIFY_GATE": "strict"})
            assert rc == EXIT_OK, "grandfather 가 통과하지 않음: %s (%s)" % (rc, err)
            # B8: 괄호 교정 — WARN 문구는 무조건 필수 + grandfather 표기 or 연산은 괄호로 묶음
            assert "verify-gate WARN" in err and \
                ("grandfather" in err.lower() or "grandfathered" in err), \
                "grandfather WARN 문구 누락: %s" % err
            assert read_task(groot, "Tg-gf").get("grandfathered") is True, \
                "grandfathered:true 표식 누락"

            # ── set-verify-spec: 유효 spec 등록 → strict checkout 통과(무경고) ──
            rc, _, e = run(groot, ["create", "Tg-spec", "--id", "Tg-spec"])
            assert rc == 0
            rc, out, e = run(groot, ["set-verify-spec", "Tg-spec", "--json",
                                     json.dumps(good_spec)])
            assert rc == EXIT_OK, "set-verify-spec 유효 등록 실패: %s (%s)" % (rc, e)
            rec = read_task(groot, "Tg-spec")["verify_spec"]
            assert rec["mode"] == "command" and rec["risk"]["class"] == "auto", \
                "risk.class=auto 예약 기록 누락: %s" % rec
            rc, _, err = run(groot, ["checkout", "Tg-spec", "--owner", "w1"],
                             {"JAVIS_VERIFY_GATE": "strict"})
            assert rc == EXIT_OK and "verify-gate" not in err, \
                "spec 보유 strict checkout 실패: rc=%s err=%s" % (rc, err)

            # ── set-verify-spec 스키마 거부: mode 무효·미지 키 → exit 2 ──
            rc, _, err = run(groot, ["set-verify-spec", "Tg-spec", "--json",
                                     json.dumps({"mode": "vibes"})])
            assert rc == EXIT_USAGE and "mode 무효" in err, "무효 mode 미거부: %s (%s)" % (rc, err)
            rc, _, err = run(groot, ["set-verify-spec", "Tg-spec", "--json",
                                     json.dumps({"mode": "command", "cmd": "true",
                                                 "pass_rule": {"kind": "exit_map",
                                                               "pass_exits": [0]},
                                                 "score": 99})])
            assert rc == EXIT_USAGE, "금지 키(score) 미거부: %s (%s)" % (rc, err)
            # ── B5: risk 비-dict 입력 → exit 2 거부(무언 치환 제거·부재 시에만 auto 주입) ──
            rc, _, err = run(groot, ["set-verify-spec", "Tg-spec", "--json",
                                     json.dumps({"mode": "command", "cmd": "true",
                                                 "pass_rule": {"kind": "exit_map",
                                                               "pass_exits": [0]},
                                                 "risk": "high"})])
            assert rc == EXIT_USAGE and "risk" in err, \
                "B5 비-dict risk 미거부: %s (%s)" % (rc, err)

            # ── waiver 소비: 유효/유예(만료+2일)/유예 경과 3상 ──
            vibe = os.path.join(groot, ".vibecoding")
            os.makedirs(vibe, exist_ok=True)
            import datetime as _dt
            today = _dt.date.today()
            wrecs = [
                {"waiver_id": "WVR-ok", "target_rule": "r", "approver": "master", "risk": "low",
                 "expiry": (today + _dt.timedelta(days=3)).isoformat()},
                {"waiver_id": "WVR-grace", "target_rule": "r", "approver": "master", "risk": "low",
                 "expiry": (today - _dt.timedelta(days=1)).isoformat()},
                {"waiver_id": "WVR-dead", "target_rule": "r", "approver": "master", "risk": "low",
                 "expiry": (today - _dt.timedelta(days=5)).isoformat()},
            ]
            with open(os.path.join(vibe, "waivers.jsonl"), "w", encoding="utf-8") as f:
                for r in wrecs:
                    f.write(json.dumps(r, ensure_ascii=False) + "\n")
            for tid, ref, want_rc, want_sub in (
                    ("Tg-wv-ok", "WVR-ok", EXIT_OK, None),
                    ("Tg-wv-grace", "WVR-grace", EXIT_OK, "유예"),
                    ("Tg-wv-dead", "WVR-dead", EXIT_VERIFY, "유예 경과")):
                rc, _, e = run(groot, ["create", tid, "--id", tid])
                assert rc == 0
                rc, _, e = run(groot, ["set-verify-spec", tid, "--json",
                                       json.dumps({"mode": "waiver", "waiver_ref": ref})])
                assert rc == EXIT_OK, "waiver spec 등록 실패(%s): %s" % (tid, e)
                rc, _, err = run(groot, ["checkout", tid, "--owner", "w1"],
                                 {"JAVIS_VERIFY_GATE": "strict"})
                assert rc == want_rc, "waiver %s: rc=%s 기대=%s (%s)" % (ref, rc, want_rc, err)
                if want_sub:
                    assert want_sub in err, "waiver %s 문구 %r 누락: %s" % (ref, want_sub, err)
                if want_rc == EXIT_VERIFY:  # B1: waiver 계열 거부 stderr 분기(문면 고정)
                    assert "verify-waiver expired(6): 재승인(신규 issue) 후 재위임" in err, \
                        "B1 waiver 거부 문면 누락: %s" % err
                    assert "verify-spec missing(6)" not in err, \
                        "B1 분기 실패 — missing 문면 혼입: %s" % err

            # ── 시스템 클래스 create → 읽기전용 템플릿 자동 부여 + 스키마 유효 + strict 통과 ──
            rc, _, e = run(groot, ["create", "Tg-heal", "--id", "Tg-heal",
                                   "--origin-kind", "healing"])
            assert rc == 0, "healing create 실패: %s" % e
            tspec = read_task(groot, "Tg-heal").get("verify_spec")
            assert tspec and tspec.get("template") == "system-readonly-v1" \
                and tspec.get("template_origin") == "healing", "시스템 템플릿 미부여: %s" % tspec
            assert validate_verify_spec(tspec) == [], \
                "시스템 템플릿이 자체 스키마 위반: %s" % validate_verify_spec(tspec)
            rc, _, err = run(groot, ["checkout", "Tg-heal", "--owner", "w1"],
                             {"JAVIS_VERIFY_GATE": "strict"})
            assert rc == EXIT_OK, "시스템 클래스 strict checkout 실패: %s (%s)" % (rc, err)

            # ── checkout --brief: verify[] 있는 브리프 통과 / 없는 브리프 exit 6(hard) ──
            bok = mkfile(groot, "brief-ok.md",
                         "# task\n구현\n# verify\n- python3 x.py self-test → exit 0\n")
            bbad = mkfile(groot, "brief-bad.md", "# task\n구현만 서술·verify 없음\n")
            rc, _, e = run(groot, ["create", "Tg-brief", "--id", "Tg-brief"])
            assert rc == 0
            rc, _, err = run(groot, ["checkout", "Tg-brief", "--owner", "w1", "--brief", bok])
            assert rc == EXIT_OK, "유효 브리프 checkout 실패: %s (%s)" % (rc, err)
            rc, _, e = run(groot, ["release", "Tg-brief", "--owner", "w1"])
            assert rc == 0
            rc, _, err = run(groot, ["checkout", "Tg-brief", "--owner", "w1", "--brief", bbad])
            assert rc == EXIT_VERIFY and "verify[]" in err, \
                "무verify 브리프가 6 을 안 냄: rc=%s err=%s" % (rc, err)

            # ── B6: 게이트 평가 전 생존 소유자 선확인 — 산 소유자 충돌 = exit 9 우선(무통지) ──
            rc, _, e = run(groot, ["create", "Tg-own", "--id", "Tg-own"])
            assert rc == 0
            rc, _, e = run(groot, ["checkout", "Tg-own", "--owner", "w1"])  # warn 모드 선점
            assert rc == EXIT_OK, "Tg-own 선점 실패: %s" % e
            rc, _, err = run(groot, ["checkout", "Tg-own", "--owner", "w2"],
                             {"JAVIS_VERIFY_GATE": "strict"})
            assert rc == EXIT_CONFLICT, "B6 산 소유자 충돌이 9 를 안 냄: %s (%s)" % (rc, err)
            assert "verify-spec missing" not in err, "B6 exit 9 우선 위반 — 게이트 문면 출력: %s" % err
            assert not os.path.isfile(os.path.join(tasks_dir, "Tg-own.verify-notified")), \
                "B6 무통지 위반 — exit 9 경로에서 통지 마커 생성"

            # ── 동시성(R1-contract): set-verify-spec ∥ set-status — lost-update 0 ──
            import threading
            for it in range(3):
                tid = "Tg-conc%d" % it
                rc, _, e = run(groot, ["create", tid, "--id", tid])
                assert rc == 0
                rc, _, e = run(groot, ["checkout", tid, "--owner", "w1"])
                assert rc == 0, "conc checkout 실패: %s" % e
                results = {}

                def _sv(tid=tid):
                    results["sv"] = run(groot, ["set-verify-spec", tid, "--json",
                                                json.dumps(good_spec)])

                def _ss(tid=tid):
                    results["ss"] = run(groot, [
                        "set-status", tid, "in_review"])
                th1, th2 = threading.Thread(target=_sv), threading.Thread(target=_ss)
                th1.start(); th2.start(); th1.join(30); th2.join(30)
                assert results["sv"][0] == 0 and results["ss"][0] == 0, \
                    "동시 mutator exit 비0: %s" % (results,)
                final = read_task(groot, tid)
                assert final.get("status") == "in_review" and \
                    (final.get("verify_spec") or {}).get("mode") == "command", \
                    "lost-update 발생(iter %d): status=%s spec=%s" \
                    % (it, final.get("status"), final.get("verify_spec"))

        # ── B7: 마커 자동생성 경로 — 마커 부재 strict 최초 checkout = 승격 집행(grandfather
        #    WARN 통과+마커 생성) → 직후 신규 create checkout → exit 6 (별도 밀폐 root) ──
        with tempfile.TemporaryDirectory(prefix="javis-task-t1b7-") as b7root:
            b7_tasks = os.path.join(b7root, "_round", "tasks")
            rc, _, e = run(b7root, ["create", "Tb7-old", "--id", "Tb7-old"])
            assert rc == 0, "B7 create 실패: %s" % e
            time.sleep(1.1)  # created_at(초 해상도) < 마커 생성 시각 엄밀 보장
            rc, _, err = run(b7root, ["checkout", "Tb7-old", "--owner", "w1"],
                             {"JAVIS_VERIFY_GATE": "strict"})
            assert rc == EXIT_OK and "grandfathered" in err, \
                "B7 최초 strict 가 grandfather 통과 안 함: rc=%s err=%s" % (rc, err)
            assert os.path.isfile(os.path.join(b7_tasks, GATE_MARKER)), \
                "B7 마커 자동생성 안 됨"
            rc, _, e = run(b7root, ["create", "Tb7-new", "--id", "Tb7-new"])
            assert rc == 0
            rc, _, err = run(b7root, ["checkout", "Tb7-new", "--owner", "w2"],
                             {"JAVIS_VERIFY_GATE": "strict"})
            assert rc == EXIT_VERIFY, \
                "B7 마커 생성 직후 신규 태스크가 6 을 안 냄: rc=%s err=%s" % (rc, err)

        # ══ B1 guard-claim 파일 계약 배터리(§2-1 — 별도 밀폐 root) ══
        with tempfile.TemporaryDirectory(prefix="javis-task-b1-") as croot:
            c_tasks = os.path.join(croot, "_round", "tasks")

            def claim_path(sid):
                return os.path.join(c_tasks, ".guard-claim.%s" % sid)

            def read_claim(sid):
                with open(claim_path(sid), encoding="utf-8") as f:
                    return json.load(f)

            # checkout(CYS_SURFACE_ID 있음) → claim 기록(task_id·pid·ts)
            rc, _, e = run(croot, ["create", "Tc1", "--id", "Tc1"])
            assert rc == 0, "B1 create 실패: %s" % e
            rc, _, e = run(croot, ["checkout", "Tc1", "--owner", "w1", "--pid", "12345"],
                           {"CYS_SURFACE_ID": "42", "CYS_TASK_GUARD_CLAIM": "1"})
            assert rc == EXIT_OK, "B1 checkout 실패: %s" % e
            cl = read_claim("42")
            assert cl["task_id"] == "Tc1" and cl["pid"] == 12345 and cl.get("ts"), \
                "B1 claim 내용 오기록: %s" % cl
            # release → claim 소거
            rc, _, e = run(croot, ["release", "Tc1", "--owner", "w1"])
            assert rc == EXIT_OK, "B1 release 실패: %s" % e
            assert not os.path.isfile(claim_path("42")), "B1 release 후 claim 잔존"
            # 재checkout + set-status done(타 surface 호출 시뮬 — env 무 sid) → claim 소거
            rc, _, e = run(croot, ["checkout", "Tc1", "--owner", "w1"],
                           {"CYS_SURFACE_ID": "42", "CYS_TASK_GUARD_CLAIM": "1"})
            assert rc == EXIT_OK, "B1 재checkout 실패: %s" % e
            assert os.path.isfile(claim_path("42")), "B1 재checkout claim 미기록"
            rc, _, e = run(croot, ["set-status", "Tc1", "done",
                                   "--skip-reason", "B1 claim 소거 검증"] + OV)
            assert rc == EXIT_OK, "B1 done 실패: %s" % e
            assert not os.path.isfile(claim_path("42")), \
                "B1 터미널 전이(타 surface) 후 claim 잔존(교차 소거 실패)"
            # ★E1-4(F12/R-25): ambient CYS_SURFACE_ID 만으로는 claim 을 만들지 않는다
            #   (팩 install 즉시 발효 강등 — 기본 OFF). 명시 인자는 아래 F3 케이스가 증명한다.
            rc, _, e = run(croot, ["create", "Tc1b", "--id", "Tc1b"])
            assert rc == 0
            rc, _, e = run(croot, ["checkout", "Tc1b", "--owner", "w1"],
                           {"CYS_SURFACE_ID": "77"})
            assert rc == EXIT_OK, "E1-4 ambient checkout 실패: %s" % e
            assert not os.path.isfile(claim_path("77")), \
                "E1-4 ambient claim 이 기본값에서 생성됨(무게이트 발효 재발)"
            # CYS_SURFACE_ID 부재 → claim 미생성(무동작)
            rc, _, e = run(croot, ["create", "Tc2", "--id", "Tc2"])
            assert rc == 0
            rc, _, e = run(croot, ["checkout", "Tc2", "--owner", "w1"])
            assert rc == EXIT_OK, "B1 무sid checkout 실패: %s" % e
            claims = [n for n in os.listdir(c_tasks) if n.startswith(".guard-claim.")]
            assert claims == [], "B1 무sid 인데 claim 생성: %s" % claims
            # 거부 경로(exit 6)는 claim 을 만들지 않는다 — 마커를 과거로 선치(grandfather 배제)
            with open(os.path.join(c_tasks, GATE_MARKER), "w", encoding="utf-8") as f:
                f.write(time.strftime("%Y-%m-%dT%H:%M:%S%z",
                                      time.localtime(time.time() - 3600)) + "\n")
            rc, _, e = run(croot, ["create", "Tc3", "--id", "Tc3"])
            assert rc == 0
            rc, _, e = run(croot, ["checkout", "Tc3", "--owner", "w1"],
                           {"CYS_SURFACE_ID": "43", "JAVIS_VERIFY_GATE": "strict",
                            "CYS_TASK_GUARD_CLAIM": "1"})
            assert rc == EXIT_VERIFY, "B1 strict 거부 기대: %s" % rc
            assert not os.path.isfile(claim_path("43")), "B1 거부 경로가 claim 생성"

            # ══ F3 --claim-surface: 명시 인자 claim 기록 + env 대비 우선 ══
            rc, _, e = run(croot, ["create", "Tc4", "--id", "Tc4"])
            assert rc == 0, "F3 create 실패: %s" % e
            rc, _, e = run(croot, ["checkout", "Tc4", "--owner", "master",
                                   "--claim-surface", "55", "--pid", "777"],
                           {"JAVIS_VERIFY_GATE": "warn"})
            assert rc == EXIT_OK, "F3 checkout --claim-surface 실패: %s" % e
            cl = read_claim("55")
            assert cl["task_id"] == "Tc4" and cl["pid"] == 777, "F3 claim 오기록: %s" % cl
            rc, _, e = run(croot, ["release", "Tc4", "--owner", "master"])
            assert rc == EXIT_OK and not os.path.isfile(claim_path("55")), \
                "F3 release 후 claim 잔존"
            rc, _, e = run(croot, ["checkout", "Tc4", "--owner", "master",
                                   "--claim-surface", "56"],
                           {"CYS_SURFACE_ID": "42", "JAVIS_VERIFY_GATE": "warn"})
            assert rc == EXIT_OK, "F3 우선순위 checkout 실패: %s" % e
            assert os.path.isfile(claim_path("56")) and not os.path.isfile(claim_path("42")), \
                "F3 명시 인자 우선 실패(56 기대·42 금지): %s" \
                % [n for n in os.listdir(c_tasks) if n.startswith(".guard-claim.")]

            # ══ F5 사이드카 네임스페이스 격리: list 오염 0 + 예약 접미 id 거부 ══
            with open(os.path.join(c_tasks, "Tc4.guard.json"), "w", encoding="utf-8") as f:
                json.dump({"schema_version": 1, "task_id": "Tc4", "block_count": 0}, f)
            with open(os.path.join(c_tasks, "Tc4.esc-bundle.json"), "w",
                      encoding="utf-8") as f:
                json.dump({"schema_version": 1, "task": "Tc4"}, f)
            rc, out, e = run(croot, ["list"])
            assert rc == EXIT_OK, "F5 list 실패: %s" % e
            ids = [t.get("id") for t in json.loads(out)]
            assert "Tc4.guard" not in ids and "Tc4.esc-bundle" not in ids, \
                "F5 사이드카가 list 에 유령 태스크로 오염: %s" % ids
            assert "Tc4" in ids, "F5 필터가 실태스크를 삼킴: %s" % ids
            rc, _, e = run(croot, ["create", "X", "--id", "X.guard"])
            assert rc == EXIT_USAGE and "reserved" in e, \
                "F5 예약 접미(.guard) id 미거부: rc=%s err=%s" % (rc, e)
            rc, _, e = run(croot, ["create", "X", "--id", "X.esc-bundle"])
            assert rc == EXIT_USAGE, "F5 예약 접미(.esc-bundle) id 미거부: %s" % rc
            rc, _, e = run(croot, ["show", "Tc4.guard"])
            assert rc == EXIT_USAGE, "F5 show 예약 접미 id 미거부: %s" % rc

            # ══ F6 set-verify-spec probe args 형식 경고(fail-open) ══
            rc, _, e = run(croot, ["set-verify-spec", "Tc4", "--json",
                                   json.dumps({"mode": "probe",
                                               "probe": {"name": "artifact",
                                                         "args": ["path-positional"]}})])
            assert rc == EXIT_OK and "플래그형 아님" in e, \
                "F6 위치 인자형 probe.args 경고 부재/차단: rc=%s err=%s" % (rc, e)
            rc, _, e = run(croot, ["set-verify-spec", "Tc4", "--json",
                                   json.dumps({"mode": "probe",
                                               "probe": {"name": "artifact",
                                                         "args": ["--path", "/tmp/x"]}})])
            assert rc == EXIT_OK and "플래그형 아님" not in e, \
                "F6 플래그형 probe.args 에 오경고: %s" % e

        # ══ B2 배터리(§3 — 서명·고정 거부·done-deferred·demote·checkout 강등 경고) ══
        with tempfile.TemporaryDirectory(prefix="javis-task-b2-") as broot:
            b_tasks = os.path.join(broot, "_round", "tasks")
            exit_map_ok = {"kind": "exit_map", "pass_exits": [0]}
            del_spec = json.dumps({"mode": "command", "cmd": "rm -rf /tmp/b2-junk",
                                   "pass_rule": exit_map_ok})

            # ── B2① 삭제 어휘: 무서명 = exit 2 / --signed-by = 등록 성공 + high 승격 ──
            rc, _, e = run(broot, ["create", "Tb2", "--id", "Tb2"])
            assert rc == 0, "B2 create 실패: %s" % e
            rc, _, e = run(broot, ["set-verify-spec", "Tb2", "--json", del_spec])
            assert rc == EXIT_USAGE and "high-risk" in e and "--signed-by" in e, \
                "B2 삭제 어휘 무서명 등록 미거부: rc=%s err=%s" % (rc, e)
            rc, _, e = run(broot, ["set-verify-spec", "Tb2", "--json", del_spec,
                                   "--signed-by", "master"])
            assert rc == EXIT_OK, "B2 서명 등록 실패: %s" % e
            b2risk = read_task(broot, "Tb2")["verify_spec"]["risk"]
            assert b2risk["class"] == "high" and b2risk["signed_by"] == "master" \
                and b2risk["signed_at"], "B2 서명 승격 오기록: %s" % b2risk
            rc, _, e = run(broot, ["set-verify-spec", "Tb2", "--json",
                                   json.dumps({"mode": "command",
                                               "cmd": "curl https://x.test/health",
                                               "pass_rule": exit_map_ok})])
            assert rc == EXIT_USAGE and "네트워크" in e, \
                "B2 네트워크 어휘 무서명 미거부: rc=%s err=%s" % (rc, e)
            # ── B2② push생산·서버기동 = 서명 불문 고정 거부 ──
            for bad_cmd in ("cys send --to master done", "bun run server.ts"):
                rc, _, e = run(broot, ["set-verify-spec", "Tb2", "--json",
                                       json.dumps({"mode": "command", "cmd": bad_cmd,
                                                   "pass_rule": exit_map_ok}),
                                       "--signed-by", "master"])
                assert rc == EXIT_USAGE and "고정 거부" in e, \
                    "B2 고정 거부 어휘(%r)가 서명으로 통과: rc=%s err=%s" % (bad_cmd, rc, e)
            # ── B2③(G2 수리 반영) 일반 cmd: 기존 high spec 교체는 G2 게이트 — 무 force 거부 →
            #    --force-respec 통과+auto 강등(세탁 경로가 원장 기록 없이는 닫힘을 실증) ──
            benign_spec = json.dumps({"mode": "command", "cmd": "echo ok",
                                      "pass_rule": exit_map_ok})
            rc, _, e = run(broot, ["set-verify-spec", "Tb2", "--json", benign_spec])
            assert rc == EXIT_USAGE and "respec blocked" in e, \
                "G2 기존 high spec 의 무 force 교체 미거부: rc=%s err=%s" % (rc, e)
            rc, _, e = run(broot, ["set-verify-spec", "Tb2", "--json", benign_spec,
                                   "--force-respec"])
            assert rc == EXIT_OK, "G2 --force-respec 일반 cmd 등록 실패: %s" % e
            assert read_task(broot, "Tb2")["verify_spec"]["risk"]["class"] == "auto", \
                "B2 일반 cmd 가 auto 아님: %s" % read_task(broot, "Tb2")["verify_spec"]["risk"]
            # 신규 태스크 첫 등록(기존 상태 없음) = G2 무발동·일반 cmd 무서명 통과(회귀 0)
            rc, _, e = run(broot, ["create", "Tb2f", "--id", "Tb2f"])
            assert rc == 0
            rc, _, e = run(broot, ["set-verify-spec", "Tb2f", "--json", benign_spec])
            assert rc == EXIT_OK, "B2 첫 등록 일반 cmd 실패(G2 오발동 의심): %s" % e
            # ── G4: 위험 어휘 대소문자 무시 — RM/CURL 케이스 변조 우회 차단 ──
            for g4_cmd, g4_word in (("RM -rf /tmp/b2-x", "삭제"),
                                    ("CURL https://x.test", "네트워크")):
                rc, _, e = run(broot, ["set-verify-spec", "Tb2f", "--json",
                                       json.dumps({"mode": "command", "cmd": g4_cmd,
                                                   "pass_rule": exit_map_ok})])
                assert rc == EXIT_USAGE and g4_word in e, \
                    "G4 대문자 어휘(%r) 미검출: rc=%s err=%s" % (g4_cmd, rc, e)

            # ── B2④ done-deferred: 일반 spec = 통과+라벨 병기 ──
            setup_checked_out(broot, "Tb3")
            with open(os.path.join(b_tasks, "Tb3.guard.json"), "w", encoding="utf-8") as f:
                json.dump({"schema_version": 1, "task_id": "Tb3", "deferred": True,
                           "last_verdict": "SKIPPED_BUDGET"}, f)
            art3 = mkfile(broot, "b2-ev3.txt", "budget deferred evidence\n")
            rc, _, e = run(broot, ["set-status", "Tb3", "done", "--evidence",
                                   "budget 소진 관측 완료", "--evidence-artifact", art3] + OV)
            assert rc == EXIT_OK and "verify-deferred" in e, \
                "B2 일반 deferred done 통과+고지 실패: rc=%s err=%s" % (rc, e)
            ev3 = read_task(broot, "Tb3")["evidence"]
            assert ev3.get("verify_deferred") is True \
                and "[verify-deferred]" in ev3.get("text", ""), \
                "B2 verify-deferred 라벨 병기 누락: %s" % ev3
            # high spec + 미해소 VERIFY_FAIL = exit 4 차단
            setup_checked_out(broot, "Tb4")
            rc, _, e = run(broot, ["set-verify-spec", "Tb4", "--json", del_spec,
                                   "--signed-by", "master"])
            assert rc == EXIT_OK, "B2 Tb4 서명 등록 실패: %s" % e
            with open(os.path.join(b_tasks, "Tb4.guard.json"), "w", encoding="utf-8") as f:
                json.dump({"schema_version": 1, "task_id": "Tb4", "deferred": False,
                           "last_verdict": "VERIFY_FAIL"}, f)
            art4 = mkfile(broot, "b2-ev4.txt", "high-risk attempt evidence\n")
            rc, _, e = run(broot, ["set-status", "Tb4", "done", "--evidence",
                                   "고위험 미검증 시도", "--evidence-artifact", art4] + OV)
            assert rc == EXIT_BLOCKED and "done blocked(4)" in e, \
                "B2 high deferred done 이 exit 4 아님: rc=%s err=%s" % (rc, e)
            assert read_task(broot, "Tb4")["status"] != "done", "B2 차단인데 done 전이됨"
            # guard.json 부재 = 무간섭(회귀 0)
            setup_checked_out(broot, "Tb5")
            art5 = mkfile(broot, "b2-ev5.txt", "clean done evidence\n")
            rc, _, e = run(broot, ["set-status", "Tb5", "done", "--evidence",
                                   "정상 완료 검증됨", "--evidence-artifact", art5] + OV)
            assert rc == EXIT_OK and "verify-deferred" not in e, \
                "B2 guard.json 부재 done 회귀: rc=%s err=%s" % (rc, e)

            # ── G1(수리): done-deferred 내구 신호 합성 — SKIPPED_* 마스킹 창 차단 ──
            #    VERIFY_FAIL(block_count 적재) 후 SKIPPED_CYCLE 1회가 last_verdict 를 덮어도
            #    block_count·last_sig(PASS 로만 리셋)가 잔존을 증언 → high done 차단 유지.
            setup_checked_out(broot, "Tb8")
            rc, _, e = run(broot, ["set-verify-spec", "Tb8", "--json", del_spec,
                                   "--signed-by", "master"])
            assert rc == EXIT_OK, "G1 Tb8 서명 등록 실패: %s" % e
            with open(os.path.join(b_tasks, "Tb8.guard.json"), "w", encoding="utf-8") as f:
                json.dump({"schema_version": 1, "task_id": "Tb8", "deferred": False,
                           "last_verdict": "SKIPPED_CYCLE", "block_count": 3,
                           "last_sig": "sigG1"}, f)
            art8 = mkfile(broot, "b2-ev8.txt", "masked residue evidence\n")
            rc, _, e = run(broot, ["set-status", "Tb8", "done", "--evidence",
                                   "마스킹 잔존 시도", "--evidence-artifact", art8] + OV)
            assert rc == EXIT_BLOCKED and "block_count=3" in e, \
                "G1 마스킹 잔존(SKIPPED_CYCLE·block_count 3) done 미차단: rc=%s err=%s" % (rc, e)
            # PASS 상태(내구 신호 전부 리셋: block_count 0·sig None) → 통과·라벨 없음
            with open(os.path.join(b_tasks, "Tb8.guard.json"), "w", encoding="utf-8") as f:
                json.dump({"schema_version": 1, "task_id": "Tb8", "deferred": False,
                           "last_verdict": "PASS", "block_count": 0, "last_sig": None}, f)
            rc, _, e = run(broot, ["set-status", "Tb8", "done", "--evidence",
                                   "PASS 후 완료", "--evidence-artifact", art8] + OV)
            assert rc == EXIT_OK and "verify-deferred" not in e, \
                "G1 PASS 리셋 후 done 미통과: rc=%s err=%s" % (rc, e)
            # 일반(비-high) spec 의 NO_BLOCK_PASS 잔존(last_sig) → 통과+라벨 병기
            setup_checked_out(broot, "Tb9")
            with open(os.path.join(b_tasks, "Tb9.guard.json"), "w", encoding="utf-8") as f:
                json.dump({"schema_version": 1, "task_id": "Tb9", "deferred": False,
                           "last_verdict": "NO_BLOCK_PASS", "block_count": 0,
                           "last_sig": "sigNB"}, f)
            art9 = mkfile(broot, "b2-ev9.txt", "nbp residue evidence\n")
            rc, _, e = run(broot, ["set-status", "Tb9", "done", "--evidence",
                                   "NO_BLOCK_PASS 잔존", "--evidence-artifact", art9] + OV)
            assert rc == EXIT_OK and "verify-deferred" in e, \
                "G1 일반 spec NBP 잔존(last_sig) 라벨 미병기: rc=%s err=%s" % (rc, e)

            # ══ E1-1(BLOCKER R-08): 긴급 롤백 비대칭 — done-deferred 안전밸브 ═══════════
            #    고위험 + VERIFY_FAIL 잔존 고정 픽스처를 만들어 놓고 밸브만 바꿔가며 실측한다.
            def _e1_fixture(tid, disarm=False):
                setup_checked_out(broot, tid)
                rc_, _o, e_ = run(broot, ["set-verify-spec", tid, "--json", del_spec,
                                          "--signed-by", "master"])
                assert rc_ == EXIT_OK, "E1-1 %s 서명 등록 실패: %s" % (tid, e_)
                with open(os.path.join(b_tasks, "%s.guard.json" % tid), "w",
                          encoding="utf-8") as f:
                    json.dump({"schema_version": 1, "task_id": tid, "deferred": True,
                               "last_verdict": "VERIFY_FAIL", "block_count": 2,
                               "last_sig": "sigE1"}, f)
                if disarm:
                    with open(os.path.join(b_tasks, "%s.guard-disarmed" % tid), "w",
                              encoding="utf-8") as f:
                        f.write("2026-07-31T00:00:00Z infra_count 3회 연속(마지막: spawn 실패)\n"
                                "재무장: master 가 이 마커 파일을 삭제(명시 명령)\n")
                return mkfile(broot, "e1-%s.txt" % tid, "e1 valve evidence\n")

            def _bypass_audit():
                p = os.path.join(broot, "_round", "evidence", "guard_bypass_audit.jsonl")
                if not os.path.isfile(p):
                    return []
                with open(p, encoding="utf-8") as f:
                    return [json.loads(ln) for ln in f if ln.strip()]

            # ① 결함 재현 — guard env(CYS_COMPLETION_GUARD) 제거·부재만으로는 되돌아가지 않는다.
            #    (javis_task 는 그 env 를 읽지 않는다 — 잔존 guard.json 만 본다.)
            artE1 = _e1_fixture("Te1")
            rc, _, e = run(broot, ["set-status", "Te1", "done", "--evidence",
                                   "guard env 제거 후 시도", "--evidence-artifact", artE1] + OV,
                           {"CYS_COMPLETION_GUARD": ""})
            assert rc == EXIT_BLOCKED and "done blocked(4)" in e, \
                "E1-1① guard env 제거로 차단이 풀렸다(재현 실패): rc=%s err=%s" % (rc, e)
            assert "되돌리는 법" in e and "CYS_TASK_GUARD_SIGNAL_GATE" in e, \
                "E1-1① 차단 문면에 안전밸브 안내 부재: %s" % e
            assert _bypass_audit() == [], "E1-1① 차단인데 우회 원장이 기록됨"

            # ② 명시 안전밸브 warn — 고위험도 통과 + 라벨 + 우회 원장 1줄
            rc, _, e = run(broot, ["set-status", "Te1", "done", "--evidence",
                                   "안전밸브 warn 통과", "--evidence-artifact", artE1] + OV,
                           {"CYS_TASK_GUARD_SIGNAL_GATE": "warn"})
            assert rc == EXIT_OK and "guard-signal-gate warn" in e and "verify-deferred" in e, \
                "E1-1② warn 밸브 통과 실패: rc=%s err=%s" % (rc, e)
            evE1 = read_task(broot, "Te1")["evidence"]
            assert evE1.get("verify_deferred") is True \
                and evE1.get("guard_gate_bypass", {}).get("reason") == "env-warn", \
                "E1-1② 우회 병기 누락: %s" % evE1
            aud = _bypass_audit()
            assert len(aud) == 1 and aud[0]["task"] == "Te1" and aud[0]["reason"] == "env-warn", \
                "E1-1② 우회 원장 1줄 아님: %s" % aud

            # ③ off 밸브 — 검사 자체 생략(고지 문면 분기)·라벨/원장은 유지
            artE2 = _e1_fixture("Te2")
            rc, _, e = run(broot, ["set-status", "Te2", "done", "--evidence",
                                   "안전밸브 off 통과", "--evidence-artifact", artE2] + OV,
                           {"CYS_TASK_GUARD_SIGNAL_GATE": "off"})
            assert rc == EXIT_OK and "guard-signal-gate off" in e, \
                "E1-1③ off 밸브 통과 실패: rc=%s err=%s" % (rc, e)
            assert read_task(broot, "Te2")["evidence"].get("guard_gate_bypass", {}) \
                .get("reason") == "env-off", "E1-1③ off 우회 병기 누락"

            # ④ alias(CYS_TASK_GUARD_DEFERRED_GATE) 동작 — 대장·OT 롤백 표 문면 호환
            artE3 = _e1_fixture("Te3")
            rc, _, e = run(broot, ["set-status", "Te3", "done", "--evidence",
                                   "alias 밸브 통과", "--evidence-artifact", artE3] + OV,
                           {"CYS_TASK_GUARD_DEFERRED_GATE": "off"})
            assert rc == EXIT_OK and "guard-signal-gate off" in e, \
                "E1-1④ alias 밸브 미인식: rc=%s err=%s" % (rc, e)

            # ⑤ 자동 강등 — guard-disarmed(회로차단기 개방) 존재 시 strict 라도 차단 금지
            artE4 = _e1_fixture("Te4", disarm=True)
            rc, _, e = run(broot, ["set-status", "Te4", "done", "--evidence",
                                   "회로차단기 개방 상태 완료", "--evidence-artifact", artE4] + OV)
            assert rc == EXIT_OK and "guard-disarmed" in e, \
                "E1-1⑤ disarmed 자동 강등 실패(영구 차단 잔존): rc=%s err=%s" % (rc, e)
            aud4 = [r for r in _bypass_audit() if r["task"] == "Te4"]
            assert len(aud4) == 1 and aud4[0]["reason"] == "auto-demote-disarmed", \
                "E1-1⑤ 자동 강등 원장 누락: %s" % aud4

            # ⑥ 미지 값은 strict 로 고정 — 오타가 게이트를 조용히 열지 않는다
            artE5 = _e1_fixture("Te5")
            rc, _, e = run(broot, ["set-status", "Te5", "done", "--evidence",
                                   "오타 밸브 시도", "--evidence-artifact", artE5] + OV,
                           {"CYS_TASK_GUARD_SIGNAL_GATE": "warnn"})
            assert rc == EXIT_BLOCKED, \
                "E1-1⑥ 미지 밸브 값이 게이트를 열었다: rc=%s err=%s" % (rc, e)

            # ⑦ 회귀 0 — 밸브 미설정·guard.json 부재 = 종전과 동일(우회 원장도 늘지 않는다)
            n_before = len(_bypass_audit())
            setup_checked_out(broot, "Te6")
            artE6 = mkfile(broot, "e1-Te6.txt", "clean evidence\n")
            rc, _, e = run(broot, ["set-status", "Te6", "done", "--evidence",
                                   "정상 완료 검증됨", "--evidence-artifact", artE6] + OV)
            assert rc == EXIT_OK and "verify-deferred" not in e \
                and "guard-signal-gate" not in e, \
                "E1-1⑦ 기본 경로 회귀: rc=%s err=%s" % (rc, e)
            assert len(_bypass_audit()) == n_before, "E1-1⑦ 무우회인데 원장이 증가"

            # ── G2(수리): 미해소 guard 상태(VERIFY_FAIL)의 무서명 재등록 거부 + force 원장 ──
            b2_runs = os.path.join(broot, "probe_runs.jsonl")
            rc, _, e = run(broot, ["set-verify-spec", "Tb4", "--json", benign_spec])
            assert rc == EXIT_USAGE and "respec blocked" in e and "미해소 guard" in e, \
                "G2 high+VERIFY_FAIL 무서명 재등록 미거부: rc=%s err=%s" % (rc, e)
            rc, _, e = run(broot, ["set-verify-spec", "Tb4", "--json", benign_spec,
                                   "--force-respec"],
                           {"CYS_PROBE_RUNS": b2_runs})
            assert rc == EXIT_OK, "G2 --force-respec 통과 실패: %s" % e
            with open(b2_runs, encoding="utf-8") as f:
                fr = [json.loads(ln) for ln in f if ln.strip()]
            fr = [r for r in fr if r.get("probe") == "task-audit"
                  and r.get("audit") == "force-respec" and r.get("target") == "Tb4"]
            assert len(fr) == 1, "G2 force-respec 원장 1줄 아님: %s" % fr

            # ── B2⑤ --demote-calibrated: true→false 전이·멱등·spec 부재 거부 ──
            rc, _, e = run(broot, ["create", "Tb6", "--id", "Tb6"])
            assert rc == 0
            rc, _, e = run(broot, ["set-verify-spec", "Tb6", "--json",
                                   json.dumps({"mode": "command", "cmd": "echo ok",
                                               "pass_rule": exit_map_ok,
                                               "calibrated": True})])
            assert rc == EXIT_OK, "B2 calibrated spec 등록 실패: %s" % e
            rc, out, e = run(broot, ["set-verify-spec", "Tb6", "--demote-calibrated"])
            assert rc == EXIT_OK and json.loads(out)["demoted"] is True, \
                "B2 demote 전이 실패: rc=%s out=%s" % (rc, out)
            _tb6_spec = read_task(broot, "Tb6")["verify_spec"]
            assert _tb6_spec["calibrated"] is False, "B2 demote 후 calibrated 가 false 아님"
            assert _tb6_spec.get("demoted_at"), \
                "G7d demoted_at 병기 누락: %s" % _tb6_spec
            rc, out, e = run(broot, ["set-verify-spec", "Tb6", "--demote-calibrated"])
            assert rc == EXIT_OK and json.loads(out)["demoted"] is False, \
                "B2 demote 멱등 위반: %s" % out
            rc, _, e = run(broot, ["create", "Tb7", "--id", "Tb7"])
            assert rc == 0
            rc, _, e = run(broot, ["set-verify-spec", "Tb7", "--demote-calibrated"])
            assert rc == EXIT_USAGE and "강등 대상 없음" in e, \
                "B2 spec 부재 demote 미거부: rc=%s err=%s" % (rc, e)
            # checkout 강등 경고(G7d 수리): 근거 = spec.demoted_at — guard.json **부재**인 채로
            # strict 통과+경고 표면화(사이드카 소실과 무관함을 실증)
            assert not os.path.isfile(os.path.join(b_tasks, "Tb6.guard.json")), \
                "G7d 전제 위반: Tb6.guard.json 이 존재(사이드카 무관성 검증 불가)"
            rc, _, e = run(broot, ["checkout", "Tb6", "--owner", "w1"],
                           {"JAVIS_VERIFY_GATE": "strict"})
            assert rc == EXIT_OK and "calibrated 강등" in e and "demoted_at" in e, \
                "G7d 강등 상태 strict checkout 경고(demoted_at 기준) 부재/차단: rc=%s err=%s" \
                % (rc, e)
    except AssertionError as ex:
        print("javis_task self-test FAIL: %s" % ex, file=sys.stderr)
        return 1
    print("javis_task self-test OK (E1 evidence-artifact 게이트 — 부정4·긍정·폴백·skip감사·"
          "재진입·warn/off 경계·off밸브 skip감사·하위호환 A/B · 밀폐 tmpdir+JAVIS_ROOT / "
          "T1 verify 게이트 — warn·off·strict 문면·wakeup 1건+drain 흔적+A4 마커 억제/해제·"
          "grandfather·set-verify-spec 유효/거부·B5 risk 비-dict 거부·waiver 3상+B1 분기·"
          "시스템 템플릿·--brief hard·B6 산소유자 9 우선·동시성 lost-update 0·B7 마커 자동생성·"
          "A3 스키마 parity / B1 guard-claim — checkout 기록·release/터미널 소거(교차 surface)·"
          "무sid 무동작·거부 경로 무claim / F3 --claim-surface 기록·env 대비 우선 / "
          "F5 사이드카 list 오염 0·예약 접미 id 거부(create·show) / "
          "F6 probe args 형식 경고 fail-open / "
          "B2 — 삭제·네트워크 무서명 거부·--signed-by high 승격·push생산/서버기동 고정 거부·"
          "일반 cmd auto(첫 등록)·done-deferred(일반 라벨 병기·high exit4·부재 회귀 0)·"
          "--demote-calibrated 전이/멱등/부재 거부·strict checkout 강등 경고(demoted_at 기준·"
          "guard.json 무관 — G7d) / "
          "B2 수리 — G1 내구 신호 합성(SKIPPED_CYCLE+block_count 마스킹 차단·PASS 리셋 통과·"
          "NBP last_sig 라벨)·G2 재등록 게이트(high 교체/미해소 guard 거부·--force-respec "
          "통과+task-audit 원장 1줄)·G4 대소문자 어휘(RM·CURL 검출)"
          " / E1-1 롤백 비대칭(R-08) — guard env 제거만으로 차단 잔존 재현·차단 문면 안전밸브 안내·"
          "warn 밸브 통과+우회 원장 1줄·off 밸브 검사 생략·alias 인식·guard-disarmed 자동 강등·"
          "미지 값 strict 고정·기본 경로 회귀 0)")
    return EXIT_OK


def main(argv=None):
    # preflight 호환: `--self-test`는 subcommand 없이도 동작해야 한다(orchestra 관례 준용·가로채기).
    if "--self-test" in (sys.argv if argv is None else argv):
        return cmd_self_test(None)
    p = argparse.ArgumentParser(description="원자적 태스크 체크아웃 (P0-1)")
    sub = p.add_subparsers(dest="cmd", required=True)

    c = sub.add_parser("create")
    c.add_argument("title")
    c.add_argument("--id")
    c.add_argument("--status", default="todo", choices=STATUSES)
    c.add_argument("--why", help="이 일이 왜 존재하는가(goal 체인)")
    c.add_argument("--goal")
    c.add_argument("--blocked-by", nargs="*", dest="blocked_by")
    c.add_argument("--origin-kind", default="manual")
    c.add_argument("--origin-fingerprint")
    c.set_defaults(fn=cmd_create)

    c = sub.add_parser("checkout")
    c.add_argument("id")
    c.add_argument("--owner", required=True, help="워커/세션 식별자")
    c.add_argument("--pid", type=int, help="락 생존 판정에 쓸 장수 프로세스 pid(기본: 호출자)")
    c.add_argument("--claim-surface", dest="claim_surface", default=None,
                   help="B1 F3: guard-claim 을 기록할 surface id — master 가 위임 대상 워커"
                        " pane 을 지정(명시 인자 우선·기본은 env CYS_SURFACE_ID)")
    c.add_argument("--claim-pid", dest="claim_pid", type=int, default=None,
                   help="E2-1(R-02): guard-claim 에 박을 **위임 대상 pane 의 pid**. 미지정 +"
                        " --claim-surface 위임이면 pid 를 기록하지 않는다(호출자 pid 는 워커와"
                        " 무관 — 거짓 생존 신호 금지). guard 는 pid 부재 시 태스크 레코드"
                        " 상태로 claim 생존을 판정한다")
    c.add_argument("--brief", default=None,
                   help="T1: 위임 브리프 파일 — javis_brief_lint hard 경로(verify[] 형식 위반 = "
                        "exit 6 거부). 인세션 경고 경로는 hooks/brief-lint-warn.sh(fail-open)")
    c.set_defaults(fn=cmd_checkout)

    c = sub.add_parser("release")
    c.add_argument("id")
    c.add_argument("--owner", required=True)
    c.add_argument("--force", action="store_true")
    c.add_argument("--force-reason", dest="force_reason", default=None,
                   help="★G11: --force 필수 동반 — 강제 해제 사유(task JSON force_history 감사)")
    c.set_defaults(fn=cmd_release)

    c = sub.add_parser("set-status")
    c.add_argument("id")
    c.add_argument("status", choices=STATUSES)
    c.add_argument("--settled-override", dest="settled_override", default=None,
                   help="★G6: 완료 하한선 우회(사유 필수 기록) — owner 자신이 done을 선언하는 "
                        "등 안착 관측이 불가한 경우만")
    c.add_argument("--evidence", default=None,
                   help="W0-3: 검증 증거 '<검증명령 → 결과>' (최소 8자) — done 전이 필수")
    c.add_argument("--evidence-artifact", dest="evidence_artifact", action="append",
                   metavar="PATH",
                   help="E1: 검증 증거 파일 경로(반복 가능) — 기계 검사(실존·비어있지않음·신선도) 후 "
                        "task.evidence.artifacts 기록. done 전이 필수(strict) 또는 --skip-reason")
    c.add_argument("--skip-reason", dest="skip_reason", default=None,
                   help="W0-3·E1: 검증 불가 사유 — evidence/artifact 대체(사용 시 skip_audit.jsonl 감사 기록)")
    c.add_argument("--refs", default=None,
                   help="AA33(b)①: 산출물이 사용한 피어 radio 레코드 msg-id(쉼표 구분) — "
                        "task.refs 에 영속. 킬 지표 계수와 철회 역추적(§7.4(b))의 입력이며 "
                        "radio done 게이트의 인용 검사에도 그대로 전달된다")
    c.add_argument("--radio-node", dest="radio_node", default=None,
                   help="radio done 게이트의 판정 노드(미지정 시 task.owner) — "
                        "밸브 CYS_TASK_RADIO_GATE=strict|warn|off(기본 strict)")
    c.set_defaults(fn=cmd_set_status)

    c = sub.add_parser("set-verify-spec",
                       help="T1: verify_spec 등록(스키마 검증+wlock 직렬화) — 작성 3분할: "
                            "선언=master·하네스/calibrate=워커·봉인=master(조건 27)")
    c.add_argument("id")
    g = c.add_mutually_exclusive_group(required=True)
    g.add_argument("--file", default=None, help="verify_spec JSON 파일 경로")
    g.add_argument("--json", default=None, help="verify_spec JSON 문자열")
    g.add_argument("--demote-calibrated", dest="demote_calibrated", action="store_true",
                   help="B2(조건 07②): calibrated=false 강등 전이(guard timeout_violations≥2 "
                        "소비 경로 · wlock 직렬화 · 멱등)")
    c.add_argument("--signed-by", dest="signed_by", default=None,
                   help="B2(조건 30③): 삭제·네트워크 어휘 cmd 의 인간 서명(approver) — "
                        "서명 전 리뷰어 ACCEPT 의무는 계약 문서 조항")
    c.add_argument("--force-respec", dest="force_respec", action="store_true",
                   help="B2 G2: 미해소 guard 상태/기존 high spec 교체의 명시 강제 재등록 — "
                        "task-audit 원장 1줄(force-respec) 기록 동반")
    c.set_defaults(fn=cmd_set_verify_spec)

    c = sub.add_parser("ready")
    c.add_argument("id")
    c.set_defaults(fn=cmd_ready)

    c = sub.add_parser("show")
    c.add_argument("id")
    c.set_defaults(fn=cmd_show)

    c = sub.add_parser("list")
    c.add_argument("--open-only", action="store_true")
    c.set_defaults(fn=cmd_list)

    sub.add_parser("self-test", help="E1 게이트 밀폐 자기검증(tmpdir + JAVIS_ROOT 주입)"
                   ).set_defaults(fn=cmd_self_test)

    a = p.parse_args(argv)
    # ★G10: 모든 진입 id(본 id + create --blocked-by)를 경로 조합 전에 검증 — 단일 집행 지점.
    for tid in [getattr(a, "id", None)] + list(getattr(a, "blocked_by", None) or []):
        if tid is not None and not _ID_RE.match(tid):
            print("error: invalid task id %r — 허용 [A-Za-z0-9._-]{1,80} (G10 traversal 차단)"
                  % tid, file=sys.stderr)
            return EXIT_USAGE
        # F5(G10 확장): 예약 사이드카 접미 id 거부 — <id>.guard.json 등과 파일명 충돌 차단
        if tid is not None and tid.endswith(RESERVED_ID_SUFFIXES):
            print("error: reserved task id %r — guard 사이드카 예약 접미(%s) 는 태스크 id 로"
                  " 쓸 수 없음 (F5 네임스페이스 격리)"
                  % (tid, "·".join(RESERVED_ID_SUFFIXES)), file=sys.stderr)
            return EXIT_USAGE
    return a.fn(a)


if __name__ == "__main__":
    sys.exit(main())
