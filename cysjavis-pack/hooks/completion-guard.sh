#!/bin/sh
# W0-1 Stop hook 래퍼 — 라이브 배선 승인 시 ~/.cys/pack/hooks/ 로 복사 등록.
# 격리 기간엔 이 원본만 존재(설계서 §6 규약).
# ── 공용 프리루드(CS-4①) — loud-skip: 소실 시 조용히 꺼지지 않고 stderr 1줄 후 강등 ──
. "$(dirname "$0")/_lib.sh" 2>/dev/null \
  || . "${CYS_PACK_DIR:-$HOME/.cys/pack}/hooks/_lib.sh" 2>/dev/null \
  || { echo "[cys-hook] _lib.sh 소실 — 훅 강등(completion-guard)" >&2; exit 0; }

# ── 무장 스위치 선(先)판정(조건 37 강화 · E1-2) ──────────────────────────────
# guard main() 첫 줄과 **같은 판정**을 셸에서 한 번 더 한다. 종전엔 미무장 pane 에서도
# python 인터프리터를 띄운 뒤 guard 가 exit 0 했다 — 관측 결과는 같지만 매 turn 기동 비용을
# 냈다. 여기서 끊으면 미무장 pane 은 **프로세스 스폰 0**이고, 아래 외곽 데드라인(경로에 따라
# python 실행기 1개)의 비용도 미무장 pane 에는 지워지지 않는다.
# ※ 판정 SOT 는 guard main() 이다 — env 이름이 바뀌면 양쪽을 함께 고쳐야 한다.
# ★env 휴면 1회 고지(0.14.30 · SURVEY 2026-09-03 §C1 '이중 휴면'): 이 훅의 휴면은 두 층이다 —
#   등록 휴면(Stop 미등록 · 어떤 등록기도 자동 등록하지 않는다 · 유일 등록기 javis_guard_register.py 는
#   수동·dry-run 기본) + env 휴면(등록돼도 무장 env 미설정 → 여기서 exit 0). 둘 다 무음이라 "등록했는데
#   왜 안 도나"가 증상 없는 실패였다(OT-DOSSIER:88 · hook-targets.json.example:88 "실제 격리는 pane 단위
#   env 뿐"). 미무장 exit 0 계약은 그대로 두고, 그 직전에 stderr 1줄만 남긴다 — surface 당 1회.
#   ★마커 이름에 `$(id -u)`·`$(date)` 를 쓰지 않는 이유: 위 :10-13 계약(미무장 pane = 프로세스 스폰 0)을
#     지켜야 하므로 외부 명령 스폰이 금지다. `${CYS_SURFACE_ID}` 는 cysd 가 pane 에 주입한 env
#     (_lib.sh:47)라 스폰 없이 pane 식별이 되고, cys 밖 세션은 nosurface 한 장으로 코얼레싱된다.
#     `[ -f ]`·`:`·리다이렉션·echo 는 전부 셸 내장(PortableGit sh 포함) · 마커 위치는 TMPDIR(:44 와
#     동일 규약 — 저장소·팩 오염 금지). 마커 생성 실패는 무시한다(고지가 turn 마다 반복될 뿐 차단·스폰 0).
#   무장 절차·되돌리기는 hooks/README.md §3.
if [ "${CYS_COMPLETION_GUARD:-}" != "1" ]; then
  _cg_dormant="${TMPDIR:-/tmp}/.cys-guard-env-dormant-${CYS_SURFACE_ID:-nosurface}"
  if [ ! -f "$_cg_dormant" ]; then
    : > "$_cg_dormant" 2>/dev/null || :
    echo "[cys-hook] completion-guard: env 휴면 — CYS_COMPLETION_GUARD=1 미설정이라 등록돼 있어도 무발동(무장은 pane 기동 env 로만 · hooks/README.md 참조)" >&2
  fi
  exit 0
fi

PACK="${CYS_PACK_DIR:-$HOME/.cys/pack}"
# 대상 존재 가드 — .py 부재 시 exec 실패로 hook이 exit 2(오류)를 내는 걸 막고 조용히 통과(H-HOOK-2).
# ★본체 부재도 1회 고지(SURVEY §C1): 0.14.29 배포본엔 본체가 있다(sha 3곳 동일)지만 구세대·부분 복사 팩에서는
#   이 분기가 '무장까지 했는데 무발동'의 세 번째 무음 축이 된다. exit 0(H-HOOK-2) 불변 · 마커 규약은 위와 같다.
if [ ! -f "$PACK/bin/javis_completion_guard.py" ]; then
  _cg_absent="${TMPDIR:-/tmp}/.cys-guard-body-absent-${CYS_SURFACE_ID:-nosurface}"
  if [ ! -f "$_cg_absent" ]; then
    : > "$_cg_absent" 2>/dev/null || :
    echo "[cys-hook] completion-guard: 본체 bin/javis_completion_guard.py 부재 — 무발동" >&2
  fi
  exit 0
fi
# G22: 인터프리터 해소(python3→python→py) + cygpath 네이티브 경로. 미해소면 통과(기존 계약).
[ -n "$CYS_PY" ] || exit 0

# ── F9 → E1-2(BLOCKER R-01) 외곽 타임아웃: 상한 사다리의 **실제 겹 수** ──────────
# 종전 문면: "3중 상한 60>50>30". 실측 반증: 이 배포 머신에 coreutils `timeout` 이 없어
# (`command -v timeout` rc=1) 래퍼가 항상 폴백으로 떨어졌고, 등록 도구도 settings 항목에
# `timeout` 필드를 넣지 않아 **바깥 두 겹이 동시에 부재** → 실제로는 guard SIGALRM 50s 1겹이었다.
# 지금의 사다리(안쪽이 먼저 끊는다 — 바깥으로 갈수록 느슨):
#   ① verify 개별 상한 30s        (guard VERIFY_TIMEOUT_MAX · 항상 존재)
#   ② guard 자체 데드라인 50s      (POSIX=SIGALRM + 전 플랫폼 워치독 스레드 · 항상 존재)
#   ③ 이 래퍼의 외곽 60s           (cys_timeout_run: timeout → gtimeout → CYS_PY 실행기)
#   ④ settings 훅 항목 timeout 60  (javis_guard_register 가 등록 시 기입 — 하네스 층)
# ③은 `_lib.sh` 의 `cys_timeout_run` 을 쓴다 — 종전처럼 `timeout` 유무로 있고/없고가 갈리지
# 않고, 부재 환경에서는 python 실행기가 프로세스 **그룹** 단위로 SIGKILL 한다(고아 0).
# 타임아웃 rc=124 는 Stop 훅에서 블록(2)이 아니므로 fail-open 계약은 그대로다.
_cg_py="$(cys_native_path "$PACK/bin/javis_completion_guard.py")"
# 외곽 상한 초 — 기본 60. env 는 **검증 주입용**이다(guard 의 CYS_GUARD_SELF_DEADLINE_SEC 와
# 같은 규약): 비수치·1 미만이면 기본값으로 되돌린다(오타가 상한을 지우지 못한다).
_cg_to="${CYS_GUARD_OUTER_TIMEOUT_SEC:-60}"
case "$_cg_to" in ''|*[!0-9]*) _cg_to=60 ;; esac
[ "$_cg_to" -ge 1 ] 2>/dev/null || _cg_to=60
if ! command -v timeout >/dev/null 2>&1 && ! command -v gtimeout >/dev/null 2>&1; then
  # coreutils 부재 — python 실행기로 강등한다. **조용히 넘어가지 않는다**(R-01 의 본체는
  # "3중"이라 주장하면서 실제로는 1겹이던 상태를 아무도 몰랐다는 것이다). 하루 1회 코얼레싱·
  # 마커는 TMPDIR — 저장소·팩 오염 금지(R-13/R-19 교훈).
  _cg_mark="${TMPDIR:-/tmp}/.cys-guard-timeout-absent-$(id -u 2>/dev/null || echo 0)-$(date -u +%Y%m%d 2>/dev/null || echo x)"
  if [ ! -f "$_cg_mark" ]; then
    : > "$_cg_mark" 2>/dev/null || :
    echo "[cys-hook] completion-guard: coreutils timeout/gtimeout 부재 — 외곽 상한을 CYS_PY 실행기로 강등(60s·그룹 SIGKILL). 사다리는 30<50<60(+settings 60)이며 겹 구성이 환경마다 다르다는 사실을 이 줄로 표면화한다." >&2
  fi
fi
cys_timeout_run "$_cg_to" "$CYS_PY" "$_cg_py"
exit $?
