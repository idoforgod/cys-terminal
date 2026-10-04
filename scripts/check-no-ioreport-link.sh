#!/usr/bin/env bash
# check-no-ioreport-link.sh — 산출물 게이트: cysd 바이너리가 비공개 IOReport 를 **강하게 링크**하지 않았는가
# (0.14.43 성찰 R1F-PK · S3 minor 8 · E2 의 재발 방지).
#
# 왜 존재하는가: E2 가 `src/bin/cysd/hwmon.rs` 의 IOReport 강링크(link 속성으로 프로세스에 묶기)를 `dlopen`/`dlsym` 지연 로딩으로 바꿨다.
#   강링크는 **로드 시점 의존**이다 — dyld 가 프로세스 시작 때 풀기 때문에, 그 dylib(또는 심볼)이 없는 macOS 에서는 cysd 가 main 에 닿기도 전에
#   "Library not loaded" 로 죽는다(데몬 기동 불능 = 모든 pane 사망). 소스 핀(`hwmon.rs` 의 link 속성 · lib 의
#   `e2_cysd_hwmon_source_has_no_ioreport_strong_link`)은 소스의 모양만 보지만, 진짜 판정자는 **산출물**이다 — 빌드 스크립트의
#   `cargo:rustc-link-lib`·링커 인자·다른 파일의 속성·의존 크레이트 어디서 들어와도 `otool -L` 에는 그대로 나온다.
#
# 사용:   scripts/check-no-ioreport-link.sh <cysd 바이너리 경로>
# 판정:   `otool -L <바이너리>` 의 **의존 라이브러리 줄**(탭 들여쓰기 — 첫 줄 `<경로>:` 와 fat 바이너리의 `(architecture …):` 머리는 제외)에
#         `IOReport`(대소문자 무시)가 있으면 실패.
# 종료:   0 = 통과(IOReport 링크 없음) 또는 건너뜀(macOS 가 아니다 — 리눅스·윈도우 러너에서 깨지지 않게)
#         1 = 실패(IOReport 링크 발견 — 그 줄을 출력한다)
#         2 = 판정 불가(인자 누락 · 파일 없음 · **macOS 인데 otool 이 없다** · otool 실패 · 의존 라이브러리 줄 0건). 측정 불능은 통과가 아니다.
#         (★0.14.43 성찰 R2F-PK · A4 m2: 종전에는 '맥인데 otool 이 없다'도 건너뜀(통과)이었다 — 이 게이트는 맥 러너에서만 의미가 있는데 거기서 도구가 없다는 것은 '재지 못했다'이지 '링크가 없다'가 아니다.
#          같은 스크립트의 다른 갈래(아래 파일 없음·otool 실패·의존 줄 0건 = 2)와 맞췄다. 세 갈래(0/1/2)를 재는 검체: scripts/tests/test_check_no_ioreport_link.py — 브랜치 CI 가 매 push 돈다.)
#
# 한계(정직): 로드 명령(LC_LOAD_DYLIB·약한/재수출/지연 로드)만 본다. `dlopen` 으로 런타임에 여는 것은 의도된 방식이라 여기 나오지 않는다.
#   정적 라이브러리로 IOReport 를 섞어 넣는 경우(공개 .a 가 없다)도 이 검사가 잡는 대상이 아니다. 이 스크립트는 macOS 에서만 판정한다.
set -u

me="[check-no-ioreport-link]"

if [ "$#" -lt 1 ] || [ -z "${1:-}" ]; then
  echo "$me 사용: $0 <cysd 바이너리 경로>  (인자 누락 — 판정 불가 · exit 2)" >&2
  exit 2
fi
bin="$1"

# 맥이 아니면 건너뜀 — 리눅스·윈도우 러너에서 깨지지 않게 통과로 센다(건너뛴 사실은 한 줄로 남긴다). Mach-O 가 없는 곳에는 otool -L 이 판정할 대상이 없다.
os="$(uname -s 2>/dev/null || echo unknown)"
if [ "$os" != "Darwin" ]; then
  echo "$me 건너뜀: macOS 가 아니다($os) — otool -L 은 Mach-O 전용이다"
  exit 0
fi
# ★R2F-PK(A4 m2): **맥인데 otool 이 없으면 판정 불가(exit 2)다 — 건너뜀(통과)이 아니다.** 이 게이트가 의미 있는 곳이 바로 맥(브랜치 CI 맥 레인 · 릴리스 맥 레그)이고, 거기서 otool 이 없다는 것은
#   Xcode Command Line Tools 가 없거나 PATH 가 깨졌다는 뜻이다 — 그 상태의 '건너뜀 0'은 링크가 없다는 증거가 아니라 **재지 못했다**는 사실이다(측정 불능은 통과가 아니다 · 이 파일 아래 갈래들과 같은 계급).
if ! command -v otool >/dev/null 2>&1; then
  echo "::error::$me 판정 불가: macOS 인데 otool 이 없다(Xcode Command Line Tools 부재 또는 PATH 문제) — 이 러너에서는 IOReport 링크를 재지 못했다. 측정 불능은 통과가 아니다(exit 2)" >&2
  echo "$me 판정 불가: otool 이 없다(macOS 에서는 건너뛰지 않는다 · exit 2)" >&2
  exit 2
fi

if [ ! -f "$bin" ]; then
  echo "$me 실패: 파일이 없다: $bin  (판정 불가 · exit 2)" >&2
  exit 2
fi

out="$(otool -L "$bin" 2>&1)"
rc=$?
if [ "$rc" -ne 0 ]; then
  echo "$me 판정 불가: otool -L 이 실패했다(rc=$rc): $bin  (exit 2)" >&2
  printf '%s\n' "$out" | head -5 >&2
  exit 2
fi

# 의존 라이브러리 줄 = 탭(또는 공백)으로 들여쓴 줄. 첫 줄(`<경로>:`)과 fat 바이너리의 `(architecture …):` 머리는 칸 0 이라 빠진다 —
# 경로 이름에 우연히 IOReport 가 들어 있어도 거짓 양성이 나지 않는다.
deps="$(printf '%s\n' "$out" | LC_ALL=C awk '/^[ \t]/')"
if [ -z "$deps" ]; then
  echo "$me 판정 불가: otool -L 출력에 의존 라이브러리 줄이 없다 — Mach-O 가 아니거나 출력 형식이 바뀌었다: $bin  (exit 2)" >&2
  printf '%s\n' "$out" | head -5 >&2
  exit 2
fi

hits="$(printf '%s\n' "$deps" | LC_ALL=C awk 'tolower($0) ~ /ioreport/')"
if [ -n "$hits" ]; then
  echo "::error::$me $bin 이(가) IOReport 를 링크한다 — 그 dylib 이 없는 macOS 에서 데몬이 뜨지 못한다(dlopen 지연 로딩으로 바꿔야 한다)" >&2
  echo "$me 실패: IOReport 링크 발견 — otool -L 의 해당 줄:" >&2
  printf '%s\n' "$hits" >&2
  exit 1
fi

n="$(printf '%s\n' "$deps" | wc -l | tr -d ' ')"
echo "$me 통과: $bin — 링크된 라이브러리 ${n}개 중 IOReport 없음"
exit 0
