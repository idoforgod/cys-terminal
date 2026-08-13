#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""javis_actprobe.py — 행동 경계 결정론 probe (설계 §2.1 컴포넌트 A).

'최고 품질의 결과물 생산'을 막는 실증 최대 결함원 — "행동 순간에 검증 규율이 적용되지
않음" — 을 LLM 회상이 아니라 exit code로 차단한다. 5종 probe가 각기 하나의 실패 양식을
결정론으로 판정한다(근거 기억은 각 서브커맨드 docstring).

공통 exit 규약:
    0 = PASS       (행동 허용)
    2 = FAIL       (행동 금지 — 판정이 '위험/불일치'로 확정)
    3 = 판정불가    (행동 금지 + 수동 확인 — 읽을 수 없음/모호/내부 예외)
probe 내부 예외는 반드시 3으로 수렴한다(침묵 통과 금지). 인자 오류도 3(안전측).

영수증: 매 실행마다 append-only JSONL 1행 —
    {schema_version:1, ts, probe, target, exit, argv_digest, caller}
경로 = --runs-path > env CYS_PROBE_RUNS > <pack>/state/probe_runs.jsonl.
append-only·끝개행 보정·flock(단일 writer). 테스트는 반드시 _work 안 경로로(라이브 금지).

출력: 사람용 1줄(stdout) 기본, --json이면 기계 판독 JSON 1줄.

의존성: 파이썬 표준 라이브러리만(pack 관례). 라이브 조회(cys/ps)는 subprocess,
테스트는 전량 주입 모드(--screen-file/--ps-file/--status-file/--verdict-cli)로 대체 가능.
"""

import argparse
import getpass
import hashlib
import json
import os
import re
import shlex
import subprocess
import sys
import time

# ★Windows 즉사 차단(P0): top-level `import fcntl` 은 Windows 에 그 모듈이 없어
#   ModuleNotFoundError 로 이 스크립트 **전체**를 불능화한다 — javis_org.py:9-22 가 같은
#   사고를 먼저 겪고 msvcrt 폴백으로 지혈한 선례가 있다.
#   여기서는 **이름 `fcntl` 을 그대로 유지하는 shim** 을 바인딩한다. 아래 `_append_receipt`
#   의 `fcntl.flock(fd, fcntl.LOCK_EX)` 호출을 한 글자도 바꾸지 않으므로
#     · posix: 진짜 fcntl 모듈이 그대로 바인딩되어 동작이 **바이트 단위로 보존**된다.
#     · Windows: msvcrt 바이트락으로만 접힌다.
#   (`_append_receipt` 의 규약은 javis_task.py:1278-1300 이 복제해 쓰는 공유 계약이라
#    본문 변경은 양쪽 드리프트를 만든다 — import 층에서만 접는다.)
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

SCHEMA_VERSION = 1

EXIT_PASS = 0
EXIT_FAIL = 2
EXIT_INDET = 3

VERDICT_NAME = {EXIT_PASS: "PASS", EXIT_FAIL: "FAIL", EXIT_INDET: "INDET"}

CYS_BIN = os.environ.get("CYS_BIN", "cys")


# ── 영수증 ────────────────────────────────────────────────────────────

def _now():
    return time.strftime("%Y-%m-%dT%H:%M:%S%z")


def _runs_path(args):
    """영수증 경로: --runs-path > env CYS_PROBE_RUNS > <pack>/state/probe_runs.jsonl."""
    if getattr(args, "runs_path", None):
        return args.runs_path
    env = os.environ.get("CYS_PROBE_RUNS")
    if env:
        return env
    pack = os.environ.get("CYS_PACK_DIR") or os.path.expanduser("~/.cys/pack")
    return os.path.join(pack, "state", "probe_runs.jsonl")


def _argv_digest(argv):
    """호출 인자의 결정론 다이제스트(동일 argv → 동일 digest)."""
    joined = "\x00".join(argv)
    return hashlib.sha256(joined.encode("utf-8")).hexdigest()[:16]


def _resolve_caller(args):
    """caller: --caller > env CYS_ACTPROBE_CALLER > cys identify surface_ref > OS 사용자."""
    if getattr(args, "caller", None):
        return args.caller
    env = os.environ.get("CYS_ACTPROBE_CALLER")
    if env:
        return env
    try:
        out = subprocess.run([CYS_BIN, "identify"], capture_output=True,
                             text=True, timeout=5)
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


def _append_receipt(path, rec):
    """append-only + 끝개행 보정 + flock 단일 writer. 실패는 best-effort(경고)."""
    line = json.dumps(rec, ensure_ascii=False, sort_keys=True) + "\n"
    d = os.path.dirname(path) or "."
    os.makedirs(d, exist_ok=True)
    fd = os.open(path, os.O_RDWR | os.O_CREAT | os.O_APPEND, 0o644)
    try:
        fcntl.flock(fd, fcntl.LOCK_EX)
        size = os.fstat(fd).st_size
        if size:  # 끝개행 보정: 직전 행이 개행 없이 끝났으면 개행 하나를 먼저 붙인다
            os.lseek(fd, size - 1, os.SEEK_SET)
            if os.read(fd, 1) != b"\n":
                os.write(fd, b"\n")
        os.write(fd, line.encode("utf-8"))  # O_APPEND → 항상 끝에 원자 write
    finally:
        try:
            fcntl.flock(fd, fcntl.LOCK_UN)
        finally:
            os.close(fd)


# ── ANSI/화면 유틸 ─────────────────────────────────────────────────────

_ANSI_RE = re.compile(r"\x1b\[[0-9;?]*[A-Za-z]|\x1b\][0-9;].*?(?:\x07|\x1b\\)|\x1b[=>78]")


def _strip_ansi(text):
    return _ANSI_RE.sub("", text)


# 활동(생성 중) 마커 — 실측 근거: phoenix_harness READY_MARKERS 'esc to interrupt'.
ACTIVITY_MARKERS = ("esc to interrupt",)
# 스피너 글리프: 생성 중 애니메이션 프레임만. U+00B7('·')는 R1 실측 오탐으로 제외 —
# 유휴 화면의 잔여 출력("발화 2 · 도구 3" 등)에 흔히 섞여 미제출을 활동으로 오인시킨다.
SPINNER_GLYPHS = set("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏✳✶✻✽")
# 입력창 캐럿(내부 박스): 테두리 뒤 프롬프트 기호.
_BOXED_CARET_RE = re.compile(r"[│|]\s*[>❯›]\s(.*)$")
_BARE_CARET_RE = re.compile(r"^\s*[>❯›]\s(.*)$")


def _analyze_screen(text):
    """(located, input_empty, active) 반환.

    located    — 입력창(캐럿 줄)을 찾았는가
    input_empty— 입력창에 대기 텍스트가 없는가(제출됐다는 방증). located=False면 None
    active     — 활동 마커(스피너/esc to interrupt)가 있는가
    텍스트 존재 매칭 금지 규율(screen_grep_submission_blindspot·watcher_self_referential):
    '입력창이 비었고' + '활동 마커'라는 레이아웃 기준으로만 제출을 확정한다.
    """
    clean = _strip_ansi(text)
    active = any(m in clean.lower() for m in ACTIVITY_MARKERS) or \
        any(g in clean for g in SPINNER_GLYPHS)
    located = False
    input_empty = None
    # 박스형 캐럿 우선(출력 본문의 인용 '>' 오탐 회피). 없으면 무테두리 캐럿.
    for line in clean.splitlines():
        m = _BOXED_CARET_RE.search(line)
        if m:
            rest = m.group(1).rstrip("│| ").strip()
            located, input_empty = True, (rest == "")
            break
    if not located:
        for line in clean.splitlines():
            m = _BARE_CARET_RE.match(line)
            if m:
                rest = m.group(1).strip()
                located, input_empty = True, (rest == "")
                break
    return located, input_empty, active


# ── 1. submit ─────────────────────────────────────────────────────────

class _ReadError(Exception):
    pass


def _read_screen_live(ref):
    out = subprocess.run([CYS_BIN, "read-screen", "--surface", ref],
                         capture_output=True, text=True, timeout=12)
    if out.returncode != 0:
        raise _ReadError(f"cys read-screen exit {out.returncode}")
    return out.stdout


def _load_files(paths):
    """--screen-file 인자들을 원문으로 로드. 디렉터리/부재는 예외로 전파(→ exit3)."""
    return [open(p, encoding="utf-8", errors="replace").read() for p in paths]


def _judge_submit(pair):
    """두 판독(screen text 2개)으로 제출 판정 → (exit, reason)."""
    a1 = _analyze_screen(pair[0])
    a2 = _analyze_screen(pair[1])
    # 하나라도 입력창 미발견 → 판독 불가
    if not a1[0] or not a2[0]:
        return EXIT_INDET, "input box not located in one or both reads"
    # 대기 텍스트가 남아 있으면 제출 미확정 → 금지(Return 재발화 필요)
    if a1[1] is False or a2[1] is False:
        return EXIT_FAIL, "pending text in input box — submission not confirmed"
    # 둘 다 비었음: 활동 마커가 있어야 제출→생성 개시로 확정
    if a1[2] or a2[2]:
        return EXIT_PASS, "input empty + activity marker — submission confirmed"
    return EXIT_INDET, "input empty but no activity marker — cannot confirm submission"


def probe_submit(args):
    """Return 미발화·미제출 차단(…return_not_firing, …submission_blindspot).

    fixture 모드(--screen-file): 1개면 두 판독 모두 동일 파일, 2개면 각각 read1/read2.
    live 모드: read-screen 2회(간격) → 모호 시 재판독 예산 N회.
    """
    if args.screen_file:
        files = args.screen_file[:2]
        texts = _load_files(files)  # 디렉터리/부재 → 예외 → 상위 guard → exit3
        if len(texts) == 1:
            texts = [texts[0], texts[0]]
        return _judge_submit(texts)
    # live
    budget = max(1, args.reread)
    last = (EXIT_INDET, "no read performed")
    for _ in range(budget):
        try:
            s1 = _read_screen_live(args.surface)
            time.sleep(args.interval)
            s2 = _read_screen_live(args.surface)
        except (_ReadError, subprocess.SubprocessError, OSError) as e:
            last = (EXIT_INDET, f"read failure: {e}")
            time.sleep(args.interval)
            continue
        last = _judge_submit((s1, s2))
        if last[0] != EXIT_INDET:
            return last
        time.sleep(args.interval)
    return last


# ── 2. kill-preflight ─────────────────────────────────────────────────

# 데몬·타부서 소유 마커(부모 체인에 살아있으면 = 고아 아님 → kill 금지).
#
# ★휴리스틱임을 자인한다. 2026-07-18 `ps -axo pid,ppid,command` 실측 정합:
#   - "cys.app/Contents/Resources/runtime" — cys 터미널 앱 런타임(데몬 호스트). 관측된
#     실제 데몬 프로세스가 이 런타임 python으로 기동됨.
#   - "javis_hud_bridge" — 실관측된 상주 데몬 스크립트(cys.app 런타임이 구동).
#   - "cys-dept-" — 부서 레인 소유. javis_bootstrap.py:141·177 명명 계약(소켓 경로
#     .../cys-dept-<name>/cys.sock)에 근거(이 샌드박스엔 부서 미기동이라 라인 미관측).
#   - "cysd" — 정본 데몬 바이너리명. 현재 미관측이나 계약상 잔존(존재 시 포착).
# launchd/launch-agent는 **의도적으로 제외** — launchd(pid1)는 모든 프로세스의 조상이라
# 마커에 넣으면 전 kill이 FAIL로 막힌다(구 하드코딩의 'launch-agent'는 R1 오류였음).
# 방향성: 이 검사는 over-block(FAIL=kill 금지)이 안전측이다 — 고아 오판 kill이
# 훨씬 위험하므로, 의심스러우면 막고 수동 확인(3/2)한다.
DAEMON_MARKERS = (
    "cys.app/Contents/Resources/runtime",
    "javis_hud_bridge",
    "cys-dept-",
    "cysd",
)


def _parse_ps(text):
    """`ps -axo pid,ppid,command` → {pid: (ppid, command)}."""
    table = {}
    for line in text.splitlines():
        parts = line.split(None, 2)
        if len(parts) < 2:
            continue
        try:
            pid, ppid = int(parts[0]), int(parts[1])
        except ValueError:
            continue  # 헤더/잡음 행
        cmd = parts[2] if len(parts) > 2 else ""
        table[pid] = (ppid, cmd)
    return table


def _ps_live():
    out = subprocess.run(["ps", "-axo", "pid,ppid,command"],
                         capture_output=True, text=True, timeout=12)
    if out.returncode != 0:
        raise _ReadError(f"ps exit {out.returncode}")
    return out.stdout


def _is_daemon(cmd):
    return any(m in cmd for m in DAEMON_MARKERS)


def probe_kill_preflight(args):
    """고아 오판 kill 차단(…cross_workspace_kill_judgment).

    pid→ppid 부모 체인을 추적해 체인에 데몬/타부서 프로세스가 살아있으면(=고아 아님)
    exit 2. 체인이 깨끗하면 0. pid를 ps 테이블에서 못 찾으면 판정불가(3).
    """
    if args.ps_file:
        table = _parse_ps(open(args.ps_file, encoding="utf-8", errors="replace").read())
    else:
        table = _parse_ps(_ps_live())
    pid = args.pid
    if pid not in table:
        return EXIT_INDET, f"pid {pid} not present in process table"
    seen = set()
    cur = pid
    while cur and cur not in seen:
        seen.add(cur)
        node = table.get(cur)
        if node is None:
            break
        ppid, cmd = node
        if _is_daemon(cmd):
            role = "target" if cur == pid else "ancestor"
            return EXIT_FAIL, f"daemon/other-owner in chain ({role} pid {cur}: {cmd.strip()[:60]})"
        if ppid == cur or ppid == 0:
            break
        cur = ppid
    return EXIT_PASS, "no live daemon/other-owner in parent chain"


# ── 3. artifact ───────────────────────────────────────────────────────

def _parse_since(val):
    """epoch(float) 또는 ISO8601 → epoch float. 파싱 실패 시 ValueError."""
    if val is None:
        return None
    try:
        return float(val)
    except ValueError:
        pass
    s = val.strip().replace("Z", "+00:00")
    dt = __import__("datetime").datetime.fromisoformat(s)  # 실패 시 ValueError 전파
    return dt.timestamp()


def probe_artifact(args):
    """exit0만 믿고 산출물 미확인 차단(…build_exit0_artifact_check).

    실재 + 최소크기 + mtime(since 이후) 검사. since 파싱 불가는 판정불가(3).
    """
    try:
        since = _parse_since(args.since)
    except ValueError:
        return EXIT_INDET, f"unparseable --since: {args.since!r}"
    # os.stat 직접 사용: NUL 등 비정상 경로는 ValueError(비 OSError)를 던져 상위 guard→3으로
    # 수렴한다(os.path.isfile은 이를 삼켜 False로 만들어 오탐 위험).
    # FileNotFoundError만 FAIL(부재=산출물 없음). 그 밖의 stat 이상(NotADirectory·Permission·
    # NUL 등)은 catch하지 않아 상위 guard로 수렴→판정불가(3, 수동 확인) — 안전측.
    try:
        st = os.stat(args.path)
    except FileNotFoundError:
        return EXIT_FAIL, f"artifact missing: {args.path}"
    import stat as _stat
    if not _stat.S_ISREG(st.st_mode):
        return EXIT_FAIL, f"artifact not a regular file: {args.path}"
    if st.st_size < args.min_size:
        return EXIT_FAIL, f"artifact too small: {st.st_size} < {args.min_size} bytes"
    if since is not None and st.st_mtime < since:
        return EXIT_FAIL, f"artifact mtime {st.st_mtime:.0f} predates since {since:.0f}"
    return EXIT_PASS, f"artifact present ({st.st_size} bytes)"


# ── 4. verdict-match ──────────────────────────────────────────────────

_VERDICT_NAME_RE = re.compile(
    r"^REVIEWER_(?P<rev>[A-Za-z0-9._-]+)_VERDICT_(?P<task>[A-Za-z0-9._-]+)\.json$")


def probe_verdict_match(args):
    """verdict 대상 불일치 차단(…reviewer_verdict_target_mismatch).

    ①파일명 관례 REVIEWER_<X>_VERDICT_<TASK>.json의 TASK ↔ --task(1차 판정)
    ②javis_verdict.py validate exit0(스키마 유효)
    ③파일 실제 mtime(ref 내 수기 mtime 아님) > --since
    관례 위반 파일명은 판정불가(3).
    """
    fname = os.path.basename(args.file)
    m = _VERDICT_NAME_RE.match(fname)
    if not m:
        return EXIT_INDET, f"filename breaks REVIEWER_<X>_VERDICT_<TASK>.json convention: {fname}"
    fname_task = m.group("task")
    if fname_task != args.task:
        return EXIT_FAIL, f"verdict target mismatch: filename task {fname_task!r} != dispatch {args.task!r}"
    # ② 스키마 검증 (read-only 호출)
    if args.verdict_cli:
        cli = shlex.split(args.verdict_cli)
    else:
        pack = os.environ.get("CYS_PACK_DIR") or os.path.expanduser("~/.cys/pack")
        cli = [sys.executable, os.path.join(pack, "bin", "javis_verdict.py")]
    try:
        vr = subprocess.run(cli + ["validate", args.file],
                            capture_output=True, text=True, timeout=20)
    except (OSError, subprocess.SubprocessError) as e:
        return EXIT_INDET, f"verdict validator unavailable: {e}"
    if vr.returncode != 0:
        return EXIT_FAIL, f"verdict schema invalid (validator exit {vr.returncode})"
    # ③ mtime > since (파일 실제 mtime)
    if args.since is not None:
        try:
            since = _parse_since(args.since)
        except ValueError:
            return EXIT_INDET, f"unparseable --since: {args.since!r}"
        mtime = os.path.getmtime(args.file)
        if mtime <= since:
            return EXIT_FAIL, f"verdict mtime {mtime:.0f} not after dispatch {since:.0f} (stale)"
    return EXIT_PASS, f"verdict target {fname_task} matches, schema valid"


# ── 5. ctx-compare ────────────────────────────────────────────────────

def _load_status(args):
    if args.status_file:
        return json.loads(open(args.status_file, encoding="utf-8").read())
    out = subprocess.run([CYS_BIN, "status", "--json"],
                         capture_output=True, text=True, timeout=12)
    if out.returncode != 0:
        raise _ReadError(f"cys status exit {out.returncode}")
    return json.loads(out.stdout)


def _find_surface(data, ref):
    for s in (data.get("surfaces") or []):
        if s.get("surface_ref") == ref:
            return s
    return None


def probe_ctx_compare(args):
    """자기보고 ctx% 맹신 차단(…false_clear_threshold, …semantic_contract).

    데몬 실측 usage.ctx_pct(source=statusline) ↔ 자기보고 status.context_pct 대조.
    괴리 > 임계(기본 15)면 exit 2. usage null(예: gemini)/데몬 무응답이면 판정불가(3).
    """
    try:
        data = _load_status(args)
    except (_ReadError, subprocess.SubprocessError) as e:
        return EXIT_INDET, f"daemon status unavailable: {e}"
    s = _find_surface(data, args.surface)
    if s is None:
        return EXIT_INDET, f"surface {args.surface} not found in status"
    usage = s.get("usage") or {}
    status = s.get("status") or {}
    measured = usage.get("ctx_pct")
    reported = status.get("context_pct")
    if not isinstance(measured, (int, float)):
        return EXIT_INDET, f"measured usage.ctx_pct null/absent (e.g. gemini) for {args.surface}"
    if not isinstance(reported, (int, float)):
        return EXIT_INDET, f"self-report status.context_pct absent — nothing to compare"
    diff = abs(float(reported) - float(measured))
    if diff > args.threshold:
        return EXIT_FAIL, (f"ctx divergence {diff:.1f} > {args.threshold} "
                           f"(measured {measured}, self-report {reported})")
    return EXIT_PASS, f"ctx aligned (diff {diff:.1f} <= {args.threshold})"


# ── dispatch / main ───────────────────────────────────────────────────

DISPATCH = {
    "submit": probe_submit,
    "kill-preflight": probe_kill_preflight,
    "artifact": probe_artifact,
    "verdict-match": probe_verdict_match,
    "ctx-compare": probe_ctx_compare,
}


def _target_of(args):
    if args.cmd == "submit":
        return args.surface or (args.screen_file[0] if args.screen_file else "")
    if args.cmd == "ctx-compare":
        return args.surface
    if args.cmd == "kill-preflight":
        return str(args.pid)
    if args.cmd == "artifact":
        return args.path
    if args.cmd == "verdict-match":
        return args.file
    return ""


class _Parser(argparse.ArgumentParser):
    def error(self, message):
        # 인자 오류도 안전측(행동 금지+수동확인)으로 수렴 — 기본 argparse exit 2(=FAIL)와의 혼동 차단.
        sys.stderr.write(f"actprobe: argument error: {message}\n")
        sys.exit(EXIT_INDET)


# 공통(전역) 인자 — 부모 파서에 정의해 최상위 파서와 **각 서브파서 양쪽**에 parents=로 단다.
# default=SUPPRESS: 어느 위치에서든 미지정이면 속성을 아예 쓰지 않아, 서브파서 재파싱이
# 앞 위치에서 이미 채운 값을 기본값으로 덮어쓰는 argparse parent/subparser clobber를 차단한다.
# → `--task T submit …`(앞)·`submit … --task T`(뒤) 둘 다 정상, 전 probe 위치 일관.
def _common_parser():
    c = argparse.ArgumentParser(add_help=False)
    c.add_argument("--json", action="store_true", default=argparse.SUPPRESS,
                   help="기계 판독 JSON 1줄 출력")
    c.add_argument("--runs-path", default=argparse.SUPPRESS,
                   help="영수증 JSONL 경로 오버라이드(테스트는 _work 안 경로)")
    c.add_argument("--caller", default=argparse.SUPPRESS, help="영수증 caller 오버라이드")
    c.add_argument("--task", default=argparse.SUPPRESS,
                   help="이 probe 실행을 묶을 태스크 id — 영수증 task 필드로 기록(§2.1b 리플레이 "
                        "차단의 근거 필드. 대조는 P1b done 게이트 소관). verdict-match는 필수")
    return c


# SUPPRESS로 미설정된 공통 속성의 폴백 기본값 — 파싱 후 정규화(항상 존재 보장).
_COMMON_DEFAULTS = {"json": False, "runs_path": None, "caller": None, "task": None}


def build_parser():
    common = _common_parser()
    p = _Parser(prog="javis_actprobe.py", parents=[common],
                description="행동 경계 결정론 probe (5종)")
    sub = p.add_subparsers(dest="cmd", required=True)

    s = sub.add_parser("submit", parents=[common], help="Return 미제출 차단")
    s.add_argument("--surface", help="cys surface ref (live 모드)")
    s.add_argument("--screen-file", action="append", help="주입 판독 파일(최대 2회분)")
    s.add_argument("--reread", type=int, default=3, help="live 재판독 예산(기본 3)")
    s.add_argument("--interval", type=float, default=1.0, help="live 판독 간격 초(기본 1.0)")

    k = sub.add_parser("kill-preflight", parents=[common], help="고아 오판 kill 차단")
    k.add_argument("--pid", type=int, required=True)
    k.add_argument("--ps-file", help="주입 ps 테이블 파일")

    a = sub.add_parser("artifact", parents=[common], help="산출물 실재/크기/mtime 검사")
    a.add_argument("--path", required=True)
    a.add_argument("--min-size", type=int, default=1)
    a.add_argument("--since", help="epoch 또는 ISO8601 — 이 시각 이후 mtime 요구")

    # verdict-match의 --task는 공통 인자와 동일 필드(대조 대상 = 영수증 바인딩) — required는
    # main에서 강제(공통 부모라 서브파서 required 지정 불가). --since/--verdict-cli만 고유.
    v = sub.add_parser("verdict-match", parents=[common], help="verdict 대상 일치 검사")
    v.add_argument("--file", required=True)
    v.add_argument("--since", help="디스패치 시각(epoch/ISO) — verdict mtime 하한")
    v.add_argument("--verdict-cli", help="검증 CLI 오버라이드(테스트 스텁)")

    c = sub.add_parser("ctx-compare", parents=[common], help="실측/자기보고 ctx-pct 대조")
    c.add_argument("--surface", required=True)
    c.add_argument("--threshold", type=float, default=15.0)
    c.add_argument("--status-file", help="주입 status --json 파일")

    return p


def main(argv=None):
    argv = list(sys.argv[1:] if argv is None else argv)
    args = build_parser().parse_args(argv)
    # SUPPRESS로 누락된 공통 속성 정규화(항상 존재 보장 — 위치 무관 파싱의 대가).
    for name, default in _COMMON_DEFAULTS.items():
        if not hasattr(args, name):
            setattr(args, name, default)
    # submit 은 live/주입 중 하나는 필요
    if args.cmd == "submit" and not args.screen_file and not args.surface:
        sys.stderr.write("actprobe: submit requires --surface or --screen-file\n")
        sys.exit(EXIT_INDET)
    # verdict-match는 --task 필수(공통 부모라 서브파서 required 불가 → 여기서 강제)
    if args.cmd == "verdict-match" and not args.task:
        sys.stderr.write("actprobe: verdict-match requires --task\n")
        sys.exit(EXIT_INDET)

    probe = args.cmd
    target = _target_of(args)
    try:
        code, reason = DISPATCH[probe](args)
    except SystemExit:
        raise
    except Exception as e:  # probe 내부 예외는 3으로 수렴(침묵 통과 금지)
        code, reason = EXIT_INDET, f"internal-exception: {type(e).__name__}: {e}"

    rec = {
        "schema_version": SCHEMA_VERSION,
        "ts": _now(),
        "probe": probe,
        "target": target,
        "exit": code,
        "argv_digest": _argv_digest(argv),
        "caller": _resolve_caller(args),
    }
    if args.task:  # 추가 필드는 하위호환 — 없으면 생략, schema_version 1 유지
        rec["task"] = args.task
    try:
        _append_receipt(_runs_path(args), rec)
    except OSError as e:
        sys.stderr.write(f"actprobe: WARN receipt append failed: {e}\n")

    if args.json:
        obj = {"probe": probe, "target": target, "exit": code,
               "verdict": VERDICT_NAME[code], "reason": reason}
        if args.task:
            obj["task"] = args.task
        print(json.dumps(obj, ensure_ascii=False))
    else:
        print(f"[{VERDICT_NAME[code]}] {probe} {target}: {reason}")
    return code


if __name__ == "__main__":
    sys.exit(main())
