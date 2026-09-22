# v0.14.39 — 사람이 쓰던 줄을 기계가 지우지 않고, 좌석이 조용히 멈추지 않습니다

이번 판을 한 문장으로 줄이면 이렇습니다 — **파일을 놓은 자리에 파일이 들어가고, 부서 좌석의 훅이
조용히 꺼지지 않고, 사람이 입력줄에 쓰던 글자를 자동화가 지우거나 그 위에 끼어들지 않고, 자동
사이클이 빈손으로 "성공" 이라고 말하지 않습니다.**

> ### ★ 먼저 하실 것 — 지침 병합 (`cys pack-merge`)
>
> 이번 판은 **사용자 소유(user-owned) 파일**을 고칩니다 — `agents.json` ·
> `directives/REVIEWER_DIRECTIVE.md` · `directives/MASTER_DIRECTIVE.md` ·
> `directives/CSO_DIRECTIVE.md` · `directives/CEO_TEMPLATE.md`. 이 등급은 매니페스트 해시가 맞아도,
> `--force` 여도 **덮이지 않습니다** — 신버전이 `<파일>.new` 로 나란히 도착할 뿐입니다. 신규
> 설치본에만 자동 도달하므로, 기존에 쓰시던 분은 아래를 한 번 돌려 주셔야 새 내용이 발효됩니다.
>
> ```
> cys pack-merge                                                   # 병합 대기 목록
> cys pack-merge --file directives/MASTER_DIRECTIVE.md --take-new  # 손대지 않으신 파일
> cys pack-merge --file agents.json --keep-mine                    # 또는 3-way
> ```
>
> **CEO 승격 기계는 `.md` 가 CEO 템플릿 사본이라 `--take-new` 를 쓰지 마시고** preflight
> `C03.pin.master` 안내를 따라 주십시오. 병합이 밀려 있으면 preflight `C03.boot-contract` 가
> **WARN** 으로 뜹니다(부트 차단이 아니라 병합 대기 가시화입니다).
>
> ⚠ **`memory/**` 는 다릅니다.** 이 등급은 **seed-once** 라 파일이 이미 있으면 불가침이고
> **`.new` 병치조차 하지 않습니다**(`src/pack.rs:2087-2089`). 이번 판이 손댄
> `memory/feedback_autonomous-pilot-mandate.md` 는 **기존 설치에 도달하지 않습니다** —
> 필요하시면 릴리스 아카이브에서 직접 대조해 주십시오.
>
> 이 판의 팩 하한(`PACK_MIN_BINARY`)은 **0.14.31 그대로**입니다(아래 「14. 업그레이드하실 때」).
> 이 노트의 파일:줄은 전부 **커밋된 최종 코드**(통합 브랜치 `fix/0.14.39-bugfix-3problems`) 기준이고,
> 수치는 릴리스 준비 직전 상태의 실측입니다. 실기 검증을 못 한 항목은 그 자리에 "코드 수정(실기
> 미검증)" 이라고 적었고, **아직 닫지 못한 결함은 「11. 알려진 잔여」에 등급과 함께 그대로
> 적었습니다** — 이번 판에서 가장 먼저 읽어 주셔야 할 절입니다.

| 갈래 | 내용 | 코드 변경 |
|---|---|---|
| 문제 1 | OS 파일 드래그앤드롭이 **왼쪽 pane 으로 오배달** 되던 것 (좌표 단위) | GUI(`ui/`) |
| 문제 2 | **부서 레인 좌석에서 훅 전부가 무음으로 꺼지던** 것 (레인 가드 조기 종료) | 팩 훅(`hooks/`) · 팩 파이썬 |
| 문제 3 | 입력 계수·직접 주입·리뷰어 보류·사이클 맹목 주입·고아 데몬·자원 게이트 등 11항목 | 본체(`src/`·`src-tauri/`) · 팩 |
| 기반 | v0.14.38 이후 미출하분(phoenix 스냅샷 정본 · 검증자 worker 고정 · 상태 회전기 · 부서 schedule 시드) 동반 출하 | 본체 · 팩 |

전체 변경: v0.14.38 이후 **116 파일 · 커밋 82**(통합 merge 6 포함 · 버전 범프와 이 노트 자신의
커밋은 제외한 실질 변경분).

---

## 1. 문제1 — 파일을 오른쪽 pane 에 놓으면 왼쪽으로 들어가던 것

**무엇이 잘못돼 있었나.** 창을 좌우로 나눠 쓰실 때, 오른쪽 pane 에 파일을 끌어다 놓으면 경로가
**왼쪽 pane** 에 들어갔습니다. 원인은 드롭 좌표의 단위였습니다 — `paneAtPointStrict` 가 OS 가 준
좌표를 **전 플랫폼에서** `devicePixelRatio` 로 나눴는데, macOS·Linux 는 그 좌표를 이미 논리 픽셀로
주고 **Windows 만** 물리 픽셀로 줍니다. Retina(dpr=2) 맥에서 좌표가 절반이 되니 화면 오른쪽 절반의
점이 왼쪽 절반으로 접혔습니다.

**고친 것.**

- 좌표 환산을 순수 모듈 하나로 모았습니다 — `ui/src/droppoint.ts:17` 의 `dropPointToCss(pos, dpr,
  platform)`. macOS·Linux 는 항등, **Windows 만 `/dpr`**, 비정상 dpr 은 1 로 접습니다.
  `ui/src/main.ts:3344` 가 이 함수 하나만 부릅니다.
- **놓기 전에 어디에 들어갈지 보여 드립니다.** `tauri://drag-enter/over/leave` 를 받아 대상 pane 에
  테두리(`.drop-target`)를 켭니다(`ui/src/main.ts:7507-7515`). 기존 파일 트리 드래그와 같은 표시입니다.
- **오류 문구를 갈랐습니다.** 폴더를 읽지 못한 경우(권한·IO)는 "경로 확인 실패" + 사유, 항목이 실제로
  없어진 경우는 "경로가 더 이상 없음" 입니다. 여러 경로를 한 번에 놓으시면 "(n/총)" 순번이 붙고, 하나가
  실패하면 종전처럼 전부 중단합니다.
- 진단이 필요할 때만 `localStorage.cysDropDebug="1"` 로 원좌표·dpr·플랫폼·환산값을 토스트에 찍습니다.

**정직하게.** 실기기 드롭은 GUI 자동화 금지 규약 때문에 재지 못했습니다. 합성 payload 로 핸들러만
검증했습니다(단위 49건 + e2e 게이트 7 케이스, macOS dsf=2 오배달을 RED 로 재현 후 GREEN). 수정 빌드에서
`cysDropDebug=1` 로 1회 계측하시면 남은 y 오프셋(tauri#10744) 여부까지 확정됩니다.

---

## 2. 문제2 — 부서 좌석에서 훅이 조용히 전부 꺼지던 것

**무엇이 잘못돼 있었나.** 부서 pane 을 개인 프로필(`claude-N`)로 띄우면 팩 레인이 어긋나고, 그때
훅 앞단의 **레인 가드가 훅을 통째로 조기 종료**했습니다. 그 결과 `PreToolUse` 안전 게이트가 **무음으로
허용**되고(리뷰어의 Write 차단이 사라집니다), 부서 preflight 는 그 좌석을 C08·C33 READY 로
오판하고, 가드가 발동한 흔적은 **어디에도 남지 않았습니다**.

**고친 것 — 조기 종료가 아니라 '자기 레인 훅으로 위임' 합니다(R안).**

- 레인이 어긋나면 가드가 그 이벤트에 해당하는 **자기 레인의 훅으로 `exec` 위임**합니다
  (`cysjavis-pack/hooks/_lib.sh:345` `cys_lane_guard`). 게이트가 사라지지 않고 옳은 레인에서 한 번 돕니다.
- **표식을 남깁니다.** `<pack>/state/lane-guard-tripped` 에 `reason=` 을 씁니다(`_lib.sh:294-304`) —
  `absent`(위임 대상 부재) · `unreadable`(판독 불가) · `already-redirected`(재불일치) ·
  `no-redirect-line`(그 훅에 위임 진입점이 없음). 진단 C83 이 이 사유를 그대로 인용하고, 조치 뒤
  표식을 지우면 통과합니다(24h 지나면 자동 통과).
- **래치는 1홉만 삽니다.** `CYS_LANE_REDIRECTED` 를 위임 대상 훅의 첫 줄에서 읽고 즉시 `unset` 합니다 —
  자손 프로세스(python·cys·자동기동 데몬·좌석 훅)로 상속되지 않습니다. 이 규율이 없으면 위임 아래에서
  기동된 데몬이 래치를 함대 전체에 물려줘 훅이 대규모로 무음 사망합니다.
- **혼재 프로필에서 두 번 돌지 않습니다.** 위임을 예약하기 전에 실사용 설정 4파일
  (`CLAUDE_CONFIG_DIR`|`~/.claude` · `$PWD/.claude` 의 `settings(.local).json`)에 대응 훅이 이미
  등록돼 있으면 표식 없이 종료합니다 — 그 이벤트에서는 레인 훅이 자기 등록으로 한 번만 돕니다.
- **위임 진입점이 없는 훅은 fail-closed** 입니다. `$0` 본문에 위임 토큰이 없으면 표식을 남기고 종료합니다.
- `javis_bootstrap` · `javis_mission` 의 `hooks_effective` 는 **측정하지 못하면 `null`** 입니다(종전에는
  24h 재구현으로 참/거짓을 지어냈습니다).

**정직하게.** 부서 실좌석(개인 프로필로 띄운 pane)에서의 왕복 검증은 라이브 설정·설치 팩 무접촉 규약
때문에 이번에도 하지 못했습니다. 밀폐 검체 54건 + 리뷰어 프로브 6종으로만 쟀습니다.

---

## 3. 사람이 쓰던 줄 — 자동응답을 사람 입력으로 세던 것, 기계가 그 줄에 끼어들던 것

### ⓐ 미제출 입력 계수 (D-01 · D-03)

에이전트 TUI 가 스스로 보내는 **자동응답(괄호 붙임 모드 봉투 등)** 을 사람이 친 글자로 세는 바람에,
빈 입력줄인데도 큐 배달이 보류됐습니다. 계수를 surface 별 상태기계로 옮기고 규칙을 명시했습니다 —
봉투 자체는 제외, 봉투 **안** 의 개행은 가산, 자동응답 세그먼트는 면제, 표식은 청크 경계를 넘어
이월, 미종결 봉투는 해제. 단독 `ESC` 청크는 이월하지 않고, 분할된 Meta-Enter 는 같은 청크의
`ESC`+`CR` 일 때만 제출로 봅니다.

진단이 필요하실 때 `surface.list`(`src/bin/cysd/handlers.rs:4145-4147`) · `org.status`(`:7261-7263`) ·
`queue.list`(`:8022-8024`) 가 `pending_input_bytes` · `pending_input_human_bytes` ·
`input_paste_open` 을 싣습니다(**additive** — 기존 필드·형식 무변경). 거부가 났을 때의
`queue.draft_gate_denied` 이벤트에도 같은 계수가 붙습니다(`:2230-2231`).

### ⓑ 직접 주입 게이트 (D-12)

사람이 입력줄에 쓰다 만 글자가 있는데 기계가 `send`/`send-key Return` 을 쏘면 **사람의 문장과 기계의
문장이 붙어 제출**됐습니다. 이제 그 줄에는 직접 주입이 거부되고 `--queued` 로 안내됩니다.

- 판정 축은 키 **이름** 이 아니라 **생성되는 바이트** 입니다 — `Text` · `SubmitKey`(CR/LF) ·
  `CancelKey`(0x15 C-u · 0x03 C-c) · `ClearFirst`. 별칭으로 우회할 수 없습니다.
- **GUI 가 조립한 문안**(`cmd + "\n"`)도 같은 게이트를 지납니다. 본문 안의 0x15·0x03 도 `CancelKey`
  축으로 잡습니다.
- **기계 잔여와 사람 초안을 가릅니다.** 자동응답만 남은 줄(`pending_input>0` · human=0)은
  `--clear-first` 로 통과하고, 사람이 친 글자가 있으면 거부를 유지합니다.
- 면제는 정확히 둘입니다 — 호출자 pane 의 role 이 `master`/`cso` 이거나, 살아 있는 restore-root 의
  자손일 때. **pane 에 귀속되지 않은 호출자는 `authoritative:true` 를 실어도 거부됩니다**(음성 검체로
  고정). 데몬 스케줄이 띄운 자식(autopilot 등)이 여기에 해당합니다.

### ⓒ 자가치유가 사람 초안 좌석을 죽이지 않습니다 (rc 79)

위 게이트 때문에 `cys node-recover` 의 선정리 `C-u` 나 기동 문안이 거부될 수 있습니다. 종전에는 그
거부가 rc 1 로 나가 `escalate_reclaim` = **좌석 kill** 로 이어졌습니다. 이제 타이핑 가드·초안 게이트
거부는 전용 코드 **79(EXIT_RECOVER_REFUSED)** 로 접히고(`src/bin/cys.rs:948`), `cys boot` 는 그 좌석을
`skipped_unconfirmed`(liveness `recover_refused`)로 계상한 뒤 **다음 좌석으로 넘어갑니다** — 회수·파괴·
스폰 0. 원인 원문은 `node-recover` 의 stderr `안전 거부: recover-refused: … [draft_gate:…]` 에 남습니다.

**운영상 알아두실 것(대가).** 사람 초안이 남은 좌석은, 그 pane 에서 **Ctrl-U 를 한 번 치시거나**
데몬이 재시작될 때까지 자동 복구·자동 배달이 보류됩니다. 종전(0.14.38)은 3초 가드만 있어 오래된 초안을
**지우고** 복구했습니다 — 이제는 지우지 않고 기다립니다. 관측 신호는 `queue.draft_gate_denied` 이벤트
반복(`reason=human_draft`)과 `cys boot` 결과의 `outcome:"skipped_unconfirmed"` 입니다.

---

## 4. 리뷰어 좌석이 `prompt_unknown` 으로 영구 보류되던 것 (D-04)

gemini·codex 좌석의 composer 글리프가 `agents.json` 의 단일 `prompt_marker` 와 달라 데몬이 프롬프트를
알아보지 못하고 큐를 영구 보류했습니다.

- `prompt_marker` 가 **목록** 을 받습니다 — codex `["›","»"]` · gemini `[">"]`. (`ready_marker` 는
  종전대로 **문자열 전용** 입니다 — 모든 소비처의 규약을 하나로 통일했습니다.)
- 옛 vendor 기본값(`"›"` 단독 등)은 **읽는 시점에 승격** 됩니다. 사용자가 고쳐 둔 값은 건드리지
  않습니다. 고정하고 싶으시면 `["›"]` 처럼 **목록으로** 선언해 주세요.
- composer 마커는 **커서 행의 행 선두(공백 제외) 후보만** 인정합니다(여러 개면 더 긴 것). 출력 행
  끝에 우연히 찍힌 `->` 나 초안 속 글리프(`» abc » `)를 빈 입력으로 오인하던 경로를 전 어댑터에서
  닫았습니다.
- gemini 작업 중 어휘 `esc to cancel` 을 busy 축에 넣어, 턴 중간에 주입하지 않습니다.

**정직하게.** 라이브 좌석의 실배달 해제는 재지 못했습니다(라이브 데몬 무접촉). 설치 후 `cys queue list`
로 리뷰어 좌석의 `blocked_by` 가 `prompt_unknown` 에서 벗어나는지 한 번 봐 주십시오.

---

## 5. 자동 사이클이 빈손으로 "성공" 이라고 말하던 것 (D-16 · D-10)

`cys cycle-agent` 가 대상 좌석의 **턴 상태를 한 번도 묻지 않고** `/clear` 와 디렉티브를 시간 지연만
두고 쏜 뒤 exit 0 을 냈습니다. 화면이 받지 못했어도 "cycle complete" 였습니다.

- **4단계 직전에 대상의 유휴를 확인** 합니다. 확인 신호는 계수(`pending_input`)·출력 정적
  시간(`quiet_secs`)·composer 축입니다.
- **clear 를 데몬의 원자 경로로 보냅니다.** `Ctrl-U` → 정착 → 본문 → `Return` 의 순서를 데몬이
  한 writer 에서 처리합니다(`surface.send_text` + `clear_first`). 종전 3분할은 본문 뒤에 사람 입력이
  끼어들면 clear 와 초안이 섞였습니다. **launch-agent 등록을 잃은 좌석** 만 종전 3분할로 폴백합니다 —
  그 좌석을 원자 경로로 강제하면 영영 clear 가 나가지 않기 때문입니다(`src/bin/cys.rs:18170-18216`).
- **clear 실효 관측을 재주입 앞으로** 옮겼습니다. 판정은 세 갈래입니다(`src/bin/cys.rs:979-999`):
  실효를 **관측** 하면 디렉티브·RESUME 재주입 후 `exit 0`; 전 값은 쟀는데 창 안에 교체가 보이지
  않으면 **rc 80**(재주입 없음 — 실패 쪽); statusline 을 보고하지 않는 어댑터라 전 값을 **잴 수 없으면
  rc 81** 입니다. rc 81 에서는 실효 미확인 상태로 **화면 유휴를 확인한 뒤 최선노력 재주입** 을 하고
  (부트 체인을 복원하기 위해서입니다) 그래도 rc 는 81 로 남습니다 — **통과가 아닙니다**. RESUME 문안은
  실경로로 나갑니다(D-10).
- **SessionStart 훅이 도는 좌석** 은 디렉티브 재주입을 생략합니다. 다만 "훅이 실제로 발화했다" 는
  관측은 이 구조에서 얻을 수 없어서, **선언 ∧ 등록 확인 ∧ 최근 레인 가드 실패 표식 없음** 세 가지가
  모두 참일 때만 생략합니다(`src/bin/cys.rs:17792-17797`). 등록 확인은 설정의 등록 문자열이 실제 파일을
  가리키는지(같은 디렉터리에 `_lib.sh` 가 있는지)를 봅니다. 하나라도 어긋나면 CLI 가 한 번 주입합니다 —
  **0회 주입(치명)보다 2회 주입(무해)** 쪽으로 넘어집니다.
- **새 보류 종료코드 3종**(`src/bin/cys.rs:1011-1020`):

| 코드 | 뜻 | 조치 |
|---|---|---|
| **84** | 대상이 유휴 대기 창 내내 유휴가 아니었다 — **clear 송신 0건** | 손대지 마십시오. autopilot 이 쿨다운 뒤 자동 재시도합니다 |
| **85** | 대상 입력줄에 사람 초안(또는 미제출 입력)이 있다 — **clear 는 제출되지 않았습니다** | 그 pane 의 초안을 제출·삭제해 주십시오 |
| **86** | **clear 는 이미 나갔고(실효까지 확인)** 그 뒤 재주입이 보류됐다 | **손으로 다시 clear 하지 마십시오** |

> **85 의 정확한 뜻** — 원자 경로에서 거부되면 정말로 **송신 0건 · composer 무변경** 입니다. 반면
> **등록을 잃어 3분할로 폴백한 좌석** 에서는 `C-u` 1키가, 또는 `C-u` 와 **clear 본문까지** 입력줄에
> 들어간 뒤 마지막 `Return` 만 거부될 수 있습니다. 그 경우 오류 문면이 그대로 말해 줍니다 —
> "clear 본문은 composer 에 들어갔지만 제출되지 않았다 · 손으로 다시 clear 하지 마라 · 남은 clear
> 본문은 다음 재시도의 `C-u` 가 지운다"(`src/bin/cys.rs:18205-18215`). 어느 경우든 **초안은 지워지지
> 않습니다.**

- **84·85 는 실패가 아니라 비파괴 보류** 입니다. `javis_cycle_autopilot` 이 `held_noop` 으로 종결하고
  **지수 쿨다운(300 → 600 → 1200초, 정상 cadence 에서 포화)** 뒤 자동 재시도합니다 — reset 불필요.
  **구조적 보류**(rc 84 + 진단 토큰 `[diag=quiet_secs_unreported]` = 0.14.30 이하 옛 데몬. 재시도가
  원리적으로 무의미)만 3회에서 멈추고 그 순간 1회 알립니다 — 데몬을 갱신하신 뒤 reset 해 주십시오.
- **통지 홍수를 막았습니다.** tick 은 더 이상 어떤 통지도 내지 않습니다. 보류 통지는 **연속 1회째와
  이후 3회마다**(streak 1·3·6·9…)이고(`javis_cycle_autopilot.py:512`·`:564-566`), 원장 재조회가
  예측과 어긋난 경우에만 예외로 한 번 더 보장합니다(경합 때 보류가 무통지로 지나가는 것을 막습니다).
  구조적 상한 도달 시 별도로 1회. 종전 구현은 매 틱 escalate 라 `javis_wakeup` 멱등키가 배달 뒤
  소멸하는 성질 때문에 **매분 master 로 push** 가 나갔습니다. 비구조 보류는 재시도에 상한이 없으므로
  통지 **총량** 에 상한이 있는 것은 아닙니다 — 최악 cadence 에서 약 1시간에 1건 꼴입니다.

---

## 6. 고아 데몬 · `cysd` 인자 (D-06 · 신규)

- `cys connect --socket` 이 **자격 검사 없이** 임의 소켓 이름으로 데몬을 자동 기동하던 경로를 닫았습니다.
  자격이 없으면 스폰 0 · `exit 2`. 정상 3경로(본부·부서·복원)는 그대로 통과합니다.
- `cysd` 가 **인자를 전혀 보지 않아** `cysd --version` 이 데몬을 띄웠습니다. 이제 `--version`/`-V` ·
  `--help` 를 처리하고, 모르는 인자는 usage 후 `exit 2`, **무인자일 때만** 기동합니다.

---

## 7. 팩 쪽 정비

| 항목 | 내용 |
|---|---|
| **D-07** | phoenix 스냅샷의 본부 SOT 누락 계수(TODO glob) · `autopilot.json` 을 optional 로 분류(manifest additive) · `--dry-run` 이 정말 아무것도 기록·삭제하지 않습니다 · `.tmp-*` 청소가 **살아 있는 pid 의 것을 보호**하고 최소 나이를 지킵니다(진행 중 스냅샷 파괴 경로 제거) |
| **D-09** | CSO 의 `cys feed reply` — 검증자를 worker 로 고정하고 호출자==검증자를 사전 차단(미출하분 동반 출하) |
| **D-10** | RESUME 문안 경로 — 파이썬(미출하분)·러스트 양쪽 |
| **D-11** | 자원 게이트 `nodes` 축이 함대 소유가 아닌 node 프로세스를 세던 것. 이제 `_fleet_owner ∈ {claude,agy,codex,gemini}` ∧ 제외 패턴 아님 ∧ 앱 번들 argv0 아님 으로 셉니다(실측 21 → 10). `servers` 축에서도 앱 번들 node 5건을 뺍니다. **수치가 내려가는 방향** 이라 편성·부트가 자원 부족으로 막히던 오발화가 줄어듭니다 |
| **D-13** | `review-prompt` 에 verdict 계약을 탑재했습니다. `javis_verdict contract` 서브커맨드 · `REVIEWER_ENUM` · 골격 · 선택 키 `revision`. 계약 문서 `<pack>/round/REVIEWER_VERDICT_CONTRACT.md` 는 상수에서 생성되고 드리프트 검체가 잠급니다 |
| **D-14** | **무조치** — 문서의 논거가 배포본 기준으로 성립하지 않습니다. 결정 사유를 코드 주석으로 기록했습니다(코드 변경 0) |
| **pyc 누수** | 설치 팩 `bin/__pycache__` 에 `.pyc` 가 남던 경로. 정확한 호출자는 확정하지 못했고, 방어로 **형제 모듈을 import 하는 `bin/*.py` 32개가 스스로 `sys.dont_write_bytecode = True`** 를 겁니다(SEAL-1 층4 · census 로 고정). 행동 실증: 같은 절차에서 base 사본 1건 → 이번 0건 |
| 동반 | 상태 파일 회전기 · 부서 `schedule` 시드(미출하분) |

---

## 8. 통합 단계에서 더 고친 것

6개 작업묶음을 합친 뒤, 적대 검증 2회·아키텍트 검증 2회·부트 체인 전용 검토 2회를 돌려 나온 것들입니다.

- **죽은 셸 pane 의 GUI 재기동이 스스로 풀리지 않던 것** — 기계 잔여만 남은 pane 에서 GUI 의 `restartNode`·
  `launch` 자동 문안이 `[draft_gate:pending_input]` 로 거부되고, 빈 composer 를 전제로 하는 stale 리셋은
  죽은 셸에서 발동하지 않았습니다. 이제 그 경로가 `clear_first` 를 싣습니다(**기계 잔여 통과 · 사람 초안
  거부** 는 유지).
- **GUI 거부 사유가 한국어로 보입니다.** `send_input` 이 데몬의 `error.code` 를 UI 까지 올리도록
  `rpc_full` 로 바뀌었고(`src-tauri/src/main.rs:556-565`), 거부 번역이 **프로덕션 문면** 으로 검증됩니다.
  종전 검체는 조작된 전제로 초록이었고 실제로는 한 번도 발화하지 않았습니다.
- **사이클 clear 가 첫 실행 관문 화면을 건너뛰지 못합니다.** 폴더 신뢰·테마·로그인·OAuth 코드 같은
  관문 프레임에서는 `Ctrl-U`+`/clear`+CR 이 나가지 않습니다(관문 7종 보류 · 건강 화면 4종은 그대로
  Idle). 기계가 사람 승인 관문을 대신 누르던 경로가 닫혔습니다.
- **훅 선언을 런타임으로 검증합니다**(5절) — 등록만 보고 디렉티브를 생략하던 것을, 등록 확인에
  **레인 가드 실패 표식 부재** 를 AND 해 0회 주입 쪽으로 넘어지지 않게 했습니다. 실제 발화 관측은
  이 구조에서 얻을 수 없어 **보수적 근사** 입니다(코드 주석에도 그렇게 적혀 있습니다).
- **secret-scan** — 6개 작업묶음이 새로 들인 개인 경로 11건 정규화(`--all` 1000 파일 clean).
- **상태 회전기의 임시파일** 고정 `.tmp` → `mkstemp`(동시 회전 충돌 제거).

---

## 9. 새로 생긴 것 · 바뀐 계약

- 종료코드: `cys cycle-agent` **80**(실효 미관측) · **81**(측정 불능) · **84·85·86**(보류 3종) ·
  `cys node-recover` **79**(비파괴 거부) · `cys connect` 자격 거부 **2**. 80·81 은 동반 출하되는
  미출하분(`8991b894`)의 계약이고 0 은 **실효 관측에만** 씁니다.
- RPC(전부 **additive** — 기존 필드·형식 무변경): `queue.list` · `org.status` · `surface.list` 에
  `pending_input_bytes` · `pending_input_human_bytes` · `input_paste_open`.
- `agents.json`: `prompt_marker` 가 문자열 **또는 목록**. `ready_marker` 는 문자열 전용.
  claude 블록에 `hooks_inject_directive`.
- autopilot 원장: `held_noop` detail 에 `structural` · `alive_evidence` · `keys_sent` · `held_streak` ·
  `held_structural_streak` · `cooldown_secs` · `retry_after_ts` · `residual_window_secs`.
  ⚠ `fired`/`abort` 의 `residual_window` 키는 **`residual_window_note` 로 개명** 했습니다 — 이 원장을
  읽는 대시보드가 있으시면 갱신 대상입니다.
- 팩 표식: `<pack>/state/lane-guard-tripped`(`reason=` 4종).
- 새 공개 API: `cys::agent_markers::pick_marker_leading` · `cys::readiness::gate_or_modal_present` ·
  `cys::pack::lane_guard_tripped`.
- 새 서브커맨드: `javis_verdict contract`.

---

## 10. 이관(업그레이드) 주의

1. **`cys pack-merge` 가 필요합니다**(노트 맨 위 참조) — `agents.json` ·
   `directives/REVIEWER_DIRECTIVE.md`(동반 출하분에는 `MASTER_DIRECTIVE.md` · `CSO_DIRECTIVE.md` ·
   `CEO_TEMPLATE.md` 도 포함)는 `.new` 로만 도착합니다. `memory/**` 는 seed-once 라 기존 설치에
   아예 도달하지 않습니다. 계약 문서 `round/REVIEWER_VERDICT_CONTRACT.md` 와
   `schemas/verdict_schema.json` 은 시스템 소유라 자동 배치됩니다.
2. **`prompt_marker` 승격은 자동** 입니다 — 옛 vendor 기본값은 읽는 시점에 목록으로 올라갑니다.
   손으로 고쳐 두신 값은 보존됩니다. 고정을 원하시면 목록(`["›"]`)으로 선언해 주십시오.
3. **훅 위임이 켜집니다.** 부서 pane 을 개인 프로필로 띄우신 적이 있다면, 갱신 직후 `state/lane-guard-tripped`
   표식이 보일 수 있습니다 — 그 자리가 종전에 **훅이 조용히 꺼져 있던** 자리입니다. 사유를 확인하고
   조치한 뒤 표식을 지우시면 됩니다(24h 지나면 자동 통과).
4. **버전이 섞였을 때(스큐) 어떻게 되나.** 두 방향이 다릅니다.
   - **옛 데몬 × 새 CLI** — 0.14.30 이하 데몬은 `quiet_secs` 를 보고하지 않아 사이클이 구조적 보류
     (rc 84 + `[diag=quiet_secs_unreported]`)가 되고 3회 뒤 멈춥니다. 본체를 갱신하신 뒤
     `reset` 해 주십시오.
   - **옛 팩 × 새 본체** — 이쪽이 종료코드 해석이 없는 방향입니다. 옛 autopilot 은 84·85 를 보류가
     아니라 **실패** 로 계상하고, 86 을 사후검증 없이 넘깁니다(파괴적이지는 않지만 원장이 어긋납니다).
     본체와 팩을 함께 갱신하시는 편을 권합니다.
   - **새 팩 × 옛 본체** — 새 autopilot 은 84·85 를 이미 `held_noop` 으로 다루고 86 은 held 대상에서
     빼 반드시 사후검증합니다(`javis_cycle_autopilot.py:394`·`:397`). 옛 본체는 그 코드를 아예 내지
     않으므로 종전 계약대로 돕니다.
   - 팩 하한(`PACK_MIN_BINARY`)은 이번 판에서 **0.14.31 그대로** 두었습니다 — 상향 여부는 오너 결정
     항목입니다(아래 11-⑧).
5. **사람 초안 좌석의 보류**(3-ⓒ) — 종전과 달리 초안을 지우지 않습니다. 그 pane 에서 `Ctrl-U` 한 번이
   해제 조작입니다.
6. 세션·부서·직원은 유지됩니다.

---

## 11. 알려진 잔여 — 정직하게

### ★ 이번 판에서 닫지 못한 결함 (릴리스 판단에 직접 걸립니다)

통합 뒤 적대 검증(reflect1) · 아키텍트 검증(reflect2) · 부트 체인 전용 검토가 **각각 독립으로** 같은
자리를 지목했고, 수정 라운드 상한(2회)을 소진한 뒤에 나온 것이라 **이 판에는 들어가지 못했습니다.**
수리안은 세 검토가 같은 것을 제시했고 실측으로 비-vacuous 까지 확인돼 있습니다.

**① [blocking] 관문·모달 술어에 '생애 창' 이 없어, 지나간 화면 때문에 좌석의 `/clear` 가 영영 나가지
않을 수 있습니다.**

- 자리: `src/readiness.rs:325` `gate_or_modal_present`(모달 축이 `modal_signature` /
  `modal_signature_with_marker` 를 **날것으로** 씁니다) → `src/bin/cys.rs:1165` `cycle_target_observation`
  → `cycle_target_state` 세 팔(`:1065` 이하) → `wait_cycle_target_idle("clear 직전")`.
- 무슨 일이 벌어지나: 화면 **스크롤백에** 모달 어휘가 한 번 인쇄돼 있기만 하면(예: 워커가 관문 화면을
  읽어 그 내용을 자기 출력으로 찍은 경우 — `1. Yes, I trust this folder` / `2. No, exit` 두 줄) 그 아래
  composer 가 **비어 있고 출력이 120초째 조용해도** 대상이 "턴 중" 으로 판정됩니다. 같은 화면에서 저장소
  자신의 전경 술어 `modal_foreground(screen, Some("❯"))` 는 `None`(= 이 문면은 역사)이라고 답합니다.
- 귀결: 유휴 대기가 창 내내 실패 → rc 84 → autopilot 이 `held_noop` + 쿨다운으로 **무한 재시도**
  (비구조 rc84 는 상한 밖). 그 문면은 새 출력이 밀어내야 사라지는데 **유휴 좌석에는 새 출력이 없습니다** —
  조용한 자기잠금입니다. 게다가 진단 문구가 "rc84: 대상이 유휴 대기 창 내내 턴 중(살아 있음)" 이라
  **유휴 좌석을 턴 중으로 오진** 해, 사람이 개입해도 엉뚱한 곳을 보게 됩니다.
- 같은 등급의 평시 오탐도 실측됐습니다 — 본문에 `Enter to confirm` 과 `Esc to cancel` 이 함께 있는
  프레임, gemini 좌석의 마크다운 인용 목록(`> 1. …`), `$ cat a > 1. txt` 같은 한 줄.
- 수리안(실측 검증 완료): 관문 축(`first_run_gates::identify`)은 그대로 두고 **모달 축만**
  `modal_foreground` 로 교체하거나, `gate_block_left_behind`/`modal_left_behind` 를 AND 한 재주입판
  술어를 신설합니다. 13 픽스처(관문 7 · 건강 4 · 역사 2) 판정이 현행과 **전부 동일** 하고, 적용 후
  `cargo test --bin cys` 354/0 · `--lib` 605/0 GREEN 입니다. 함께 '역사 모달 어휘 + 건강한 composer →
  Idle' 음성 검체를 박아야 합니다(현행 검체 집합은 두 술어를 구별하지 못합니다).
- **운영 회피책(수리 전까지):** 좌석 화면에 관문·모달 문면을 **인쇄하지 마십시오**(감사표 `cat` ·
  관문 좌석 `read-screen` 출력 인용). 이미 걸린 좌석은 그 pane 에서 아무 출력이나 한 번 내어 문면을
  밀어내면 풀립니다.

**② [major] `gate_carry_ok` 의 첫 팔이 관문 부재를 보지 않고 이월을 풉니다(옛 데몬 한정).**

- 자리: `src/bin/cys.rs:13629` `None if idle_axis_capable == Some(false) => true` → 소비처
  `boot_agent_on_surface`(:13241 부근) · `gate_pending_reobserve_once`(:13885 부근).
- `quiet_secs` 를 보고하지 않는 옛 데몬(0.14.30 이하)에서는 팔 2·팔 3 의 방어(글리프 축 · 관문 AND)
  앞에 이 팔이 있어, **관문 화면인데도 이월이 풀립니다**(실측: OAUTH_CODE·THEME·LOGIN_METHOD·
  FOLDER_TRUST 에서 `carry_ok=true`). 도달 조건은 **새 CLI × 옛 데몬 스큐** = 옛 설치본 이관의 정상
  경로입니다. 귀결은 디렉티브가 선택기에 붙고 Return 이 `No, exit` 를 누르는 것입니다.
- 오늘 이 경로가 실제로 터지는지는 소비처에 달려 있습니다 — 두 소비처 모두 `Site::Boot` 라 데몬 judge 가
  먼저 `GateHeld` 를 내는 것을 실측으로 확인했습니다(OAUTH_CODE·THEME·FOLDER_TRUST 전부). 즉 **오늘은
  2차 방벽이 살아 있고, 소비처가 재주입 사이트로 늘어나는 순간 fail-open 이 됩니다.**
- 수리안: `None if idle_axis_capable == Some(false) => !gate_or_modal`. '영구 보류' 회귀 우려는 이 AND
  에는 성립하지 않습니다 — quiet 축 부재는 그 좌석의 구조적 결측이라 영원하지만, 관문 프레임은 사람이
  통과하면 사라지는 일시 상태입니다. 검체는 기존 `gate_carry_ok_*` 에 `capable=Some(false)` ×
  {OAUTH_CODE, THEME} 두 행을 더해 팔 순서 변경을 회귀 대상으로 만듭니다.

### 설계 단계에서 '이번 판 밖' 으로 결정한 것

| 항목 | 상태 | 사유 |
|---|---|---|
| **D-02** (편집키 감산 없음) | **보류** · 검체 2건 `#[ignore]` | 의도된 fail-closed + stale 리셋이 회수합니다. 감산을 넣으면 사람 초안 보호가 약해집니다 — 오너 결정 항목 |
| **D-05** (경보 5분 무한 반복) | **정책 후속** | 버그가 아니라 박제된 정책입니다. 변경은 별도 결정 |
| **D-15(B)** (`duplicate_procs` 60초 반복 + killable) | **정책 후속 · 이번 범위 밖(무변경 확인)** | 옵트인 자동 kill 의 위험이 소음보다 큽니다 |
| **D-14** (회신을 `--queued` 로 주입) | **무조치** | 문서의 논거가 배포본 기준으로 성립하지 않습니다(결정 사유를 주석으로 기록) |
| **X-01** (bookwriting M14 UTC 판정) | **별도 저장소** | `cys-bookwriting-skills` — 이 배포본 밖 |
| **F-01·F-02·F-03** | 결함 아님(기능 요청) | 장치 부재 전제는 참. 별도 결정 |

### 그 밖의 잔여(minor) · 고지

1. **레인 가드 표식에 회복 신호가 없습니다.** 표식은 훅이 성공해도 지워지지 않고 레인당 1개라,
   다른 훅(`user-prompt-submit.sh` 등)의 트립 하나가 24시간 동안 `session_start` 선언을 강등합니다 →
   그동안 '훅 1회 + CLI 1회' 상시 이중 주입. 수리는 `LaneGuardTrip.script` 를 소비하는 한 줄입니다.
   실패 방향은 무해 쪽(주입 0회가 아니라 2회)입니다.
2. **`gates` 는 agent 메타가 없으면 빈 배열** 이라 관문 축이 통째로 꺼집니다(`--clear-cmd` 수동 경로 한정).
3. **`LIVE_PERMISSION_PROMPT` 는 보류로 분류** 됩니다 — 사람 승인을 오래 기다리는 좌석은 그동안 clear 가
   나가지 않습니다(의도된 방향이나 위 ①과 인접).
4. **거부 번역 문면에 정의처 핀이 없습니다** — `ui/src/restartplan.test.ts` 의 `PROD_*` 는 손으로 옮긴
   리터럴이라, Rust 쪽 문면이 바뀌면 그 축이 조용히 죽습니다.
5. **`bootstrap-backoff` 에 같은 통지 패턴이 선재** 합니다(워치독 10분 주기 × 60분 창 = 최대 6건/시간).
   이번 범위 밖 — 5절의 처방을 그대로 적용하면 닫힙니다.
6. **에이전트 사망 후 맨 셸의 PS2 `> `** 는 행 선두라 gemini composer 와 구별되지 않습니다.
7. **`cys::pack::LANE_GUARD_RECENT_SECS`(Rust)와 `javis_preflight.LANE_GUARD_RECENT_S`(Python)는 사본
   2벌** 이고 파리티 핀이 없습니다.
8. **`PACK_MIN_BINARY` 상향 여부** — 이번 팩의 autopilot 은 본체의 새 종료코드(84/85/86)와 새 RPC
   필드를 **전제로 설계** 됐습니다. 옛 본체에 얹으면 그 코드가 아예 나오지 않으므로 팩은 종전 계약대로
   돌고(파괴적이지 않음) **D-16 이 고친 것이 그 좌석에는 도달하지 않습니다** — 즉 팩만 갱신해서는
   '빈손 성공' 이 닫히지 않습니다. 하한을 0.14.39 로 올리면 이 스큐가 구조적으로 막히지만, 하한 상향은
   옛 본체 사용자를 팩 갱신에서 잘라내는 쪽이기도 합니다. 판별기는 경로만 보므로 이것은 **사람 판단**
   항목입니다(`scripts/release-lane-check.sh:9`).
9. **옛 설치본의 msys 훅 등록형**(`/c/Users/…`)은 실재 판정에서 미인정 → 강등(이중 주입 · 무해 방향 ·
   미측정).
10. **H-CONC-3**(`javis_state_rotate.py` 관련 health 검체)은 base 에 이미 있던 결함이며 이번에 건드리지
    않았습니다.

---

## 12. 검증

전량을 CI 동형으로 돌렸습니다. 원출력은 증거 폴더
`~/Desktop/CYSjavis/_evidence/impl-3problems-20260921/integ/` 에 있습니다.

| 스위트 | 결과 | 원출력 |
|---|---|---|
| `cargo test --bin cysd -- --test-threads=1 --skip hwmon` | **1387 passed / 0 failed / 3 ignored** (215.9s) | `boot3-03-cysd.log` |
| `cargo test --lib -- --test-threads=1` | **605 / 0 / 1 ignored** (424.1s) | `boot3-02-lib.log` |
| `cargo test --bin cys -- --test-threads=1` | **351 / 0** (103.1s) | `boot3-01-cys-bin.log` |
| `cargo test -p cys-app --bins` | **135 / 0** | `r2b-04-cys-app.log` |
| `run_bootstrap_health.py --json`(전량) | **GREEN · 151 pass / 0 fail / 1 skip** (344.8s) | `12-health-full.json` |
| ubuntu-pack-suite 루프(`test_*` 전종) | **ALL PASS · fails=0** | `40-pack-suite.log` |
| `test_pyseal_census.py` | **PYSEAL-CENSUS-OK** | `boot3-41-pyseal.log` |
| `javis_cycle_autopilot self-test` | **PASS 324 / FAIL 0** | `boot3-63-autopilot-selftest.log` |
| `lane-parity-rehearsal.sh --strict` / `--self-test` | rc 0 / rc 0 | `30-`·`31-lane-parity-*.log` |
| ci-branch 인라인 LANE-GATE(3레인 등재 대조) | 비대칭 0 · rc 0 | `32-lane-gate-inline.log` |
| `ui: bun test` / `bunx tsc` / `ui/build.sh` | **994 pass / 0 fail** · rc 0 · rc 0 | `boot3-20`·`21-*`·`22-ui-build.log` |
| `secret-scan.sh --all` | **clean (1000 파일)** | `33-secret-scan.log` |
| `win-typecheck.sh`(x86_64-pc-windows-msvc) | **판정 0 — 컴파일 오류 0**(경고 4 = 기존 dead_code) | `boot3-65-win-typecheck.log` |
| rc 79 자가치유 격리 데몬 E2E | `verdict pass=true` · 잔존 프로세스 0 | `60-rc79-e2e.log` |
| 버전 SOT 8곳 | **✅ 8곳 일치: 0.14.39** | 이 커밋 |
| 릴리스 레인 | **BINARY 레인**(팩 외 변경 26건 · `v0.14.38..HEAD`) | `70-release-lane-check.log` |

플래키로 알려진 2건(`handlers::a_cancelled_attempt_never_commits` · `reclaim_commit_aborts`)은 이번
전량에서 재실행 없이 통과했습니다.

**변이(뮤테이션)로 확인한 것** — 검체가 빈 통과가 아닌지 각 작업묶음에서 술어를 꺼 보고 해당 검체가
빨개지는 것을 확인했습니다(입력 게이트 축 제거 · rc 79 접기 제거 · autopilot 통지 정책 · `HELD_RCS`
변경 · D-04 마커 규율 등).

---

## 13. Windows

- Windows 분기(`cfg(windows)` · cygpath · 경로 2벌 · NSIS · plist) **신규 변경 0줄** 입니다.
  플랫폼 분기를 새로 만든 곳은 두 군데뿐이고 둘 다 보수 방향입니다 —
  `ui/src/droppoint.ts` 의 `platform==="windows"` 일 때만 `/dpr`(wry 의 `ScreenToClient` 가 물리 px 를
  주는 근거), `javis_state_snapshot._pid_alive` 가 `os.name=="nt"` 에서 `os.kill(pid,0)`
  (Windows 에서는 `TerminateProcess` 입니다) 대신 `OpenProcess` 로 판정하고 **판정 불가는 '살아 있음'**
  으로 접어 지우지 않습니다.
- 교차 타입체크: **오류 0 · 판정 0 통과**.
- **실기 미검증** 입니다 — 드롭 좌표, `_pid_alive` 분기, 훅 등록형 경로 인정은 정적 근거와 단위
  검체까지입니다.
- `docs/WDSI_SUBMISSION.md` §5 절차를 재확인했습니다. **이번 판도 제출 전제조건 미충족** 입니다 —
  에이전트가 접근 가능한 Windows 기기가 없어 "탐지가 실제로 났는지" 를 관측할 수 없습니다. 탐지가
  관측되지 않으면 제출하지 않는 것이 규약입니다(v0.14.31·v0.14.32 와 동일 사유). 태그 후 `cys.exe` ·
  `cysd.exe` 해시로 재신고해야 하며, `SHA256SUMS.txt` 전 자산 갱신은 릴리스 체크리스트 ⑤ 에 있습니다.

### Windows Defender / SmartScreen 안내 (유지)

Windows용 설치 파일은 Authenticode 서명이 없어(인증서 미보유) 실행 시 SmartScreen
"알 수 없는 게시자" 경고가 뜰 수 있습니다(**추가 정보 → 실행**으로 진행).
홈페이지의 `SHA256SUMS.txt` 와 대조해 무결성을 확인할 수 있습니다.
Defender 오탐은 WDSI 신고로 낮춰 갑니다(`docs/WDSI_SUBMISSION.md`).

---

## 14. 업그레이드하실 때

- 본체(GUI·CLI·데몬) 변경이 있는 **바이너리 릴리스** 입니다(`ui/` · `src/bin/cys.rs` ·
  `src/bin/cysd/**` · `src-tauri/src/main.rs` · `src/readiness.rs` · `src/agent_markers.rs`).
  홈페이지에서 설치본을 받아 덮어 설치해 주세요.
- 팩 변경은 릴리스의 `pack.tar.gz` 로도 실립니다. 본체를 아직 갱신하지 않은 설치본은 인앱 Update 의
  `↻` 배지 또는 `cys pack-update` 로 받습니다 — 다만 이번 팩은 본체의 새 종료코드 계약에 기대므로
  **본체를 함께 갱신하시는 편을 권합니다**(10-4).
- **`cys pack-merge` 를 한 번 돌려 주십시오**(10-1) — 사용자 소유 지침·`agents.json` 이 `.new` 로 옵니다.
- 팩 하한(`PACK_MIN_BINARY`)은 **0.14.31 그대로** 입니다.
- 세션·부서·직원은 유지됩니다.
- 태그 전 확인: `windows-build.yml` 최신 run 의 T3 확장 행(ng1~ng3 이름검증 · tpl 템플릿 바이트 ·
  c03 preflight · alt_screen) green 증적 — 릴리스 시퀀싱 1단계입니다.
