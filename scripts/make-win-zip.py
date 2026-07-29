#!/usr/bin/env python3
"""Windows 설치본 ZIP 변형 생성 + 왕복 검증 (2026-07-29 신설).

★왜 필요한가: 홈페이지(www.cysinsight.com)의 **4번째 다운로드 버튼**이 쓰는 자산이다.
이 파일은 **CI 가 만들지 않는다** — 릴리스 CI 완주 뒤 사람/에이전트가 로컬에서 만들어
GitHub 릴리스와 홈페이지에 올려 왔다(v0.14.2·0.14.3·0.14.4 실측: exe 업로드 03:06 →
zip 업로드 07:03). 그 손절차를 **스크립트로 고정**해 재현 가능하게 만든 것이 이 파일이다.

★기존 발행본 바이트 재현 확인(2026-07-29): v0.14.4 의 setup.exe 를 입력으로 넣으면
  `sha256=45a4a345…` 로 **발행된 zip 과 완전히 동일**한 바이트가 나온다.

용도: `.exe` 직다운이 차단되는 환경(기업 프록시·구형 브라우저)용 포장.
그래서 내용은 **setup.exe 그대로 한 개만** 담는다 — 홈페이지 카피
("압축 해제 후 setup.exe 실행")와 정확히 일치해야 한다.

결정론: **고정 타임스탬프 2020-01-01**·경로 없는 단일 엔트리·고정 퍼미션.
같은 입력이면 같은 바이트가 나와야 재현 가능한 배포가 된다.
★mtime 2020-01-01 은 **기존 발행 관례**다(v0.14.0·v0.14.1 인계 기록: "zip 변형 생성(내부 exe
바이트 SAME·mtime 2020-01-01 관례)"). 바꾸면 과거 자산과 바이트가 갈린다.

왕복 검증: 압축을 풀어 sha256 이 원본과 같은지 확인한다 — 포장 사고(잘린 파일·잘못된 엔트리)를
업로드 전에 잡는다. 실패 시 exit 1.

사용: python3 scripts/make-win-zip.py <setup.exe 경로> [출력 zip 경로]
      (출력 생략 시 같은 폴더에 확장자만 .zip 으로)
"""
import hashlib
import os
import shutil
import subprocess
import sys
import tempfile
import time
import zipfile

# 발행 관례: zip 엔트리에 **2020-01-01 00:00:00 이 찍히도록** 한다.
# ★Info-ZIP 은 엔트리 시각을 **로컬 시각**으로 기록한다 — UTC 로 잡으면 KST(+9)에서 09:00 이 찍혀
#   과거 발행본과 바이트가 갈린다(실측으로 확인). 그래서 timegm 이 아니라 mktime 을 쓴다.
MTIME = time.mktime((2020, 1, 1, 0, 0, 0, 0, 0, -1))


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def main(argv):
    if len(argv) < 2:
        print(__doc__.strip(), file=sys.stderr)
        return 2
    exe = argv[1]
    out = argv[2] if len(argv) > 2 else os.path.splitext(exe)[0] + ".zip"
    if not os.path.isfile(exe):
        print("::error::setup.exe 없음: %s" % exe, file=sys.stderr)
        return 1

    with open(exe, "rb") as fh:
        payload = fh.read()
    if not payload:
        print("::error::setup.exe 가 0바이트다: %s" % exe, file=sys.stderr)
        return 1

    name = os.path.basename(exe)
    if os.path.exists(out):
        os.remove(out)

    # ★Info-ZIP `zip -X -j`(기본 레벨 6) — **기존 발행본과 바이트 동일**을 내는 도구다(실측).
    #   python `zipfile` 은 같은 레벨에서도 deflate 구현이 달라 csize 가 어긋난다
    #   (v0.14.4 발행본 csize=130153637 vs zipfile=130171483). 과거 자산과의 연속성을 위해
    #   Info-ZIP 을 쓰고, 없을 때만 zipfile 로 폴백한다(내용은 동일·바이트만 다름).
    stage = tempfile.mkdtemp(prefix="winzip_")
    try:
        staged = os.path.join(stage, name)
        shutil.copy2(exe, staged)
        os.utime(staged, (MTIME, MTIME))          # 2020-01-01 00:00:00 (발행 관례)
        zipbin = shutil.which("zip")
        if zipbin:
            r = subprocess.run([zipbin, "-q", "-X", "-j", os.path.abspath(out), staged])
            if r.returncode != 0 or not os.path.exists(out):
                print("::error::Info-ZIP 실패 (rc=%d)" % r.returncode, file=sys.stderr)
                return 1
        else:
            print("  ⚠ Info-ZIP(`zip`) 부재 — zipfile 폴백(내용 동일·바이트는 과거본과 다름)")
            zi = zipfile.ZipInfo(name, date_time=(2020, 1, 1, 0, 0, 0))
            zi.compress_type = zipfile.ZIP_DEFLATED
            zi.external_attr = 0o644 << 16
            with zipfile.ZipFile(out, "w") as z:
                z.writestr(zi, payload)
    finally:
        shutil.rmtree(stage, ignore_errors=True)

    # ── 왕복 검증 ──
    with zipfile.ZipFile(out) as z:
        names = z.namelist()
        if names != [name]:
            print("::error::zip 엔트리가 기대와 다르다: %r (기대 %r)" % (names, [name]), file=sys.stderr)
            return 1
        got = z.read(name)
    if sha256(got) != sha256(payload):
        print("::error::zip 내용이 setup.exe 와 다르다 — 포장 사고", file=sys.stderr)
        return 1

    print("✓ ZIP 변형 생성: %s (%d bytes, 원본 %d bytes)" % (out, os.path.getsize(out), len(payload)))
    print("  엔트리: %s" % names[0])
    print("  sha256(원본) = %s" % sha256(payload))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
