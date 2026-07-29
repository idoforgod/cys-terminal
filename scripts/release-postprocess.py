#!/usr/bin/env python3
"""릴리스 후처리 — CI 완주 후 배포 자산을 완성한다 (2026-07-29 신설).

★배경: 릴리스 CI(`release.yml`)는 DMG 2종 · setup.exe · 업데이터 자산 · 팩 3종까지 만들지만,
**홈페이지가 쓰는 나머지 2종은 만들지 않는다**:
  · `cys_<V>_x64-setup.zip`  — 4번째 다운로드 버튼(.exe 직다운 차단 환경용)
  · `SHA256SUMS.txt`         — 전 자산 무결성 목록
실측(v0.14.2·0.14.3·0.14.4): exe 업로드 03:06 → zip·SUMS 업로드 07:03. 즉 CI 밖의 손절차였다.
이 스크립트가 그 손절차를 **재현 가능하게 고정**한다.

하는 일 (기본은 dry-run — `--apply` 를 줘야 실제로 업로드한다)
  1. GitHub 릴리스에서 전 자산 다운로드 → `~/cys-release-backup/<tag>-assets/`
  2. `make-win-zip.py` 로 zip 변형 생성(기존 발행본 바이트 재현 확인됨)
  3. `SHA256SUMS.txt` 생성 — **자기 자신을 뺀 전 자산**(과거 관례: 13자산)
  4. 자기 검증: zip 왕복 · SUMS 전 줄 재계산 대조 · 누락 0
  5. `--apply` 면 zip·SUMS 를 릴리스에 업로드

인증: `git credential fill`(osxkeychain)에서 GitHub 토큰을 얻는다. gh CLI 불요.

사용:
  python3 scripts/release-postprocess.py v0.14.5            # dry-run(다운로드·생성·검증만)
  python3 scripts/release-postprocess.py v0.14.5 --apply    # 업로드까지
"""
import hashlib
import json
import os
import subprocess
import sys
import urllib.request

REPO = "idoforgod/cys-terminal"
BACKUP_ROOT = os.path.expanduser("~/cys-release-backup")
SUMS_NAME = "SHA256SUMS.txt"          # ★과거 관례 — `SHA256SUMS`(확장자 없음) 아님
HERE = os.path.dirname(os.path.abspath(__file__))


def token():
    """osxkeychain 의 git 자격에서 GitHub 토큰을 얻는다(값은 출력하지 않는다)."""
    p = subprocess.run(["git", "credential", "fill"], input="protocol=https\nhost=github.com\n\n",
                       capture_output=True, text=True)
    for line in p.stdout.splitlines():
        if line.startswith("password="):
            return line[len("password="):]
    raise SystemExit("::error::GitHub 토큰을 얻지 못했다(git credential fill)")


def api(path, tok, method="GET", data=None, ctype="application/json"):
    req = urllib.request.Request("https://api.github.com" + path, method=method, data=data)
    req.add_header("Authorization", "Bearer " + tok)
    req.add_header("Accept", "application/vnd.github+json")
    if data is not None:
        req.add_header("Content-Type", ctype)
    with urllib.request.urlopen(req) as r:
        body = r.read()
    return json.loads(body) if body else {}


def sha256_file(p):
    h = hashlib.sha256()
    with open(p, "rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def download(url, dest, tok):
    req = urllib.request.Request(url)
    req.add_header("Authorization", "Bearer " + tok)
    req.add_header("Accept", "application/octet-stream")
    with urllib.request.urlopen(req) as r, open(dest, "wb") as fh:
        while True:
            chunk = r.read(1 << 20)
            if not chunk:
                break
            fh.write(chunk)


def main(argv):
    if len(argv) < 2:
        print(__doc__.strip(), file=sys.stderr)
        return 2
    tag = argv[1]
    apply_ = "--apply" in argv[2:]
    version = tag.lstrip("v")
    tok = token()

    # ★draft 릴리스는 `/releases/tags/<tag>` 로 조회되지 않는다(404 — 실측).
    #   목록 조회로 폴백해야 CI 직후(draft 상태)에도 후처리가 돈다.
    try:
        rel = api("/repos/%s/releases/tags/%s" % (REPO, tag), tok)
    except urllib.error.HTTPError as e:
        if e.code != 404:
            raise
        rel = None
        for page in (1, 2):
            for r in api("/repos/%s/releases?per_page=50&page=%d" % (REPO, page), tok):
                if r.get("tag_name") == tag:
                    rel = r
                    break
            if rel:
                break
        if rel is None:
            raise SystemExit("::error::릴리스 %s 를 찾지 못했다(draft 포함 조회 실패)" % tag)
        print("  (draft — 목록 조회로 찾음)")
    print("릴리스 %s — draft=%s · 자산 %d종" % (tag, rel.get("draft"), len(rel.get("assets", []))))

    outdir = os.path.join(BACKUP_ROOT, tag + "-assets")
    os.makedirs(outdir, exist_ok=True)

    # ── 1. 전 자산 다운로드 ──
    by_name = {}
    for a in rel.get("assets", []):
        dest = os.path.join(outdir, a["name"])
        if os.path.exists(dest) and os.path.getsize(dest) == a["size"]:
            print("  (캐시) %-34s %12d" % (a["name"], a["size"]))
        else:
            print("  받는 중 %-34s %12d …" % (a["name"], a["size"]), flush=True)
            download(a["url"], dest, tok)
            got = os.path.getsize(dest)
            if got != a["size"]:
                print("::error::크기 불일치 %s: %d != %d" % (a["name"], got, a["size"]), file=sys.stderr)
                return 1
        by_name[a["name"]] = dest

    # ── 2. zip 변형 (없으면 생성) ──
    exe = "cys_%s_x64-setup.exe" % version
    zipname = "cys_%s_x64-setup.zip" % version
    if exe not in by_name:
        print("::error::%s 가 릴리스에 없다 — CI 완주를 먼저 확인하라" % exe, file=sys.stderr)
        return 1
    zippath = os.path.join(outdir, zipname)
    if zipname in by_name:
        print("  zip 이미 릴리스에 있음 — 재생성 생략")
    else:
        r = subprocess.run([sys.executable, os.path.join(HERE, "make-win-zip.py"),
                            by_name[exe], zippath])
        if r.returncode != 0:
            return 1
        by_name[zipname] = zippath

    # ── 3. SHA256SUMS.txt — 자기 자신 제외 전 자산 ──
    names = sorted(n for n in by_name if n != SUMS_NAME)
    lines = ["%s  %s\n" % (sha256_file(by_name[n]), n) for n in names]
    sums_path = os.path.join(outdir, SUMS_NAME)
    with open(sums_path, "w") as fh:
        fh.writelines(lines)
    print("  %s 생성 — %d자산" % (SUMS_NAME, len(names)))

    # ── 4. 자기 검증 ──
    bad = 0
    for line in lines:
        want, name = line.split("  ", 1)
        name = name.strip()
        if sha256_file(by_name[name]) != want:
            print("::error::SUMS 불일치: %s" % name, file=sys.stderr)
            bad += 1
    if bad:
        return 1
    # 홈페이지 4종이 전부 들어 있는가(누락 0 — 오너 지시 ⓑ)
    want4 = ["cys_%s_aarch64.dmg" % version, "cys_%s_x64.dmg" % version, exe, zipname]
    missing = [w for w in want4 if w not in by_name]
    if missing:
        print("::error::배포 4종 누락: %s" % ", ".join(missing), file=sys.stderr)
        return 1
    print("  ✓ 자기 검증 통과 — 전 줄 재계산 일치 · 배포 4종 누락 0")

    # ── 5. 업로드 ──
    if not apply_:
        print("\n[dry-run] 업로드하지 않았다. 실제 업로드는 --apply")
        print("산출물: %s" % outdir)
        return 0

    up_base = rel["upload_url"].split("{")[0]
    existing = {a["name"]: a["id"] for a in rel.get("assets", [])}
    for name, ctype in ((zipname, "application/zip"), (SUMS_NAME, "text/plain")):
        if name in existing:      # --clobber 동등: 기존 자산 삭제 후 재업로드
            api("/repos/%s/releases/assets/%d" % (REPO, existing[name]), tok, method="DELETE")
        with open(os.path.join(outdir, name), "rb") as fh:
            body = fh.read()
        req = urllib.request.Request(up_base + "?name=" + name, method="POST", data=body)
        req.add_header("Authorization", "Bearer " + tok)
        req.add_header("Content-Type", ctype)
        with urllib.request.urlopen(req) as r:
            r.read()
        print("  ✓ 업로드 %s (%d bytes)" % (name, len(body)))
    print("\n✅ 후처리 완료 — %s" % outdir)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
