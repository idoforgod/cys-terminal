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
  2단(규칙): 그 외 exe 는 같은 디렉토리에 vcruntime140.dll 동봉 시만 허용(app-local =
     실제 런타임 안전 불변식. v0.13.22 실측: runtime/python/{python,python3,pythonw}.exe
     3종이 자연 통과. 경로 하드코딩 없음 — 레이아웃 변화 내성).

종료 코드: 0=통과 / 1=검사A 위반 / 2=검사B 마커 결손 / 3=PE 파싱 실패(fail-closed) /
          4=입력·추출 오류. 모든 판정 내역을 stdout 에 남긴다(릴리스 로그 증거).

사용:
  python3 scripts/verify_win_crt.py --setup release-candidate/cys_X.Y.Z_x64-setup.exe
  python3 scripts/verify_win_crt.py --tree <추출된 설치 트리>  [--sevenzip 7zz]
"""

import argparse
import os
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

# 0.14.1 수리 세대 마커 — cysd 소스의 `#[used]` static(CYS_FIX_GENERATION_A1A2) 바이트열.
# ★코드 경로 문자열 휴리스틱 폐기(CI run 30357918475 실증 · 2026-07-28):
#   ①'command 비정상 종료'는 구베이스의 'text_command 비정상 종료'의 부분열이라 판별력 0
#   ②cfg(windows) 경로 리터럴('shell-fallback')은 컴파일러 재량으로 실빌드에서 미검출됨.
# `#[used]` static 은 참조 여부·최적화와 무관하게 링커가 보존하는 결정론 임베드이고
# 전 플랫폼 컴파일이라 맥 로컬 릴리스 빌드에서도 사전 검증된다. 0.14.0 이하엔 존재하지
# 않는 신조어 바이트열 — 판별력은 CI 음성 대조(0.14.0 실물 0/1 단언)가 계속 증명한다.
FIX_MARKERS = [
    b"cys-fix-a1a2-gen-0.14.1",
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
            if fn.lower() in STRICT_BINARIES:
                violations.append((rel, "정책 바이너리 — 예외 불허(crt-static 회귀)"))
            elif os.path.isfile(os.path.join(dirpath, "vcruntime140.dll")):
                allowed.append(rel)
            else:
                violations.append((rel, "동봉 vcruntime140.dll 없음 — 클린 머신에서 기동 불능"))
    return violations, unparsed, allowed, total


def check_markers(tree):
    """검사 B. 반환: (결손 마커 목록, cysd 경로 or None)."""
    cysd = None
    for dirpath, _, files in os.walk(tree):
        for fn in files:
            if fn.lower() == "cysd.exe":
                cysd = os.path.join(dirpath, fn)
                break
        if cysd:
            break
    if cysd is None:
        return [m.decode("utf-8", "replace") for m in FIX_MARKERS], None
    with open(cysd, "rb") as f:
        blob = f.read()
    missing = [m.decode("utf-8", "replace") for m in FIX_MARKERS if m not in blob]
    return missing, cysd


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
