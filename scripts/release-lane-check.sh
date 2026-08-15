#!/bin/bash
# 릴리스 레인 판별기 (2026-07-13 오너 승인 — 팩-only 레인 신설과 동시 도입)
# "이번 릴리스가 본체 범프인가, 팩-only인가"를 결정론으로 판정한다.
#
# 판정 규칙:
#   PACK-ONLY  — 마지막 태그 이후 변경이 전부 cysjavis-pack/ 내부 (인테리어만 교체)
#   BINARY     — 그 외 경로(src/·src-tauri/·ui/·build.rs·Cargo.* 등) 변경 존재 (건물 재시공)
#   사람 판단 예외: 팩이 '새 바이너리 기능/RPC'를 요구하면 PACK-ONLY라도 본체 레인
#                   (또는 PACK_MIN_BINARY 상향)으로 — 판별기는 경로만 본다.
#
# 버전 충돌 가드: 본체 릴리스의 다음 버전은 반드시 최신 pack-v 버전보다 커야 한다.
#   (동일 pack_version 재사용 시 팩 갱신 감지가 무음 실패 — 아래에서 기계 검사)
#
# 사용: bash scripts/release-lane-check.sh [기준태그]   (기본: 최신 태그)
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

BASE="${1:-$(git tag --sort=-creatordate | head -1)}"
test -n "$BASE" || { echo "태그 없음 — 기준태그를 인자로 지정하라"; exit 2; }

CHANGED=$(git diff --name-only "$BASE"..HEAD)
if [ -z "$CHANGED" ]; then
  echo "판정: 변경 0건 ($BASE..HEAD) — 릴리스 불요"
  exit 0
fi

NONPACK=$(echo "$CHANGED" | grep -v "^cysjavis-pack/" || true)
N_ALL=$(echo "$CHANGED" | wc -l | tr -d ' ')

# 최신 버전들(양 레인) — 충돌 가드용
LATEST_BIN=$(git tag --list 'v*' --sort=-v:refname | grep -Ev '^pack-' | head -1 || true)
LATEST_PACK=$(git tag --list 'pack-v*' --sort=-v:refname | head -1 || true)

echo "── 기준: $BASE..HEAD · 변경 $N_ALL 건"
echo "── 최신 본체 태그: ${LATEST_BIN:-없음} · 최신 팩 태그: ${LATEST_PACK:-없음}"
echo

if [ -z "$NONPACK" ]; then
  echo "판정: ★PACK-ONLY 레인 (전 변경이 cysjavis-pack/ 내부 — 인테리어만 교체)"
  echo "  절차: ① 버전 6곳 범프 불요·version-check 불요"
  echo "        ② git tag pack-vX.Y.Z && git push origin pack-vX.Y.Z"
  echo "        ③ CI(pack-release.yml)가 서명 팩 3종+latest.json 캐리포워드 발행"
  echo "  버전: pack-vX.Y.Z 는 현재 pack_version(최신 릴리스 manifest)보다 커야 한다"
  echo "  min_binary(정책 파생 — 해법③⑤): $(bash scripts/min-binary-policy.sh 2>&1 | tr '\n' ' ')"
  echo "  ⚠ 예외(사람 판단): 팩이 새 바이너리 기능을 요구하면 본체 레인 또는 PACK_MIN_BINARY 상향"
else
  N_NP=$(echo "$NONPACK" | wc -l | tr -d ' ')
  echo "판정: 본체(BINARY) 레인 — 팩 외 변경 $N_NP 건 (건물 재시공)"
  echo "$NONPACK" | head -15 | sed 's/^/    /'
  [ "$N_NP" -gt 15 ] && echo "    … 외 $((N_NP-15))건"
  echo "  절차: 버전 6곳 범프 + Cargo.lock + version-check + git tag vX.Y.Z"
  # ★A13(v4-repair · 2026-08-16): 릴리스 시퀀싱 1단계는 'win T3 green 증적→태그'다. 태그 파이프라인
  #   (release.yml)과 windows-build.yml 은 기계 결합이 없으므로(트리거 분리·의도), 이 판별기가
  #   런북 수동 순서를 상기시키는 것이 명시 배선이다 — 증적 없이 태그하면 게이트를 사람 손으로 건너뛴 것.
  echo "  ★A13(win T3): 태그 **전** windows-build.yml 최신 run 의 T3 확장 행"
  echo "    (ng1~ng3 이름검증·tpl 템플릿 바이트·c03 preflight·alt_screen) green 증적을 확보하라"
  echo "    — 릴리스 시퀀싱 1단계(win T3 green 증적→태그). 실행: Actions 탭 workflow_dispatch."
  if [ -n "$LATEST_PACK" ]; then
    PKV="${LATEST_PACK#pack-v}"
    echo "  ★버전 충돌 가드: 다음 본체 버전은 반드시 $PKV (최신 pack-v) 보다 커야 한다"
    echo "    (같거나 낮으면 본체 릴리스의 pack_version이 팩 릴리스와 충돌 — 갱신 감지 무음 실패)"
  fi
fi
