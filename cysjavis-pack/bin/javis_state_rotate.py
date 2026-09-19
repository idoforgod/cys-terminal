#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""javis_state_rotate.py — 상태 파일 회전기 (결재 10 · 상한 게이트의 짝).

## 왜 존재하는가 (2026-09-17 실측)
상한 게이트(`role-capability-gate.sh:823 CSO_TODO_CAP`)는 파일이 커지는 것을 막지만 **줄이는 수단이
없었다.** 그래서 사람이 손으로 이관하다 **사본-원본 경쟁 상태**가 실제로 났다(20:08 · 사본 64,666B 대
원본 64,886B · 그 사이 원본이 220B 자랐다). 손 재현이 그 경쟁을 만든다 — 그래서 **복사·검증·절단·재대조를
한 도구 안에서** 한다.

## 불가침 계약 (오너 승인 규율 4)
원본이 살아 있는 동안 뜬 사본은 **자르기의 근거가 되지 못한다.**
  ① 사본 생성 → ②사본 해시 = 원본 해시 확인 → ③**자르기 직전 원본을 다시 읽어** 해시가 그대로인지 확인
  (달라졌으면 **자르지 않는다** · exit 9) → ④원자 교체(temp+fsync+os.replace) → ⑤**자른 직후 재대조**
  (사본 무결 · 새 파일이 더 작음 · 머리글 보존).
축소 방향이라 상한 게이트를 통과한다(`:1991` — 축소·동률은 언제나 허용).

## 무엇을 남기는가 (규율 12)
**제목 존치 · 본문 삭제**: 머리말 선언 줄(`<!-- javis:todo ... -->`)·마크다운 제목(`#`)·체크박스 줄은
남기고, 그 밖의 본문 줄을 **위에서부터**(오래된 것부터) 버린다. 체크박스를 지우지 않으므로 진행% 집계가
흔들리지 않는다.

사용:
    javis_state_rotate.py <파일> [--target-bytes N] [--archive-dir D] [--dry-run] [--self-test]
exit: 0=회전함 · 2=회전 불요(상한 이하) · 5=검증 실패(무변경) · 9=경쟁 감지(무변경) · 1=오류
"""
import argparse
import hashlib
import os
import shutil
import sys
import tempfile
import time

DEFAULT_TARGET = 48 * 1024          # 상한 64KiB 의 75% — 회전 직후 여유를 남긴다
KEEP_PREFIXES = ("#", "<!--", "- [x]", "- [ ]", "> ")


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(65536), b""):
            h.update(chunk)
    return h.hexdigest()


def plan_keep(text, target):
    """남길 텍스트(순수 함수) — 제목·선언·체크박스는 보존, 본문은 오래된 것부터 버린다."""
    lines = text.splitlines(keepends=True)
    keep_flags = [any(l.lstrip().startswith(p) for p in KEEP_PREFIXES) for l in lines]
    out = list(lines)
    for i, l in enumerate(lines):
        if len("".join(x for x in out if x is not None).encode("utf-8")) <= target:
            break
        if not keep_flags[i]:
            out[i] = None
    kept = "".join(x for x in out if x is not None)
    return kept


def rotate(path, target=DEFAULT_TARGET, archive_dir=None, dry_run=False, now=None):
    """→ (exit_code, 메시지, 정보dict). 파일을 건드리는 유일한 경로."""
    if not os.path.isfile(path):
        return 1, "파일 없음: %s" % path, {}
    size0 = os.path.getsize(path)
    if size0 <= target:
        return 2, "회전 불요 — %dB <= 목표 %dB" % (size0, target), {"size": size0}
    h1 = sha256(path)
    with open(path, encoding="utf-8") as f:
        text = f.read()
    kept = plan_keep(text, target)
    kept_b = len(kept.encode("utf-8"))
    if kept_b >= size0:
        return 5, "축소되지 않음(%dB >= %dB) — 자르지 않는다" % (kept_b, size0), {}
    stamp = time.strftime("%Y%m%dT%H%M%S", time.localtime(now or time.time()))
    adir = archive_dir or os.path.join(os.path.dirname(os.path.abspath(path)), "archive")
    apath = os.path.join(adir, "%s.%s.md" % (os.path.basename(path), stamp))
    if dry_run:
        return 0, "dry-run — %dB → %dB · 사본 %s" % (size0, kept_b, apath), {
            "size_before": size0, "size_after": kept_b, "archive": apath}
    os.makedirs(adir, exist_ok=True)
    # ① 사본 → ② 사본 무결 확인
    tmp_a = apath + ".tmp"
    shutil.copyfile(path, tmp_a)
    if sha256(tmp_a) != h1:
        os.unlink(tmp_a)
        return 5, "사본 해시 불일치 — 자르지 않는다", {}
    os.replace(tmp_a, apath)
    # ③ ★자르기 직전 재확인 — 원본이 그 사이 바뀌었으면 자르지 않는다(경쟁 감지)
    h2 = sha256(path)
    if h2 != h1:
        return 9, "경쟁 감지: 사본 이후 원본이 변경됨(%s → %s) — 자르지 않는다. 사본은 %s 에 보존" % (
            h1[:12], h2[:12], apath), {"archive": apath}
    # ④ 원자 교체
    d = os.path.dirname(os.path.abspath(path))
    fd, tmp_n = tempfile.mkstemp(dir=d, prefix=".rot-")
    with os.fdopen(fd, "w", encoding="utf-8") as f:
        f.write(kept)
        f.flush()
        os.fsync(f.fileno())
    os.replace(tmp_n, path)
    # ⑤ 자른 직후 재대조
    if sha256(apath) != h1:
        return 5, "자른 뒤 사본 무결성 붕괴 — 즉시 보고 대상", {"archive": apath}
    size1 = os.path.getsize(path)
    if size1 >= size0:
        return 5, "자른 뒤에도 축소되지 않음(%dB)" % size1, {"archive": apath}
    return 0, "회전 완료 — %dB → %dB · 사본 %s (사본 sha %s)" % (size0, size1, apath, h1[:12]), {
        "size_before": size0, "size_after": size1, "archive": apath, "sha_before": h1}


def _self_test():
    fails = []

    def ck(name, cond, detail=""):
        print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
        if not cond:
            fails.append(name)

    d = tempfile.mkdtemp(prefix="rot-")
    p = os.path.join(d, "CSO_TODO.md")
    body = "".join("본문 줄 %d 채우기 %s\n" % (i, "가" * 40) for i in range(400))
    head = "<!-- javis:todo v1 owner=cso -->\n# CSO_TODO\n- [x] 완료 항목\n- [ ] 미완 항목\n"
    open(p, "w", encoding="utf-8").write(head + body)
    size0 = os.path.getsize(p)
    rc, msg, info = rotate(p, target=4096)
    ck("① 회전 성공(exit 0)", rc == 0, msg)
    after = open(p, encoding="utf-8").read()
    ck("② 제목·선언·체크박스 보존", all(x in after for x in ("javis:todo", "# CSO_TODO", "- [x] 완료 항목", "- [ ] 미완 항목")))
    ck("③ 실제 축소", os.path.getsize(p) < size0, "%d → %d" % (size0, os.path.getsize(p)))
    ck("④ 사본이 원본 전체를 보존", os.path.getsize(info["archive"]) == size0)
    rc2, msg2, _ = rotate(p, target=4096)
    ck("⑤ 이미 목표 이하면 회전 불요(exit 2)", rc2 == 2, msg2)

    # ⑥ ★경쟁 감지 — 사본 직후 원본이 바뀌면 자르지 않는다
    p2 = os.path.join(d, "RACE.md")
    open(p2, "w", encoding="utf-8").write(head + body)
    orig_sha = sha256

    def racing_sha(path):                    # 사본 무결 확인(=.tmp 읽기) 직후에 원본을 키운다 →
        r = orig_sha(path)                   #   다음 호출인 '자르기 직전 재확인'이 변경을 봐야 한다
        if str(path).endswith(".tmp"):
            with open(p2, "a", encoding="utf-8") as f:
                f.write("남이 그 사이에 쓴 줄\n")
        return r
    g = globals()
    g["sha256"] = racing_sha
    try:
        rc3, msg3, info3 = rotate(p2, target=4096)
    finally:
        g["sha256"] = orig_sha
    ck("⑥ 경쟁 감지 시 exit 9 · 원본 무변경", rc3 == 9 and "남이 그 사이에 쓴 줄" in open(p2, encoding="utf-8").read(), msg3)
    ck("⑦ 경쟁이어도 사본은 남는다(증거 보존)", os.path.isfile(info3.get("archive", "")))
    p3 = os.path.join(d, "DRY.md")
    open(p3, "w", encoding="utf-8").write(head + body)
    sha_before = sha256(p3)
    rc4, _m4, _i4 = rotate(p3, target=4096, dry_run=True)
    ck("⑧ dry-run 은 파일을 건드리지 않는다(해시 불변 · 사본 미생성)",
       rc4 == 0 and sha256(p3) == sha_before and not os.path.isdir(os.path.join(d, "archive2")))
    shutil.rmtree(d, ignore_errors=True)
    print("\n=== %d/%d PASS ===" % (8 - len(fails), 8))
    return 1 if fails else 0


def main(argv=None):
    ap = argparse.ArgumentParser(description="상태 파일 회전기(복사·검증·절단·재대조 원자 수행)")
    ap.add_argument("path", nargs="?")
    ap.add_argument("--target-bytes", type=int, default=DEFAULT_TARGET)
    ap.add_argument("--archive-dir")
    ap.add_argument("--dry-run", action="store_true")
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args(argv)
    if a.self_test:
        return _self_test()
    if not a.path:
        ap.error("파일 경로가 필요하다")
    rc, msg, _ = rotate(a.path, a.target_bytes, a.archive_dir, a.dry_run)
    print(msg, file=sys.stderr if rc not in (0, 2) else sys.stdout)
    return rc


if __name__ == "__main__":
    raise SystemExit(main())
