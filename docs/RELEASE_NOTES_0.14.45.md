# v0.14.45 (초안) — 윈도우 pane 의 휠 스크롤이 돌아오고, 막힌 큐가 화면 사본 어긋남에서 스스로 풀리고, 부서 카드가 노드별 계정을 보여 줍니다

> **이 문서는 발행 전 초안입니다** — 수치는 이 워크트리에서 실제로 잰 것만 적었고, 재지 못한 것(윈도우 실기 전부)은 §7 에 숨기지 않고 적었습니다.
> 발행 때 §9 의 실측 수치와 윈도우 실기 항목을 갱신합니다.

세 갈래를 한 판에 묶었습니다. 한 문장으로 줄이면 이렇습니다 —
**윈도우에서 master·worker pane 의 휠로 대화가 올라가지 않던 것(Claude Code 전체화면 렌더러)을 좌석 설정에 `"tui": "default"` 를 넣어 classic 화면으로 고정해 고쳤고,
큐가 `prompt_unknown`·`input_pending` 으로 영영 멈추던 것(데몬의 화면 사본 어긋남)을 키 입력 없이 창 높이를 한 줄 흔들어 다시 그리게 해 풀었고,
어느 부서·노드가 어느 계정으로 돌고 있는지 부서 카드와 사용량 칸에서 바로 보이게 했습니다.**
그 위에 설계 검토(성찰) 두 회차의 지적을 반영했습니다 — 다시 그린 화면의 재방송이 경보·완료 대기를 속이지 않게, 복원 좌석이 '계정 불일치' 로 오표시되지 않게,
사람이 해야 할 처방이 조용한 토스트로 묻히지 않게, 완전 초기화가 좌석 설정에 넣은 값까지 되돌리게.

> **쓰시는 분이 먼저 알아 둘 것**
> - **윈도우**: 업데이트 뒤 claude 좌석이 처음 뜰 때 그 좌석 설정 폴더의 `settings.json` 에 `"tui": "default"` 가 **없을 때만** 들어갑니다(이미 떠 있는 전체화면 pane 은
>   다음 기동부터 · 지금 바꾸려면 그 pane 에서 `/tui default`). 끄는 법·되돌리는 법은 USER-MANUAL §4.6b. 그 폴더(`~/.claude-N` 등)는 cys 밖에서 같은 계정으로 띄우는
>   claude 도 읽으므로 그 claude 도 classic 으로 뜹니다(전체화면이 필요하면 그쪽에서 `/tui fullscreen`).
> - 맥·리눅스: 화면 사본 재동기·계정 표시는 그대로 켜지고, `tui` 기록은 하지 않습니다(`CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN` 기본 주입이 같은 역할).
> - 처방 코드가 13종에서 **14종**(`stale_screen`)이 됐습니다 — 구버전 화면(0.14.44 GUI)은 모르는 코드라 조치 문장 대신 사유만 보이고 배너는 냅니다(모르면 알린다).
> - 이번 판은 역할 지침(`directives/`) 문서를 바꾸지 않았습니다 — `cys pack-merge` 를 새로 돌리실 일은 없습니다.

## 1. 윈도우 — claude pane 에서 휠로 대화가 올라가지 않던 것

- **원인(제보 · 2026-10 윈도우 11 · Claude Code 2.1.291)**: master·worker pane 은 Claude Code 의 **전체화면(alt screen) 렌더러**로, CSO·reviewer pane 은 classic 으로 떠 있었습니다.
  윈도우의 cys 는 전체화면 pane 의 휠을 일부러 삼키므로(휠이 방향키로 합성돼 프롬프트 히스토리를 오염시키는 원 결함의 방어) 그 pane 에서 휠은 완전 무동작이었습니다.
  렌더러는 Claude Code 가 프로세스마다 바뀌는 공유 상태(env · 크래시 카나리 · 설정 `tui` · 업셀 · 서버 게이트)로 정하므로 같은 폴더의 좌석끼리도 갈립니다.
- **고친 것**: 윈도우에서 claude 를 띄우기 직전(`cys boot`·`launch-agent`·GUI·node-recover·restore 공통 한 곳) 좌석 설정 폴더 `settings.json` 에 `tui` 키가 **없을 때만**
  `"tui": "default"`(classic) 를 넣습니다. 개인 기본 프로필 `~/.claude` 는 쓰지 않고, 계정 선택으로 좌석에 명시된 `~/.claude-N` 은 좌석 계정이라 씁니다. 외과 수술 규약
  (다른 바이트 불변 · 백업 `.bak-cys-tui` · 쓰기 직전 재판독 · 원자 쓰기 · 되읽기) · 실패는 기동을 막지 않습니다. 쓴 폴더는 원장 `~/.cys/claude-tui-written.json` 에 남깁니다.
- 킬스위치 `CYS_WIN_TUI_CLASSIC_OFF=1` 또는 `~/.cys/win-tui-classic-off` — 이후 기록을 멈추고 원장의 폴더에서 **값이 아직 정확히 `"default"` 일 때만** 되돌립니다(사용자 변경값 보존).
- `"tui": "fullscreen"` 이 이미 있으면 덮지 않되 침묵하지 않습니다 — 기동 로그 폴더당 1회 · `cys doctor` 의 `claude-tui-fullscreen` 항목(윈도우) · 그 pane 에서 휠이 처음 억제되는 순간 토스트 1회.
- **완전 초기화(`cys factory-reset`)가 이 기록도 되돌립니다**(성찰 2회차): 원장 항목대로 같은 규약으로 빼고, cys 가 만든 `.bak-cys-tui` 만 격리 폴더로 옮기며, 원장은 `~/.cys` 와 함께 격리됩니다.
- 화면 파서(vt100 0.15.2)를 저장소 안(`vendor/vt100/`)으로 들여와 폭 축소 뒤 넓은 글자(한글) 반쪽 · 범위 밖 짝 칸 · u16 언더플로 · 축소 뒤 DECRC 로 패닉하던 결함을 고쳤습니다.
- 윈도우 Antigravity(agy) 상태줄 자동 연결 — `.cmd` 래퍼 + 쓰기 전 실연 검사(실패 = 파일 무변경 · 버전 단위 1회 · 같은 버전도 24시간 뒤 재검사) · 비ASCII 사용자 폴더는 `%USERPROFILE%` 꼴.

## 2. 큐가 `prompt_unknown`·`input_pending` 으로 영영 멈추던 것 — 화면 사본 재동기

- **원인**: 데몬의 화면 사본은 좌석 출력만으로 만듭니다. Claude Code 는 바뀐 곳만 다시 그리고 유휴 좌석은 프롬프트 줄을 다시 그리지 않으므로, 사본이 한 번 어긋나면(파서 패닉 격리
  뒤 빈 파서 · 버려진 청크) 커서 행에서 프롬프트 표지를 영영 못 찾습니다(윈도우 11 제보 — master 16분 · worker 빈 프롬프트). 사람이 send+Return 을 치면 풀렸던 것은 그것이 다시
  그리기를 일으켰기 때문입니다.
- **고친 것**: 프롬프트 표지를 아는 에이전트 좌석이 **큐가 막힌 채** 화면을 못 읽는 상태로 1분(파서 패닉 뒤 10초) 넘게 이어지면 **키를 한 바이트도 보내지 않고** PTY **높이만**
  한 줄 줄였다 약 0.5초 뒤 되돌려 다시 그리기를 요청합니다(이벤트 `screen.repaint_requested`). 좌석당 5분 이상 간격 · 미복구 시 두 배씩(최대 80분) · 끄기 `CYS_SCREEN_REPAINT_NUDGE=0`.
  크기 변경은 GUI `surface.resize` 와 같은 락으로 직렬화되고, 흔드는 사이 바깥 변경이 있었으면 되돌리지 않습니다.
- 요청하는 꼴 셋 — ① 커서 행 판독 불가(`prompt_unknown`·`input_pending_unknown`) ② 출력이 60초 넘게 없는데 `busy`·`modal_pending` 표지가 남은 좌석(낡은 사본일 수 있음) ③ 전체화면 좌석 가운데
  전경이 에이전트인 좌석. 배달 판정은 바꾸지 않습니다.
- **다시 그린 화면은 옛 줄의 재방송입니다 — 반향 제외 창**(성찰 2회차 M1): 흔들기 시작부터 **되돌린 뒤 최소 3초, 재방송이 이어지면 마지막 출력 뒤 1초까지**(상한 60초 · 초안의 고정 3초는 바닥으로 남기고 위로 늘렸습니다 —
  윈도우 classic 좌석의 긴 재방송이 창을 넘길 수 있었습니다) 그 좌석의 출력은 건강 룰(경보·pause-queue 조치)·회상 색인을 타지 않습니다. 창 안에 줄 버퍼로 들어온 줄은 번호 구간으로
  남아 `surface.wait_for` 와 `surface.read_text since_line`(새 키 `repaint_echo_skipped`) 이 건너뜁니다 — 옛 "완료" 표지에 `cys wait-for` 가 거짓 일치하거나 옛 오류 줄이 새 오류로
  보이지 않게.
- **처방 문장은 사실만 말합니다**: 노브 끔·대상 아닌 좌석·창 4행 미만·요청했으나 미복구는 "cys 가 요청한다" 대신 그 사실을 적습니다. 이 가운데 `busy` 가 60초 넘게 정적인데 cys 가
  다시 그리기를 약속할 수 없는 경우는 **새 처방 코드 `stale_screen`**(사람 조치 — GUI 가 OS 배너를 냅니다 · 성찰 2회차 M3)입니다. 종전에는 같은 문장이 calm `wait` 로 나가
  사람에게 하라고 적고는 알리지 않았습니다.
- 큐 막힘 경보·`queue-blocked.json` 에 화면 진단 `screen_diag`(커서 행 · 표지 행 · 커서 행 선두 코드포인트 · 마지막 파서 패닉 · 마지막 재동기 요청 시각)이 붙습니다 —
  `queue-blocked.json` 의 값은 그 사유가 기록된 순간의 스냅샷입니다(지금 값은 `cys queue list --json`).

## 3. 부서 카드 — 어느 노드가 어느 계정인가

- 부서(워크스페이스) 탭 둘째 줄에 노드별 계정 묶음(`claude-4 master·worker · claude cso · Codex rv-codex`)이 붙고, 사용량 칸의 계정마다 `사용처:` 가 붙습니다. 이름은 사용량 칸과
  같은 규칙(별명 > 폴더 `claude-N` > 제공자) · 이메일은 툴팁에만(🔒 가림 적용).
- 모르면 짐작하지 않습니다 — `계정 미확인`(사유는 툴팁). 복원 좌석처럼 데몬이 기록한 폴더를 **아직 확인하지 못한** 좌석은 `계정 확인 중`(`account.state:"pending"`), 실제 대화 기록 폴더가
  기록 폴더 **밖**인 좌석만 `계정 불일치·확인 필요`(`"mismatch"` · 실제 폴더와 기록 폴더를 함께 보임)입니다(성찰 2회차 M2 — 1회차 초안은 복원 좌석도 불일치로 보였습니다).
- **표시와 경보는 다른 규칙입니다**: 사용량 경보·낡은 경보 억제(`in_use`)·`rate_in_use` 가 쓰는 좌석 신원은 0.14.44 와 같이 데몬이 기록한 설정 폴더입니다 — 복원 좌석이라고
  모든 Claude 계정이 '사용 여부 모름' 으로 접히지 않습니다(검체로 고정).
- 데몬은 좌석 행에 `account {provider, agent, account_id, profile, state[, recorded_profile]}` 를 싣습니다(이메일 없음 · 좌석 행 조립과 워치독 경로는 추가 IO 0 — 표시 폴더가 경보 폴더와 다른 좌석(불일치 등)만 그 폴더의 로그인 파일을 60초 캐시로 한 번 더 읽습니다). 에이전트→제공자 표는 사용량 귀속과 한 정의처입니다(C2).

## 4. 그 밖에 바뀐 것

- `cys doctor`: `claude-tui-fullscreen`(윈도우) · agy 상태줄 실연 검사 실패 사유 표시 · 알려진 좌석 폴더는 이 팩과 모든 `pack-dept-*` 의 agents.json 계정 폴더(기동과 같은 env 전개).
- 부서 카드 계정 라벨은 🔒 상태와 무관하게 가린 꼬리표 · 탭 렌더당 계정 표 1회.
- 원장 `~/.cys/claude-tui-written.json` 갱신은 **원장 전용 잠금 파일**(`claude-tui-written.json.lock` · 생성 배타 · 모든 OS) 아래 병합형 RMW(충돌 시 다시 읽어 그 위에 적용 · 최대 5회)로 합니다 — 윈도우에는 설정 락이 없어 동시 launch-agent 둘이 서로의 항목을 지울 수 있었습니다(C5). 잠금을 2초 안에 얻지 못하면(상대가 죽어 남긴 잠금은 60초 뒤 치웁니다) 잠금 없이 병합 재시도만으로 진행합니다 — 그 좁은 창의 경쟁은 남습니다(정직 고지).
- 내부 정리: JSON 설정 외과 수술 도우미를 `src/settings_surgery.rs` 로 순수 이동(agy 상태줄·claude tui 공용 · 동작 변경 0 · C1).

## 5. 새로 생긴 것 · 바뀐 계약 (전부 더하기)

- 새 이벤트 `screen.repaint_requested` {surface_id, surface_ref, reason(`parser_panic`|`screen_unreadable`), unreadable_secs, parser_panics, attempt, rows, cols, next_min_interval_secs, note}.
- `queue.starved` payload · `queue-blocked.json` 행에 `screen_diag` 객체.
- `surface.read_text since_line` 응답에 `repaint_echo_skipped`(정수). 커서 셈은 종전 그대로.
- 좌석 행 `account` 객체(`surface.list`·`org.status`) · `account.state` 에 `pending` · `mismatch` 에 `recorded_profile`.
- 처방 코드 14종 — `stale_screen` 추가(`queue.starved`·`queue.list`·`org.status`·`queue-blocked.json`·경보 라우팅 `remedy=`).
- 새 env/파일: `CYS_SCREEN_REPAINT_NUDGE`(0=끔) · `CYS_WIN_TUI_CLASSIC_OFF` / `~/.cys/win-tui-classic-off`(윈도우 전용) · 원장 `~/.cys/claude-tui-written.json`(완전 초기화 목록 등재).
- 팩: `hooks/cys-agy-statusline.cmd`(윈도우 agy 상태줄 래퍼). 가산 RPC 메서드는 없습니다.

## 6. 윈도우 — 켜진 것과 끈 채로 낸 것

- 켜짐(윈도우 기본 on): claude 좌석 classic 고정(§1) · agy 상태줄 자동 연결(`.cmd` + 실연 검사) · 화면 사본 재동기(전체화면 좌석은 전경을 알 수 없어 프롬프트 표지를 아는 에이전트 좌석이면 요청).
- 맥·리눅스와 같음: 처방 코드 · 계정 표시 · 반향 제외 창.
- 윈도우에서만 다른 것: 설정 파일 락(flock)이 없어 원장은 전용 잠금 파일 + 병합 재시도로 보호합니다(§4 · 잠금 획득 실패 2초 뒤의 좁은 경쟁 창은 남습니다).

## 7. 알려진 한계 — 정직하게

- **윈도우 ConPTY 의 재방송 길이는 실기로 재지 않았습니다.** 높이만 흔드는 선택(너비 변경은 버퍼 전체 재줄바꿈)과 '되돌린 뒤 바닥 3초 + 재방송 정적 1초' 창·줄 구간 표식은 공개 동작 기록에 근거한
  설계이지 측정 결과가 아닙니다. 윈도우 classic 좌석에서 다시 그리기 뒤 `cys read-screen --since` 의 `repaint_echo_skipped` 가 크게 나오거나 경보가 재발하면 제보해 주세요(끄기: `CYS_SCREEN_REPAINT_NUDGE=0`).
- 반향 제외 창은 최선 노력입니다 — 창 안에 시작돼 창이 닫힌 뒤 완성되는 미완성 줄 하나는 룰을 탈 수 있고, 창은 유한합니다(상한 60초).
- 다시 그리기 요청은 **큐가 막힌 좌석**에서만 납니다 — 큐가 빈 좌석의 화면 어긋남은 cys 가 모르고 고치지 않습니다(일반 화면 어긋남 수리 기능이 아닙니다).
- 대체 화면 판정은 파서 사본에서 나오므로 파서 패닉 직후에는 전체화면 앱도 흔들릴 수 있습니다(남는 방어는 마커 좌석 조건 하나 · 손실은 한 번의 다시 그리기).
- classic 고정은 Claude Code 사용자 설정 스키마가 모르는 키를 벗겨낼 뿐 파일을 버리지 않는다는 정적 판독(2.1.289~2.1.292)에 기댑니다 — 2.1.233 미만 판은 디스크에 없어 판독하지 못했습니다.
- `계정 확인 중`→확인은 그 좌석에서 대화가 시작돼 transcript 가 관측될 때 바뀝니다(사이드바 10초 주기 + 신원 캐시 60초).
- Codex·Antigravity 는 기기의 로그인 하나를 계정으로 봅니다 — 좌석마다 다른 계정을 쓰는 구성은 구분하지 못합니다.

## 8. 되돌리는 손잡이

- 화면 사본 재동기: `CYS_SCREEN_REPAINT_NUDGE=0`.
- 윈도우 classic 고정: `CYS_WIN_TUI_CLASSIC_OFF=1` 또는 `~/.cys/win-tui-classic-off`(원장의 폴더에서 cys 가 넣은 값만 되돌림 · 사용자 변경값 보존).
- agy 상태줄 자동 연결: `CYS_AGY_STATUSLINE=0` 또는 `~/.cys/agy-statusline-off`.
- 처방 코드 · 계정 표시: 되돌리는 노브가 없습니다(표시·안내뿐이며 어떤 배달 판정도 바꾸지 않습니다).

## 9. 검증 (이 워크트리 · 맥 · 발행 때 갱신)

- `cargo test --lib`: 738 통과 · 0 실패 · 1 ignored (C1 이동 뒤 · 성찰 2회차 전체 반영 뒤 재측정 예정)
- `cargo test --bin cysd`: (재측정 예정 — 허용된 유일 실패 `b3_status_polling_does_not_restat_the_identity_file`)
- `cargo test --bin cys`: (재측정 예정)
- ui `bun test`: 2312 통과 · 0 실패 (61 파일) · `bunx tsc -p tsconfig.check.json`: 오류 0
- `scripts/secret-scan.sh --all`: clean(1113 파일) · `scripts/scan-pack-secrets.sh`: OK
- 윈도우 실기: **미측정**(§7).

## 10. 업그레이드하실 때

- 평소처럼 업데이트하면 됩니다. 윈도우는 업데이트 뒤 첫 claude 기동 때 좌석 설정에 `tui` 가 들어가며, 이미 떠 있는 전체화면 pane 은 다음 기동부터 바뀝니다.
- 구버전 데몬과 새 GUI 가 섞여도 됩니다 — 새 키(`account`·`screen_diag`·`repaint_echo_skipped`·`stale_screen`)는 전부 더하기이며 없으면 종전처럼 보입니다.
