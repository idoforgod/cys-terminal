# cys 0.14.53 — 릴리스 노트

## Windows 스마트 앱 컨트롤(SAC) · Windows Smart App Control

<!-- cys-win-sac: unsigned -->
**Windows 스마트 앱 컨트롤(SAC):** 이 판의 cys 설치 파일과 cys 실행파일은 코드 서명이 없습니다. 스마트 앱 컨트롤이 켜진 PC에서는 실행이 막힐 수 있으며, 이 판은 스마트 앱 컨트롤 적용 PC에서 검증하지 않았습니다.
**Windows Smart App Control:** this release's cys installer and cys executables are not code-signed. They may be blocked on PCs with Smart App Control on, and this release was not verified on a Smart App Control PC.

- 이 문단은 릴리스 본문과 앱 업데이트 정보(`latest.json` notes)에도 그대로 실린다. 공개 직전 검사(`scripts/release-verify.py` 8단계)가 이 고지와 설치 파일의 실제 서명 상태가 맞는지 확인하고, 다르면 공개를 막는다.
- 코드 서명은 오너의 서명 자격증명(조직 대상 Microsoft Artifact Signing 또는 신뢰 CA 인증서)이 마련되면 별도 설계로 켠다. 이 판에서 서명 모드는 `unsigned` 하나만 허용된다.

> 통합 기준: `2c8140937547ca3f42af9a157cca961753debc6e`. 승인된 v0.14.53 태그는 측정용 **DRAFT 릴리스**를 만들기 위한 것이며, 태그나 DRAFT 생성은 공개가 아닙니다. 공개는 별도의 오너 관문을 거칩니다.

**한 줄씩** (상태는 위 기준 시각)
- **들어간 것 [IN]**: ① 부서 개수 기본 상한 없음 · 노드 수만의 자원 경고가 팀 만들기를 막지 않음(WP1) ② CSO 자신의 컨텍스트 임계 알림이 master 에게 감
  (WP5) ③ 릴리스 도구: Gatekeeper 사용자 경로 검사가 GUI 확인 창에 갇히지 않음(WP3 · 내부 도구) ④ 윈도우: 설치했는데 경로를 못 찾던 claude CLI 를
  공식 설치 위치에서 찾음(WP8 · 오너 지시) ⑤ 기계가 보낸 지침 확인 핑이 오너 임무로 기록되던 결함 수리(WP9) ⑥ 인입 감사가 「읽을 수 없음」을
  「바뀜」과 구별(WP10 · 내부 도구) ⑦ 탭 점: 일하면 채운 원 + 깜박임, 대기는 빈 원, 죽으면 회색 X(WP11 · 오너 지시) ⑧ `new-surface` 로 만든 창을
  만든 좌석이 닫을 수 있음(WP16 · 오너 F6) ⑨ 윈도우 스마트 앱 컨트롤: 서명 없음을 밝히는 문장과, 그 문장이 없거나 설치 파일 서명 상태와 다르면
  공개를 막는 검사(WP12 slice A · 오너 요청 3).
- **추가로 들어간 것 [IN]**: ⑩ agy 상태줄 마이그레이션·실패 창 기록·빈 셸 지침 주입 차단(WP15, `d0a4f626`) ⑪ 슬래시 명령 단건 배달과 인식된 입력 상자의 초안 보호(WP17, `9f2d1dca`) ⑫ 대기 중 재주입 기록의 영속·동시 갱신 보호(WP6, `cfc10dde`) ⑬ 시험 데몬 정리(WP2, `dcbf659a`, 사용자 앱 동작 변화 없음). 출처는 각 §1 항목에 적었습니다.
- **들어간 것 [IN]**: WP13 탭 제목 번호·`cys set-title`·중복 역할 표시, `9e963979` → 통합 `2c589e7e`; 17:19:47 +0900 push. 출처: `push-out.txt, sha256 1083a2c9fb601bfa84535a257ad13a7ae5aac8f8bf857c8ae1bdb6066f08ae7e, 332 bytes; internal evidence, not published`·`push-time.txt`, `integ/INTEGRATION-LOG-WP13.txt`의 ACCEPTANCE ADDENDUM.
- **들어간 것 [IN · 수용·push 완료]**: WP18 스킬 검사 병렬화·실패 표시·시간 초과 정리, 원본 `df355061`; 통합 pick과 근거·잔여는 §1-16.
- **빠진 것 [OUT]**: WP7 홈페이지 전환 도구(B-456 · 홈페이지 전환 도구는 이번 판에서 제외) · 윈도우 코드 서명(오너 요청 1 · 서명 자격 없음) ·
  agy 남은 한도를 사용률로 표시(오너 F4 · 재현 표본 없음) · PowerShell 에 Ctrl+C 전달(오너 F5 뒷부분 · B-459) · 팩 소실의 전면 재설계(B-458) ·
  업데이트 때 팩 소실 완화(WP14L · 오너 F2 · 설계 한 회차 미수용 · B-462) · phoenix 배포 경로의 실행 확인(B-457).
- **재지 못한 것**: 윈도우 실기 전부 · 스마트 앱 컨트롤 적용 PC · **0.14.52 설치본에서 0.14.53 으로의 윈도우 업데이트 경로**(측정이 baseline 에서 멈춤 · 깨졌다는 뜻이 아니라 재지 못했다는 뜻) · 고정 통합 커밋 `319c2171` 위의 최종 전체 시험 묶음(§7) — §4.

## 1. 들어간 것

### 1-1. [IN] WP1 — 부서 개수에 기본 상한이 없습니다 · 노드 수만의 자원 경고는 팀 만들기를 막지 않습니다 (B-430 · B-431, `0b2027d7`)
- **왜**: 오너 지시(2026-10-22, CEO 전달) — 「부서 만들기 무한으로 하고, 제약을 해제해. 내가 자원보고 조절할 것이다.」(`0b2027d7`; 앞선 초안 §1-1)
- **부서 상한**: `cys-dept` 의 할당·생성에서 `CYS_DEPT_CAP` 의 기본값이 「무제한」입니다. 0 보다 큰 값을 줄 때만 상한이 걸리고, 그때는 살아 있는
  부서 수가 그 값 이상이면 종전처럼 exit 8 로 거절합니다. 미설정·빈 값·0·음수는 무제한입니다(빈 값은 종전에 비정상 종료였습니다)(`0b2027d7`).
- **팀 만들기·팀 부트**: 자원 관문의 하드 초과가 **전부 「노드 수」** 일 때는 stderr 한 줄을 남기고 진행합니다. 서버 수 · 함대 CPU 비율 · 컨텍스트 ·
  팀 만들기 예산은 종전대로 막습니다. `CYS_FORMATION_NODES_HARD=1` 이면 종전 차단으로 돌아갑니다(`0b2027d7`).
- **이번 판에서 미룬 한 줄**: 지침(`MASTER_DIRECTIVE.md` · `CEO_TEMPLATE.md`)의 「생성 실패」 표 한 줄은 0.14.52 바이트 그대로이고 「팀 수 상한 8」이라고
  적혀 있습니다. CEO 결정(2026-10-23 E1 (c))으로 미뤘습니다(`83d29ea5` · `_evidence/dept5-0.14.53-release-integ-20261023/RULING-C-NOTE.md`).
- **리뷰·수렴**: codex · gemini r1 ACCEPT(`_reviews/WP1-DEPTCAP-0.14.53-r1-{codex,gemini}.json`, 고정 `0b2027d7`) · gate-status 네 ✓.
- **검증(커밋 설명에서)**: 뮤턴트 19 개 중 19 개 사살, 대조군 초록. 미시험: Python 3.12(CI 인터프리터) · 윈도우 · 업데이트 뒤에도 0.14.52 바이트로
  남은 부서 팩(`0b2027d7`).

- **개정 4 [IN] 상태 근거**: `0b2027d7` · `integ/INTEGRATION-LOG.txt` + `_reviews/WP1-DEPTCAP-0.14.53-r1-{codex,gemini}.json`.

### 1-2. [IN] WP5 — CSO 자신의 컨텍스트 임계 알림이 master 에게 갑니다 (B-439, `7e90f5e7` · `76b9ee63`)
- **문제였던 것**: CSO **자신의** 임계는 세 군데에서 버려져 아무에게도 닿지 않았습니다(CSO 는 스스로 clear 할 수 없고, master 좌석은 `cys events` 를
  읽지 않습니다)(`7e90f5e7`).
- **이제**: CSO 좌석의 임계는 master 좌석 큐로 `[alert] …` 한 줄이 가고, 「master 가 `cys cycle-agent --role <그 CSO 역할> --fire <발화 id>` 로 clear
  하라」는 문장이 붙습니다. 닿지 못하면 overflow 원장(`alert-route-refused.jsonl`)에 남기고 이벤트를 냅니다. 같은 발화가 아직 배달되지 않은 동안
  다시 오면 두 번째 줄을 쌓지 않습니다(`7e90f5e7`, `76b9ee63`).
- **리뷰·수렴**: r2 codex · gemini ACCEPT(`_reviews/WP5-CSOSELF-0.14.53-r2-*.json`, 고정 `c4c42d39` — 통합 가지의 `76b9ee63` 은 patch-id 동일 ·
  `integ/INTEGRATION-LOG.txt`). 범위 한정 B-442 는 Known limits.
- **검증(커밋 설명과 측정에서)**: 경보 라우터 시험 169/0 · 데몬 전체 2834/0/13 · 뮤턴트 9 개 전부 빨강(`76b9ee63`).

- **개정 4 [IN] 상태 근거**: `c4c42d39` / `76b9ee63` · `integ/INTEGRATION-LOG.txt` + `_reviews/WP5-CSOSELF-0.14.53-r2-{codex,gemini}.json`.

### 1-3. [IN] WP3 — 릴리스 도구: Gatekeeper 사용자 경로 검사가 GUI 확인 창에 무한히 갇히지 않습니다 (B-437, `d68dd18c` · `19aeacc8` · `0f3d73b7` · `9e987a8d`) — 내부 도구
- **사용자가 받는 앱은 바뀌지 않습니다.** 이것은 릴리스 담당이 맥에서 돌리는 `scripts/verify-gatekeeper-user-path.sh` 의 수리입니다.
- **문제였던 것**: 검역 표시를 붙인 시험 사본의 번들 python3 을 실행하는 단계에서 macOS 가 「처음 실행」 GUI 확인 창을 띄우고, 프로세스가 0 CPU 로 멈춘 채
  끝나지 않았습니다(2026-10-22 오너 맥 실측 · `_evidence/dept5-0.14.53-wp3-gkprompt-20261023/TICKET.md`).
- **이제**: 그 실행에 시간 상한을 두고, 넘으면 그 축을 「측정 못함(UNMEASURED)」으로 적고 exit 2 로 끝납니다(통과가 아님). 상한 안에 끝나면 종전 판정과
  같습니다. 정리 유예 동안 하위 프로세스 소유와 신호를 지킵니다(R3, `0f3d73b7`). CI 빨강을 고친 등록 한 줄(`9e987a8d`).
- **리뷰·수렴**: r3 codex · gemini ACCEPT(고정 `cc93b458`) · CI 빨강 수리 r1 ACCEPT ×2(고정 `502896e9`) · gate-status 네 ✓(`notes/raw/gate-status-snapshot.txt`).

- **개정 4 [IN] 상태 근거**: `502896e9` / `9e987a8d` · `INTEG-REPORT.md` UPDATE BUILD GO 6 + `_reviews/WP3-CIRED-0.14.53-r1-{codex,gemini}.json` (전용 통합 로그 dataless 대체 근거).

### 1-4. [IN] WP8 — 윈도우: 설치한 claude CLI 를 찾지 못해 실행하지 못하던 결함 (오너 지시, `62d1abc9` · `aba4505f` · `adfcb349`)
- **왜**: 오너 지시(2026-10-23) — 「윈도우 사용자 중에서 클로드 cli를 설치했지만, 경로를 찾지못해서 클로드 런칭을 하지 못하는 것에 대한 버그 수정도 함께 하라. 이 버그는 윈도우 사용자 일부에서만 나타난다.」(`dept-5/dept-ops/intake/OWNER-20261023-0.14.53-fast-release-win-claude-path.txt`)
- **원인**: 설치 확인과 실행기가 각자 다른 「CLI 가 있을 수 있는 곳」 목록을 들고 있었습니다. 확인 쪽은 공식 윈도우 설치기가 `claude.exe` 를 두는
  `%USERPROFILE%\.local\bin`(그 설치기는 PATH 에 추가하지 않음)과 cys 가 쓰는 `%LOCALAPPDATA%\cys-npm` 을 보지 않아, 그 사용자는 「설치 안 됨」으로
  판정되고 필수 claude 좌석이 만들어지지 않았습니다(`62d1abc9`).
- **이제**: 목록이 하나입니다 — 확인이 받아들이는 모든 폴더는 실행기가 좌석 PATH 에 넣는 폴더입니다. 기존 `where` 조회(1–2단계)는 바이트 그대로 먼저
  돌기 때문에, 지금 claude 를 찾는 기계는 같은 경로를 찾습니다. 「못 찾음」 안내는 찾아본 모든 곳을 적고 「이미 설치한 경우」를 먼저 안내합니다
  (`62d1abc9`). 좌석이 해석할 수 없는 상대 경로 후보는 버립니다(`adfcb349`).
- **영향 범위**: 윈도우 경로만 바뀝니다 — 맥 · 리눅스는 바이트가 같습니다. 윈도우 좌석 PATH 끝에 4 항목(145 B)이 붙습니다(`62d1abc9` Constraint ·
  Scope-risk). 확신도 medium(`62d1abc9`, `adfcb349`).
- **리뷰·수렴**: r2 gemini ACCEPT · codex r2 ESCALATE 뒤 r2b ACCEPT(고정 `65c449b7`) · gate-status 네 ✓.
- **검증**: 맥에서 `--lib` 791/0/1 · `--bin cys` 593/4/1(4 건은 기준판에도 있는 b360 시험) · `--bin cysd` 2826/1/13 두 번(매번 다른 간헐 시험 1 건) ·
  윈도우 대상 형식 검사 `cargo check --target x86_64-pc-windows-msvc --bin cys --lib` rc 0(`_evidence/dept5-0.14.53-wp8-winclaudepath-20261023/REPORT-R2-SLOT4.md` §1).
  **실제 윈도우에서는 한 번도 돌지 않았습니다**(같은 파일 §7 UNMEASURED-1).

- **개정 4 [IN] 상태 근거**: `65c449b7` · `integ/INTEGRATION-LOG-WP8-WP9.txt` + `_reviews/WP8-WINCLAUDEPATH-0.14.53-r2b-codex.json`.

### 1-5. [IN] WP9 — 기계가 보낸 지침 확인 핑이 오너 임무로 기록되던 결함 (B-443, `b69b232f` · `48dff99b` · `211e9527`)
- **문제였던 것**: 데몬이 좌석에 넣는 지침 확인 핑이 「오너 임무」로 기록되어, 임무 게이트가 함대가 스스로 쓴 글을 오너 지시로 인정했습니다(자기 승인).
  그날 아침 9 개 레인 중 6 개에 그 기록이 있었습니다(`b69b232f`).
- **원인**: CLI 가 본문을 붙여넣기 표지(bracketed paste)로 감싼 뒤 보내고, 데몬은 감싼 바이트로 배달을 기록하는데, 훅은 좌석에 실제로 제출된 본문으로
  대조했습니다 — 두 해시가 달라 기계 배달로 인식되지 않았습니다(`b69b232f`).
- **이제**: 같은 해시 비교 안에 「표지로 다시 감싼 형태」를 후보로 하나 더 넣었습니다. 해시 동치라 오너가 친 글을 기계 배달로 삼키지 않습니다. 파이썬 쪽
  (`javis_mission.py`)도 같은 규칙입니다(`b69b232f`). 미리보기 칸이 없는 배달도 접습니다(R2 F1, `211e9527`).
- **리뷰·수렴**: gemini r1 · codex r2 ACCEPT(고정 `47f262c7`) · gate-status 네 ✓.
- **검증**: `--lib` 786/0/1 · `--bin cysd` 2827/0/13 · `--bin cys` 589/4/1(4 건은 기준판의 b360)(`_evidence/dept5-0.14.53-wp9-missionping-20261023/REPORT.md`).
  윈도우 대상 형식 검사는 돌리지 않았습니다(같은 파일 「UNMEASURED here」).

- **개정 4 [IN] 상태 근거**: `47f262c7` · `integ/INTEGRATION-LOG-WP8-WP9.txt` + `_reviews/WP9-MISSIONPING-0.14.53-r2-codex.json`.

### 1-6. [IN] WP10 — 인입 감사가 「읽을 수 없음」을 「바뀜」과 구별합니다 (`664056e1` · `126fdc39`) — 내부 도구
- **문제였던 것**: `javis_intake.py audit` 가 크기는 그대로인데 읽을 수 없는 증거(macOS 의 dataless/압축)를 「증거 바이트가 변경됐다 … 재지불하라」로
  표시해, 읽지 못한 증거를 대신할 새 증거를 만들라고 지시했습니다(CEO 결함 보고 · 901 참조 중 24 개가 그 경우)(`664056e1`).
- **이제**: 읽기 결과가 넷(OK · 없음 · 보통 파일 아님 · 읽을 수 없음)으로 갈리고, 「읽을 수 없음」은 errno 와 플래그를 적되 지불을 무효로 하지 않으며
  처방을 내지 않습니다. 감사 종료 코드 8(`664056e1`).
- **이 판에서 바로 드러난 오보**: CEO 관찰(출처: CEO message to the dept-5 head, relayed — CEO 「[CEO 판정 개정] Ruling 1 AMENDED」, ~13:0x +0900 ·
  원문 파일 없음, master 가 파일을 주면 그 경로로 바꿉니다) — WP10 자신의 증거 행이 iCloud 가 파일을 비우면서(dataless)
  6 개 중 4 개로 떨어져 보였습니다. 그 오보(읽을 수 없는 증거를 「바뀜」으로 보고)를 고치는 수리가 이 판의 WP10 입니다. CEO 상시 지시 「NOBODY
  re-pays unreadable intake rows」(`_evidence/dept5-disk-hold-20261023/BUILD-QUEUE.md` 「CEO AMENDED RULING 1 (13:09 +0900 received)」 — 부서장 전사).
- **리뷰·수렴**: codex · gemini r1 ACCEPT(고정 `0f944f05`) · gate-status 네 ✓ · 통합 `integ/INTEGRATION-LOG-WP10.txt`(patch-id 동일 · diff cmp 0).
- **검증**: 그때의 통합 끝 커밋 `126fdc39`(12:05) 에서 `javis_intake.py selftest` rc 0 · `test_intake_exit_contract` 62/62(`checks-wp10/RUN.txt`). CI 의 Python 3.12 와
  윈도우는 재지 않았습니다(`_evidence/dept5-0.14.53-wp10-intake-unreadable-20261023/REPORT.md`).

- **개정 4 [IN] 상태 근거**: `0f944f05` · `integ/INTEGRATION-LOG-WP10.txt` + `_reviews/WP10-INTAKE-UNREADABLE-0.14.53-r1-{codex,gemini}.json`.

### 1-7. [IN] WP11 — 탭 점: 일하는 노드는 채운 원 + 깜박임, 대기 노드는 빈 원 + 멈춤, 죽은 노드는 회색 X (오너 지시, `cdbef079` · `affda1e5`)
- **왜**: 오너 지시(2026-10-23) — 「노드가 작업 중일 때는 탭의 점이 안이 채워진 원이 되고 동시에 깜박깜박거리도록 만들고, 작업을 하지 않고 대기 중일 때는 점이 안이 채워지지 않는 원이 되고, 동시에 깜박거리지 않고 멈춰있게 만들자.」(`dept-5/dept-ops/intake/OWNER-20261023-tab-dot-node-state.txt`)
- **원인(실측)**: 데몬의 claude 화면 문법이 창 제목이 「Claude Code」로 끝나야 한다고 요구했는데, Claude Code 2.1.309 는 제목을 `<기호> <세션 이름>` 으로
  씁니다. 그래서 모든 claude 좌석이 「알 수 없음」(빈 원 · 멈춤)으로 보였습니다(`_evidence/dept5-0.14.53-wp11-tabdot-20261023/REPORT.md` Summary).
- **바뀌는 것**: 문법 r2(`claude-2.1.309-r2`) — 2.1.309 채집본에서 일하는 표본 100 % 활성, 대기 100 % 대기, 권한 8/8. 화면은 오너의 세 모습과 그 밖의 상태별
  모습, 죽은 좌석의 회색 X. 점은 상태 표시일 뿐 알림·배지·소리를 내지 않습니다. 윈도우 고대비 모드에서도 모습이 남게 했습니다(같은 파일 T5).
- **지원 판 표기**: 0.14.52 노트의 「Claude Code 2.1.301」 지원 문구를 2.1.309 로 바꿔야 합니다(같은 파일 「Deferred to the head」 R1-12).
- **리뷰·수렴**: codex r1 ESCALATE 뒤 r1b ACCEPT · gemini r1 ACCEPT(12:55)(고정 `3ee59738`) · gate-status 네 ✓(`notes/raw/gate-status-snapshot-2.txt`) ·
  통합 `cdbef079`(patch-id 동일 · diff cmp 0) + 주석만 고친 `affda1e5`(`_evidence/dept5-0.14.53-release-integ-20261023/integ/INTEGRATION-LOG-WP11.txt`).
- **검증**: UI 2617/0 · 데몬 활동 필터 67/0/3 · 데몬 전체 2829/1/13 두 번(부하에 민감한 시험 1 건, 단독 통과)(같은 파일 T6).

- **개정 4 [IN] 상태 근거**: `3ee59738` · `integ/INTEGRATION-LOG-WP11.txt` + `_reviews/WP11-TABDOT-0.14.53-r1b-codex.json`.

### 1-8. [IN] WP13 — 탭 제목 앞 터미널 번호 · `cys set-title` · 같은 역할 중복 표시 (오너 F1, `9e963979`)
- **왜**: 오너 요청(2026-10-24, 항목 1) — 같은 역할 좌석 둘(`reviewer-codex-codex · cys`)을 화면에서 구별할 수 없어 닫을 창을 찾지 못함
  (`dept-5/dept-ops/intake/OWNER-20261024-seven-fixes.txt`).
- **바뀌는 것**: 탭이 `[381] reviewer-codex-codex · cys` 처럼 번호로 시작합니다. `cys set-title --surface <번호> "<제목>"` 이 생기고 `""` 는 자동 제목으로
  돌립니다. 같은 역할의 살아 있는 좌석이 둘 이상이면 제목 뒤에 ` (2)` 를 붙입니다(`_evidence/dept5-0.14.53-wp13-tabtitle-20261024/REPORT.md` §1).
- **알아 둘 것**: 바꾼 제목은 앱·데몬을 다시 시작하면 기본 제목으로 돌아갑니다(B-455 · Known limits). `--to <역할>` 은 중복 역할 중 하나만 고르므로
  둘 이상이면 `--surface <번호>` 를 쓰십시오(같은 파일 §7).
- **상태 [IN]**: `9e963979`이 17:19:47 +0900 `fix/0.14.53-tabtitle`로 push되고 `2c589e7e`에 통합됐습니다. 출처: `push-out.txt, sha256 1083a2c9fb601bfa84535a257ad13a7ae5aac8f8bf857c8ae1bdb6066f08ae7e, 332 bytes; internal evidence, not published`·`push-time.txt`, `integ/INTEGRATION-LOG-WP13.txt`의 ACCEPTANCE ADDENDUM. 첫 pick은 patch-id가 다르지만 WP11 문맥 줄 차이이며 변경 줄은 같다는 부서장 수용 기록입니다. 최종 통합 시험 통과를 뜻하지 않습니다.
- **검증 범위**: codex r1이 확인한 로컬 결과는 clean-env CLI 593/0/1, lib 788/0/1, cysd 2828/0/13, UI 2620/0입니다. 예전 CLI 589/4는 기준판에서도 재현되고 rig HOME에서는 통과한 시험 환경 문제로 기록돼 있습니다. 이번 문서 작업에서 재실행하지 않았습니다. 제목 비영속성과 Windows 미측정은 Known limits에 유지합니다.

### 1-9. WP12 — 윈도우 스마트 앱 컨트롤: 서명 상태 명시와 그 검사 (요청 3 · slice A [IN], `e5f49fbb` · `8b59ef75`) · 업데이트 전 새 실행파일 실행 확인 (요청 2 · [OUT])
- **왜**: 오너 전달 보고(2026-10-23) — 스마트 앱 컨트롤이 켜진 윈도우 11 PC 에서 v0.14.52 의 서명 없는 `cys.exe`·`cysd.exe` 가 막혔습니다(CodeIntegrity
  3077 · 3033). 요청 1 코드 서명 · 요청 2 교체 전 실행 확인 · 요청 3 릴리스 노트 명시(`dept-5/dept-ops/intake/OWNER-20261023-win-sac-codesign.txt`).
- **요청 1(코드 서명)은 이 판에 없습니다** — 오너가 가진 서명 자격이 필요하고 아직 없습니다. 0.14.53 은 서명 없이 나갑니다(부서장 판정 0 ·
  `_evidence/dept5-0.14.53-wp12-winsac-20261023/HEAD-RULINGS-DESIGN-R1.md`).
- **요청 2는 [OUT]입니다. 0.14.53에는 교체 전 실행 확인이 없습니다 — 설치기와 앱/셸 경로 모두 제외입니다. Windows 빌드는 서명 없이 출하하며 Smart App Control 동작은 UNMEASURED(검증하지 못함)입니다.** B1 `de2ee33f`·B2 `c0db2658`의 제외 근거는 아래와 같습니다. 다음은 미출하 설계 이력입니다: 설치기가 옛 파일을 지우거나 바꾸기 전에 새 실행파일을 짧게 실행하고 실패하면 옛 설치를 그대로 두는
  설계입니다(`HEAD-RULINGS-DESIGN-R1.md` 판정 1). 설계 1회차 codex BLOCK · gemini REVISE · 설계 2회차 gemini ACCEPT(13:05) · codex REVISE(13:04, 차단 0 ·
  주요 3 · 사소 2)(`_reviews/WP12-WINSAC-DESIGN-0.14.53-r{1,2}-*.json`).
- **부서장 판정(설계 2회차 뒤, `_evidence/dept5-0.14.53-wp12-winsac-20261023/HEAD-RULING-DESIGN-R2.md` sha256-16 `a67f8a03`, 13:09:35 +0900)**: 판정 8 을
  고쳐 구현 단계 B 로 갑니다. **요청 3(문장)과 D6(그 문장이 없으면 빨개지는 릴리스 검사)을 먼저, 따로 낼 수 있는 커밋으로** 내고, 요청 2 는 그 뒤입니다.
  **요청 2 가 22:00 +0900 까지 네 자 수렴하지 못하면 0.14.53 은 요청 3 + D6 만 내고 요청 2 는 다음 판으로 갑니다**(릴리스 노트가 그렇게 말함).
  이것은 설계 단계의 결정 기록이며, 현재 구현 상태는 아래 B1/B2 항목입니다.
- **[OUT] B1 설치기 교체 전 실행 확인**, 미출하 `de2ee33f`, B-483: Windows CI `40085224043` win-update-probe는 NOT GREEN입니다. 정책 준비 중 AllowAll.xml 접근 거부; SUMMARY 8 checks/1 failed/1 unmeasured, W1 3/3 PASS지만 CTL은 0/5로 실행되지 않았습니다. ci-branch `40085224060`도 C8의 `actions/download-artifact@v4` 허용 목록 검사에서 실패했습니다. 출처: `JUDGMENT.md, sha256 f72a733f954cabc1096af8cb20e21c60239b9e238c4fbfcd7582f1e3396f47e2, 6643 bytes; internal evidence, not published` 및 `TICKET-NOTES-REV5a-w13.md, sha256 aaecff97b043500017f014b36a2cd3bab2a526ab67d03e9c122008ee9e1a9289, 7131 bytes; internal evidence, not published`. W1 성공을 정상 설치 진행·차단 정책 거부·전체 보호 성공으로 확대하지 않습니다.
- **[OUT] B2 앱/셸 교체 전 실행 확인**, 미출하 `c0db2658`, B-482: r3 codex-2 REVISE, gemini ACCEPT이나 세 회차 상한으로 제외됐습니다. 남은 차단 2건은 부서가 없는 정상 본부 재시작도 무관한 부서 실행파일 검사로 거부하는 경로, 줄바꿈 없는 셸 시작 문구가 정상 경로 레코드를 가리는 경로입니다. 정적 소스 도출이며 Windows 실기 장애 측정 주장이 아닙니다. 출처: `PARKED.txt, sha256 e582db27a278c5b9ab706a4b88677f6791d69e05ec02941066a574ea22c3c183, 1941 bytes; internal evidence, not published` 및 `verdicts/WP12-WINSAC-B2-0.14.53-r3-codex.json`.
- **slice A [IN] (요청 3 + D6)**: 리뷰 1회차 codex · gemini REVISE(고정 `fd2044f7`), 2회차 둘 다 ACCEPT(고정 `4ee6586b`)
  (`_reviews/WP12-WINSAC-A-0.14.53-r{1,2}-*.json`) · gate-status 네 ✓(14:47:50 +0900, `notes/raw/gate-status-WP12A.txt`) · 통합
  `integ/INTEGRATION-LOG-WP12A.txt`(`fd2044f7` → `e5f49fbb`, `4ee6586b` → `8b59ef75`, patch-id 동일). 바뀌는 것: 릴리스 본문과 `latest.json` notes 에
  서명 없음 문장을 싣고(`release.yml:1126-1131`), 공개 직전 검사(`scripts/release-verify.py` 8단계)가 그 문장과 설치 파일의 실제 서명 상태를 대조해
  다르면 공개를 막습니다. 자기 Windows 실행파일 전부를 서명 조사에 넣습니다(`8b59ef75` 「require every own Windows binary in the signature census」).
  `docs/RELEASE_NOTES_0.14.53.md` 를 만듭니다 — 이 초안이 들어갈 파일입니다.
- **phoenix 배포 경로도 교체 전 실행 확인이 없습니다(B-457)**: 조용한 업데이트·설치 마법사·먼저 제거·본부/부서 데몬 교체도 이번 판에는 요청 2의 확인을 받지 않습니다. 미출하 설계의 대상 범위와 출하 보호를 구별합니다(§1-9 B1/B2 출처).
- **요청 3(명시)**: 문장은 slice A 가 만든 `docs/RELEASE_NOTES_0.14.53.md` 머리의 스마트 앱 컨트롤 절입니다(5-7 행 · 릴리스 본문과 같은 바이트). 이
  초안에는 다시 적지 않습니다 — 공개 직전 검사는 표지 문자열이 두 번 나오면 막습니다(Known limits 첫 항목).

- **개정 4 출처 확인**: slice A `4ee6586b`의 두 r2 ACCEPT와 `notes/raw/gate-status-WP12A.txt` 네 체크를 읽었습니다. WP15의 resident `integ/INTEGRATION-LOG-WP15.txt`는 통합 시작점을 `8b59ef75`로 기록합니다. WP12A 전용 통합 로그는 dataless라 열지 않았습니다.

### 1-10. [OUT] WP14L — 업데이트 때 팩(`~/.cys/pack`)이 불완전본으로 바뀌던 결함의 완화 (오너 F2) — 설계 미수용(B-462) · 전면 재설계도 [OUT](B-458)
- **왜**: 오너 요청(2026-10-24, 항목 2) — 0.14.52 적용 뒤 재시작하자 팩이 800 개 파일짜리 불완전본(`.pack-version` 0.14.51)으로 바뀌고 완전본은
  `~/.cys/pack.prev` 로 옮겨졌습니다. round 기록 · soul.md · schedule.json(정기 잡 72 개) · skills · memory 가 사라졌습니다. 0.14.50 때도 같은 일이
  있었습니다(`dept-5/dept-ops/intake/OWNER-20261024-seven-fixes.txt`).
- **전면 재설계는 이 판에 없습니다(B-458)**: 모든 설치기를 기록되는 한 명의 쓰기 주체로 묶는 설계(`DESIGN-R2.md`, 파일 sha256-16 `1c10e8f80b9c155d`)는 설계 2회차에서 codex 가 차단 2 ·
  주요 3 으로 REVISE 했고(gemini 는 ACCEPT), 판정 12 대로 다음 판으로 갑니다(`_evidence/dept5-0.14.53-wp14-packloss-20261024/HEAD-RULING-DESIGN-R2.md`,
  13:03:00 +0900 · `LEDGER.md` B-458 행).
- **들어가지 못한 작은 작업 WP14L(완화)이 하려던 것**: 측정된 소실 기제(시험 자리에서 살아 있는 팩 953 → 22 파일, 영구 소실)를 설치기 재설계 없이, 데몬 부트 경로를
  건드리지 않고 없앱니다 — L1 살아 있는 팩의 이름 바꾸기가 거절되면 「복사 후 지우기」로 넘어가지 않는다(팩은 그대로, 설치는 이름 붙은 오류와 0 이 아닌
  종료로 실패하고 앱이 보여 준다) · L2 새 팩이 자리 잡기 전에는 `pack.prev` 를 지우지 않는다 · L3 버전 가드가 모든 쓰기를 막았으면 교체하지 않는다 ·
  L4 그 밖에는 바꾸지 않는다(같은 파일).
- **결정(13:26:01 +0900)**: 설계 한 회차(`DESIGN-L.md`, 14,053 B)에서 gemini ACCEPT, codex REVISE — 차단 3 · 주요 2 · 사소 1(L-F1~L-F6:
  보통의 업데이트 실패가 여전히 좌석 복원을 막음 · 백업 가드 g1~g3 가 비켜 둔 사본에 사용자 고유 파일이 없음을 증명하지 못함 · 파일 개수 가드가 정상
  업데이트를 거절할 수 있음 등)(`_reviews/WP14L-PACKLOSS-MITIGATION-DESIGN-0.14.53-r1-codex.json`). 부서장 판정: **설계 수용 안 됨 → WP14L 은 0.14.53 에
  없습니다.** 0.14.53 은 오너 F2 문장 「pack loss on update not fixed」로 나갑니다. codex 지적 L-F1~L-F6 은 B-458 에 붙여 B-462 로 남깁니다. 구현은
  시작하지 않았습니다(`_evidence/dept5-0.14.53-wp14-packloss-20261024/HEAD-RULING-DESIGN-L.md`, sha256-16 `9117766a65736d3c` ·
  `dept-5/dept-ops/next-release-backlog/LEDGER.md` B-462 행). Known limits 의 팩 소실 항목이 그 문장입니다.

- **개정 4 상태 확인 [OUT]**: 출하 수정 커밋 없음(기준판 `f0ac3b0c`). WP14/B-458은 master 티켓과 `_reviews/WP14-PACKLOSS-DESIGN-0.14.53-r2-codex.json`, WP14L/B-462는 resident `HEAD-RULING-DESIGN-L.md`로 확인했습니다. 전면 설계 R2 판정 파일은 dataless라 열지 않았습니다. F2는 NOT FIXED로 유지합니다.

### 1-11. [IN] WP15 — 윈도우: agy 상태줄 명령 따옴표 · 실패한 좌석 화면 기록 · 빈 셸에 지침 입력 차단 (오너 F3 · F5, `d0a4f626`)
- **왜**: 오너 요청(2026-10-24, 항목 3 · 5)(`OWNER-20261024-seven-fixes.txt`).
- **바뀌는 것(부서장 판정 1)**: ⓐ cys 가 써 둔 옛 agy 상태줄 명령(`cys-agy-statusline.sh` + `--cys-autolink`)만 오늘의 생성 형식으로 고칩니다 — 다른
  명령은 건드리지 않습니다. ⓑ `launch-agent` 가 실패한 창을 닫기 전에 그 화면을 파일로 남기고 경로를 알립니다(쓰기 실패면 화면을 stderr 로 내고 종전대로
  닫음). ⓒ 마지막 화면이 PowerShell 프롬프트나 `>>` 이어쓰기면 빈 셸로 보고 지침을 넣지 않습니다
  (`_evidence/dept5-0.14.53-wp15-winlaunch-20261024/HEAD-RULING-1.md`, `RULING-1-PLAN.md`).
- **빠지는 것**: ⓓ PowerShell 에 Ctrl+C 전달 — 이 판에서 제품 변경 없음(같은 파일 판정 (d) · B-459: Ctrl+C 바이트(ETX 0x03)는 이미 보내고 있고,
  윈도우에서 빠져나오지 못하는 원인은 재지 못함 · `LEDGER.md` B-459 행). agy 1.4.6 실물은 재지 않았습니다(이 맥의 agy 는 1.4.5).
- **상태 [IN]**: `d0a4f626` · `_reviews/WP15-WINLAUNCH-0.14.53-r2-{codex,gemini}.json` 둘 다 ACCEPT. `push/ls-remote-after.txt`에서 `fix/0.14.53-winlaunch`와 같은 SHA를 확인했습니다. `integ/INTEGRATION-LOG-WP15.txt`가 4자 수렴과 통합 `ced9c78e`(patch-id 동일)를 기록합니다.
- **push 시각의 출처**: `15:49:20 +0900`은 master 티켓 값입니다. `push/push-time.txt`와 `push/push-out.txt`가 dataless라 그 초 단위 시각을 독립 재확인하지 않았습니다. 통합 로그의 원격 참조 관측은 15:50:10입니다.
- **공백 있는 Windows 경로는 지원하지 않습니다(B-461)**: pack/home 경로에 공백이 있으면 상태줄 명령을 새로 쓰지 않고 기존 설정을 그대로 둔 채 이유와 수동 조치 안내를 표시합니다. `_evidence/dept5-0.14.53-wp15-winlaunch-20261024/HEAD-RULING-R1-SPACES.md`와 두 r2 ACCEPT의 범위입니다. 실제 agy 1.4.6/ConPTY 동작은 UNMEASURED입니다.

### 1-12. [IN] WP16 — `new-surface` 로 만든 창을 만든 좌석이 닫을 수 있습니다 (오너 F6, `6f477de5` → 통합 `7c605e0f`) · agy 한도 표시(F4)는 고치지 않았습니다
- **F6 바뀌는 것**: 서버가 확인한 생성자 신원을 창마다 기록하고, 자기 창 또는 그 창을 만든 좌석이면 닫을 수 있습니다(요청 본문의 신원은 받지 않음)
  (`_evidence/dept5-0.14.53-wp16-agyquota-close-20261024/DESIGN.md` F6 · 커밋 `6f477de5` 「allow creators to close their own surfaces」).
- **리뷰·수렴**: codex r1 ESCALATE — 제품 결함은 없고 이름 붙은 뮤턴트 실행 증거가 빠짐(minor) · 제출자 전체 시험 4201/0/15(`_reviews/WP16-AGYQUOTA-CLOSE-0.14.53-r1-codex.json`
  justification). 그 뒤 codex r1b ACCEPT(`_reviews/WP16-AGYQUOTA-CLOSE-0.14.53-r1b-codex.json`, sha256-16 `06b5926b3683d76c`) · gemini r1 ACCEPT
  (`_reviews/WP16-AGYQUOTA-CLOSE-0.14.53-r1-gemini.json`, sha256-16 `d701e9d8ebaa411d`) — 둘 다 고정 `6f477de5` · gate-status 네 ✓(13:38:14 +0900,
  `notes/raw/gate-status-WP16.txt`; 종료 코드 4 = 수렴했으나 임무 미지정, 체크리스트 C1 읽는 법) · 이름 붙은 뮤턴트 표
  `_evidence/dept5-0.14.53-wp16-agyquota-close-20261024/MUTANTS.md`(sha256-16 `541999dc16bab172`). 가지 `fix/0.14.53-agyquota-close`(master: `6f477de5` push 됨).
- **통합**: `cherry-pick -x 6f477de5` → `7c605e0f`, 충돌 0, patch-id 동일, diff cmp 0(33,939 B), cysd 3 파일(governance.rs · handlers.rs · state.rs)
  (`integ/INTEGRATION-LOG-WP16.txt`). 통합 위에서는 빌드·시험을 돌리지 않았습니다(같은 파일 · `notes/INTEG-TIP-RUNBOOK.md` 가 돌릴 순서).
- **윈도우 SAC·서명(오너 상시 점검)**: 출하 Windows 파일 이름·종류와 updater 교체 경로를 추가/변경하지 않습니다. 재빌드 바이너리의 서명·공개 검사는 WP12 게이트로 별도 확인해야 합니다
  (`integ/INTEGRATION-LOG-WP16.txt`). 윈도우의 생성자 판별(파이프 상대 pid + 부모 pid 조상 걷기)은 실기에서 돌지 않았습니다(같은 파일 · 커밋 trailer Not-tested).
- **F4(agy 남은 한도를 사용률로 표시) — 이 판에 없습니다**: 우리 코드는 이미 남은 비율을 사용률로 바꾸고 있고(이 맥은 agy 1.4.5), 1.4.6 의 실제 값을
  재현할 표본이 없어 추측한 두 번째 뒤집기를 내보내지 않습니다(`dept-5/dept-ops/ceo-line/sent/20261024-1147-agy146-sample-request.md`). Known limits.

- **개정 4 [IN] 상태 근거**: `6f477de5` / `7c605e0f` · `notes/raw/gate-status-WP16.txt` + `_reviews/WP16-AGYQUOTA-CLOSE-0.14.53-r1b-codex.json` + resident `integ/INTEGRATION-LOG-WP17.txt`의 WP16 합성 근거 (WP16 전용 통합 로그 dataless).

### 1-13. [IN] WP17 — 자동 시작 슬래시 명령이 일반 글 묶음으로 배달되던 결함 (오너 F7, `9f2d1dca`)
- **바뀌는 것**: 명령 모양(`/` + 영문자로 시작)의 글은 묶음(다이제스트)에 넣지 않고 한 번에 하나씩 제출합니다. 묶음은 그 앞에서 멈춥니다
  (`_evidence/dept5-0.14.53-wp17-slashcmd-20261024/REPORT.md` 「Behavior and scope」).
- **입력 상자 보호**: 인식된 테두리 안에서 커서 행 위나 아래에 초안이 있으면 기존 input_pending 사유로 pre-write 기록 전에 거절합니다. 상자를 비우면 같은 항목이 진행합니다. 마커 없는 좌석·알 수 없는 힌트/레이아웃은 기존 메시지 관문과 진단으로 진행하며 새 무한 보류를 만들지 않습니다. 여러 줄 본문은 한 항목·한 제출로 보존합니다(`REPORT-R2.md`).
- **상태 [IN]**: `9f2d1dca` · gemini r2 ACCEPT, codex r2b ACCEPT(`_reviews/WP17-SLASHCMD-0.14.53-r2-gemini.json`, `...-r2b-codex.json`). `push/push-time.txt` 15:54:52 +0900, `push/push-out.txt`의 `fix/0.14.53-slashcmd`, `integ/INTEGRATION-LOG-WP17.txt`의 4자 수렴/통합 `2be1c383`(patch-id 동일)을 확인했습니다.
- **검증 한계**: 로컬 targeted 12/12와 이름 붙은 변이 7/7 사살. 원래 전체 cysd의 C8 Locked 빨강은 원인 UNMEASURED로 보존하며 최종 통합 관문은 `INTEGRATION-LOG-WP17.txt`의 S4 인용을 따릅니다. Windows와 실제 어댑터 명령 효과는 UNMEASURED입니다.

### 1-14. [IN] WP6 — 대기 중 지침 재주입의 영속·동시 갱신 보호 (B-428 · B-446, `cfc10dde` → 통합 `2d218d33`)
- **바뀌는 것**: 재주입 결과가 UNKNOWN일 수 있는 입력보다 먼저 영속 의도를 남기고, 영속 실패를 성공 종료로 숨기지 않습니다. 모든 레인이 공유하는 대기 파일 잠금(`~/.cys/.pack-reinject-pending.lock`)으로 오래된 pass가 다른 pack-update의 UNKNOWN 기록을 지우지 못하게 합니다. 실제 잠금을 얻은 File만 성공 토큰이 됩니다.
- **거절과 재시도**: 다른 갱신이 잠금을 120초 동안 쥐고 있거나 잠금 파일을 만들/열 수 없으면 `pending-lock-refused`와 exit 1로 거절하고 손대지 않은 항목을 이름으로 알립니다. S1 pending pass 거절은 fetch/apply 전에 끝납니다. S2 apply 단계 거절은 그 단계가 적용/재주입하지 않았다는 뜻이며, 같은 호출에서 먼저 실행된 S1 결과는 별도로 보고합니다. 무한 대기가 아닙니다. 출처: `_evidence/dept5-0.14.53-wp6-b428-20261023/r3-fix/REPORT-R3.md` §1·§2·§9, 같은 폴더의 `../r2-fix/REPORT-R2.md` §2(최대 두 번의 120초 대기).
- **상태 [IN]**: `cfc10dde` · `_reviews/WP6-B428-0.14.53-r3-{codex,gemini}.json` 모두 ACCEPT. `push/push-time.txt` 16:26:56 +0900, `push/push-out.txt`의 `fix/0.14.53-b428`, `integ/INTEGRATION-LOG-WP6.txt`의 4자 수렴/통합 `2d218d33`(다섯 커밋 patch-id 동일)을 확인했습니다. Machine 118/0/1은 WP6 가지의 결과이지 통합 끝 커밋의 결과가 아닙니다.
- **한계 유지**: Windows File::try_lock/LockFileEx 및 ACL 거부 동작은 UNMEASURED이며 행동 시험은 cfg(unix)입니다. 기존 읽기 전용 lock 파일의 잠금이 OS에서 실패해도 보수적으로 거절합니다. GUI와 CLI의 최종 통합 검증은 별도입니다.

### 1-15. [IN] WP2 — 데몬 시험 정리 (B-433, `dcbf659a`) — 내부 · 사용자 앱 동작 변화 없음
- **무엇**: 데몬 시험 일부가 상태 폴더를 지운 뒤에도 그 시험 데몬의 큐 저장 틱이 살아 있어, 릴리스 CI 로그에 `[queue] 영속 중단` 소음이 찍혔습니다. 시험이
  데몬을 먼저 멈추게 했습니다(`_evidence/dept5-0.14.53-wp2-wp5hygiene-20261023/REPORT.md` §2).
- **제품 영향**: 제품 코드에 들어간 줄은 그 메시지를 조용히 할지 묻는 함수 호출 하나이고, 시험이 아닌 빌드에서 그 함수는 늘 `false` 입니다 — 출하 바이너리의
  동작은 같습니다(`git show 3324a525:src/bin/cysd/state.rs` 3341–3349).
- **상태 [IN]**: `dcbf659a` · codex r2b와 gemini r2 ACCEPT(`_reviews/WP2-WP5HYGIENE-0.14.53-r2b-codex.json`, `...-r2-gemini.json`). Resident machine cysd 2828/0/13은 해당 WP 가지의 측정입니다. `push/push-time.txt` 17:00:59 +0900, `push/push-out.txt`의 `fix/0.14.53-wp5hygiene`, `integ/INTEGRATION-LOG-WP2.txt`의 4자 수렴/통합 `319c2171`(patch-id 동일)을 확인했습니다. 내부 시험 정리이며 사용자 앱의 동작을 바꾸는 항목이 아닙니다.

### 1-16. [IN · 수용·push 완료] WP18 — 스킬 검사 병렬화·실패 표시·시간 초과 정리
- **수용된 원본**: `df355061f9fd5382aca109a9a409b7e26ea5818e` (`1992a358` 위 R3, 두 파일 +135/-29). **통합 pick SHA: `2c8140937547ca3f42af9a157cca961753debc6e`**. 부서장 실측 판정은 네 자 CONVERGED: reviewer-codex ACCEPT 0/0/0, reviewer-gemini ACCEPT 0/0/0, machine PASS, master ACCEPT입니다. 부서장은 후보 52/52 GREEN과 부모판 13 FAIL도 재실행했습니다. 이는 생산자의 최초 RED 10건과 별도 측정입니다. 출처: master의 이번 수용·push 통보(신뢰도 High, 부서장 측정으로 귀속); 생산자 원본은 `REPORT-R3.md, sha256 9c37efbbc32925f9e40991fab73f640a9c730c531c9ce65cf02bd8dae6bc80bf, 5287 bytes; internal evidence, not published`.
- **push 확인**: 2026-10-25 **06:58:33 +0900**, `df355061` → `fix/0.14.53-skillscan`; ref census **558→559**, 새 ref 하나입니다. `wp18-df355061 push records — wp18-df355061-push-records.manifest.json, sha256 b10d8d5e7bb61f73feb9c72f0381647779b7286a136ae14b294efd98b62558b7, 704 bytes; internal evidence, not published`의 `push-out.txt`, `push-time.txt`, `ls-remote-before.txt`, `ls-remote-after.txt`, `census-diff.txt`를 직접 대조했습니다. 신뢰도 High. worker-4가 `2dda5a69` 위에 8개 pick으로 통합했습니다. **8/8 patch-id EQUAL, 충돌 0**이며 실제 통합 tree와 검증된 dry-run tree가 모두 `02633fa3c9e4c8f6e255ebe864e8e88f33800f3f`로 같습니다. 부서장 직접 실측 통보를 근거로 하며, 이 문안 작업에서도 `2c8140937547ca3f42af9a157cca961753debc6e`의 tree 해시를 읽어 일치함을 확인했습니다. 최종 시험·공개 성공을 뜻하지 않습니다.
- **바뀌는 동작**: 스킬 검사를 여러 프로세스로 나눠 실행하며 입력 순서·판정 규칙을 유지합니다. 병렬 기동 실패 시 순차 검사로 돌아가고 stderr에 알립니다. 시작 전 점검 C60은 빈 출력·잘못된 요약 값·종료 코드와 차단 목록의 불일치를 「차단 없음」으로 간주하지 않고 FAIL로 표시합니다. 검사 수는 bool을 제외한 음이 아닌 정수, 차단 목록은 비어 있지 않은 이름 문자열이어야 합니다. 유효한 차단 목록의 이름 표시와 합법적인 검사 수 0의 정책은 유지합니다. 출처: `REPORT.md, sha256 7f01df580f80d52d73ddb2962b8ec07ab20f06e75a1f7b4efcf4f4c71fe87372, 19760 bytes; internal evidence, not published` §1–2·10, R3 보고의 unchanged-accepted-scope.json. 신뢰도 High(지정 구현·검증 기록).
- **R3 정리 경로**: 스캐너의 20초 제한이 발동하면 자식 프로세스의 식별자를 아직 보유한 동안에만 종료를 요청합니다. 자식을 수거한 뒤에는 옛 숫자 pid/pgid로 다시 종료하지 않습니다. POSIX에서 ESRCH만 그룹 부재를 증명하고, EPERM 등 관측 오류는 확인 불가입니다. 원래 식별자를 놓은 뒤 같은 번호의 그룹이 보이면 원래 자손으로 단정하지 않습니다. C60 FAIL 행은 **빈 그룹 확인 / 잔존 감지 / 확인 불가**를 사유와 함께 구별합니다. Windows는 taskkill의 종료 코드·오류를 남기되 트리 관측을 **UNMEASURED**로 표시합니다. 출처: `REPORT-R3.md, sha256 9c37efbbc32925f9e40991fab73f640a9c730c531c9ce65cf02bd8dae6bc80bf, 5287 bytes; internal evidence, not published` X1·X2, `df355061`의 javis_preflight.py:5334·5353·5367·5373·5442. 신뢰도 High(호스트 실측 및 안전한 분기 대역 검사); 실제 Windows는 미측정입니다.
- **성능 표본**: 앞선 114개 스킬 표본에서 순차 중앙값 7.988초(n=3), 병렬 1.684초(n=3), stdout 14,316바이트 동일과 ACCEPT 91 / REVISE 13 / BLOCK 10이 보고됐습니다. 별도 R2 n=10 교차 측정은 흡수한 병렬판 1.698초, 후보 1.694초입니다. R3는 스캐너와 이 판정 규칙·측정 스크립트를 바꾸지 않았으며 성능을 새로 측정한 것은 아닙니다. 표본의 4ms 차이를 일반적인 체감 향상으로 확대하지 않습니다. 출처: `REPORT.md, sha256 7f01df580f80d52d73ddb2962b8ec07ab20f06e75a1f7b4efcf4f4c71fe87372, 19760 bytes; internal evidence, not published` §2·12, `1992a358` 메시지, `REPORT-R3.md, sha256 9c37efbbc32925f9e40991fab73f640a9c730c531c9ce65cf02bd8dae6bc80bf, 5287 bytes; internal evidence, not published`. 신뢰도 Med(앞선 생산자 측정 인용; R3의 스캐너 바이트 불변은 직접 대조).
- **R3 검증**: 전용 검사 **52 PASS / 0 FAIL**, 기존 10개와 새 5개 변이 **15/15 제거**, 전체 pack 시험 **95 PASS / 1 FAIL(96개)**, lane-parity **exit 0**. 유일한 실패 `test_bootstrap_chain`의 boot-last.json 부재는 R3 단독과 `f0ac3b0c`에서도 같은 격리 환경으로 재현했습니다. 통과·flake로 바꾸어 기록하지 않습니다. 새 커밋의 두 파일 해시는 전체 실행 전후 해시와 같고 라이브 설정 해시도 불변입니다. 재사용된 번호 보호는 안전한 OS 대역 검사이며 실제 PID 재사용 사고를 일으켜 잰 것이 아닙니다. 출처: `REPORT-R3.md, sha256 9c37efbbc32925f9e40991fab73f640a9c730c531c9ce65cf02bd8dae6bc80bf, 5287 bytes; internal evidence, not published` Verification·Commit binding, 같은 폴더 `full-run-provenance.json`, `bootstrap-controls.json`, `mutants.json`. 신뢰도 High(직접 실행 기록).
- **남은 한계**: **상위 checklist의 시간 제한이 preflight를 먼저 종료하면, 이 예외 처리 자체가 돌지 않아 스캐너 실패 사유와 자손 정리가 실행되지 않습니다.** 이번 R3는 checklist를 수정하지 않았습니다. 스캐너 자손이 스스로 프로세스 그룹을 벗어나는 경우도 범위 밖입니다. 다른 검사 다섯 곳의 빈 출력 대체, 같은 C60 안의 형제 60초 제한 세 곳, 검사 수 0과 잘못된 스킬 루트의 구별 정책은 미해결로 남습니다. 출처: `REPORT.md, sha256 7f01df580f80d52d73ddb2962b8ec07ab20f06e75a1f7b4efcf4f4c71fe87372, 19760 bytes; internal evidence, not published` §4·13 및 `REPORT-R3.md, sha256 9c37efbbc32925f9e40991fab73f640a9c730c531c9ce65cf02bd8dae6bc80bf, 5287 bytes; internal evidence, not published` Risks. 신뢰도 High(범위와 잔여를 명시한 기록).
- **Windows — UNMEASURED**: 실제 프로세스 생성·종료, 번들 Python의 프로세스 생성·바이트코드 동작, Authenticode·SAC는 이번 호스트 검사로 검증하지 않았습니다. 기존 **unsigned / SAC UNMEASURED / 설치기·앱·셸 교체 전 실행 확인 없음**은 그대로입니다. 새 exe/DLL 이름·교체 경로를 추가하지 않았다는 사실이 릴리스 서명 검사를 면제하지 않습니다. 출처: `REPORT-R3.md, sha256 9c37efbbc32925f9e40991fab73f640a9c730c531c9ce65cf02bd8dae6bc80bf, 5287 bytes; internal evidence, not published` Risks, 기존 §1-9. 신뢰도 High(미측정 범위의 명시).

## 2. 안 들어간 것과 이유
1. **[OUT] WP7 — 홈페이지 전환 도구(B-438)**: 세 회차 상한에서 합격선을 채우지 못해 닫았습니다. 남은 결함 R3-F1(첫 link 성공 뒤 unlink 오류가 나면 공개
   이름이 생겼는데도 「아무것도 옮기지 않음」 코드를 돌려줌)과 census 한계. 커밋 `f67d8c96` · `ae912a8a` · `551ab1c3` 은 로컬 가지에만 있습니다.
   이 도구는 이번 판에 포함하지 않습니다(B-456 · `dept-5/dept-ops/next-release-backlog/LEDGER.md` B-456 행 · CEO 결정 2).
2. **[OUT] 오너 SAC 요청 1 — 윈도우 코드 서명**: 서명 자격이 오너에게 아직 없습니다(1-9).
3. **[OUT] 오너 F4 — agy 남은 한도 표시**: 재현 표본이 없습니다(1-12).
4. **[OUT] 오너 F5 뒷부분 — PowerShell 에 Ctrl+C**: 부서장 판정으로 이 판 제외(1-11 · B-459).
5. **[OUT] 팩 소실의 전면 재설계(B-458)**: 설계 2회차 codex 차단 2 · 주요 3(1-10). 완화 WP14L 도 이 판에 없습니다:
   - **[OUT] WP14L — 팩 소실 완화(B-462)**: 한 번뿐인 설계 회차에서 codex 차단 3 · 주요 2 · 사소 1(L-F1~L-F6) · 부서장 판정 「설계 수용 안 됨」
     (13:26:01 +0900, `HEAD-RULING-DESIGN-L.md`)(1-10). 이 판의 문장은 「pack loss on update not fixed」입니다(Known limits).
6. **[OUT] phoenix 배포 경로의 교체 전 실행 확인(B-457)**: 운영자 복구 도구라 이 판에서 제외(1-9). 다음 판에서 같은 확인 도우미를 부르게 합니다.
7. **[OUT] WP12 요청 2**: 설치기 B1 `de2ee33f`(B-483, `JUDGMENT.md, sha256 f72a733f954cabc1096af8cb20e21c60239b9e238c4fbfcd7582f1e3396f47e2, 6643 bytes; internal evidence, not published`)와 앱/셸 B2 `c0db2658`(B-482, `PARKED.txt, sha256 e582db27a278c5b9ab706a4b88677f6791d69e05ec02941066a574ea22c3c183, 1941 bytes; internal evidence, not published`) 모두 제외입니다. 0.14.53은 어느 경로에도 이 교체 전 실행 확인을 싣지 않습니다.
8. **이월(B-351 · CEO 결정 2026-10-23 E1 (c))**: 지침 두 파일의 상한 문구 한 줄(1-1).

**개정 4 §2 상태별 근거** (출하 수정이 없는 항목의 해시는 출시 기준판이며 수정 완료 해시가 아닙니다):
- WP7 [OUT], 미출하 `551ab1c3`: `_reviews/WP7-B438-0.14.53-r3-codex.json` REVISE + master `TICKET-NOTES-REV4-w13.md` B-456 지시.
- WP12 Req1 [OUT], unsigned 고지 조각 `4ee6586b`: `HEAD-RULINGS-DESIGN-R1.md` 판정 0 + master 티켓 B-460; 실제 서명에는 오너 자격이 필요합니다.
- F4 [OUT], WP16 `6f477de5`에서 quota 수정 없음: master 티켓의 기존 한계 유지 지시 + 개정 3 원문/§1-12. 표본 부재 한계는 그대로입니다.
- PowerShell Ctrl+C [OUT], WP15 `d0a4f626`: resident WP15 `REPORT.md` item d + `HEAD-RULING-R1-SPACES.md`의 수용 범위 + master 티켓의 기존 한계 유지 지시.
- WP14/WP14L [OUT], 출하 수정 없음·기준판 `f0ac3b0c`: `_reviews/WP14-PACKLOSS-DESIGN-0.14.53-r2-codex.json` + resident `HEAD-RULING-DESIGN-L.md` + master 티켓 B-458/B-462; F2는 NOT FIXED.
- Phoenix probe [OUT], 기준판 `f0ac3b0c`: resident WP12 `HEAD-RULING-R2-PHOENIX.md` B-457. Req2의 B1/B2도 제외됐으므로 다른 경로에도 이 보호가 없습니다(§1-9).
- WP12 B1 [OUT] `de2ee33f`: `JUDGMENT.md, sha256 f72a733f954cabc1096af8cb20e21c60239b9e238c4fbfcd7582f1e3396f47e2, 6643 bytes; internal evidence, not published` + `TICKET-NOTES-REV5a-w13.md, sha256 aaecff97b043500017f014b36a2cd3bab2a526ab67d03e9c122008ee9e1a9289, 7131 bytes; internal evidence, not published` B-483; B2 [OUT] `c0db2658`: `PARKED.txt, sha256 e582db27a278c5b9ab706a4b88677f6791d69e05ec02941066a574ea22c3c183, 1941 bytes; internal evidence, not published` + r3 codex REVISE B-482.
- 지침 상한 문구 이월, `83d29ea5`: resident `INTEG-REPORT.md`의 E1 (c) 기록 및 기존 §1-1; 이번 문서 작업의 변경 대상이 아닙니다.

## 3. 그대로인 것
- WP1/WP5 범위에서 자원 관문 자체(`javis_resource_gate.py`)와 경보 라우터의 다른 경로는 그대로입니다(`0b2027d7` Constraint · `76b9ee63` Scope-risk). WP17의 슬래시 명령 배달 변경은 §1-13에 따로 적었습니다.
- 맥 · 리눅스의 claude CLI 찾기(WP8 은 윈도우 경로만 · `62d1abc9` Constraint).
- 출하 바이너리의 큐 저장 실패 메시지(WP2 · 1-15).
- 홈페이지 전환 도구는 제외(WP7 OUT). 이 문서는 공개 다운로드 페이지의 상태나 링크를 전제하지 않습니다.

## 4. 재지 못한 것 — 숨기지 않고 적습니다
- **윈도우 실기 전부**: 이 판의 어느 작업도 실제 윈도우 기계에서 돌지 않았습니다(CI 러너만 · `PLAN.md` 「Honest limits」). WP 별로:
  - WP8 — 윈도우에서 실제로 claude 를 찾는지(UNMEASURED-1), `#[cfg(windows)]` 배선, `dirs::home_dir()` 가 None 일 수 있는지(`REPORT-R2-SLOT4.md` §7).
  - WP12 — Smart App Control 동작은 UNMEASURED. 제외된 B1 `de2ee33f`의 CI에서 W1 3/3은 측정됐지만 정책 준비 실패로 CTL 0/5는 실행되지 않았습니다. 실제 사용자 SAC PC 검증을 뜻하지 않습니다(`JUDGMENT.md, sha256 f72a733f954cabc1096af8cb20e21c60239b9e238c4fbfcd7582f1e3396f47e2, 6643 bytes; internal evidence, not published`).
  - WP6 — 잠금(LockFileEx)의 윈도우 동작(문서 근거뿐)(`r2-fix/REPORT-R2.md`).
  - WP11 — 윈도우 고대비 화면과 윈도우 터미널의 제목 통과(CSS·TS 만 바뀜 · `REPORT.md` T5).
  - WP13 — 윈도우에서 탭 제목 렌더링(`REPORT.md` §7-4).
  - WP15 — agy 1.4.6 실물 · ConPTY Ctrl+C(`HEAD-RULING-1.md`).
  - WP16 — 윈도우 수신자 · Authenticode · SAC(codex r1 「missing」) · 윈도우 생성자 판별의 실기 실행(`integ/INTEGRATION-LOG-WP16.txt`).
  - WP17 — 윈도우 실기와 어댑터(`FINAL-REPORT-LINE.txt`).
  - WP9 — 윈도우 대상 컴파일(`REPORT.md` 「UNMEASURED here」) · WP10 — 윈도우 경로(`REPORT.md`).
- **고정 통합 커밋 `319c2171` 위의 최종 전체 시험 묶음**: 아직 없습니다. 지금 있는 묶음 결과는 그 전 끝 커밋 `9e987a8d` 의 것입니다(§7).
- **CI Python 3.12 로 돈 팩 시험**: 로컬은 3.14.7(`INTEG-REPORT.md` §11).

## 5. 성능 — 잰 것과 결정
- 이 판에서 성능을 바꾸는 변경으로 잰 것은 없습니다. 출처에 있는 수치와 상한만 옮깁니다.
  - WP11 탭 점: 만료 시간 31 초 / 47 초 그대로(`_evidence/dept5-0.14.53-wp11-tabdot-20261023/REPORT.md` T5).
  - WP8 윈도우 좌석 PATH: 끝에 4 항목 145 B(환경 블록 32,767 B 의 0.44 %)(`62d1abc9` Scope-risk).
  - WP6 [IN] (`cfc10dde`): 잠금 대기 최대 120 초, pack-update 한 번에 최대 두 번(`_evidence/dept5-0.14.53-wp6-b428-20261023/r2-fix/REPORT-R2.md` §2).
  - WP9: Rust 쪽 판정 지연은 재지 않았습니다(`_evidence/dept5-0.14.53-wp9-missionping-20261023/REPORT.md` 「UNMEASURED here」).

## 6. 성찰 효과 최종 보고 (성찰 측정 규약 §7)
- 릴리스 마감 계산(`dept-5/dept-ops/reflection-measurement/LEDGER.csv`)은 아직입니다 — 0.14.52 §6 과 같은 모양으로 공개 전에 채웁니다(채움 대기).

## 7. 검증 (맥 · 통합 가지)
- **`9e987a8d`(WP1 · WP5 · CEO 결정 (c) · WP3) 위**: `--lib` 780/0/1 · `--bin cysd` 2834/0/13 · `--bin cys` 593/0/1 · 비밀 검사 깨끗(1213 파일) · pyseal 초록
  (`_evidence/dept5-0.14.53-release-integ-20261023/suites/RUN-tip2.txt`). 열린 것: cys-app 측정 못함(rc 101 · 하네스에 CI 스텁 준비가 없었음) ·
  relgates 빨강 1 건 `test_31_6a`(6.30 초, 상한 6.0 초 · WP3 시간 시험)(`INTEG-REPORT.md` 「UPDATE 2026-10-23 23:4x」).
- **`126fdc39` 위**: WP10 의 `javis_intake.py selftest` · `test_intake_exit_contract` 62/62 만(`checks-wp10/RUN.txt`). `affda1e5`(WP11 추가) 위: 빌드
  없음(`integ/INTEGRATION-LOG-WP11.txt`). `7c605e0f`(WP16 추가) 위: 빌드 없음(`integ/INTEGRATION-LOG-WP16.txt`). `8b59ef75`(WP12 slice A 추가) 위: 빌드 없음
  (`integ/INTEGRATION-LOG-WP12A.txt`). 전체 묶음은 아직입니다 —
  돌릴 순서와 읽는 법은 `notes/INTEG-TIP-RUNBOOK.md`.
- 이번 개정에서 제품 빌드·제품 시험은 돌리지 않았습니다. Python-only 텍스트/병합 검증 결과는 내부 변경 기록에 적습니다.

- **개정 4 통합 이력**: `ced9c78e` WP15, `2be1c383` WP17, `2d218d33` WP6, `319c2171` WP2. 각 `integ/INTEGRATION-LOG-WP{15,17,6,2}.txt`는 해당 통합에서 빌드·시험을 하지 않았다고 명시합니다. WP별 측정 결과를 이 통합 스냅샷의 통과로 옮기지 않습니다. 최종 tip 검증은 별도 게이트입니다.

- **통합에서 확인한 기존 실패(원인 UNMEASURED)**: `test_inject_context_role_seat`는 rc=1·실패 7개, `test_preflight_phase1_checks`는 rc=1·24개 중 실패 3개입니다. 부서장이 일회용 작업 트리에서 `f0ac3b0c`, `2dda5a69`, `2c814093`을 측정했으며 각 검사의 실패 집합과 개수가 세 판에서 동일했습니다. 좌석 환경변수를 해제해도 동일했습니다. 새로 생긴 실패로 분류하지 않으며 **원인은 UNMEASURED**입니다. 출처: 부서장의 이번 통합 GO 직접 실측 통보(신뢰도 High, 부서장 측정으로 귀속). 이 문안 작업에서 재실행한 결과는 아닙니다.

## 8. 공개 전에 아직 열려 있는 것
1. **WP18 통합 완료**: source `df355061` → integrated `2c8140937547ca3f42af9a157cca961753debc6e`. 8/8 patch-id EQUAL, 충돌 0, 실제 통합 tree와 검증된 dry-run tree가 모두 `02633fa3c9e4c8f6e255ebe864e8e88f33800f3f`로 같습니다. 부서장이 브랜치·끝 커밋·깨끗한 트리·두 tree 해시를 직접 확인했습니다. 이 사실은 최종 전체 시험이나 공개 성공을 의미하지 않습니다.
2. **스마트 앱 컨트롤 문장**: 결정됨 — slice A 의 문장(`docs/RELEASE_NOTES_0.14.53.md` 5-7 행). 초안의 A · B 는 거둡니다. 검사가 보는 두 핵심 문구는
   `scripts/release-verify.py:167-170`, 표지 문자열은 검사하는 글에 정확히 한 번(`:463`). 요청 2는 제외됐으며, 이번 판에는 교체 전 실행 확인이 없습니다(§1-9).
3. **팩 소실 문장**: 결정됨 — 「고치지 못함」(Text N). WP14L 은 [OUT](`HEAD-RULING-DESIGN-L.md`, 13:26:01 +0900)(Known limits).
4. **최종 통합 끝 커밋 위의 전체 시험 묶음** · **통합 끝 커밋 리뷰(이종 둘)** · **같은 커밋의 윈도우 CI** — `notes/RELEASE-CHECKLIST.md` C1–C3.
5. **원격 검증의 합격선**: 「모든 원격 검사 통과, 개수는 도구가 찍은 그대로」 — 도구는 8/8 을 찍습니다(CEO 문장의 7/7 은 검사가 늘기 전의 수).
   개수를 문장에 맞춰 고치지 않습니다(checklist C6).
6. 버전 번호 커밋(8곳)과, 이 초안을 slice A 가 만든 `docs/RELEASE_NOTES_0.14.53.md` **안으로 합치는** 커밋(마지막 · 새 파일 아님 · 그 파일 1-10 행은 바이트
   그대로 · `notes/INTEG-TIP-RUNBOOK.md` §2-B)(`INTEG-REPORT.md` §6 4번).

## Known limits in 0.14.53

- **Windows Smart App Control (owner SAC request 3) — the published statement is WP12 slice A's block, not text in this draft.** It is the section
  at the top of `docs/RELEASE_NOTES_0.14.53.md` (created by `e5f49fbb`; lines 5-7 at `8b59ef75`, the same bytes as the release body in
  `release.yml:1129-1131`): unsigned cys installer and executables, may be blocked with Smart App Control on, not verified on a Smart App Control PC.
  This draft does not repeat it: the publication check (`scripts/release-verify.py` stage 8) refuses a text in which the marker string occurs more
  than once (`:463`) and needs slice A's two sentences (`SAC_KEY_PHRASES`, `:167-170`). Revision 2's Variant A/B are withdrawn (kept in
  `notes/raw/rev2/RELEASE-NOTES-DRAFT.md`). WP12 Req2 is OUT: no installer or app/shell pre-swap run check ships; SAC behavior remains UNMEASURED (sources in §1-9).
- **Agy remaining quota shown as usage (owner F4) — NOT fixed:** agy 1.4.6 can show "7d 100%" in red while its own quota screen shows the quota full. Our code
  already converts agy's remaining fraction to usage, the 1.4.6 payload could not be reproduced (this Mac runs agy 1.4.5), and no sample from an affected
  Windows seat was available, so 0.14.53 ships no guessed second inversion (`dept-5/dept-ops/ceo-line/sent/20261024-1147-agy146-sample-request.md`). The
  red value can also raise an `alert.account_rate` alert (owner report, `OWNER-20261024-seven-fixes.txt` item 4).
- **Custom tab titles are not persisted (B-455) — WP13 [IN]:** a title set with `cys set-title` reverts to `{role}-{agent} · {folder}` after an
  app or daemon restart. Deferred to keep the boot and restore chain untouched (`dept-5/dept-ops/next-release-backlog/LEDGER.md` B-455 row;
  `_evidence/dept5-0.14.53-wp13-tabtitle-20261024/REPORT.md` §7-1; resident `_reviews/WP13-TABTITLE-0.14.53-r1-codex.json`, `9e963979`, verifies the CLI non-persistence disclosure).
- **Pending directive re-injection (B-428, B-446):** WP6 is IN at `cfc10dde` (integrated `2d218d33`). The old silent persistence failure and unlocked cross-lane overwrite are repaired: durable intent precedes UNKNOWN-capable writes, and concurrent updates share a real pending-file lock. An update that cannot acquire it refuses with `pending-lock-refused` (exit 1), names untouched pending entries and can be retried. A pending-pass refusal occurs before fetch/apply; an apply-phase refusal describes that phase only and separately reports any earlier pending-pass effects. The bounded waits remain up to 120 s at each of two acquisition points; an uncreatable lock refuses immediately. A read-only-handle lock error refuses safely. Windows File::try_lock/LockFileEx, ACL-denied creation and cross-lane behavior remain UNMEASURED (behavioral specimens are cfg(unix)). Sources: `_evidence/dept5-0.14.53-wp6-b428-20261023/r3-fix/REPORT-R3.md` §1–§2, §7, §9; `r2-fix/REPORT-R2.md` §2; `integ/INTEGRATION-LOG-WP6.txt`.
- **Pack loss on update (owner F2) — NOT fixed: ship Text N.** The full redesign is OUT (B-458, `_evidence/dept5-0.14.53-wp14-packloss-20261024/HEAD-RULING-DESIGN-R2.md`,
  13:03:00 +0900), and WP14L (the mitigation) is OUT too: "the design round is NOT accepted, so WP14L is NOT in 0.14.53. 0.14.53 ships with the owner F2
  release note "pack loss on update not fixed" (worker-5 uses the not-fixed text)" (`_evidence/dept5-0.14.53-wp14-packloss-20261024/HEAD-RULING-DESIGN-L.md`,
  sha256-16 `9117766a65736d3c`, 13:26:01 +0900); its open codex findings are backlog B-462 (`dept-5/dept-ops/next-release-backlog/LEDGER.md` B-462 row).
  Text N restates the owner report and the two backlog rows; nothing in it is measured beyond what those files say.
  - **Text L (mitigated) — WITHDRAWN, do not ship:** WP14L is OUT (ruling above). Its revision-1 wording is kept only in `notes/raw/rev1/RELEASE-NOTES-DRAFT.md`.
  - **Text N — SHIP THIS** (WP14L is OUT): "Pack loss on update not fixed in 0.14.53: after an update and a restart, `~/.cys/pack` can be replaced by an
    incomplete pack while the previous complete pack is moved to `~/.cys/pack.prev` (reported on 0.14.52 and on 0.14.50). The reporter recovered by
    copying `pack.prev` back over the pack. The full redesign is planned for the next release (B-458). A smaller mitigation (no copy-and-delete fallback
    of the live pack, `pack.prev` never deleted first, no swap when the version guard blocked every write) was designed but not accepted in its one
    review round and also moves to the next release (B-462)."
    Sources: owner report `dept-5/dept-ops/intake/OWNER-20261024-seven-fixes.txt` item 2; the B-458 and B-462 rows of
    `dept-5/dept-ops/next-release-backlog/LEDGER.md` (the B-462 parenthesis is that row's wording). Whether to add advice to users (for example, not to
    delete `pack.prev`) is the head's call; this draft does not add it.
- **Ctrl+C into PowerShell (owner F5, second half) — not changed (B-459):** `cys send-key C-c` did not get a Windows PowerShell out of a `>>`
  continuation in the owner's report. The source already emits ETX 0x03 after its guards; the native cause (ConPTY input mode, console mode) is
  unmeasured. No product change in 0.14.53 (WP15 head ruling 1 (d); `LEDGER.md` B-459 row).
- **No pre-swap run check ships in 0.14.53 (B-483/B-482; Phoenix B-457 remains uncovered):** installer B1 `de2ee33f` and app/shell B2 `c0db2658` are OUT. Silent updater, wizard reinstall, uninstall-first, main/department rotation and Phoenix therefore do not gain this protection. Windows builds remain unsigned; Smart App Control behavior is UNMEASURED. Sources: `JUDGMENT.md, sha256 f72a733f954cabc1096af8cb20e21c60239b9e238c4fbfcd7582f1e3396f47e2, 6643 bytes; internal evidence, not published`, `PARKED.txt, sha256 e582db27a278c5b9ab706a4b88677f6791d69e05ec02941066a574ea22c3c183, 1941 bytes; internal evidence, not published`, `TICKET-NOTES-REV5a-w13.md, sha256 aaecff97b043500017f014b36a2cd3bab2a526ab67d03e9c122008ee9e1a9289, 7131 bytes; internal evidence, not published`.
- **Windows update from an installed 0.14.52 to 0.14.53 — UNMEASURED, which is not the same statement as "broken":**
  the one measurement attempted for this path (`windows-measure` run `40173384370`) stopped in its **baseline** stage and
  never reached the update step, so its `verdict.json` reads `status: UNMEASURED` (reasons: `logs_complete`, `baseline`,
  `versions`, `daemon`, `pack`, `seat`, `installer_exit`, `tracebacks`). We do **not** know that updating 0.14.52 to
  0.14.53 on Windows fails, and we do **not** claim that it works: past the baseline, nothing was observed either way.
  The three named gaps are observer-side, not product findings: the log inventory is written only after the update step,
  so a baseline stop leaves it absent while the finalizer still reduced its mandatory set; `.pack-state.json` was required
  by the harness but a normal fresh embedded install legitimately does not create it, so that "failure" was a harness
  premise; and the CDP version read failed at **target discovery** (`no page target on the debugging port`, five attempts
  of about 60 s), with the retained evidence insufficient to name a native cause. The observer repair is approved for a
  later release and is deliberately **not** a 0.14.53 blocker. What *was* measured for Windows here: the signing state.
  Signing mode is `unsigned`; all **eight** of our own executables in the shipped Windows artifact
  (`cys_0.14.53_x64-setup.exe`, `cys-app.exe`, `cys-app.new.exe`, `cys.exe`, `cys.new.exe`, `cysd.exe`, `cysd.new.exe`,
  `uninstall.exe`) read `embedded_signature=false`, `authenticode_status=NotSigned`, `signature_type=None`; and a census
  of **all 743 PE files inside the installer** counts **163 embedded-signed and 0 of them ours** (malformed 0 — the
  signed ones are vendor files, recorded in the log and outside the gate). The four required executables all read `ok`,
  and the same run's control reads `red-as-expected`: a synthesized signed PE was read as signed, so the detector's
  discrimination was proved in that same run instead of assumed, and the report's `verdict` is `PASS`. That makes
  "unsigned" a measurement rather than an assertion (`windows-signing-report`, artifact `12578553755`, schema
  `cys-win-pe-census/1`; re-measured and read by the head on 2026-10-25 20:1x). This is a **signature census, not a
  Smart App Control measurement**: the signing notice at the top of this document is the published statement, and real
  Smart App Control enforcement on a Smart App Control PC remains UNMEASURED (see the pre-swap bullet above).
  If an update does not complete on your PC, the 0.14.52 release stays
  published and installable, and the previous pack directory is kept at `~/.cys/pack.prev` — the pack-loss limit above
  describes that route, and this note adds no advice beyond it. Sources: frozen R1 artifact
  (`PROVENANCE.txt` run `40173384370`, artifact `12591711071`, zip sha256
  `38b0cdee3603968b90c8bb6349e503ca75762a3a2374be86315b49b233dcd362`; `artifact/verdict.json`) and
  `DESIGN-NOTE.md, sha256 8d560eeaa17bb70a1eee30fee126edbb201c192cec3fc9f33cb8e664be40c084, 17934 bytes; internal
  evidence, not published`; the signing artifact and its schema above, as re-measured by the head. A CI result is only
  the named CI measurement, never a real-user-PC claim.
- **In 0.14.53 the pending diagnostic prints on stderr (B-428) — a stream change, not a breakage:**
  the human-readable *pending* diagnostic — the prose line a person reads (find it by its stable text `pending reinject 소비`), not a machine record — is written to
  **stderr** in this release; the B-428 repair of the pack-update "up to date" report is what settled it there, so
  the stream this line arrives on is one of the things that changed in 0.14.53.
  Nothing machine-readable changed: the machine-prefixed records, the exit codes and the UI events are **unchanged**.
  A census of this repository's consumers of that prose counts **zero product consumers** and **one test consumer**;
  the Tauri and Python callers parse the machine prefixes, the UI consumes events, and the CI workflows read the exit
  status — so nothing inside this repository breaks. **Custom scripts outside this repository are UNMEASURED**, and
  that one cell is the only reason this note exists. **If a script of yours greps that diagnostic on stdout, point it
  at stderr** — read stderr, or merge the streams with `2>&1`; a script that already keys on the machine prefixes or
  on the exit code needs no change. Sources: the department's census of in-repository consumers for the B-428 repair
  (internal evidence, not published).
- **WP7 not in 0.14.53 (B-456):** the homepage switch tool (B-438) closed at its 3-round cap with residue R3-F1 and stays excluded;
  no download-page state or distribution link is asserted here (`LEDGER.md` B-456 row).
- **Tab dot (WP11, IN):** gemini/agy seats keep the "unobserved" look (their grammar stays provisional), and one codex seat on the base daemon read
  "unknown" for 11,092 s for an unmeasured reason (`_evidence/dept5-0.14.53-wp11-tabdot-20261023/REPORT.md` "Deferred to the head" R1-6, R1-7).
- **WP8 confidence medium:** a Windows pane's PATH gains 4 appended entries (145 B); detection widens for codex/agy in the same fail-open direction; real
  Windows execution unmeasured (`62d1abc9` Scope-risk / Not-tested).
- **CSO's own context-threshold notice after a paused or full master queue (B-439 R1-6):** the notice is not queued; the fact goes to `alert-route-refused.jsonl`
  and an event, but is not re-sent when the pause ends (`7e90f5e7`; earlier draft Known limits).
- **Duplicate notices after an operator revive (B-439 RF1-7):** the second `cys cycle-agent --fire` is refused with rc 87 (`76b9ee63`).
- **Scope of the WP5 review result (B-442):** the r2 ACCEPTs certify the reviewers' reading of the producer's evidence; full suite, mutants and the fire-id
  guarantee were not independently re-executed (head re-ran the alert-route tests, 169/0) (`LEDGER.md` B-442 row).
- **No default department cap (B-430/B-431):** with `CYS_DEPT_CAP` unset, empty or 0 there is no limit; set `CYS_DEPT_CAP=<N>` for a cap or
  `CYS_FORMATION_NODES_HARD=1` for the old nodes block (`0b2027d7`).
- **Directive cap wording deferred (B-351, CEO ruling 2026-10-23 E1 (c)):** the two directive files keep their v0.14.52 bytes (`83d29ea5`), so their
  `create_failed` row still names a team cap of 8 while the code default is unlimited.
- **Windows:** no work in this release ran on real Windows hardware (CI runners only); per-WP list in §4.

- **WP15 Windows paths containing spaces (B-461):** at `d0a4f626`, a spaced pack/home path is visibly refused; the existing statusLine.command stays byte-identical and the message explains the manual remedy. Native agy 1.4.6 spaced-path support remains unmeasured and deferred. This accepted refusal is not a claim that all Windows paths now work (`HEAD-RULING-R1-SPACES.md`; `integ/INTEGRATION-LOG-WP15.txt`).
- **b360 test hermeticity gap — HOME-sensitive, not a product-defect claim:** the WP13 reviews at `9e963979` retain four b360 failures under the seat HOME and 4/4 success with rig HOME; the one-factor HOME bisect covers one representative. This is a test-environment/backlog issue, not a new user-facing defect or a reason to describe an unmeasured product failure. Source: `_reviews/WP13-TABTITLE-0.14.53-r1-codex.json` justification/evidence and master `TICKET-NOTES-REV4-w13.md`.
- **WP17 admission and command execution:** at `9f2d1dca`, recognized framed upper/lower drafts refuse before publication; unknown hints/layouts and markerless seats retain ordinary-gate fallback. A command-shaped multiline body remains one submission. Real Claude/codex/agy command effects and native Windows delivery remain UNMEASURED (`_evidence/dept5-0.14.53-wp17-slashcmd-20261024/REPORT-R2.md`).
- **Windows native paths — UNMEASURED by name (no Windows hardware; CI runners only):** WP8 installed-CLI discovery/PATH and Windows launch; WP9 mission-origin behavior on Windows; WP10 Windows unreadable-path/errno behavior; WP6 LockFileEx/ACL denial/shared pending-lock behavior; WP11 title grammar/high-contrast rendering; WP13 native identity/PTY/tab rendering, the cargo stack setting and packaged Windows stack checks; WP15 agy 1.4.6 statusline migration/visible spaced-path refusal, launch capture/bare-PowerShell guard and ConPTY Ctrl+C; WP16 native creator-PID ancestry/close permission; WP17 native queued slash delivery and adapter interpretation; the excluded WP12 B1 candidate (`de2ee33f`): NSIS worker termination, old-app handoff, wizard/uninstall-first/passive GUI/refusal-record behavior, and excluded B2 (`c0db2658`) full app/shell resolved-pair pre-drain/rotation/latch routes. WP18 native process-tree termination/spawn and bundled-runtime bytecode behavior (`df355061`, §1-16) are also UNMEASURED. Real SAC-PC enforcement and release Authenticode/pre-swap coverage are not established by these local slices. Sources: per-WP §4 and reports/reviews cited in §1; `_reviews/WP13-TABTITLE-0.14.53-r1-codex.json`, WP12 B1 r2 and B2 r1 codex missing lists; master ticket. A CI result is only the named CI measurement, never a real-user-PC claim.

- **WP18 outer-checklist timeout residual (accepted source `df355061`; reported-not-fixed):** if the checklist terminates preflight before the scanner's own timeout handler runs, neither that handler's diagnostic nor its process-group cleanup executes. R3 leaves javis_checklist.py unchanged. Scanner self-detachment, sibling 60-second bounds and the deferred zero-scanned policy remain outside this repair. Native Windows cleanup is UNMEASURED. Sources: `REPORT.md, sha256 7f01df580f80d52d73ddb2962b8ec07ab20f06e75a1f7b4efcf4f4c71fe87372, 19760 bytes; internal evidence, not published` §4·13; `REPORT-R3.md, sha256 9c37efbbc32925f9e40991fab73f640a9c730c531c9ce65cf02bd8dae6bc80bf, 5287 bytes; internal evidence, not published` Risks; §1-16 above.

- **Pre-existing integration reds — cause UNMEASURED:** `test_inject_context_role_seat`: rc=1, 7 failures; `test_preflight_phase1_checks`: rc=1, 3 failures out of 24. The head measured identical failure sets and counts on `f0ac3b0c`, `2dda5a69` and `2c814093` in a throwaway worktree, unchanged with seat environment variables unset. These are pre-existing observations; their cause is UNMEASURED. Source: the head's direct integration GO measurement notice. No release-severity conclusion is assigned here.

