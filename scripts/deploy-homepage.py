#!/usr/bin/env python3
"""홈페이지(www.cysinsight.com) 배포 — 자산 업로드 + 페이지 버전 범프 + 구버전 정리.

★설계 근거 (2026-07-29 실측):
  · 배포처는 Hostinger 공유호스팅(LiteSpeed). 자격은 `~/.cys/hostinger-ftp.env`(600).
    프로토콜은 **FTPS(명시적 AUTH SSL, 포트 21)** 이고, 인증서가 `*.hstgr.io` 라
    IP 접속 시 SAN 불일치 → `--ftp-ssl -k` 가 필요하다.
  · FTP 루트가 곧 docroot 다(`/index.html` · `/downloads/`). `domains/...` 접두 불요.
  · ★**로컬 `cys-homepage` 리포는 라이브보다 뒤처져 있다**(실측: 로컬 0.13.18 vs 라이브 0.14.4,
    미커밋 511건). 그래서 **로컬에서 빌드해 올리면 라이브가 퇴행한다.**
    → 이 스크립트는 **라이브 페이지를 받아 외과적으로 버전·용량 토큰만 치환**한다.
  · 용량 표기 관례는 **MiB 버림**이다(실측: 231,644,695B → 라이브 220MB · 반올림이면 221).
  · 자산 보존 정책은 **최신 1개**(v0.14.1 인계 기록: "0.14.0 자산 rm").

기본은 dry-run. `--apply` 를 줘야 실제로 올린다.

사용:
  python3 scripts/deploy-homepage.py 0.14.5 <자산디렉터리>           # 계획만
  python3 scripts/deploy-homepage.py 0.14.5 <자산디렉터리> --apply   # 집행
"""
import os
import re
import subprocess
import sys
import tempfile

ENV = os.path.expanduser("~/.cys/hostinger-ftp.env")
SITE = "https://www.cysinsight.com"
UA = "Mozilla/5.0"


def load_env():
    if not os.path.exists(ENV):
        raise SystemExit("::error::자격 파일 없음: %s" % ENV)
    out = {}
    for line in open(ENV, encoding="utf-8"):
        line = line.strip()
        if line.startswith("export ") and "=" in line:
            k, v = line[len("export "):].split("=", 1)
            out[k.strip()] = v.strip().strip('"').strip("'")
    for k in ("FTP_HOST", "FTP_USER", "FTP_PASS"):
        if not out.get(k):
            raise SystemExit("::error::%s 미설정" % k)
    return out


def curl(args, **kw):
    return subprocess.run(["curl", "-s", "--max-time", "600"] + args,
                          capture_output=True, **kw)


def ftp_base(e):
    return "ftp://%s/" % e["FTP_HOST"]


def ftp_auth(e):
    return ["-k", "--ftp-ssl", "-u", "%s:%s" % (e["FTP_USER"], e["FTP_PASS"])]


def ftp_list(e, path):
    r = curl(ftp_auth(e) + [ftp_base(e) + path])
    return r.stdout.decode("utf-8", "replace").splitlines()


def ftp_put(e, local, remote):
    r = curl(ftp_auth(e) + ["-T", local, ftp_base(e) + remote])
    return r.returncode == 0


def ftp_delete(e, remote):
    d = os.path.dirname(remote)
    r = curl(ftp_auth(e) + ["-Q", "-DELE %s" % os.path.basename(remote),
                            ftp_base(e) + (d + "/" if d else "")])
    return r.returncode == 0


def fetch(url):
    r = curl(["-A", UA, url])
    return r.stdout.decode("utf-8", "replace")


def mib(n):
    """용량 표기 = MiB **버림**(반올림 아님). 실측: 231,644,695B → 라이브 표기 220MB
    (반올림이면 221). x64/exe 는 두 방식이 같아 구분되지 않으므로 aarch64 가 판별자다."""
    return int(n // 1024 // 1024)


def main(argv):
    if len(argv) < 3:
        print(__doc__.strip(), file=sys.stderr)
        return 2
    ver = argv[1]
    assets = argv[2]
    apply_ = "--apply" in argv[3:]
    e = load_env()

    four = ["cys_%s_aarch64.dmg" % ver, "cys_%s_x64.dmg" % ver,
            "cys_%s_x64-setup.exe" % ver, "cys_%s_x64-setup.zip" % ver]
    upload = [f for f in sorted(os.listdir(assets)) if os.path.isfile(os.path.join(assets, f))]
    missing = [f for f in four if f not in upload]
    if missing:
        print("::error::배포 4종 누락: %s" % ", ".join(missing), file=sys.stderr)
        return 1

    # ── 현재 라이브 상태 ──
    live = ftp_list(e, "downloads/")
    old_versions = sorted({m.group(1) for l in live for m in [re.search(r"cys_(\d+\.\d+\.\d+)_", l)] if m}
                          - {ver})
    print("라이브 downloads/ 항목 %d · 구버전 %s" % (len(live), old_versions or "없음"))

    # ── 페이지 범프 계획 ──
    main_html = fetch(SITE + "/")
    dl_html = fetch(SITE + "/downloads/")
    if not main_html or not dl_html:
        print("::error::라이브 페이지를 받지 못했다", file=sys.stderr)
        return 1
    # 이전 버전 = 페이지에 **가장 많이** 등장하는 토큰(밴드 9곳). 문자열 정렬은
    # 잔존 토큰(예: 0.12.97 한 곳)을 집어 오답을 낸다 — dry-run 으로 실측 확인.
    counts = {}
    for v in re.findall(r"0\.\d+\.\d+", main_html):
        if v != ver:
            counts[v] = counts.get(v, 0) + 1
    prev = max(counts, key=counts.get) if counts else None
    print("메인 페이지: 현재 %s 토큰 %d개 → %s 로 범프" % (prev, main_html.count(prev or "\0"), ver))

    sizes = {f: os.path.getsize(os.path.join(assets, f)) for f in four}
    for f in four:
        print("  %-32s %12d B = %d MB(MiB 버림)" % (f, sizes[f], mib(sizes[f])))

    new_main = main_html.replace(prev, ver) if prev else main_html
    new_dl = dl_html.replace(prev, ver) if prev else dl_html
    # 용량 토큰 — MiB 버림. 순서는 aarch64 → x64 → exe → zip 로 페이지에 등장한다고 가정하지 않고
    # **구 자산의 실제 MiB → 신 자산 MiB** 로 1:1 치환한다(오탐 방지).
    if prev:
        for f in four:
            oldf = f.replace(ver, prev)
            h = curl(["-A", UA, "-I", "%s/downloads/%s" % (SITE, oldf)]).stdout.decode("utf-8", "replace")
            m = re.search(r"(?i)content-length:\s*(\d+)", h)
            if not m or int(m.group(1)) == 0:
                print("  ⚠ 구자산 크기 조회 실패 — 용량 토큰 치환 생략: %s" % oldf)
                continue
            o, n = mib(int(m.group(1))), mib(sizes[f])
            if o == n:
                continue
            tok = "%dMB" % o
            if tok not in new_main:
                print("  ⚠ 페이지에 %s 토큰이 없다 — 치환 생략(%s)" % (tok, f))
                continue
            new_main = new_main.replace(tok, "%dMB" % n)
            print("  용량 토큰 %s → %dMB (%s)" % (tok, n, f))

    if not apply_:
        print("\n[dry-run] 아무것도 올리지 않았다. 집행은 --apply")
        print("  올릴 자산: %d개" % len(upload))
        print("  삭제 대상 구버전: %s" % (old_versions or "없음"))
        return 0

    # ── 집행 ──
    print("\n── 자산 업로드 ──")
    for f in upload:
        ok = ftp_put(e, os.path.join(assets, f), "downloads/" + f)
        print("  %s %s (%d bytes)" % ("✓" if ok else "✗", f, os.path.getsize(os.path.join(assets, f))))
        if not ok:
            print("::error::업로드 실패: %s" % f, file=sys.stderr)
            return 1

    print("── 페이지 업로드 ──")
    with tempfile.TemporaryDirectory() as td:
        mp, dp = os.path.join(td, "index.html"), os.path.join(td, "dl.html")
        open(mp, "w", encoding="utf-8").write(new_main)
        open(dp, "w", encoding="utf-8").write(new_dl)
        for local, remote in ((mp, "index.html"), (dp, "downloads/index.html")):
            ok = ftp_put(e, local, remote)
            print("  %s %s" % ("✓" if ok else "✗", remote))
            if not ok:
                return 1

    print("── 구버전 자산 정리 (최신 1개 정책) ──")
    for l in live:
        name = l.split()[-1] if l.split() else ""
        m = re.search(r"cys_(\d+\.\d+\.\d+)_", name)
        if m and m.group(1) != ver:
            print("  %s rm %s" % ("✓" if ftp_delete(e, "downloads/" + name) else "✗", name))
    print("\n✅ 홈페이지 배포 완료")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
