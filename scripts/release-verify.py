#!/usr/bin/env python3
"""릴리스 자산 검증 — 공개 발행 직전, 받아둔 자산 묶음이 온전한지 오프라인으로 판정한다 (2026-08-18).

`release-publish.yml` 이 draft 릴리스의 전 자산을 내려받은 뒤 이 스크립트를 부른다.
여기서 비영(非零)으로 죽으면 공개 발행(`--draft=false`)은 일어나지 않는다 — 이 파일이
공개 레인의 **마지막 fail-closed 관문**이다.

★왜 "복원"이 아니라 "재작성"인가 (실측 근거)
  이 파일은 0418f17 `feat(release): make signed promotion fail closed` 에서 24개 형제 파일과
  함께 태어났다. 그런데 main 에는 d67d8c3 `ci: register protected release publishing` 이
  **워크플로 1개(66줄)만 체리픽**해 얹었다 — 스크립트도 정책파일도 따라오지 않았다.
  그래서 `release-publish.yml` 이 부르는 이 파일이 main·태그 어디에도 없다(결함 6-2).

  옛 판본(0418f17)을 그대로 되살려도 v0.14.19 는 통과하지 못한다. 실측 3연속 실패:
    1) `release/assets-policy.json` 이 main·v0.14.19 **양쪽에 없다**(체리픽에서 누락)
       → `rendered_policy()` 가 시작하자마자 FileNotFoundError.
    2) 정책을 실제 14종에 맞춰 합성해 줘도 → `release verification failed: latest.json notes mismatch`.
       옛 판본은 notes 를 `f"cys {version}"` 로 못박았는데 실물은 한국어 안내 문장이다.
    3) 그 뒤로도 platforms 집합이 어긋난다 — 옛 판본은 3종만 허용하나 실물은 6종
       (`-app`·`-nsis` 별칭 추가). 업데이터 자산명도 옛 판본은 `cys_<V>_aarch64.app.tar.gz`
       를 가정하나 실물은 **버전 토큰이 없는** `cys_aarch64.app.tar.gz` 다 →
       `files[...]` KeyError 가 except 튜플에 없어 사유 없는 트레이스백으로 죽는다
       (비영 종료라 fail-closed 이긴 하나 "명확한 사유 출력" 계약 위반).
  즉 옛 판본은 v0.14.19 의 자산 계약과 어긋난다. 그래서 **검증 강도는 그대로 물려받고**
  (zip↔exe 바이트 동일성 · 업데이터 서명 교차대조 · 심볼릭링크 거부)
  **기대 목록의 정본만 갈아끼웠다**: 사라진 `assets-policy.json` 대신 `SHA256SUMS.txt`.

★`SHA256SUMS.txt` 를 정본으로 삼을 때의 구멍과 그 메움 (2026-08-18 적대적 리뷰로 교정)
  SUMS 는 자기 자신을 설명하는 목록이라, "애초에 빌드되지도·등재되지도 않은 자산"은
  디렉터리↔SUMS 대조만으로 절대 잡히지 않는다 — 양쪽 다 없으면 사이좋게 일치한다.
  그대로 두면 검증이 fail-open 이다. 구멍을 세 겹으로 메운다:
    (1) `REQUIRED_ASSETS` — SUMS 와 무관한 독립 하한선 8종
        (release-postprocess.py 의 `want4` 관례와 동형).
    (2) `UPDATER_PLATFORMS` — latest.json 의 **플랫폼 키 집합과 키→자산 결속**을 못박는다.
        나머지 5종(업데이터 tar.gz 2 + .sig 3)의 유일한 포획 지점이다.
        ★초판(2026-08-18 오전)은 여기를 무단언으로 뒀다가 실측 반증당했다: macOS 업데이터
        4종을 디렉터리·SUMS·latest.json 에서 통째로 지우고 platforms 를 windows 2행만
        남기면 **exit 0** 으로 통과했다(= macOS 전 사용자의 앱 내 Update 가 죽은 묶음이
        마지막 관문을 초록으로 통과). 또 6행 전부의 url 을 Windows 설치본으로 덮어써도
        통과했다(= macOS 업데이터가 NSIS exe 를 내려받는 묶음). 둘 다 지금은 죽는다.
    (3) `ASSET_SHAPES` — 확장자별 컨테이너 지문. 0바이트·엉뚱한 포맷을 잡는다.
        ★한계를 정확히 적는다. **잘림(truncation)을 잡는 건 세 종뿐**이다:
          · `.dmg` — 검사 지점이 머리가 아니라 **꼬리 512B('koly' 트레일러)** 라 잘리면 죽는다.
          · `.zip` — 중앙 디렉터리가 파일 끝에 있고, 게다가 5단계가 멤버를 끝까지 스트리밍한다.
          · `.tar.gz` — gzip 스트림을 **끝까지 풀어** CRC32·ISIZE 를 대조한다(잘리면 EOFError).
        나머지(`.exe`)는 머리 매직만 본다 — 잘린 exe 는 여기서 안 잡힌다.
        그리고 (1)(2)는 **이름**만, (3)은 **껍데기**까지만 본다. "이 설치본이 실제로
        설치되는가"는 이 스크립트의 사거리 밖이다(그건 CI 게이트·수동 설치 검증의 몫).
  (1)+(2) 로 v0.14.19 의 13종이 **전부** 덮인다: 8 + 5 = 13, 교집합 1(setup.exe) 제외하고
  빠짐없이 대응된다 — 조용히 통째로 사라져도 통과하는 자산은 이제 남지 않는다.
  (3)은 13종 **전부**에 걸린다. 확장자 규칙에 없는 자산이 섞이면 그것도 실패다 — 배포 구성이
  바뀌는 날 사람이 상수를 고치도록 강제하는 장치다(무단 자산 유입 차단).
  ★배포 구성을 의도적으로 바꾸는 날에는 이 세 상수를 **손으로** 고쳐야 한다. 그게 fail-closed 다.

★반대 방향도 막는다
  디렉터리에 있는데 SUMS 에 없는 파일은 **오류**다. 등재되지 않은 자산이 릴리스에 섞여 있으면
  무결성 보증 없이 함께 공개된다.

★자격증명을 쓰지 않는다
  `release-postprocess.py` 는 `git credential fill` 로 토큰을 얻지만 이 스크립트는 얻지 않는다.
  일부러 그렇다 — 내려받기는 워크플로가 `gh` 로 끝내고, 검증기는 (버전, 디렉터리)만의 순수 함수여야
  누구나·어디서나 같은 판정을 재현할 수 있다. 네트워크·토큰·리포 내 다른 파일에 의존하지 않는다.
  ★(0.14.53 WP12) 입력이 하나 늘었다 — 드래프트 **본문 파일**(`--body-file`, 필수). 본문은 빌드 뒤 사람이 고치는
  문면이라(실측 v0.14.52) 자산과 따로 받는다. 릴리스 디렉터리 **밖**에 둬야 한다 — 안에 두면 1단계의 미등재 자산이다.
  ★이 자기완결성 덕분에 워크플로가 이 파일 하나만 따로 가져와 쓸 수 있다 —
  `release-publish.yml` 의 2차 체크아웃(`path: .release-tools`, ref = 워크플로 자신의
  `github.sha`)이 그것이다. 발행 대상 태그 트리에 이 파일이 없어도 검증이 성립한다
  (v0.14.19 태그는 이 파일보다 먼저 잘렸다 — 그래서 태그 트리에서 부르면 `can't open file` 로 죽는다).

동반 회귀 테스트: `python3 scripts/tests/test_release_verify.py` (합성 픽스처 · 네트워크 불요)

사용:
  python3 scripts/release-verify.py --version 0.14.54 --release-dir reviewed-release --body-file "$RUNNER_TEMP/release-body.txt"
  python3 scripts/release-verify.py --version 0.14.54 --release-dir reviewed-release --body-file body.txt --print-assets

★8단계 Windows SAC 고지(D6-b · 0.14.53): setup.exe 의 PE 보안 디렉터리(Authenticode)가 없어야 하고, 본문과 latest.json
  notes 가 각각 `<!-- cys-win-sac: unsigned -->` 마커 정확히 하나와 SAC_KEY_PHRASES 두 문구를 실어야 한다. 서명된
  설치 파일 · signed-* 마커 · 마커 0/2개 · 깨진 PE 헤더는 전부 실패다(서명이 잠든 판 — 부서장 판정 0·4).

종료코드: 0=통과, 1=검증 실패, 2=인자 오류
"""
import argparse
import base64
import gzip
import hashlib
import json
import os
import re
import struct
import sys
import zipfile
import zlib
from datetime import datetime

SUMS_NAME = "SHA256SUMS.txt"          # ★과거 관례 — `SHA256SUMS`(확장자 없음) 아님
VERSION_RE = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+$")
# `<64자 소문자 hex><공백 2칸><파일명>` — shasum -a 256 의 기본 출력 형식.
# 파일명 문자셋을 좁게 못박아 경로 탈출(`../`)·공백 트릭을 원천 차단한다.
ROW_RE = re.compile(r"^([0-9a-f]{64})  ([A-Za-z0-9_.-]+)$")
# 파일명에 박힌 버전 토큰(X.Y.Z)을 뽑는다. `cys_aarch64.app.tar.gz` 처럼 토큰이 없는 이름도
# 정상이므로(업데이터 자산은 버전을 안 달고 나온다) "있으면 일치해야 한다"로 다룬다.
TOKEN_RE = re.compile(r"[0-9]+\.[0-9]+\.[0-9]+")

# ★SUMS 와 무관한 독립 하한선 ① — 이게 없으면 "빌드 자체가 안 돼 SUMS 에도 안 실린 자산"을 못 잡는다.
#   `{v}` 는 --version 으로 채운다. 여기 없는 5종은 아래 `UPDATER_PLATFORMS` 가 강제한다.
REQUIRED_ASSETS = (
    "cys_{v}_aarch64.dmg",        # macOS Apple Silicon 다운로드 버튼
    "cys_{v}_x64.dmg",            # macOS Intel 다운로드 버튼
    "cys_{v}_x64-setup.exe",      # Windows 다운로드 버튼
    "cys_{v}_x64-setup.zip",      # .exe 직다운이 막힌 환경용 4번째 버튼
    "latest.json",                # 앱 내 Update 버튼(업데이터)의 정본
    "pack.tar.gz",                # cysjavis 팩 본체
    "pack-manifest.json",         # 팩 매니페스트
    "pack-manifest.json.minisig", # 팩 매니페스트 minisign 서명
)

# ★독립 하한선 ② — 업데이터 표면의 정본. **키 집합과 키→자산 결속을 둘 다 못박는다.**
#   집합을 안 박으면 macOS 레인이 통째로 사라져도 통과하고(행이 줄면 검사도 함께 줄어든다),
#   결속을 안 박으면 darwin 행이 Windows 설치본을 가리켜도 통과한다. 둘 다 실측으로 확인된
#   fail-open 이었다(docstring 참조).
#   출처: tauri-action 이 매트릭스 3잡의 산출을 병합해 내는 실물 표면. 로컬 백업 전수 실측
#   (2026-08-18): v0.14.0 … v0.14.19 **21개 묶음 · 126행 전부**가 이 6키·이 결속과 일치,
#   불일치 0. (v0.13.23 은 3키 시절이라 대상 밖 — `-app`·`-nsis` 는 tauri v2 업데이터의
#   별칭 키이고, 옛 판본이 3키로 못박았다가 깨진 지점이 바로 여기다.)
UPDATER_PLATFORMS = {
    "darwin-aarch64":      "cys_aarch64.app.tar.gz",
    "darwin-aarch64-app":  "cys_aarch64.app.tar.gz",
    "darwin-x86_64":       "cys_x64.app.tar.gz",
    "darwin-x86_64-app":   "cys_x64.app.tar.gz",
    "windows-x86_64":      "cys_{v}_x64-setup.exe",
    "windows-x86_64-nsis": "cys_{v}_x64-setup.exe",
}
# latest.json 최상위 필드 집합 — notes 삭제·임의 키 주입을 잡는다(0418f17 판본에서 계승).
LATEST_JSON_FIELDS = {"version", "notes", "pub_date", "platforms"}

# ★독립 하한선 ③ — 확장자별 컨테이너 지문. 이름만 맞고 알맹이가 0바이트/쓰레기인 묶음을 잡는다.
#   전부 로컬 백업 v0.14.19 실측으로 대조했다(2026-08-18 · 13종 전수):
#     dmg      = UDIF 규격의 **끝 512B 'koly' 트레일러**(머리는 zlib 압축이라 `78 01` 로 시작한다
#                — 머리 매직으로는 dmg 를 식별할 수 없다. 실측으로 확인한 지점)
#     exe      = MZ · zip = PK\x03\x04 · tar.gz = gzip 매직 1f 8b · minisig = minisign 텍스트 헤더
#     sig      = **base64 텍스트**이고 풀면 `untrusted comment:` 로 시작한다(tauri updater 서명).
#                실측: 3종 전부 `dW50cnVzdGVkIGNvbW…` = b64("untrusted com…").
#   ★suffix 매칭은 endswith 라 충돌이 없다 — `*.exe.sig`/`*.tar.gz.sig` 는 `.sig` 로,
#     `*.json.minisig` 는 `.minisig` 로만 걸린다(".minisig"[-4:] == "isig" ≠ ".sig").
#   ★사거리 명시: 여기까지가 "껍데기"다. 설치본이 실제로 설치되는지는 보지 않는다.
ASSET_SHAPES = (
    (".tar.gz",  "gzip"),        # 특수 규칙 — 매직 + 스트림을 끝까지 풀어 CRC32·ISIZE 대조
    (".minisig", b"untrusted comment:"),
    (".dmg",     "koly"),        # 특수 규칙 — 머리가 아니라 꼬리 512바이트를 본다
    (".exe",     b"MZ"),
    (".zip",     b"PK\x03\x04"),
    (".sig",     "minisign-b64"),  # 특수 규칙 — base64 로 풀리고 minisign 헤더여야 한다
    (".json",    "json"),        # 특수 규칙 — 파싱되고 최상위가 객체여야 한다
)


# ★Windows 스마트 앱 컨트롤(SAC) 고지 ↔ 실제 서명 상태 (WP12-WINSAC-0.14.53 · D6-b · 오너 상시 규칙
#   OWNER-20261023-WIN-SAC-STANDING-CHECK). 0.14.53 은 **서명이 잠든 판**이다(부서장 판정 0 — 오너 자격증명 없음).
#   그래서 통과는 단 한 상태뿐이다: setup.exe 에 Authenticode 보안 디렉터리가 **없고**(PE 데이터 디렉터리 4 크기 0),
#   본문과 latest.json notes 가 각각 **정확히 하나**의 `unsigned` 마커와 두 핵심 문구를 싣는다. 그 밖의 전부는 실패다 —
#   서명된 설치 파일(검토되지 않은 서명 경로가 돌았다는 뜻), signed-unverified·signed-verified 마커, 마커 0개·2개.
#   문구 정본은 release.yml 의 releaseBody 다 — test_release_verify.py 가 그 문면을 이 검사에 통과시켜 생산자와 관문이
#   어긋나지 않음을 태그 전에 잰다.
SAC_TOKEN = "cys-win-sac"
SAC_MARKER_RE = re.compile(r"<!--\s*cys-win-sac:\s*(.*?)\s*-->", re.S)
SAC_UNSIGNED = "unsigned"
SAC_KEY_PHRASES = (
    "이 판의 cys 설치 파일과 cys 실행파일은 코드 서명이 없습니다",
    "this release's cys installer and cys executables are not code-signed",
)
# PE 헤더는 파일 머리에 있다(실물 e_lfanew 는 수백 바이트). 1 MiB 를 넘는 헤더 위치는 손상으로 본다.
PE_HEAD_LIMIT = 1 << 20


class VerifyError(Exception):
    """검증 실패. 메시지가 곧 사용자에게 보이는 사유다."""


def sha256_file(p):
    h = hashlib.sha256()
    with open(p, "rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def collect_files(release_dir):
    """디렉터리의 일반 파일만 {이름: 경로}로 모은다. 그 외 항목은 전부 거부(fail-closed)."""
    if not os.path.isdir(release_dir):
        raise VerifyError("릴리스 디렉터리가 없다: %s" % release_dir)
    files = {}
    for name in sorted(os.listdir(release_dir)):
        path = os.path.join(release_dir, name)
        # ★심볼릭링크 거부: 링크를 끼워 넣으면 "검증한 바이트"와 "발행되는 바이트"가 갈릴 수 있다.
        if os.path.islink(path) or not os.path.isfile(path):
            raise VerifyError("일반 파일이 아닌 항목이 섞여 있다: %s" % name)
        files[name] = path
    if not files:
        raise VerifyError("릴리스 디렉터리가 비어 있다: %s" % release_dir)
    return files


def parse_sums(path):
    """SHA256SUMS.txt 를 {파일명: 다이제스트}로 판다. 한 줄이라도 이상하면 실패."""
    try:
        with open(path, encoding="utf-8") as fh:
            text = fh.read()
    except UnicodeError as e:
        raise VerifyError("%s 가 UTF-8 로 읽히지 않는다: %s" % (SUMS_NAME, e))
    rows = text.splitlines()
    if not rows:
        raise VerifyError("%s 가 비어 있다" % SUMS_NAME)
    sums = {}
    for i, row in enumerate(rows, 1):
        m = ROW_RE.match(row)
        if not m:
            raise VerifyError("%s %d번째 줄 형식 오류: %r" % (SUMS_NAME, i, row))
        digest, name = m.group(1), m.group(2)
        if name in sums:
            raise VerifyError("%s 에 중복 등재: %s" % (SUMS_NAME, name))
        sums[name] = digest
    # ★자기 자신 제외 규약(release-postprocess.py 3단계와 동일). 자기를 등재하면 논리적으로
    #   대조가 불가능하다 — 해시를 쓰는 순간 파일이 바뀌기 때문이다.
    if SUMS_NAME in sums:
        raise VerifyError("%s 가 자기 자신을 등재했다 — 자기 제외 규약 위반" % SUMS_NAME)
    return sums


def check_version_tokens(version, names):
    """파일명에 박힌 버전 토큰이 전부 --version 과 같은지 본다."""
    stamped = 0
    for name in sorted(names):
        tokens = set(TOKEN_RE.findall(name))
        if not tokens:
            continue                      # 버전을 안 다는 자산(업데이터 tar.gz 등) — 정상
        stamped += 1
        bad = sorted(t for t in tokens if t != version)
        if bad:
            raise VerifyError("파일명 버전 토큰 불일치: %s 에 %s (기대 %s)"
                              % (name, ", ".join(bad), version))
    # ★공허한 통과 차단: 전 자산이 버전을 안 달았다면 "다른 버전 묶음"을 잡을 근거가 없다.
    if stamped == 0:
        raise VerifyError("버전 토큰이 박힌 자산이 하나도 없다 — %s 묶음임을 확인할 수 없다" % version)


def check_windows_zip(version, files):
    """zip 이 setup.exe **한 개**만, 바이트 동일하게 품고 있는지(0418f17 판본에서 계승)."""
    exe = "cys_%s_x64-setup.exe" % version
    zipname = "cys_%s_x64-setup.zip" % version
    with zipfile.ZipFile(files[zipname]) as archive:
        members = archive.namelist()
        if members != [exe]:
            raise VerifyError("zip 은 루트에 %s 하나만 있어야 한다 — 실제: %r" % (exe, members))
        # 대용량이라 스트리밍으로 대조한다(통째로 메모리에 올리지 않는다).
        h = hashlib.sha256()
        with archive.open(exe) as fh:
            for chunk in iter(lambda: fh.read(1 << 20), b""):
                h.update(chunk)
    if h.hexdigest() != sha256_file(files[exe]):
        raise VerifyError("zip 안의 %s 가 릴리스의 .exe 와 바이트 동일하지 않다" % exe)


def read_text_strict(path, what):
    """텍스트 자산 읽기 — 바이너리 쓰레기가 들어오면 UnicodeDecodeError 대신 사유를 낸다."""
    try:
        with open(path, encoding="utf-8") as fh:
            return fh.read()
    except UnicodeDecodeError as e:
        raise VerifyError("%s 가 UTF-8 텍스트가 아니다: %s" % (what, e))


def check_asset_shapes(files, names):
    """확장자별 컨테이너 지문 — 이름은 맞는데 알맹이가 0바이트·쓰레기·엉뚱한 포맷인 자산을 잡는다.

    ★규칙에 없는 확장자는 **실패**다. 배포 구성이 바뀌는 날 사람이 ASSET_SHAPES 를 고치도록
      강제하는 장치이자, 무단 자산 유입을 막는 빗장이다.
    """
    for name in sorted(names):
        path = files[name]
        rule = None
        for suffix, candidate in ASSET_SHAPES:
            if name.endswith(suffix):
                rule = candidate
                break
        if rule is None:
            raise VerifyError("확장자 규칙에 없는 자산이다(배포 구성이 바뀌었으면 "
                              "ASSET_SHAPES 를 손으로 갱신하라): %s" % name)

        size = os.path.getsize(path)
        if size == 0:
            raise VerifyError("자산이 0바이트다: %s" % name)

        if isinstance(rule, bytes):                       # 머리 매직
            with open(path, "rb") as fh:
                head = fh.read(len(rule))
            if head != rule:
                raise VerifyError("컨테이너 지문 불일치: %s (머리 %r · 기대 %r)" % (name, head, rule))

        elif rule == "koly":                              # dmg — 꼬리 512B UDIF 트레일러
            if size < 512:
                raise VerifyError("dmg 가 UDIF 트레일러(512B)보다 작다: %s (%d바이트)" % (name, size))
            with open(path, "rb") as fh:
                fh.seek(-512, os.SEEK_END)
                if fh.read(4) != b"koly":
                    raise VerifyError("dmg 끝에 UDIF 'koly' 트레일러가 없다"
                                      "(잘렸거나 dmg 가 아니다): %s" % name)

        elif rule == "gzip":                              # tar.gz — 끝까지 풀어 CRC32·ISIZE 대조
            with open(path, "rb") as fh:
                if fh.read(2) != b"\x1f\x8b":
                    raise VerifyError("gzip 매직이 아니다: %s" % name)
            try:
                with gzip.open(path, "rb") as fh:
                    for _ in iter(lambda: fh.read(1 << 22), b""):
                        pass
            except (OSError, EOFError, zlib.error) as e:
                raise VerifyError("gzip 스트림이 끝까지 풀리지 않는다(잘렸거나 손상): %s — %s: %s"
                                  % (name, type(e).__name__, e))

        elif rule == "minisign-b64":                      # tauri updater 서명
            raw = read_text_strict(path, name).strip()
            try:
                decoded = base64.b64decode(raw, validate=True)
            except ValueError as e:                       # binascii.Error 는 ValueError 하위
                raise VerifyError("서명 파일이 base64 가 아니다: %s — %s" % (name, e))
            if not decoded.startswith(b"untrusted comment:"):
                raise VerifyError("서명 파일이 minisign 형식이 아니다"
                                  "(풀어도 'untrusted comment:' 로 시작하지 않는다): %s" % name)

        elif rule == "json":
            try:
                obj = json.loads(read_text_strict(path, name))
            except json.JSONDecodeError as e:
                raise VerifyError("JSON 자산이 파싱되지 않는다: %s — %s" % (name, e))
            if not isinstance(obj, dict):
                raise VerifyError("JSON 자산의 최상위가 객체가 아니다: %s" % name)

        else:                                             # 상수를 고치면서 분기를 안 단 경우
            raise VerifyError("ASSET_SHAPES 규칙을 해석할 수 없다: %s → %r" % (name, rule))


def check_latest_json(version, files, sums):
    """업데이터 정본 대조 — 앱 내 Update 버튼이 실제로 무엇을 받게 되는지 본다.

    네 겹으로 못박는다. 앞의 둘이 없으면 검사 자체가 사라지거나 엉뚱한 걸 검사한다:
      ① 최상위 필드 **집합** == LATEST_JSON_FIELDS — notes 삭제·임의 키 주입 차단.
      ② platforms **키 집합** == UPDATER_PLATFORMS 의 키 — 행이 줄면 그 행에 걸린 검사도
         함께 사라진다(fail-open). macOS 레인이 통째로 빠진 묶음이 여기서 죽는다.
      ③ 키 → **자산 결속** — darwin 행이 Windows 설치본을 가리키는 묶음이 여기서 죽는다.
      ④ 가리킨 자산과 그 `.sig` 가 SUMS 에 등재돼 있고, signature 값 == `.sig` 파일 내용.
    ①②③ 은 실측으로 반증당해 뒤늦게 채운 구멍이다 — 없앨 때는 반증부터 다시 하라.
    """
    try:
        latest = json.loads(read_text_strict(files["latest.json"], "latest.json"))
    except json.JSONDecodeError as e:
        raise VerifyError("latest.json 이 JSON 이 아니다: %s" % e)
    if not isinstance(latest, dict):
        raise VerifyError("latest.json 최상위는 객체여야 한다")
    # ── ① 최상위 필드 집합 ──
    if set(latest) != LATEST_JSON_FIELDS:
        raise VerifyError("latest.json 최상위 필드 집합 오류 — 누락 %s · 잉여 %s (기대 %s)"
                          % (sorted(LATEST_JSON_FIELDS - set(latest)) or "없음",
                             sorted(set(latest) - LATEST_JSON_FIELDS) or "없음",
                             sorted(LATEST_JSON_FIELDS)))
    if latest.get("version") != version:
        raise VerifyError("latest.json version 불일치: %r (기대 %s)" % (latest.get("version"), version))
    # notes 는 값을 못박지 않는다 — 옛 판본이 `f"cys {version}"` 로 박았다가 실물(한국어 안내
    # 문장)과 어긋나 깨졌다. "비어 있지 않은 문자열"까지만 요구한다.
    if not isinstance(latest.get("notes"), str) or not latest["notes"].strip():
        raise VerifyError("latest.json notes 가 비어 있거나 문자열이 아니다: %r" % (latest.get("notes"),))

    pub_date = latest.get("pub_date")
    if not isinstance(pub_date, str):
        raise VerifyError("latest.json pub_date 가 문자열이 아니다")
    try:
        parsed = datetime.fromisoformat(pub_date.replace("Z", "+00:00"))
    except ValueError:
        raise VerifyError("latest.json pub_date 가 RFC3339 가 아니다: %r" % pub_date)
    if parsed.tzinfo is None:
        raise VerifyError("latest.json pub_date 에 타임존이 없다: %r" % pub_date)

    platforms = latest.get("platforms")
    if not isinstance(platforms, dict) or not platforms:
        raise VerifyError("latest.json platforms 가 비어 있거나 객체가 아니다")

    # ── ② 플랫폼 키 집합 ── (행이 줄면 검사도 함께 줄어드는 fail-open 을 여기서 막는다)
    if set(platforms) != set(UPDATER_PLATFORMS):
        raise VerifyError("latest.json platforms 키 집합 오류 — 누락 %s · 잉여 %s (기대 %d종 %s)"
                          % (sorted(set(UPDATER_PLATFORMS) - set(platforms)) or "없음",
                             sorted(set(platforms) - set(UPDATER_PLATFORMS)) or "없음",
                             len(UPDATER_PLATFORMS), sorted(UPDATER_PLATFORMS)))

    base = "https://github.com/idoforgod/cys-terminal/releases/download/v%s/" % version
    for key in sorted(platforms):
        row = platforms[key]
        if not isinstance(row, dict) or set(row) != {"signature", "url"}:
            raise VerifyError("업데이터 행 필드 집합 오류: %s" % key)
        # ── ③ 키 → 자산 결속 ── (darwin 행이 Windows 설치본을 가리키는 사고를 막는다)
        asset = UPDATER_PLATFORMS[key].format(v=version)
        expected_url = base + asset
        if row["url"] != expected_url:
            raise VerifyError("업데이터 url 결속 위반: %s → %r (기대 %s)"
                              % (key, row["url"], expected_url))
        # ── ④ 등재·서명 ──
        if asset not in sums:
            raise VerifyError("업데이터가 등재되지 않은 자산을 가리킨다: %s → %s" % (key, asset))
        sig_name = asset + ".sig"
        if sig_name not in sums:
            raise VerifyError("업데이터 자산의 서명 파일이 없다: %s" % sig_name)
        expected = read_text_strict(files[sig_name], sig_name).strip()
        if not expected:
            raise VerifyError("서명 파일이 비어 있다: %s" % sig_name)
        if row["signature"] != expected:
            raise VerifyError("업데이터 서명 불일치: %s (%s)" % (key, sig_name))
    return sorted(platforms)


def pe_security_directory(path):
    """PE 데이터 디렉터리 4(IMAGE_DIRECTORY_ENTRY_SECURITY)의 (파일 오프셋, 크기). 크기 > 0 = Authenticode 서명 실재.

    ★fail-closed: 헤더가 하나라도 어긋나면 '미서명'으로 기본값을 주지 않고 VerifyError 다 — 잘린·위조된 setup.exe 를
      미서명으로 읽어 통과시키는 길을 막는다. 유효성(신뢰 체인)은 여기서 보지 않는다(Windows 레그의 일).
    """
    size_total = os.path.getsize(path)
    with open(path, "rb") as fh:
        head = fh.read(min(size_total, PE_HEAD_LIMIT))

    def bad(why):
        return VerifyError("windows-sac: %s 는 정상 PE 가 아니다(%s)" % (os.path.basename(path), why))

    if len(head) < 0x40 or head[:2] != b"MZ":
        raise bad("MZ 머리 없음")
    e_lfanew = struct.unpack_from("<I", head, 0x3C)[0]
    if e_lfanew < 0x40 or e_lfanew + 24 > len(head):
        raise bad("e_lfanew=%d 이 파일 머리 밖" % e_lfanew)
    if head[e_lfanew:e_lfanew + 4] != b"PE\0\0":
        raise bad("PE 서명 없음")
    coff = e_lfanew + 4
    opt_size = struct.unpack_from("<H", head, coff + 16)[0]
    opt = coff + 20
    if opt_size < 2 or opt + opt_size > len(head):
        raise bad("옵션 헤더 크기 %d 가 파일 밖" % opt_size)
    magic = struct.unpack_from("<H", head, opt)[0]
    if magic == 0x10B:      # PE32: NumberOfRvaAndSizes @92, DataDirectory @96
        n_off, dd = opt + 92, opt + 96
    elif magic == 0x20B:    # PE32+: NumberOfRvaAndSizes @108, DataDirectory @112
        n_off, dd = opt + 108, opt + 112
    else:
        raise bad("옵션 헤더 magic 0x%X" % magic)
    sec = dd + 4 * 8
    if sec + 8 > opt + opt_size:
        raise bad("데이터 디렉터리 4 가 옵션 헤더 밖")
    if struct.unpack_from("<I", head, n_off)[0] < 5:
        raise bad("NumberOfRvaAndSizes < 5")
    off, size = struct.unpack_from("<II", head, sec)
    if size and (off == 0 or off + size > size_total):
        raise bad("보안 디렉터리(오프셋 %d · 크기 %d)가 파일(%d B) 밖" % (off, size, size_total))
    return off, size


def check_sac_text(text, where):
    """SAC 고지 문면 대조 — 마커는 정확히 하나여야 하고 그 값은 `unsigned`, 두 핵심 문구가 모두 있어야 한다."""
    n = text.count(SAC_TOKEN)
    if n == 0:
        raise VerifyError("windows-sac: %s 에 SAC 고지 마커(<!-- cys-win-sac: unsigned -->)가 없다 — "
                          "서명 없는 Windows 판은 그 사실을 적지 않으면 공개할 수 없다" % where)
    if n > 1:
        raise VerifyError("windows-sac: %s 에 SAC 마커가 %d개다 — 정확히 하나여야 한다" % (where, n))
    m = SAC_MARKER_RE.search(text)
    value = m.group(1) if m else None
    if value is None:
        raise VerifyError("windows-sac: %s 의 SAC 마커 형식이 깨졌다(`<!-- cys-win-sac: unsigned -->`)" % where)
    if value.startswith("signed-verified"):
        raise VerifyError("windows-sac: %s 가 signed-verified 다 — 서명·SAC 검증 고지는 설치 파일 해시·버전·런 id 에 결속된 "
                          "신뢰된 실 SAC 보고가 필요하며 이 판에서는 지원하지 않는다" % where)
    if value != SAC_UNSIGNED:
        raise VerifyError("windows-sac: %s 의 SAC 마커 값 %r 은 이 판에서 허용되지 않는다 — 서명이 잠든 판이라 "
                          "'unsigned' 만 정직하다" % (where, value))
    absent = [p for p in SAC_KEY_PHRASES if p not in text]
    if absent:
        raise VerifyError("windows-sac: %s 에 SAC 고지 핵심 문구가 없다 — %s" % (where, " / ".join(absent)))


def check_windows_sac(version, files, body_text):
    """D6-b — 공개 대상 setup.exe 의 실제 서명 상태와 사용자가 읽는 두 문면(본문 · latest.json notes)을 대조한다."""
    setup = "cys_{v}_x64-setup.exe".format(v=version)
    if setup not in files:
        # REQUIRED_ASSETS(2단계)가 이미 강제하므로 도달하면 상수 회귀다 — n/a 로 통과시키지 않는다.
        raise VerifyError("windows-sac: %s 가 없다 — 대조할 대상이 없으면 판정할 수 없다" % setup)
    _, size = pe_security_directory(files[setup])
    if size:
        raise VerifyError("windows-sac: %s 에 Authenticode 서명이 있다(보안 디렉터리 %d B) — 이 판은 서명이 잠든 판이라 "
                          "서명된 설치 파일은 검토되지 않은 서명 경로가 돌았다는 뜻이다" % (setup, size))
    check_sac_text(body_text, "릴리스 본문")
    latest = json.loads(read_text_strict(files["latest.json"], "latest.json"))
    check_sac_text(latest["notes"], "latest.json notes")
    return SAC_UNSIGNED


def verify(version, release_dir, body_text):
    if not VERSION_RE.match(version):
        raise VerifyError("--version 은 X.Y.Z 여야 한다: %r" % version)

    files = collect_files(release_dir)
    if SUMS_NAME not in files:
        raise VerifyError("%s 가 없다 — 기대 자산 목록의 정본이 없으면 판정할 수 없다" % SUMS_NAME)
    sums = parse_sums(files[SUMS_NAME])

    # ── 1. 완전성: 디렉터리 ↔ SUMS 양방향 대조 (누락 0 · 미등재 0) ──
    present = set(files) - {SUMS_NAME}
    listed = set(sums)
    missing = sorted(listed - present)
    unlisted = sorted(present - listed)
    if missing:
        raise VerifyError("자산 누락 %d종 — %s" % (len(missing), ", ".join(missing)))
    if unlisted:
        raise VerifyError("SUMS 에 없는 자산이 섞여 있다 %d종(무결성 보증 없이 공개된다) — %s"
                          % (len(unlisted), ", ".join(unlisted)))

    # ── 2. 독립 하한선: SUMS 가 통째로 빠뜨린 자산을 잡는 유일한 지점 ──
    required = [a.format(v=version) for a in REQUIRED_ASSETS]
    absent = [a for a in required if a not in listed]
    if absent:
        raise VerifyError("배포 필수 자산 누락 %d종 — %s" % (len(absent), ", ".join(absent)))

    # ── 3. 파일명 버전 토큰 ──
    check_version_tokens(version, listed)

    # ── 4. 컨테이너 지문 — 이름만 맞고 알맹이가 쓰레기인 자산을 잡는다 ──
    #    ★재계산 대조보다 **먼저** 둔다. SUMS 를 쓰레기에 맞춰 재생성한 묶음은 4단계를
    #      사이좋게 통과하므로(체크섬은 일치한다), 그런 묶음의 사유를 여기서 먼저 낸다.
    check_asset_shapes(files, listed)

    # ── 5. 전 줄 재계산 대조 ──
    for name in sorted(sums):
        if sha256_file(files[name]) != sums[name]:
            raise VerifyError("체크섬 불일치: %s" % name)

    # ── 6. Windows zip ↔ exe 바이트 동일성 ──
    check_windows_zip(version, files)

    # ── 7. 업데이터(latest.json) 교차 대조 ──
    platforms = check_latest_json(version, files, sums)

    # ── 8. Windows SAC 고지 ↔ 서명 상태 (D6-b) — latest.json 이 7단계에서 검증된 뒤에 notes 를 읽는다 ──
    check_windows_sac(version, files, body_text)

    return sorted(sums), platforms


def main():
    ap = argparse.ArgumentParser(description="릴리스 자산 묶음을 오프라인으로 검증한다")
    ap.add_argument("--version", required=True, help="기대 버전 (X.Y.Z — 태그의 v 를 뗀 값)")
    ap.add_argument("--release-dir", required=True, help="전 자산을 내려받아 둔 디렉터리")
    ap.add_argument("--body-file", required=True,
                    help="드래프트 릴리스 본문을 담은 파일(--release-dir **밖**에 둔다 — 안에 두면 미등재 자산이다)")
    ap.add_argument("--print-assets", action="store_true", help="검증된 자산명을 한 줄씩 출력")
    args = ap.parse_args()

    try:
        body_text = read_text_strict(args.body_file, "--body-file")
        assets, platforms = verify(args.version, args.release_dir, body_text)
    except VerifyError as e:
        print("::error::릴리스 검증 실패 — %s" % e, file=sys.stderr)
        return 1
    except Exception as e:
        # ★fail-closed 의 마지막 빗장: 예상 못 한 예외도 "판정 불가 = 실패"로 떨어뜨린다.
        #   (옛 판본은 KeyError 를 못 받아 사유 없는 트레이스백을 냈다.)
        print("::error::릴리스 검증 중단(판정 불가) — %s: %s"
              % (type(e).__name__, e), file=sys.stderr)
        return 1

    if args.print_assets:
        print("\n".join(assets))
    print("✅ 릴리스 검증 통과 — %s · 자산 %d종(+%s) · 업데이터 %d행 전부 서명 일치 · windows-sac: unsigned 고지 일치"
          % (args.version, len(assets), SUMS_NAME, len(platforms)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
