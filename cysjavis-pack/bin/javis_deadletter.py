#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""javis_deadletter.py — dead-letter 원장 전사·다이제스트 (C7 · DESIGN §4-C7·Sim3-S3-3)

데몬은 큐에서 사라지는 모든 항목(TTL 만기·소프트캡 거부·멱등 교체·operator clear)을
state dir의 `dead-letters.jsonl`(+10MB 회전본 `dead-letters.<ts>.jsonl`)에 남긴다 — 이 파일이
무손실의 SOT다(이벤트 버스는 유실 가능·관측 전용). 그러나 JSONL은 사람이 안 읽는다.

이 도구는 **읽히는 곳으로 옮긴다**:
  1) main + dept 전 state dir 순회 (`~/.local/state/cys`, `~/.local/state/cys-dept-*`)
     — 부서 데몬은 자기 state dir에 자기 dead-letter를 가진다(Sim3 S3-3).
  2) `$JAVIS_ROOT/_round/dead_letters/<YYYY-MM-DD>.md` 로 사람 가독 전사
     (dir별 섹션 · reason별 집계표 · 행 전문)
  3) **증분**: 이미 전사한 바이트 오프셋을 `_round/dead_letters/.offsets.json` 에 기록한다.
     키는 (dev,ino) — 파일이 회전(rename)돼도 같은 아이노드라 재전사되지 않는다. 오프셋에는
     선두 지문(head)도 함께 남겨, 같은 아이노드가 **같은 크기로 교체**된 경우도 탐지해 재판독한다.
  4) 신규분이 있으면 요약 1줄을 master에게 push(`javis_wakeup.py enqueue`,
     멱등키 `deadletter-digest` — 미처리 다이제스트가 대기 중이면 억제).

원장 기록 관례(G2)와 동일하게 **전사 직전 javis_scrub** 로 비밀을 마스킹한다.

exit codes: 0 신규 전사함 · 2 usage/오류 · 5 무작업(원장 파일 부재 또는 신규 행 0)
"""
import argparse
import glob
import hashlib
import json
import os
import subprocess
import sys
import time

import javis_scrub  # 형제 모듈 — 부재 시 즉시 실패(fail-closed)

BIN_DIR = os.path.dirname(os.path.abspath(__file__))
WAKEUP = os.path.join(BIN_DIR, "javis_wakeup.py")

ROOT = os.environ.get("JAVIS_ROOT") or os.getcwd()
OUT_DIR = os.path.join(ROOT, "_round", "dead_letters")
OFFSETS = os.path.join(OUT_DIR, ".offsets.json")

DEAD_LETTER_FILE = "dead-letters.jsonl"
DIGEST_KEY = "deadletter-digest"

EXIT_OK, EXIT_USAGE, EXIT_EMPTY = 0, 2, 5


# ─────────────────── state dir 발견 ───────────────────
def default_state_dirs():
    """main + dept 전 state dir. Windows 패리티는 %LOCALAPPDATA%\\cys (javis_phoenix 규약과 정합)."""
    if os.name == "nt":
        base = os.environ.get("LOCALAPPDATA") or os.path.join(os.path.expanduser("~"), "AppData", "Local")
        dirs = [os.path.join(base, "cys")] + sorted(glob.glob(os.path.join(base, "cys-dept-*")))
    else:
        base = os.path.join(os.path.expanduser("~"), ".local", "state")
        dirs = [os.path.join(base, "cys")] + sorted(glob.glob(os.path.join(base, "cys-dept-*")))
    return [d for d in dirs if os.path.isdir(d)]


def ledger_files(state_dir):
    """원장 + 회전본. 회전본은 이름의 타임스탬프 순, 현행 파일이 마지막(시간 순 전사)."""
    rotated = sorted(glob.glob(os.path.join(state_dir, "dead-letters.*.jsonl")))
    cur = os.path.join(state_dir, DEAD_LETTER_FILE)
    files = list(rotated)
    if os.path.isfile(cur):
        files.append(cur)
    return [f for f in files if os.path.isfile(f)]


# ─────────────────── 오프셋 원장 ───────────────────
def load_offsets(path=None):
    try:
        with open(path or OFFSETS, encoding="utf-8") as f:
            obj = json.load(f)
        return obj if isinstance(obj, dict) else {}
    except (FileNotFoundError, json.JSONDecodeError, OSError):
        return {}


def save_offsets(offsets, path=None):
    path = path or OFFSETS
    os.makedirs(os.path.dirname(path), exist_ok=True)
    tmp = "%s.tmp.%d" % (path, os.getpid())
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(offsets, f, ensure_ascii=False, indent=1, sort_keys=True)
        f.flush()
        os.fsync(f.fileno())
    os.replace(tmp, path)


def file_key(path):
    """(dev,ino) 키 — rename(회전)돼도 동일. stat 실패 시 경로 폴백."""
    try:
        st = os.stat(path)
        return "%d:%d" % (st.st_dev, st.st_ino)
    except OSError:
        return "path:%s" % path


HEAD_N = 256   # 파일 교체 탐지용 선두 지문 길이


def head_fingerprint(path, n):
    """선두 n바이트 지문. 파일이 n보다 짧으면 ""(=교체로 취급)."""
    if n <= 0:
        return ""
    try:
        with open(path, "rb") as f:
            b = f.read(n)
    except OSError:
        return ""
    if len(b) < n:
        return ""
    return hashlib.sha256(b).hexdigest()[:16]


def read_new_rows(path, offsets):
    """오프셋 이후의 신규 행만 파싱. 반환 (rows, offset_record).
    - 파일이 줄었거나(truncate) 선두 지문이 바뀌었으면(동일 크기 교체 포함) 0부터 다시 읽는다.
    - 마지막 줄이 미완(개행 없음)이면 그 줄은 남기고 오프셋을 그 앞까지만 전진(반쪽 라인 방지)."""
    key = file_key(path)
    prev = offsets.get(key, {})
    start = int(prev.get("offset", 0)) if isinstance(prev, dict) else int(prev or 0)

    def rec(offset):
        n = min(HEAD_N, offset)
        return {"offset": offset, "path": path, "head_len": n,
                "head": head_fingerprint(path, n)}

    try:
        size = os.path.getsize(path)
    except OSError:
        return [], rec(start)
    if size < start:
        start = 0
    if start and isinstance(prev, dict) and prev.get("head"):
        if head_fingerprint(path, int(prev.get("head_len", 0))) != prev["head"]:
            start = 0            # 같은 아이노드인데 내용이 교체됨 → 처음부터
    if size == start:
        return [], rec(start)
    rows, consumed = [], start
    with open(path, "rb") as f:
        f.seek(start)
        blob = f.read()
    parts = blob.split(b"\n")           # 마지막 조각 = 개행 뒤 잔여(완결이면 b"")
    tail = parts.pop()                      # 개행으로 끝나면 b""
    for chunk in parts:
        consumed += len(chunk) + 1
        line = chunk.decode("utf-8", "replace").strip()
        if not line:
            continue
        try:
            obj = json.loads(line)
        except ValueError:
            obj = {"_parse_error": True, "raw": line}
        rows.append(obj)
    if tail and not tail.strip():
        consumed += len(tail)               # 공백만 남은 잔여는 소비 처리
    return rows, rec(consumed)


# ─────────────────── 전사 ───────────────────
def _fmt_ts(ts):
    try:
        return time.strftime("%Y-%m-%dT%H:%M:%S", time.localtime(float(ts)))
    except (TypeError, ValueError):
        return str(ts)


def render_section(state_dir, per_file):
    """dir 1개 섹션 렌더. per_file = [(path, rows)] — rows는 scrub 완료본."""
    total = sum(len(r) for _, r in per_file)
    lines = ["## %s — 신규 %d건" % (state_dir, total), ""]
    counts = {}
    for _, rows in per_file:
        for r in rows:
            counts[str(r.get("reason", "?"))] = counts.get(str(r.get("reason", "?")), 0) + 1
    lines.append("| reason | 건수 |")
    lines.append("|---|---|")
    for reason in sorted(counts):
        lines.append("| %s | %d |" % (reason, counts[reason]))
    lines.append("")
    for path, rows in per_file:
        if not rows:
            continue
        lines.append("### %s (%d건)" % (os.path.basename(path), len(rows)))
        for r in rows:
            lines.append("- `%s` surface=%s role=%s reason=%s origin=%s"
                         % (_fmt_ts(r.get("ts")), r.get("surface_id"), r.get("role"),
                            r.get("reason"), r.get("origin")))
            text = str(r.get("text", ""))
            for tl in text.splitlines() or [""]:
                lines.append("  > %s" % tl)
            extra = {k: v for k, v in r.items()
                     if k not in ("ts", "surface_id", "role", "reason", "origin", "text")}
            if extra:
                lines.append("  - 부가: %s" % json.dumps(extra, ensure_ascii=False, sort_keys=True))
        lines.append("")
    return lines


def transcribe(sections, out_path):
    """섹션들을 날짜 md에 append(파일 신규 생성 시 제목 1회)."""
    os.makedirs(os.path.dirname(out_path), exist_ok=True)
    new_file = not os.path.exists(out_path)
    with open(out_path, "a", encoding="utf-8") as f:
        if new_file:
            f.write("# dead-letter 전사 — %s\n\n" % time.strftime("%Y-%m-%d"))
            f.write("데몬이 큐에서 제거한 모든 메시지의 무손실 원장 전사(SOT=state dir "
                    "`dead-letters.jsonl`). 이 파일은 javis_deadletter.py가 증분 append 한다.\n\n")
        f.write("---\n\n### 전사 %s\n\n" % time.strftime("%Y-%m-%dT%H:%M:%S%z"))
        for lines in sections:
            f.write("\n".join(lines) + "\n")


def push_digest(summary, dry_run=False):
    """다이제스트 1줄 push. 반환 (rc, 메시지)."""
    argv = [sys.executable, WAKEUP, "enqueue", "--to", "master", "--task", DIGEST_KEY,
            "--reason", summary, "--idempotency-key", DIGEST_KEY]
    if dry_run:
        return 0, "DRYRUN: " + " ".join(argv[1:])
    try:
        r = subprocess.run(argv, capture_output=True, text=True, timeout=20)
        return r.returncode, (r.stdout or r.stderr or "").strip()
    except (subprocess.SubprocessError, OSError) as e:
        return 255, str(e)


# ─────────────────── 메인 ───────────────────
def collect(state_dirs, offsets):
    """반환 (sections, counts, new_offsets, total). counts = reason별 총계."""
    sections, counts, total = [], {}, 0
    new_offsets = dict(offsets)
    for d in state_dirs:
        per_file = []
        for path in ledger_files(d):
            rows, off_rec = read_new_rows(path, offsets)
            new_offsets[file_key(path)] = off_rec
            if rows:
                rows = [javis_scrub.scrub_obj(r) for r in rows]   # G2: 전사 직전 마스킹
                per_file.append((path, rows))
                for r in rows:
                    key = str(r.get("reason", "?"))
                    counts[key] = counts.get(key, 0) + 1
                total += len(rows)
        if per_file:
            sections.append(render_section(d, per_file))
    return sections, counts, new_offsets, total


def main(argv=None):
    p = argparse.ArgumentParser(description="dead-letter 원장 전사·다이제스트 (C7)")
    p.add_argument("--state-dir", action="append", dest="state_dirs",
                   help="검사할 state dir(반복 지정 가능). 미지정 시 main+dept 자동 발견")
    p.add_argument("--out-dir", dest="out_dir",
                   help="전사 출력 디렉터리(기본 $JAVIS_ROOT/_round/dead_letters)")
    p.add_argument("--dry-run", action="store_true",
                   help="전사·오프셋 갱신·push 없이 결과만 출력")
    a = p.parse_args(argv)

    out_dir = a.out_dir or OUT_DIR
    offsets_path = os.path.join(out_dir, ".offsets.json")
    state_dirs = a.state_dirs or default_state_dirs()
    if not state_dirs:
        print("state dir 없음 — 무작업")
        return EXIT_EMPTY
    have_ledger = any(ledger_files(d) for d in state_dirs)
    if not have_ledger:
        print("dead-letter 원장 없음(%d개 dir 검사) — 무작업" % len(state_dirs))
        return EXIT_EMPTY

    offsets = load_offsets(offsets_path)
    sections, counts, new_offsets, total = collect(state_dirs, offsets)
    if total == 0:
        print(json.dumps({"new_rows": 0, "dirs": len(state_dirs)}, ensure_ascii=False))
        return EXIT_EMPTY

    out_path = os.path.join(out_dir, time.strftime("%Y-%m-%d") + ".md")
    breakdown = "·".join("%s %d" % (k, counts[k]) for k in sorted(counts))
    summary = "[dead-letter] 신규 %d건(%s) — 전사: %s" % (total, breakdown, out_path)

    if a.dry_run:
        for lines in sections:
            print("\n".join(lines))
        rc, msg = push_digest(summary, dry_run=True)
        print(msg)
        print(json.dumps({"new_rows": total, "reasons": counts, "out": out_path,
                          "dry_run": True}, ensure_ascii=False))
        return EXIT_OK

    transcribe(sections, out_path)
    save_offsets(new_offsets, offsets_path)
    rc, msg = push_digest(summary)
    print(json.dumps({"new_rows": total, "reasons": counts, "out": out_path,
                      "digest_rc": rc, "digest": msg}, ensure_ascii=False))
    return EXIT_OK


if __name__ == "__main__":
    sys.exit(main())
