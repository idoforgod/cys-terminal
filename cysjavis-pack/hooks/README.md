# hooks/ — cysjavis 훅 디렉터리

이 폴더의 `.sh` 파일은 Claude Code 가 세션 이벤트마다 실행하는 훅이다. **여기에 파일이 있다는 것은
등록됐다는 뜻이 아니다** — 등록은 settings.json 이 하고, 이 폴더는 실행될 스크립트를 담고 있을 뿐이다.
이 문서는 그 구분과, 실행되고도 아무 일이 없는 경우(휴면)를 읽는 법을 적는다.

## 1. 등록 층 — 어디에 적혀야 실행되는가

| 층 | 파일 | 성격 |
|---|---|---|
| 사용자 | `~/.claude/settings.json` 또는 `CLAUDE_CONFIG_DIR` 이 가리키는 프로필의 settings.json | 그 프로필로 뜬 세션 전부에 적용 |
| 프로젝트 | `<cwd>/.claude/settings.json` | 그 폴더에서 뜬 세션에 적용 |
| 사용자 오버레이 | `~/.cys/local/hooks/<이벤트>.d/` | 업데이트·치유가 건드리지 않는 확장점(팩 밖) |

등록 항목은 `{"type":"command","command":"sh /경로/hook.sh"}` 형태다. 실제로 무엇이 등록돼 있는지는
그 settings.json 을 열어 보는 것이 유일한 진실이고, `cys doctor` 가 팩 정합·훅 등록을 함께 진단한다.

★부서 레인 주의: 부서 노드는 자기 팩(`~/.cys/pack-dept-<이름>`)을 쓰지만, cwd 가 홈이면 **프로젝트 층
경로가 본부 settings.json 과 같아져** 본부 훅이 함께 발화한다(실측 사고). 그 교차 발화를 막는 것이
아래 레인 가드다.

## 2. 공용 프리루드 `_lib.sh` — 훅이 아니다

모든 훅은 첫머리에서 이 파일을 source 한다(2단 폴백 후 loud-skip):

```sh
. "$(dirname "$0")/_lib.sh" 2>/dev/null \
  || . "${CYS_PACK_DIR:-$HOME/.cys/pack}/hooks/_lib.sh" 2>/dev/null \
  || { echo "[cys-hook] _lib.sh 소실 — 훅 강등(<이름>)" >&2; exit 0; }
```

2단(팩 경로) 폴백이 필수인 이유는 훅이 **팩 밖으로 복사돼 실행되는 경로가 실재**하기 때문이다
(배선 하네스·테스트 스텁·`~/.cys/local/hooks/<이벤트>.d/` 오버레이). 프리루드 계약은 파일 머리말
ⓐ~ⓕ에 있다 — POSIX sh 전용(bashism 금지) · **stdout 무출력**(SessionStart·UserPromptSubmit 의
stdout 은 모델 컨텍스트로 주입된다) · `set -u` 안전 · 항상 0 으로 종료 · 멱등.

`_lib.sh` 는 settings.json 에 **등록하지 않는다**. 파일명 앞의 `_` 가 그 시각 신호다.

### 레인 가드 (0.14.30 신설 · 설계 #16)

다른 레인의 팩에서 온 훅이 우연히 발화하는 것을 막는다. 발화 조건은 **양쪽이 모두 실제 팩일 때**다 —
`$CYS_PACK_DIR/hooks/_lib.sh` 가 있고, 훅 자신의 팩 루트에도 `hooks/_lib.sh` 가 있으며, 두 경로가
정규화 후 다를 때. 불일치 시 다음과 같이 처리한다.

1. 자기 레인 팩의 같은 상대경로 훅이 있고 읽을 수 있으며, 아래 등록·강등 조건에 걸리지 않으면
   그 훅으로 `exec` 한다(위임 · stdin/인자/stderr/exit 보존).
2. 대응 훅이 이 좌석의 실사용 설정에 이미 등록돼 있으면 **표식 없이 위임을 생략하고 exit 0** 한다.
   `${CLAUDE_CONFIG_DIR:-$HOME/.claude}` 와 훅 cwd 의 `.claude` 아래 `settings.json`·
   `settings.local.json` 4파일을 확인한다. 정규화 경로와 `$CYS_PACK_DIR` 원형 경로(절대 경로일 때만 —
   상대 원형은 타 레인 등록줄의 접미가 돼 오판한다)를 보며, 레인 훅은 자기 등록으로 1회만 실행된다.
3. 대응 훅 부재(`reason=absent`)·판독 불가(`reason=unreadable`)·위임된 훅의 재불일치 또는
   대상이 자기 자신(`reason=already-redirected`)이면 `<레인 팩>/state/lane-guard-tripped` 표식을
   남긴다(덮어쓰기). 등록 확인에도 걸리지 않고 발화한 훅 본문에 `cys_lane_redirect` 토큰이 없으면
   `reason=no-redirect-line` 표식을 남긴다. 이 갈래들은 본문 실행과 stdout 없이 exit 0 한다.
4. 위임 래치 `CYS_LANE_REDIRECTED=1` 은 **대상 프리루드 1홉에서 소비**한다. 프리루드가 환경에서
   래치를 지우므로 자손 프로세스는 상속하지 않고, 자손의 다른 불일치 훅도 정상 위임할 수 있다.
   위임 예약 변수 `CYS_LANE_REDIRECT` 도 가드 진입 시 초기화해 환경 상속값을 무시한다.

표식 필드는 `hook_root`·`lane_root`·`script`·`surface`·`reason`·`ts` 이며 각 줄은 `key=value` 형식이다.
원인에 맞게 팩·권한·훅 본문을 조치하고 확인한 뒤 `<레인 팩>/state/lane-guard-tripped` 표식을 삭제한다.
표식은 24h 가 지나면 자동 통과한다.

판정이 서지 않으면(팩이 아닌 트리에서 실행 — 오버레이·테스트 스텁·팩 밖 복사본) **통과**한다.
`~/.cys/local/hooks/` 에는 `_lib.sh` 가 없으므로 사용자 오버레이는 이 가드에 걸리지 않는다.
일시적으로 끄려면 `CYS_HOOK_LANE_GUARD=0`.

소비자는 preflight `C83.lane-guard-tripped`(24h 이내 표식 FAIL · 좌석 안 SessionStart 설정 대조 WARN)와
`javis_bootstrap.py` 최종 JSON / `javis_mission.py status --json` 의 `hooks_effective` 다.

훅 본문은 프리루드 줄 다음에 아래 한 줄을 둔다. census 검체(`test_lane_redirect.py` R-8)가
전수를 잰다 — 새 훅을 만들면 이 줄을 넣어라.

```sh
command -v cys_lane_redirect >/dev/null 2>&1 && cys_lane_redirect "$@"
```

## 3. completion-guard 의 이중 휴면 — "등록했는데 왜 안 도나"

`completion-guard.sh`(Stop)는 두 층이 **모두** 충족돼야 발동한다. 어느 한쪽만 빠져도 조용히 exit 0
이라, 종전에는 증상이 전혀 없었다(설계서 자인: "증상이 없는 실패").

| 층 | 무엇 | 어떻게 켜나 | 안 켜져 있으면 |
|---|---|---|---|
| 등록 휴면 | Stop 훅으로 settings.json 에 등재 | `bin/javis_guard_register.py --hook stop --profile <경로> --apply` | 훅이 아예 실행되지 않는다 |
| env 휴면 | pane 에 `CYS_COMPLETION_GUARD=1` | 그 pane 기동 시 env 로 | 실행은 되지만 즉시 exit 0 |

- 등록기는 **dry-run 이 기본**이고 쓰기는 `--apply` 를 명시해야 한다. 대상은 `--profile` 로 준
  경로뿐이며(자동 발견 없음), Stop 훅은 워커 프로필 전용이라 master 프로필은 거부한다
  (`--force-master` 로만 우회 · 사유가 기록된다).
- 등록만으로는 격리가 성립하지 않는다 — `~/.cys/claude` 프로필 하나를 CSO·워커·리뷰어가 **함께**
  쓰므로 실제 격리 단위는 pane env 다(`state/hook-targets.json.example` 에 같은 문장이 있다).
- 0.14.30 부터 두 휴면 각각에 **surface 당 1회** stderr 고지가 붙는다(마커는 TMPDIR). 미무장 경로는
  외부 명령을 하나도 띄우지 않는다(셸 내장만) — 그 무스폰 계약은 테스트로 잠겨 있다.
- 끄려면 그 pane 의 `CYS_COMPLETION_GUARD` 를 지운다. 등록은 그대로 두어도 즉시 무발동이 된다.

## 4. verify-reminder.sh — 껍데기만 남은 래거시

`bin/javis_verify_reminder.py` 는 설계에서 **폐기(descope)** 됐고(동일 기능 이중 구현 금지 · 그 역할은
`brief-lint-warn.sh` 가 흡수), 래퍼만 남아 본체 부재를 확인하고 항상 exit 0 한다. 매니페스트 호환을
위해 파일을 유지할 뿐이므로 **등록해도 아무 일도 일어나지 않는다.**

## 5. 훅 목록 (이벤트 · 목적 — 각 파일 머리말에서 발췌)

| 파일 | 이벤트 | 목적 |
|---|---|---|
| `session-start.sh` | SessionStart | 역할 지침·soul 주입, 역할 미지정 세션엔 부트 안내 |
| `inject-context.sh` | SessionStart | 작업기억(SESSION_STATE)·스냅샷·체크리스트 주입, 동일 cwd 세션 경고 |
| `role-bootstrap.sh` | UserPromptSubmit | 마스터 선언 감지 → 결정론 부트스트랩 발화 |
| `memory-trigger-inject.sh` | UserPromptSubmit | 기억 트리거 주입 래퍼(라이브 배선 승인 대상) |
| `pre-dispatch.sh` | PreToolUse | 단일 디스패처 — 아래 PreToolUse 게이트들을 한 번에 태운다 |
| `guard.sh` | PreToolUse | Autopilot 집행(deny-by-default allowlist) |
| `actprobe-kill-gate.sh` | PreToolUse | Bash kill 가드 |
| `role-capability-gate.sh` | PreToolUse | 역할 기반 능력 가드 |
| `appbuild-gate.sh` | PreToolUse | appbuild '코드 선행 금지' 게이트 |
| `grill-arm.sh` / `grill-gate.sh` | PreToolUse | grill-me 무장 · 최소 질문 게이트 |
| `serena-nudge.sh` | PreToolUse | 심볼 탐색 권고(알림 전용) |
| `cys-hook.sh` | PreToolUse | 툴 이벤트를 데몬으로 전달 |
| `pack-guard.sh` | PostToolUse | 팩 파일 직접 수정 감지 |
| `brief-lint-warn.sh` | PostToolUse(Task\|Agent) | 위임 브리프 경고 |
| `commit-memory-nudge.sh` | PostToolUse(Bash) | git commit 감지 → 기억 증류 넛지 |
| `grill-count.sh` | PostToolUse | grill 결정축 카운트 |
| `verify-reminder.sh` | PostToolUse | (래거시 껍데기 — §4) |
| `completion-guard.sh` | Stop | 완료 전 검증 강제(§3 — 이중 휴면) |
| `grill-stop.sh` | Stop | grill 수집 미충족 시 턴 종료 차단 |
| `reflect-scan.sh` | Stop·SessionEnd | 반복 신호 스캔 + 기억 정합 verify |
| `save-state.sh` | PreCompact·Stop | 작업기억 write-ahead |
| `fullauto/*.sh` | SessionStart·UserPromptSubmit·PostToolUse·Stop | 자율주행 오버레이(옵트인) |
| `vibecoding/*.sh` | PostToolUse | 바이브코딩 넛지(옵트인) |

훅이 아닌 파일: `_lib.sh`(프리루드 §2) · `cys-statusline.sh`(statusline 래퍼) ·
`inject_gate.py`(주입 포이즌 게이트 — inject-context 가 부른다) · `test_pre_dispatch.sh`(회귀 하네스).

## 6. 훅이 안 도는 것 같을 때 보는 순서

1. 등록됐는가 — 해당 settings.json 의 그 이벤트 배열에 이 파일 경로가 있는가.
2. 레인이 맞는가 — stderr 에 `타 레인 팩 훅 조기 종료` 가 있으면 §2 레인 가드가 끊은 것이다.
3. 무장 env 가 필요한 훅인가 — completion-guard 는 §3 두 층을 모두 요구한다.
4. 프리루드가 살아 있는가 — `_lib.sh 소실 — 훅 강등` 이 보이면 팩이 깨진 것이다(`cys doctor`).
5. 그래도 조용하면 훅을 직접 실행해 본다: `printf '{"source":"clear","cwd":"'"$PWD"'"}' | sh hooks/inject-context.sh`
