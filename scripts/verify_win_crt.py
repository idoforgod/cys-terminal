#!/usr/bin/env python3
"""Windows CRT 발행 차단 게이트 — VCRUNTIME 임포트 스윕 + 3카테고리 수리 마커.

무엇을 막는가:
  A) CRT 정책 회귀 — 우리 Rust 바이너리(cys/cysd/cys-browserd)가 VCRUNTIME140.dll 을
     동적 임포트하면 VC++ 재배포 패키지 없는 Windows 에서 기동 불능(2026-07-28 현장 결함).
     정책 SOT 는 .cargo/config.toml 의 crt-static — 이 게이트가 산출물에서 그 실효를 증명한다.
  B) 베이스 오염 회귀 — 0.14.1 의 축A 수리(스케줄 실패 표면화·윈도우 bash 승격·폴백 경고)가
     빠진 구베이스(0.14.0 이하) 빌드의 출하 차단.
     ★0.14 라인 적응(2026-07-28): 상류(v0.13.23) 마커 6종 중 5종은 이 라인이 의도적으로
     백포트하지 않는 큐·편성·usage 계열이라 그대로 쓰면 정상 빌드가 영구 차단된다 —
     마커를 0.14.1 수리 실체(전부 cysd 신규 문자열·0.14.0 바이너리에 부재)로 교체했다.

판정 방법(혼용 금지):
  A 는 PE 임포트 디렉토리 파싱만 사용한다 — 원시 바이트 grep 은 v0.13.22 실측에서 370개 중
    300+개 오검출(데이터 섹션 문자열 잔존)로 부적합 판명. GitHub CI 러너에는 VC 재배포가
    프리인스톨돼 있어 이 결함은 런타임 재현이 불가능하므로, 임포트 테이블 정적 검사가
    CI 에서 가능한 유일한 결정론 검증이다.
  B 는 cysd.exe 원시 바이트의 UTF-8 마커 존재 검사(≥1) — 존재 검사에는 바이트 서치가 적법.
    전제: pack 이 비압축 임베드(현행 build.rs). 임베드가 압축되면 이 검사는 시끄럽게 실패하며
    그때 방식을 갱신한다. 마커 목록은 아래 상수 한 곳에만 둔다.

임포트 판정 2단(적대 검증 반영):
  1단(엄격): cys.exe·cysd.exe·cys-browserd.exe 는 vcruntime 임포트 자체가 무조건 실패.
     같은 폴더 DLL 동봉으로도 통과 불가 — DLL 드롭은 런타임엔 유효해도 crt-static 정책
     회귀의 무음 통과 경로가 되므로 봉쇄한다.
     ★설치 훅 스테이징 별칭(2026-08-29 W4): `<이름>.new.exe`·`<이름>.prev[숫자].exe` 는
     정식 `<이름>.exe` 와 같은 등급으로 판정한다(canonical_name — NSIS-CONTRACT.md §1).
     존재하면 정식과 동일하게 검사되고, 부재해도 어떤 검사도 실패하지 않는다.
  2단(규칙): 그 외 exe 는 같은 디렉토리에 vcruntime140.dll 동봉 시만 허용(app-local =
     실제 런타임 안전 불변식. v0.13.22 실측: runtime/python/{python,python3,pythonw}.exe
     3종이 자연 통과. 경로 하드코딩 없음 — 레이아웃 변화 내성).

종료 코드: 0=통과 / 1=검사A 위반 / 2=검사B 마커 결손 / 3=PE 파싱 실패(fail-closed) /
          4=입력·추출 오류. 모든 판정 내역을 stdout 에 남긴다(릴리스 로그 증거).
  --sac-census(0.14.53 WP12 · D6-a): CRT 검사 대신 출하 PE 서명 census 만 한다. 0=통과 / 3=우리 파일을 읽을 수 없거나 ·
          MZ 가 아니거나 · 정상 PE 가 아님(이름 명시) / 4=입력·추출 오류 / 5=우리 파일에 Authenticode 서명(서명이 잠든 판 위반) /
          6=필수 우리 파일(cys·cysd·cys-app·uninstall) 부재(이름 명시) 또는 매 실행 대조가 눈멂.

사용:
  python3 scripts/verify_win_crt.py --setup release-candidate/cys_X.Y.Z_x64-setup.exe
  python3 scripts/verify_win_crt.py --tree <추출된 설치 트리>  [--sevenzip 7zz]
  python3 scripts/verify_win_crt.py --setup <setup.exe> --sac-census --report windows-signing-report.json
"""

import argparse
import json
import os
import re
import struct
import subprocess
import sys
import tempfile

# Windows 콘솔 기본 cp1252 에서 한국어 판정 출력이 UnicodeEncodeError 로 죽으면 게이트가
# 판정 이전에 crash 한다(상류 run 30311769077 실측 · 0623a0f 백포트). 워크플로우 env(PYTHONUTF8)와
# 별개로 스크립트 자체에서도 방어한다 — 출력 인코딩 문제로 게이트가 죽는 일은 없어야 한다.
for _stream in (sys.stdout, sys.stderr):
    if hasattr(_stream, "reconfigure"):
        try:
            _stream.reconfigure(encoding="utf-8", errors="replace")
        except (ValueError, OSError):
            pass

# 정책 회귀를 무조건 차단하는 우리 바이너리(경로 말단 이름, 소문자).
STRICT_BINARIES = {"cys.exe", "cysd.exe", "cys-browserd.exe"}

# ── 설치 훅 스테이징 산출물(transient artefacts) 판정 별칭 (W4 · 2026-08-29) ──
# NSIS 훅의 잠금 무관 배치(src-tauri/nsis-hooks.nsh · NSIS-CONTRACT.md §1)는 신본을
# `<bin>.new.exe` 로 추출한 뒤 rename 2단으로 정식에 앉히고, 구본은 `<bin>.prev.exe`/
# `.prev2.exe`/`.prev3.exe`/`.prev<틱숫자>.exe` 슬롯으로 대피시킨다. 이 이름들은 정식
# 바이너리의 바이트 동일 사본(.new) 또는 구세대 사본(.prev*)이므로 **정식과 같은 등급**으로
# 판정한다 — `cys.new.exe` 가 규칙 2(app-local DLL 허용)로 미끄러지면 crt-static 정책
# 회귀가 vcruntime140.dll 동봉으로 무음 통과하는 구멍이 된다(엄격 우회 봉쇄).
# ★부재는 결코 실패가 아니다: 어느 검사도 이 이름들의 존재를 요구하지 않는다(성공 설치
# 후 `.new` 는 rename 으로 소멸이 정상 — 훅 ⑧단). 접미 문법은 cysd 부팅 가드의 가족
# 문법(`prev` + 숫자만)과 일치시킨다 — `cys.prevX.exe`(숫자 아님)는 별칭이 아니다.
TRANSIENT_SUFFIX_RE = re.compile(r"\.(?:new|prev\d*)\.exe$")


def canonical_name(fn):
    """말단 파일명(소문자화) → 판정용 정식 이름.

    `cys.new.exe`·`cys.prev.exe`·`cys.prev3.exe`·`cys.prev1234567.exe` → `cys.exe`.
    접미 문법 밖 이름은 소문자화만 하고 그대로 둔다(기존 판정과 동일).
    """
    lower = fn.lower()
    m = TRANSIENT_SUFFIX_RE.search(lower)
    if m:
        return lower[: m.start()] + ".exe"
    return lower

# 0.14.4 수리 세대 마커 — cysd 소스의 FIX_GENERATION const 바이트열(live RPC 참조+main() 부팅로그 임베드 · v4).
# ★코드 경로 문자열 휴리스틱 폐기(CI run 30357918475 실증 · 2026-07-28):
#   ①'command 비정상 종료'는 구베이스의 'text_command 비정상 종료'의 부분열이라 판별력 0
#   ②cfg(windows) 경로 리터럴('shell-fallback')은 컴파일러 재량으로 실빌드에서 미검출됨.
# `#[used]` static 은 참조 여부·최적화와 무관하게 링커가 보존하는 결정론 임베드이고
# 전 플랫폼 컴파일이라 맥 로컬 릴리스 빌드에서도 사전 검증된다. 0.14.3 이하엔 존재하지
# 않는 신조어 바이트열(0.14.4 세대는 w2) — 판별력은 CI 음성 대조(0.14.0 실물 0/1 단언)가 계속 증명한다.
FIX_MARKERS = [
    b"cys-fix-w2-gen-0.14.4",
]

NEEDLE = "vcruntime140"  # vcruntime140.dll·vcruntime140_1.dll 모두 접두 매치


def imports_of(path):
    """PE 파일이 임포트하는 DLL 이름 목록. PE 아니면 None(호출측 fail-closed)."""
    with open(path, "rb") as f:
        data = f.read()
    if len(data) < 0x40 or data[:2] != b"MZ":
        return None
    e_lfanew = struct.unpack_from("<I", data, 0x3C)[0]
    if e_lfanew + 24 > len(data) or data[e_lfanew:e_lfanew + 4] != b"PE\0\0":
        return None
    coff = e_lfanew + 4
    nsec = struct.unpack_from("<H", data, coff + 2)[0]
    opt_size = struct.unpack_from("<H", data, coff + 16)[0]
    opt = coff + 20
    magic = struct.unpack_from("<H", data, opt)[0]
    if magic == 0x20B:      # PE32+ (x64)
        dd_off = opt + 112
    elif magic == 0x10B:    # PE32 (32bit — pip 벤더 스텁 등)
        dd_off = opt + 96
    else:
        return None
    # ★경계 선검증(R3 codex high 수용): 선언된 optional header 가 DataDirectory[1](+8..+16)을
    # 담을 만큼 크고 파일 안에 실재해야 한다 — opt_size 를 속인 절단·조작 PE 는 파싱 실패(exit 3).
    if dd_off + 16 > opt + opt_size or opt + opt_size > len(data):
        return None
    ndd = struct.unpack_from("<I", data, dd_off - 4)[0]
    if ndd < 2:
        return []
    imp_rva, _ = struct.unpack_from("<II", data, dd_off + 8)  # DataDirectory[1] = Import Table
    if imp_rva == 0:
        return []
    secs = []
    sec0 = opt + opt_size
    # 섹션 테이블 전체가 파일 경계 안에 있어야 한다 — 밖이면 쓰레기 헤더로 오판정할 수 있다.
    if sec0 + 40 * nsec > len(data):
        return None
    for i in range(nsec):
        s = sec0 + 40 * i
        va = struct.unpack_from("<I", data, s + 12)[0]
        vsz = struct.unpack_from("<I", data, s + 8)[0]
        off = struct.unpack_from("<I", data, s + 20)[0]
        rsz = struct.unpack_from("<I", data, s + 16)[0]
        secs.append((va, max(vsz, rsz), off))

    def rva2off(rva):
        for va, sz, off in secs:
            if va <= rva < va + sz:
                return off + (rva - va)
        return None

    dlls = []
    off = rva2off(imp_rva)
    if off is None:
        # ★fail-closed 승격(R1 codex high 수용 · 2026-07-28): Import Directory 를 선언했는데
        # 그 RVA 가 어느 섹션에도 매핑되지 않으면 손상·비정상 PE 다. 종전 [](빈 임포트=통과)는
        # 조작·손상 PE 가 VCRUNTIME 임포트를 숨긴 채 PASS 하는 fail-open 구멍이었다 → 파싱
        # 실패(None)로 승격해 exit 3 경로에 태운다.
        return None
    terminated = False
    while off + 20 <= len(data):
        ilt, _, _, name_rva, iat = struct.unpack_from("<IIIII", data, off)
        if ilt == 0 and name_rva == 0 and iat == 0:
            terminated = True
            break
        noff = rva2off(name_rva)
        if noff is None:
            # nonzero descriptor 의 name_rva 미매핑도 동일 근거로 fail-closed — 조용히
            # 건너뛰면 그 항목의 DLL 이름(잠재적 vcruntime)이 검사에서 증발한다.
            return None
        end = data.index(b"\0", noff)
        dlls.append(data[noff:end].decode("ascii", "replace"))
        off += 20
    if not terminated:
        # ★R3 codex high 수용: zero terminator 없이 EOF 에 닿아 루프가 끝난 경우 — 절단·조작 PE 가
        # 뒷부분 descriptor(잠재적 vcruntime)를 자르고 '수집분만 정상'으로 통과하던 fail-open. None.
        return None
    return dlls


def check_imports(tree):
    """검사 A. 반환: (위반 목록, 파싱실패 목록, 허용 내역, 검사한 exe 수)."""
    violations, unparsed, allowed = [], [], []
    total = 0
    for dirpath, _, files in os.walk(tree):
        for fn in files:
            if not fn.lower().endswith(".exe"):
                continue
            total += 1
            p = os.path.join(dirpath, fn)
            rel = os.path.relpath(p, tree)
            try:
                dlls = imports_of(p)
            except Exception as e:  # 손상 PE 등 — fail-closed
                dlls = None
                print(f"  parse-error: {rel}: {e}")
            if dlls is None:
                unparsed.append(rel)
                continue
            if not any(NEEDLE in d.lower() for d in dlls):
                continue
            # 훅 스테이징 별칭(cys.new.exe 등)도 정식과 같은 등급으로 — 위 canonical_name 주석.
            if canonical_name(fn) in STRICT_BINARIES:
                violations.append((rel, "정책 바이너리 — 예외 불허(crt-static 회귀)"))
            elif os.path.isfile(os.path.join(dirpath, "vcruntime140.dll")):
                allowed.append(rel)
            else:
                violations.append((rel, "동봉 vcruntime140.dll 없음 — 클린 머신에서 기동 불능"))
    return violations, unparsed, allowed, total


def check_markers(tree):
    """검사 B. 반환: (결손 마커 목록, cysd 경로 or None).

    정식 `cysd.exe` 를 우선하고, 정식이 트리 어디에도 없을 때만 훅 스테이징 사본
    `cysd.new.exe`(이번 빌드와 바이트 동일 — NSIS-CONTRACT.md §1)로 폴백한다.
    훅 도입으로 setup.exe 추출 레이아웃(7z 의 NSIS `File /oname=` 해석)이 정식 이름을
    어떻게 놓든, 이 빌드의 cysd 바이트가 존재하면 마커 검사가 성립해야 한다 —
    `.new` 의 **부재**는 어느 경로에서도 실패가 아니다(정식만 있으면 종전과 동일).
    """
    cysd = None
    fallback = None
    for dirpath, _, files in os.walk(tree):
        for fn in files:
            if fn.lower() == "cysd.exe":
                cysd = os.path.join(dirpath, fn)
                break
            if fn.lower() == "cysd.new.exe" and fallback is None:
                fallback = os.path.join(dirpath, fn)
        if cysd:
            break
    if cysd is None and fallback is not None:
        print(f"  markers: 정식 cysd.exe 부재 — 스테이징 사본으로 폴백: {fallback}")
        cysd = fallback
    if cysd is None:
        return [m.decode("utf-8", "replace") for m in FIX_MARKERS], None
    with open(cysd, "rb") as f:
        blob = f.read()
    missing = [m.decode("utf-8", "replace") for m in FIX_MARKERS if m not in blob]
    return missing, cysd


# ── D6-a(0.14.53 WP12-WINSAC · 오너 상시 규칙 OWNER-20261023-WIN-SAC-STANDING-CHECK) — 출하 Windows PE 서명 census ──
# 이 판은 **서명이 잠든 판**이다(부서장 판정 0). 그래서 census 의 붉은 규칙은 하나다: **우리 파일**(setup.exe ·
# cys.exe · cysd.exe · cys-app.exe · uninstall.exe — 훅 스테이징 별칭 포함)에 Authenticode 보안 디렉터리가 있으면
# 검토되지 않은 서명 경로가 돌았다는 뜻이므로 실패(5)다. 벤더 파일(동봉 런타임)의 서명은 기록만 한다(U2 실측).
# 열거는 이름이 아니라 **내용**으로 한다 — 확장자와 무관하게 첫 2바이트가 `MZ` 인 모든 파일(.pyd 포함).
# 필수 우리 파일(SAC_REQUIRED_OWN)은 MZ 열거와 별도로 **이름으로** 재고한다 — 없거나 · 읽을 수 없거나 · MZ 가 아니거나 ·
# 정상 PE 가 아니면 그 이름을 단 적색이다(review r1 A-F1: 종전에는 cysd.exe 만 요구했다).
# 매 실행 대조: 서명된 cysd.exe 를 가진 합성 트리를 **실제 판정 경로**(sac_rows → sac_inventory → sac_verdict)에 넣어
# FAIL(5) 가 나는지 본다(review r1 A-F2) — 서명된 우리 파일을 못 보는 census 는 게이트가 아니다. 보안 디렉터리 판독은 release-verify.py 의 같은 이름 함수와 같은 꼴이다(그 파일은
# 저장소의 다른 파일에 의존하지 않는다는 계약이 있어 공유 모듈로 빼지 않았다 — 두 사본은 각자의 검체가 핀한다).
SAC_OWN_BINARIES = {"cys.exe", "cysd.exe", "cys-app.exe", "uninstall.exe"}
# Required from a setup.exe extraction (review r1 A-F1). MEASURED on the public v0.14.52 setup.exe with 7-Zip 26.05
# (7zz l / x, 2026-10-24 - listing in the WP12 phaseB evidence): the top level holds cys.exe, cysd.exe, cys-app.exe,
# their hook staging copies *.new.exe, and uninstall.exe (169,146 B, MZ, unsigned) - 7z materialises the makensis
# uninstaller, so it is required too. canonical_name maps the *.new.exe copies onto the same names.
SAC_REQUIRED_OWN = ("cys.exe", "cysd.exe", "cys-app.exe", "uninstall.exe")
SAC_CENSUS_SCHEMA = "cys-win-pe-census/1"


def security_directory(path):
    """PE 데이터 디렉터리 4 의 크기(바이트). 0 = Authenticode 서명 없음. 정상 PE 가 아니면 None."""
    total = os.path.getsize(path)
    with open(path, "rb") as f:
        head = f.read(min(total, 1 << 20))
    if len(head) < 0x40 or head[:2] != b"MZ":
        return None
    e_lfanew = struct.unpack_from("<I", head, 0x3C)[0]
    if e_lfanew < 0x40 or e_lfanew + 24 > len(head) or head[e_lfanew:e_lfanew + 4] != b"PE\0\0":
        return None
    coff = e_lfanew + 4
    opt_size = struct.unpack_from("<H", head, coff + 16)[0]
    opt = coff + 20
    if opt_size < 2 or opt + opt_size > len(head):
        return None
    magic = struct.unpack_from("<H", head, opt)[0]
    if magic == 0x10B:
        n_off, dd = opt + 92, opt + 96
    elif magic == 0x20B:
        n_off, dd = opt + 108, opt + 112
    else:
        return None
    sec = dd + 4 * 8
    if sec + 8 > opt + opt_size or struct.unpack_from("<I", head, n_off)[0] < 5:
        return None
    off, size = struct.unpack_from("<II", head, sec)
    if size and (off == 0 or off + size > total):
        return None
    return size


def _is_mz(path):
    try:
        with open(path, "rb") as f:
            return f.read(2) == b"MZ"
    except OSError:
        return False


def authenticode_status(paths):
    """Windows 에서만: Get-AuthenticodeSignature 를 **한 번** 불러 {경로: (Status, SignatureType, 서명자)}.
    다른 OS · PowerShell 실패는 None(기록만 UNMEASURED — 붉은 규칙은 이 값에 기대지 않는다)."""
    if os.name != "nt" or not paths:
        return None
    fd, lst = tempfile.mkstemp(suffix="-census-list.txt")
    with os.fdopen(fd, "w", encoding="utf-8") as f:
        f.write("\n".join(paths))
    ps = ("$ErrorActionPreference='Stop'; [Console]::OutputEncoding=[Text.UTF8Encoding]::new($false); Get-Content -LiteralPath $env:CYS_CENSUS_LIST -Encoding UTF8 | ForEach-Object { "
          "$s = Get-AuthenticodeSignature -LiteralPath $_; [pscustomobject]@{p=$_; s=[string]$s.Status; "
          "t=[string]$s.SignatureType; n=$(if ($s.SignerCertificate) { $s.SignerCertificate.Subject } else { '' })} } "
          "| ConvertTo-Json -Compress")
    try:
        r = subprocess.run(["pwsh", "-NoProfile", "-NonInteractive", "-Command", ps], capture_output=True,
                           text=True, encoding="utf-8", env=dict(os.environ, CYS_CENSUS_LIST=lst), timeout=300)
        if r.returncode != 0:
            print(f"  authenticode: UNMEASURED (pwsh exit {r.returncode}: {r.stderr.strip()[:300]})")
            return None
        rows = json.loads(r.stdout or "[]")
        rows = rows if isinstance(rows, list) else [rows]
        return {row["p"]: (row["s"], row["t"], row["n"]) for row in rows}
    except (OSError, ValueError, subprocess.TimeoutExpired, KeyError, TypeError) as e:
        print(f"  authenticode: UNMEASURED ({type(e).__name__}: {e})")
        return None
    finally:
        os.remove(lst)


def sac_inventory(tree, required):
    """Required own binaries, looked up by canonical NAME over every file (not only the MZ ones) —
    {name: ok | missing | unreadable | non-mz | malformed}. A required file the MZ walk cannot see
    (absent, renamed by a changed 7z layout, unreadable, not starting with MZ) is named here instead
    of silently dropping out of the census (review r1 A-F1)."""
    paths = {}
    for dirpath, _, files in os.walk(tree):
        for fn in files:
            c = canonical_name(fn)
            if c in required:
                paths.setdefault(c, []).append(os.path.join(dirpath, fn))
    status = {}
    for name in required:
        found = sorted(paths.get(name, []))
        st = "ok" if found else "missing"
        for p in found:
            try:
                with open(p, "rb") as f:
                    head = f.read(2)
                size = security_directory(p)
            except OSError:
                st = "unreadable"
                break
            if head != b"MZ":
                st = "non-mz"
                break
            if size is None:
                st = "malformed"
                break
        status[name] = st
    return status


def sac_rows(tree, setup, authenticode):
    """Census rows: setup.exe (if given) + every MZ file under tree, any extension."""
    entries = []
    if setup:
        entries.append((setup, os.path.basename(setup), True))
    for dirpath, _, files in os.walk(tree):
        for fn in sorted(files):
            p = os.path.join(dirpath, fn)
            if os.path.isfile(p) and not os.path.islink(p) and _is_mz(p):
                entries.append((p, os.path.relpath(p, tree), canonical_name(fn) in SAC_OWN_BINARIES))
    auth = authenticode([p for p, _, _ in entries]) or {}
    rows = []
    for p, rel, own in entries:
        size = security_directory(p)
        st = auth.get(p)
        rows.append({"path": rel.replace(os.sep, "/"), "own": own, "pe": "ok" if size is not None else "malformed",
                     "embedded_signature": None if size is None else size > 0,
                     "authenticode_status": st[0] if st else "UNMEASURED",
                     "signature_type": st[1] if st else "UNMEASURED", "signer_subject": st[2] if st else ""})
    return rows


def sac_verdict(rows, inventory):
    """The one decision (also run on the per-run control): (code, verdict).
    3 = a required or enumerated own binary is unreadable / not MZ / not a well-formed PE ·
    5 = an own binary carries an Authenticode signature while signing is dormant ·
    6 = a required own binary is missing from the tree · 0 = PASS."""
    broken = ["%s=%s" % (n, s) for n, s in sorted(inventory.items()) if s not in ("ok", "missing")]
    broken += ["%s=malformed" % r["path"] for r in rows if r["own"] and r["pe"] != "ok"]
    if broken:
        return 3, "FAIL(3) own binary is not a usable PE: " + ", ".join(broken)
    signed = [r["path"] for r in rows if r["own"] and r["embedded_signature"]]
    if signed:
        return 5, ("FAIL(5) own binary carries an Authenticode signature while signing is dormant in this "
                   "release: " + ", ".join(signed))
    missing = sorted(n for n, s in inventory.items() if s == "missing")
    if missing:
        return 6, ("FAIL(6) required own binary missing from the shipped tree: " + ", ".join(missing)
                   + " - the census did not see what ships (a changed 7z layout fails closed)")
    return 0, "PASS"


def _census_control():
    """Per-run control through the SAME rows + inventory + verdict path as the real tree (review r1
    A-F2): a synthetic tree with every required own binary, all unsigned except a signed cysd.exe,
    must come out FAIL(5). Anything else means the census is blind to a signed own binary."""
    with tempfile.TemporaryDirectory(prefix="cys-census-control-") as ctl:
        for name, signed in (("cys.exe", False), ("cysd.exe", True), ("cys-app.exe", False), ("uninstall.exe", False)):
            with open(os.path.join(ctl, name), "wb") as f:
                f.write(_census_control_pe(signed))
        code, _ = sac_verdict(sac_rows(ctl, None, lambda _: None), sac_inventory(ctl, SAC_REQUIRED_OWN))
    return "red-as-expected" if code == 5 else "BLIND(code=%d)" % code


def sac_census(tree, setup=None, authenticode=authenticode_status, required=None):
    """D6-a census. 반환: (종료 코드, 보고서 dict) — 코드는 sac_verdict 의 정의 그대로, 대조가 눈멀면 6."""
    required = tuple(required or SAC_REQUIRED_OWN)
    rows = sac_rows(tree, setup, authenticode)
    inventory = sac_inventory(tree, required)
    own_found = sorted({canonical_name(os.path.basename(r["path"])) for r in rows if r["own"]})
    report = {"schema": SAC_CENSUS_SCHEMA, "mode": "unsigned", "files": rows, "own_found": own_found,
              "required": inventory,
              "counts": {"pe_files": len(rows),
                         "embedded_signed": sum(1 for r in rows if r["embedded_signature"]),
                         "own_signed": sum(1 for r in rows if r["own"] and r["embedded_signature"]),
                         "malformed": sum(1 for r in rows if r["pe"] != "ok")}}
    report["control"] = _census_control()
    if report["control"] != "red-as-expected":
        report["verdict"] = "FAIL(6) census control is blind: " + report["control"]
        return 6, report
    code, report["verdict"] = sac_verdict(rows, inventory)
    return code, report


def _census_control_pe(signed=True):
    """최소 PE32+ — signed 이면 보안 디렉터리가 파일 끝 16B 를 가리킨다(매 실행 대조용 표본 · 서명 바이트 자체는 의미 없다)."""
    e_lfanew, opt_size = 0x80, 112 + 8 * 16
    hdr = bytearray(e_lfanew + 4 + 20 + opt_size)
    hdr[0:2] = b"MZ"
    struct.pack_into("<I", hdr, 0x3C, e_lfanew)
    hdr[e_lfanew:e_lfanew + 4] = b"PE\0\0"
    struct.pack_into("<HH", hdr, e_lfanew + 4, 0x8664, 0)
    struct.pack_into("<H", hdr, e_lfanew + 4 + 16, opt_size)
    opt = e_lfanew + 24
    struct.pack_into("<H", hdr, opt, 0x20B)
    struct.pack_into("<I", hdr, opt + 108, 16)
    if not signed:
        return bytes(hdr)
    struct.pack_into("<II", hdr, opt + 112 + 32, len(hdr), 16)
    return bytes(hdr) + b"C" * 16


def extract_setup(setup, sevenzip):
    tmp = tempfile.mkdtemp(prefix="cys-crt-gate-")
    cmd = [sevenzip, "x", setup, f"-o{tmp}", "-y"]
    r = subprocess.run(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
    if r.returncode != 0:
        print(f"NSIS 추출 실패({sevenzip} exit {r.returncode}): {r.stderr.strip()[:500]}", file=sys.stderr)
        sys.exit(4)
    return tmp


def main():
    ap = argparse.ArgumentParser(description="Windows CRT 발행 차단 게이트")
    src = ap.add_mutually_exclusive_group(required=True)
    src.add_argument("--setup", help="NSIS setup.exe (내부에서 7z 추출)")
    src.add_argument("--tree", help="이미 추출된 설치 트리")
    ap.add_argument("--sevenzip", default="7z", help="7-Zip 실행 파일 (기본 7z, macOS 는 7zz)")
    ap.add_argument("--sac-census", action="store_true",
                    help="CRT 검사 대신 D6-a 서명 census 만 한다(0.14.53 WP12 · 종료 0/3/5/6)")
    ap.add_argument("--report", help="--sac-census 보고서(JSON)를 쓸 경로")
    args = ap.parse_args()

    if args.setup:
        if not os.path.isfile(args.setup):
            print(f"setup 파일 없음: {args.setup}", file=sys.stderr)
            sys.exit(4)
        tree = extract_setup(args.setup, args.sevenzip)
        print(f"== 입력: {args.setup} (추출: {tree})")
    else:
        tree = args.tree
        if not os.path.isdir(tree):
            print(f"트리 없음: {tree}", file=sys.stderr)
            sys.exit(4)
        print(f"== 입력 트리: {tree}")

    if args.sac_census:
        code, report = sac_census(tree, setup=args.setup)
        for r in report["files"]:
            print(f"  pe: {r['path']} own={r['own']} pe={r['pe']} embedded={r['embedded_signature']} "
                  f"status={r['authenticode_status']} type={r['signature_type']} signer={r['signer_subject'] or '-'}")
        c = report["counts"]
        print(f"windows-signing: UNSIGNED n={c['pe_files']} embedded-signed={c['embedded_signed']} "
              f"own-signed={c['own_signed']} malformed={c['malformed']} own-found={','.join(report['own_found']) or '-'} "
              f"control={report['control']}")
        if args.report:
            with open(args.report, "w", encoding="utf-8") as f:
                json.dump(report, f, ensure_ascii=False, indent=1)
        print(f"결과: {report.get('verdict', 'FAIL(6) census control is blind')}")
        sys.exit(code)

    violations, unparsed, allowed, total = check_imports(tree)
    missing, cysd = check_markers(tree)

    print(f"== 검사 A(임포트 스윕): exe {total}개 검사")
    for rel in allowed:
        print(f"  allow(app-local DLL 동봉): {rel}")
    for rel, why in violations:
        print(f"  VIOLATION: {rel} — {why}")
    for rel in unparsed:
        print(f"  UNPARSED(fail-closed): {rel}")
    print(f"== 검사 B(3카테고리 수리 마커): cysd={cysd or '미발견'}")
    for m in missing:
        print(f"  MISSING-MARKER: {m}")
    ok_markers = len(FIX_MARKERS) - len(missing)
    print(f"  markers: {ok_markers}/{len(FIX_MARKERS)} 존재")

    if violations:
        print("결과: FAIL(1) — CRT 정책 위반. 릴리스 차단.")
        sys.exit(1)
    if missing:
        print("결과: FAIL(2) — 수리 마커 결손. 구베이스 빌드 의심 — 브랜치 베이스(BASE-OK) 재검.")
        sys.exit(2)
    if unparsed:
        print("결과: FAIL(3) — PE 파싱 실패 존재(fail-closed).")
        sys.exit(3)
    print("결과: PASS — VCRUNTIME 자기완결 + 3카테고리 수리 실재.")
    sys.exit(0)


if __name__ == "__main__":
    main()
