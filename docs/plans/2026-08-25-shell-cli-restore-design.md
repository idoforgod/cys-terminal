# '셸에 cys 설치' 버튼 복원 — 설계 정본 (2026-08-25)

> **이 문서의 지위**
> 이 작업의 설계 정본(D1~D8)·IPC 계약·라운드 이력·설계 규약은 그동안 세션 워크플로우
> 스크립트(`workflows/scripts/*.js`)의 문자열 상수 안에만 존재했다. 세션이 사라지면
> "우리가 무엇을 요구했는가"를 되물을 방법이 없어진다. 그래서 레포로 승격한다.
>
> **원문 출처(승격 이전 위치)**
> `<SESSION-DIR>`
> - `shell-cli-install-restore-wf_46c444ac-96a.js` → `DESIGN` 상수 (D1~D8) · `LENSES` (1차 반증 3렌즈)
> - `shell-cli-repair-round2-wf_8ac23be3-cae.js` → `CONTRACT` 상수 · `FINDINGS` 상수 (BLOCK-1 ~ MINOR-12)
> - `shell-cli-round3-and-reflection-wf_cf07aa0d-b4f.js` → `R3` 상수 · `CONTEXT` 상수 (성찰 재료)
> - `shell-cli-round4-class-repair-wf_93b8244a-31c.js` → `CLASS` 상수 (C1~C5 · I1~I7 · 인수 기준)
>
> **제3절(IPC 계약 정본)의 지위**: 이 절은 `src-tauri/src/main.rs` 실물을 직접 읽어 적은
> 것이며, **Rust 주석과 TypeScript 헤더가 인용해야 할 단일 진실**이다. 세 곳(문서·Rust·TS)이
> 어긋나면 이 문서가 아니라 **실물 코드**가 이긴다 — 다만 어긋났다는 사실 자체가 결함이므로,
> 실물이 바뀌면 이 절을 같은 커밋에서 갱신한다.

> **이 문서의 독자는 둘이다** — ①오너(개발자가 아니어도 무슨 일이 있었는지 읽을 수 있어야 한다)
> ②이 레인을 이어받는 사람. 그래서 전문용어는 풀어 쓰되 **수치·근거·`파일:줄번호` 는 전부
> 보존한다.** 요약하거나 압축하지 않는다.

- **레인**: `<WORKTREE>` @ `fix/shell-cli-install-restore`
- **기준점(base)**: `c54c3b2` = v0.14.24 (2026-08-22 릴리스)
- **현재 HEAD**: `c6f3669` = 9·10라운드 수리 커밋(= **11라운드 판정 대상 freeze**). 유도:
  `git log --oneline c54c3b2..HEAD` → **9줄**(아래 1.2 표와 같다). 9·10라운드를 "미커밋"이라 적었던
  이전 판은 그 커밋으로 사실이 아니게 됐다 — 11라운드가 그 줄을 다시 유도해 고쳤다.
- **갱신 시점**: 2026-08-25 · **12라운드(마지막)** — BLOCK-1·MAJOR-2(다중 백업본에서 같은
  목적지로 가는 `mv` 두 줄 = 실제 손실 경로 · UI 레인) · MAJOR-3(문서 계약표가 코드와 반대로
  말했다) · MAJOR-4(머리의 단언이 맥에만 참이었다) · MAJOR-5(존재하지 않는 시험을 보증으로 들었다)
  · MAJOR-6(핀 주석의 과대선언) → **§4.16**. ★12차는 하드 스톱 라운드다 — **새 핀·새 약속·새 기계
  장치를 만들지 않고**, 손실 경로를 닫고 이미 쓴 약속을 사실로 좁히는 것만 했다.
  11라운드 갱신 이력: BLOCK-1(문서가 약속한 백업이 집행 단계에 없었다) ·
  MAJOR-2(유일한 백업 단계가 자기 실패를 숨겼다) · MAJOR-6 계열의 문서 몫(같은 대상 다중 백업)
  → §4.15.
  10라운드 갱신 이력: MAJOR-4(단계 흐름) · MINOR-5(초기화 보존 계약) ·
  MINOR-7(설치 백업 이름 충돌 가드) · MINOR-8(이 절과 §4.12~§4.13 · §4.14 ④ · §5 ⑰ · §7.3 · §9 · 부록 A·B 재유도).
  8·9라운드의 갱신 이력은 §4.11~§4.12 에 그대로 남긴다.
- **이 문서 안의 `src-tauri/src/main.rs:NNN` 은 10라운드 작업트리 기준이다.** `0b6cb24`→`e7063d3`
  구간에서 `main.rs` 는 무변경이었고(`git diff 0b6cb24 e7063d3 --stat` 에 그 파일이 없다),
  10라운드가 `build_install_script` 에 충돌 가드를 넣으면서 그 아래가 밀렸다 — 부록 B 의 값은
  손으로 세지 않고 `grep`(정의 줄 패턴 매칭)으로 다시 유도했다.
- **`ui/src/*.ts:NNN`(부록 B.9)은 같은 라운드에 UI 레인이 고치고 있던 파일이라 값이 다시 밀린다** —
  §9.3 U9 참조.

---

## 1. 배경과 이력

### 1.1 이 기능이 무엇인가 (일반 독자용 풀이)

cys 앱은 macOS의 `/Applications` 안에 들어 있는 GUI 프로그램이다. 그런데 사용자가 직접 연
**터미널 창**에서 `cys` 라고 타이핑하면 "그런 명령이 없다"고 나온다. 터미널은 `PATH` 라는
목록에 적힌 폴더들만 뒤져서 명령을 찾는데, 앱 번들 안쪽(`/Applications/cys.app/Contents/MacOS/`)
은 그 목록에 없기 때문이다.

'셸에 cys 설치' 버튼은 이 간극을 메운다. 버튼을 한 번 누르면 관리자 비밀번호를 **한 번** 묻고,
`/usr/local/bin/cys` 와 `/usr/local/bin/cysd` 라는 **바로가기(심볼릭 링크)** 를 만든다.
`/usr/local/bin` 은 거의 모든 터미널의 `PATH` 에 들어 있으므로, 그 순간부터 터미널에서 `cys` 가
동작한다. VS Code 의 "Install 'code' command in PATH" 와 정확히 같은 형태다.

- **심볼릭 링크(symlink)** = 파일의 바로가기. 실제 파일을 복사하지 않고 "저기를 보라"고 가리키는 표식.
- **실체 파일(real file)** = 바로가기가 아닌 진짜 파일. 다른 도구가 설치한 진짜 바이너리일 수 있다.
- **승격(elevation)** = `/usr/local/bin` 은 관리자만 쓸 수 있으므로 비밀번호를 받아 권한을 올리는 일.
  이 앱은 `osascript ... with administrator privileges` 로 **딱 1회** 승격한다.

### 1.2 시간 순 이력

| 시점 | 커밋 | 무슨 일이 있었나 |
|---|---|---|
| 2026-06-29 | `1882a5b` | `feat(cli-path): Control Center button to install cys CLI to PATH` — 버튼 신설. 설계서는 `docs/plans/2026-06-29-cli-path-install.md`. |
| 2026-08-20 | `3685af9` | `feat(ui): P2 — ▶CEO·▶부서장·'셸에 cys 설치' 버튼 3종 제거 + 기동·설치 안내를 이원 경로로 정합` — 오너 지시로 버튼 **제거**. |
| 2026-08-22 | `c54c3b2` | `release: v0.14.24` — 이 레인의 기준점. |
| 2026-08-25 | `3080315` | 레인 개설(TODO 정본 + 부트 사슬 실측 증거 이관). |
| 2026-08-25 | `0adc45d` | `docs(state)`: SESSION_STATE 정본 — Scope B·C 이미 구현 실측 확정. |
| 2026-08-25 | `0e60b79` | 1차 구현 — 버튼 복원 + 수리 5건 (반증 이전). |
| 2026-08-25 | `6159778` | 2차 — 적대적 반증 12건 수리(파괴→백업 전환·TOCTOU 봉합·hang 실제 차단·계약 드리프트 해소). |
| 2026-08-25 | `9f47148` | 3차 — 산문 계약 폐기(`unverified_reason` 기계 판별자)·부분실패 백업 통보·셸 종료상태 존중·펌링크 별칭 |
| 2026-08-25 | `7f8b505` | 4~6차 — 계열 수리(C1~C5·I1~I7) · 5차 `do shell script` **CR 구분자** BLOCK 폐쇄 · 6차 Windows E0425 컴파일 즉사 폐쇄 · '판정≠집행' 통일. ★이 커밋이 레포 루트의 `MASTER_TODO.md`·`SESSION_STATE.md` 를 삭제했다(→ §8) |
| 2026-08-25 | `0b6cb24` | 7차 — 등급 회귀 폐쇄 · 커맨드 3종 **비동기화** · 앱이 내던 파괴적 명령 제거 · 재발방지 핀의 사각 폐쇄 |
| 2026-08-25 | `e7063d3` | 8차 — 문서가 시키던 파괴적 명령 폐쇄 · 판독기 전 필드 변이시험 · 재진입 가드 · 설계정본 현행화(이종 판정자 2인의 판정 12건 → 4.11). **현재 HEAD = 9차 freeze** |
| 2026-08-25 | (미커밋) | 9차 — 배포 문서 코드블록 전수 스캐너 신설 · 문서↔테스트 결합 명문화. ★**검증 하네스가 문서 코드블록을 실행해 살아 있는 데몬을 죽인 실사고**가 이 라운드에서 났다(→ 4.12·4.14 ④) |
| 2026-08-25 | `c6f3669` | 9·10차 — 스코프 밖 문서 파괴명령 폐쇄 · 선언된 가드를 집행에 연결 · 변이 인수기준 충족(MAJOR-1~4 · MINOR-5~8). 이 문서의 좌표·규모·테스트 수 재유도가 MINOR-8 의 산출물이다(→ 4.12·4.13). **현재 HEAD = 11차 freeze** |
| 2026-08-25 | (미커밋) | 11차 — 같은 병(**선언과 집행의 분리**)의 **문서 축**을 닫는 라운드. 머리가 약속한 백업이 집행 단계에 없었고(BLOCK-1), 유일한 백업 단계가 자기 실패를 숨긴 채 원인을 반대로 말했다(MAJOR-2). ★수리마다 '이 검사를 우회하는 두 번째 형태'를 만들어 시험했다(→ 4.15) |

### 1.3 2026-08-20 제거 당시 남아 있던 것

제거된 것은 **호출부뿐**이었다.

- Rust 커맨드 `install_cli_to_path` 는 **무손상 존치**했다 (당시 `src-tauri/src/main.rs:1179`,
  `invoke_handler` 등록 `:3809`).
- 제거된 것은 `ui/index.html` 의 `#cc-header` 버튼 1줄과 `ui/src/main.ts` 의 핸들러 13줄.
- `main.ts:6726` 에 묘비 주석(tombstone comment)이 남아 있었다.

이 사실이 **첫 번째 설계 규약(제5절 ①조)** 의 씨앗이 된다: "무손상 존치"는 감사 면제가 아니다.
존치된 Rust 코드는 한 번도 반증 렌즈를 통과한 적이 없었고, 1차 반증에서 나온 BLOCK 2건 중
1건이 바로 그 존치 코드 안에 있었다.

---

## 2. 확정 설계 D1~D8 (전문)

> 아래는 1라운드 워크플로우의 `DESIGN` 상수 전문이다. master 가 확정한 설계이며 워커에게는
> "임의 변경 금지, 이견은 보고만" 조건으로 주어졌다.

### 배경 (설계서 원문)

'셸에 cys 설치' 버튼은 2026-06-29(1882a5b) 신설 → 2026-08-20(3685af9) 오너 지시로 제거됐다.
Rust 커맨드 `install_cli_to_path` 는 무손상 존치(`src-tauri/src/main.rs:1179`, invoke_handler 등록 `:3809`).
제거된 것은 호출부뿐: `ui/index.html` cc-header 버튼 1줄 + `ui/src/main.ts` 핸들러 13줄.
현재 `main.ts:6726` 에 묘비 주석이 있다.

### D1 [복원]

`ui/index.html` 의 `#cc-header` 안, `#btn-cc-glance-face` 와 `#cc-clock` 사이에
`<button id="btn-install-cli">` 를 되살린다. **원래 위치·원래 id 유지.**

### D2 [수리1 — 플랫폼 게이팅]

**원 결함**: HTML에 플랫폼 분기가 없어 Windows/Linux에서도 렌더됐고, Rust는 macOS 외에서
즉시 `Err("이 기능은 macOS 전용입니다")` → 사용자는 **보이는 버튼**을 누르고 실패 토스트만 받았다.

**수리**: 버튼은 HTML에서 `hidden` 으로 시작하고, `main.ts` 가 macOS 로 판정될 때만 노출한다.
Rust의 non-macOS `Err` 는 심층방어로 존치(제거 금지).

★**Windows 자동 PATH 편집은 구현하지 않는다.** `setx` 1024자 잘림 · `REG_EXPAND_SZ→REG_SZ`
변환으로 `%USERPROFILE%` 류가 깨지는 실사고가 알려져 있다(오너 절대지침 = 윈도우 신중).
대신 Windows에서는 버튼 자리에 아무것도 넣지 않고 문서 안내만 유지한다.

### D3 [수리2 — 등급 분리]

**원 결함**: PATH 선행에 다른 cys가 있어 새 심링크가 무효여도(shadowed) `ok:true` + "설치 완료"
토스트가 떴다. **측정 자체가 실패해도 마찬가지였다.**

**수리**: `InstallCliReport` 에 `status` 필드를 추가한다. 값은 정확히 셋:

| 값 | 의미 |
|---|---|
| `"installed"` | 심링크 생성 + `which -a cys` 1순위 == `/usr/local/bin/cys` |
| `"installed_shadowed"` | 심링크는 생성됐으나 앞을 가리는 다른 cys 존재 |
| `"unverified"` | `which -a` 검증을 수행하지 못함(실행 실패·타임아웃) |

**UI**: `installed` 만 성공 토스트(`"system"`). 나머지 둘은 경고 토스트(`"watchdog"`)로 등급을
낮추고, 무엇이 왜 미완료인지와 다음 조치를 문장으로 준다.

> **헌장 원칙**: "측정 불능은 어떤 게이트에서도 통과가 아니다."

### D4 [수리3 — 해제 경로]

**원 결함**: 설치만 있고 해제가 없어 앱을 지워도 root 심링크가 남았다.

**수리 (a) 새 커맨드 `uninstall_cli_from_path()`**

- `/usr/local/bin/cys`, `/usr/local/bin/cysd` 각각에 대해 **심볼릭 링크인 경우에만** 제거한다.
  일반 파일이면 절대 건드리지 말고 사유와 함께 skip.
  (다른 도구가 설치한 실체 바이너리를 지우는 것은 비가역 파괴다.)
- 링크여도 그 대상 경로가 cys.app 번들 안(`.../cys.app/Contents/MacOS/...`)을 가리킬 때만 제거.
  타 대상이면 skip + 사유.
- 판정은 **순수 함수**로 분리해 단위테스트한다(예: `fn plan_cli_uninstall(...) -> UninstallPlan`).
  파일시스템 접근은 얇은 래퍼로 분리하고, 순수부는 (경로, 심링크여부, 링크대상) 입력만 받는다.
- 삭제 스크립트도 `osascript` 1회 승격(설치와 동일 방식), 셸 인용은 기존 `sh_squote` 재사용.

**수리 (b) 새 읽기전용 커맨드 `cli_install_status()`**

- 승격 없이 두 경로의 심링크 상태만 조사해 installed 여부를 돌려준다.

**수리 (c) UI는 버튼 하나를 상태 2종으로 쓴다**

- Control Center 를 열 때 **1회** `cli_install_status()` 호출.
- 미설치면 라벨 "셸에 cys 설치", 클릭 → `install_cli_to_path`
- 설치됨이면 라벨 "셸 cys 해제", 클릭 → `uninstall_cli_from_path`
- 실행 후 상태를 다시 조회해 라벨을 갱신한다.
- **폴링 금지**(타이머 증식 차단은 이 코드베이스의 기존 원칙). CC 열림 시점 1회 + 액션 후 1회만.

### D5 [선택4 — NonStandard 거부 승격]

현재 `plan_cli_install` 은 번들이 표준 위치가 아니면 경고만 하고 진행한다. root 소유 심링크가
사용자 쓰기 가능한 임의 경로(`~/Downloads` 등)를 가리키게 되므로 **거부로 승격**한다.
메시지는 "Applications 로 옮긴 뒤 다시 시도" 안내.

기존 테스트 `plan_cli_install_warns_on_nonstandard_but_proceeds` 는 거부를 검증하도록 갱신하고,
테스트 이름도 의미에 맞게 바꾼다.

### D6 [선택5 — 타임아웃]

`bash -lc "which -a cys"` 가 무기한 매달릴 수 있다(로그인 셸 rc 지연). Rust std 에는 타임아웃이
없으므로 spawn + `try_wait` 폴링 + 기한 초과 시 kill 하는 작은 헬퍼를 만들어 **5초 기한**을 건다.
초과·실패 시 `status = "unverified"`.

### D7 [문서]

`3685af9` 가 되돌린 문서를 현재 구현에 맞게 다시 정합한다.

- `docs/INSTALL.md` §B: "1클릭 버튼(권장) + 수동 sudo(폴백)" 로. **해제 방법도 추가.**
- `USER-MANUAL.md` §2.4: macOS 는 버튼 안내 + 해제 안내. Windows 문장은 현행 유지
  (설치기가 PATH 를 구성하지 않는다는 **정정된 사실을 훼손하지 말 것**).
- `docs/INSTALL.md` 의 `INST-DENY-02` 행: GUI 버튼은 사용자 명시 클릭 + osascript 1회 승격이라
  이 경계를 위반하지 않으나 **에이전트가 자율로 누르는 것은 여전히 금지**임을 한 문장 보강.

### D8 [테스트]

새로 만든 **순수 함수 전부**에 단위테스트를 추가한다. 기존 `src-tauri` 테스트는 전부 통과해야 한다.
UI 순수 로직(상태→라벨 매핑, status→토스트 등급 매핑)은 `ui/src/` 에 별도 `.ts` 로 뽑아 `bun test`
로 검증한다(이 저장소의 기존 관행 — `shellquote.ts`/`shellquote.test.ts` 처럼 순수 함수는 파일
분리 후 테스트).

---

## 3. IPC 계약 정본

> **작성 근거**: `src-tauri/src/main.rs` 와 `ui/src/__contract__.json` **실물** @ `0b6cb24`(= 8라운드
> freeze 리비전). 아래 line 번호는 전부 그 리비전 기준이다.
>
> ★**이 절은 실물에서 유도했다.** 4라운드까지 이 절은 `9f47148` 기준으로 적혀 있었고, 그 뒤
> 4·5·7라운드가 필드를 넷 더 만드는 동안 갱신되지 않았다 — `skipped_reasons`·`skipped_benign`·
> `restored`·`backups` 가 표에서 통째로 빠져 있었고, 275행은 `skipped_benign` 을 "아직 실물에 없다"
> 고 적고 있었다(실물에는 있다). **머리말이 스스로 "실물이 바뀌면 같은 커밋에서 갱신한다"고
> 선언해 놓고 그 규칙을 자기가 어긴 것**이며, 8라운드 MAJOR-4(a)가 그 수리다.
>
> **serde rename 없음** — Rust 필드명이 그대로 snake_case 로 JavaScript 에 노출된다.
> 세 커맨드 모두 Rust 시그니처는 `async fn … -> Result<T, String>` 이며, Tauri `invoke` 에서 `Err` 는
> **Promise reject** 로 도착한다(TS 는 반드시 try/catch 로 감싼다).

### 3.0 커맨드 3종 요약

| 커맨드 | 선언 위치 | 비동기 | 승격(관리자 비밀번호) | 호출 시점 |
|---|---|---|---|---|
| `install_cli_to_path()` | `main.rs:2253` | **`async fn`** | osascript 1회 | 버튼 클릭(설치 라벨) |
| `uninstall_cli_from_path()` | `main.rs:2872` | **`async fn`** | osascript 1회 (지울 것 있을 때만) | 버튼 클릭(해제 라벨) |
| `cli_install_status()` | `main.rs:3048` | **`async fn`** | **없음** (읽기 전용) | CC 열림 1회 + 액션 직후 1회 |

> **`async fn` 이 계약의 일부인 이유**(7라운드 MAJOR-3 · 주석 원문 `main.rs:3029-3046`)
> `#[tauri::command]` 매크로는 함수에 `async` 가 **없으면** wrapper 를 `ExecutionContext::Blocking`
> 으로 만든다. Blocking 은 "별도 스레드를 띄우지 않고 IPC 핸들러 스레드(macOS 에서는 메인
> 스레드)에서 본문을 그대로 돌린다"는 뜻이고, 이 셋은 전부 오래 막힌다 — 상태 조회는 로그인 셸
> `which -a`(기한 5초, `-lc` 폴백 재시도까지 최대 10초), 설치·해제는 **기한이 아예 없는** 관리자
> 승인 창 대기. 즉 동기로 되돌리면 사용자가 비밀번호 창을 그대로 두는 동안 **앱 전체가 멎는다.**
> 그래서 세 함수의 `async` 는 취향이 아니라 계약이다 — 되돌리지 마라.
> ★단, `async` 는 **동시 진입을 허용한다**는 뜻이기도 하다. 그 대가와 그것을 막는 장치는 §3.6.

### 3.1 `install_cli_to_path()` → `InstallCliReport`

정의: `main.rs:2226-2245` (`#[derive(serde::Serialize)] struct InstallCliReport`) · 필드 **10개**

| 필드 | Rust 타입 | JSON 타입 | null 허용 | 설명 | TS 판독기(`readInstallReport`, `clipath.ts:160`)가 접는 방식 |
|---|---|---|---|---|---|
| `ok` | `bool` | boolean | 불가 | `status == "installed"` 의 **파생값**(`main.rs:2370`). 두 개의 진실을 만들지 않는다. 부분 성공(그림자·측정불능)은 `false`. | `r.ok === true` — 아니면 전부 `false` |
| `status` | `String` | string | 불가 | **enum 3값** — 아래 3.1.1 | `normalizeInstallStatus`(`clipath.ts:183`) — 계약 밖 값·누락은 전부 `"unverified"` |
| `target_dir` | `String` | string | 불가 | 항상 `"/usr/local/bin"` | `str()` — 문자열 아니면 `""` |
| `cys_link` | `String` | string | 불가 | `"/usr/local/bin/cys"` | `str()` |
| `cysd_link` | `String` | string | 불가 | `"/usr/local/bin/cysd"` | `str()` |
| `source_cys` | `String` | string | 불가 | 링크가 가리키는 번들 안 실행파일 절대경로 | `str()` |
| `effective_cys` | `Option<String>` | string \| null | **허용** | `which -a cys` 1순위. `unverified` 두 분기에서는 `null`. | `strOrNull()` |
| `shadowed_by` | `Option<String>` | string \| null | **허용** | `/usr/local/bin/cys` 앞을 가리는 다른 cys. `installed_shadowed` 에서만 `Some`. | `strOrNull()` |
| `unverified_reason` | `Option<String>` | string \| null | **허용** | **enum 2값 + null** — 아래 3.1.2. 계약 v2(3라운드). | `strOrNull()` → `unverifiedCause`(`clipath.ts:210`)가 계약 밖·null 을 `"unknown"` 으로 |
| `warnings` | `Vec<String>` | string[] | 불가(빈 배열 가능) | 사람용 설명 문장. **계약이 아니다** — 정규식 파싱 금지. | `strList()` — 문자열 아닌 원소는 버린다 |

#### 3.1.1 `status` enum 전체 값 (정확히 3개)

판정처: `classify_install_status`(`main.rs:2096-2149`)

```
"installed"           심링크 생성 + 로그인 셸 기준 `which -a cys` 1순위가 /usr/local/bin/cys 와 같다
                      (★4라운드 I1 이후 '문자열 완전일치'가 아니라 paths_equivalent 정규화 비교다)
"installed_shadowed"  심링크는 생겼으나 PATH 앞을 가리는 다른 cys 가 있다
"unverified"          확인을 못 했다(probe_failed) 또는 로그인 셸 PATH 에서 cys 를 못 찾았다(not_on_path)
```

계약 밖의 값·필드 누락은 TS 판독기가 전부 `"unverified"` 로 접는다
(`ui/src/clipath.ts:183` `normalizeInstallStatus`) — 구버전 백엔드와 붙어도 성공으로 둔갑하지
않게 하는 장치다.

#### 3.1.2 `unverified_reason` enum 전체 값 (정확히 2개 + null)

Rust 상수 정의: `main.rs:2060` / `main.rs:2063`

```
"not_on_path"    검증 명령이 정상 종료했고 로그인 셸 PATH 에서 cys 를 못 찾았다 (원인 = PATH 구성)
"probe_failed"   검증 명령 자체를 못 돌렸다 — 실행 실패·비정상 종료·타임아웃
null             status 가 "unverified" 가 아니다
```

TS 는 값이 없거나 계약 밖이면 `"unknown"` 으로 접는다(`clipath.ts:210` `unverifiedCause`) —
**모르면 모른다고 말한다.**

#### 3.1.3 상태 불변식 (`classify_install_status`, `main.rs:2096-2149`)

| `status` | `unverified_reason` | `effective_cys` | `shadowed_by` | 판정이 내는 `warnings` |
|---|---|---|---|---|
| `installed` | `null` | `Some(first)` | `null` | 빈 배열 |
| `installed_shadowed` | `null` | `Some(first)` | `Some(first)` | 1건 |
| `unverified` | `"not_on_path"` | `null` | `null` | 1건 |
| `unverified` | `"probe_failed"` | `null` | `null` | 1건 |

★`unverified_reason` 은 `status == "unverified"` 일 때만 `Some` 이고, 그 외에는 **반드시** `None`.

★위 표는 **판정 함수가 내는 것**이고, 커맨드는 그 위에 다음 세 종류를 더 얹는다
(`main.rs:2336-2367`) — 그래서 `installed` 인데도 `warnings` 가 비어 있지 않을 수 있다:
① 백업 사실 통보(`{원본}에 우리 것이 아닌 파일/링크가 있어 … {백업본}로 백업한 뒤 링크를 만들었습니다`)
② 백업을 계획했는데 사실로도 관측으로도 확인하지 못했다는 **모른다** 고지
③ cysd 그림자 경고(`cysd_shadow_warning`, `main.rs:2167`).

#### 3.1.4 `Err` 반환값 전체 (Promise reject) — 정확히 9종

| 문자열 | 발생 위치 | 언제 |
|---|---|---|
| `"이 기능은 macOS 전용입니다."` | `main.rs:2256` | non-macOS 심층방어 |
| (OS 오류 문자열 그대로) | `main.rs:2261` | `current_exe()` 실패 |
| `"번들 디렉토리 해석 실패"` | `main.rs:2264` | `current_exe().parent()` 실패 |
| `"cys.app이 Gatekeeper에 의해 임시 위치에서 실행 중입니다. …"` | `main.rs:1896` | translocation |
| `"백업 번들에서 실행 중입니다. …"` | `main.rs:1901` | 백업 번들 |
| `"cys.app이 표준 위치(Applications)가 아닌 곳에서 실행 중입니다: {bundle}\n…"` | `main.rs:1917` | D5 1차 거부 |
| `"cys.app이 표준 Applications 폴더가 아닌 곳에서 실행 중입니다: {bundle}\n…"` | `main.rs:1934` | D5 엄격 판정(`strict_install_bundle_ok`) 거부 |
| `"번들 내 cys/cysd 바이너리를 찾지 못했습니다."` | `main.rs:2272` | 번들 안 실행파일 부재 |
| `"osascript 실행 실패: {e}"` | `main.rs:2288` | 승격 프로세스 기동 자체 실패 |
| `install_failure_message("설치가 취소되었습니다.", …)` | `main.rs:2315` | 사용자가 비밀번호 창을 취소(`-128`·`User canceled`) |
| `install_failure_message("심볼릭 생성 실패: {stderr}", …)` | `main.rs:2320` | 승격 스크립트 rc != 0 |

> 마지막 두 줄의 `install_failure_message`(`main.rs:1704`)는 **실패하기 전에 이미 옮겨진 파일**이
> 있으면 그 목록(`원본 → 백업본`)을 에러 문구 뒤에 붙인다. 3라운드 MAJOR-N1 수리다 — 그전에는
> 남의 실체 바이너리가 `.cys-backup-<epoch초>` 라는 추측 불가능한 이름으로 옮겨졌는데 그 사실이
> 어디에도 남지 않았다.

### 3.2 `uninstall_cli_from_path()` → `UninstallCliReport`

정의: `main.rs:2842-2861` · 필드 **7개** (4라운드에 3개 늘었다 — 이 절이 4라운드에서 멈춰 있던 부분)

| 필드 | Rust 타입 | JSON 타입 | null 허용 | 설명 | TS 판독기(`readUninstallReport`, `clipath.ts:876`) |
|---|---|---|---|---|---|
| `ok` | `bool` | boolean | 불가 | 계획한 제거가 **전부 실측으로 확인**됐는가(`removed.len() == plan.remove.len()`, `main.rs:2988`). 지울 것이 없었던 경우도 `true`. | `r.ok === true` |
| `removed` | `Vec<String>` | string[] | 불가(빈 배열 가능) | 사후 재관측으로 **정말 사라진** 경로만 담는다(산출자의 자기신고를 믿지 않는다). 원본이 복원된 자리는 '사라짐'으로 센다(`main.rs:2955`). | `strList()` |
| `skipped` | `Vec<String>` | string[] | 불가(빈 배열 가능) | ★**문자열 배열이다.** `"경로 — 사유"` 형식. **객체 배열이 아니다.** 사람용 설명이며 **등급 판정에 쓰지 않는다.** | `strList()` |
| `skipped_reasons` | `Vec<String>` | string[] | 불가(빈 배열 가능) | ★C3(4라운드 신설) `skipped` 와 **인덱스 1:1 대응**하는 기계 태그. enum 3값 — 아래 3.2.1 | `strList()` |
| `skipped_benign` | `bool` | boolean | 불가 | ★C3(4라운드 신설) **해제 등급의 유일 계약**: 건너뛴 것이 전부 `absent`(= 지울 게 없었다) 인가. skip 이 하나도 없으면 `true`. 판정: `all_skips_benign`(`main.rs:2605`) | `r.skipped_benign === true` |
| `restored` | `Vec<String>` | string[] | 불가(빈 배열 가능) | ★I3③(4라운드 신설) 해제하면서 **되돌린 원본**의 경로(설치 때 백업해 둔 것). 없으면 빈 배열. | `strList()` |
| `warnings` | `Vec<String>` | string[] | 불가(빈 배열 가능) | 사람용. 성격이 다른 셋이 섞인다 — ①아직 남아 있는 경로 ②복원 통보 ③잔존 백업 고지. **그래서 `warnings.length > 0` 을 실패 신호로 읽으면 안 된다**(`clipath.ts:895-903`). | `strList()` |

> **`skipped` 가 문자열인 이유와, 이 한 줄이 만든 사고**
> 1차 구현의 TS 는 이것을 `{path, reason}[]` 로 읽고 `.filter(s => s && s.path)` 로 걸렀다.
> 객체가 아니므로 `s.path` 는 항상 `undefined` → **모든 skip 이 소멸**했고, 부분 실패가 성공
> 토스트로 둔갑했다. `warnings`·`ok` 는 TS 타입에 아예 없었고, 그 결과 유일한 복구 명령이
> 사용자에게 **도달하지 못했다.** 단위테스트가 초록이었던 이유는 픽스처가 Rust 실물이 아니라
> **잘못된 TS 모양**을 먹였기 때문이다(→ 규약 ③·⑨).

> **왜 `skipped_benign` 이 따로 있는가**(C3 · 4라운드)
> 그전 TS 는 "이미 해제" 라는 **Rust 산문을 정규식으로 파싱**해 등급을 정했다. Rust 가 문구를 한
> 단어만 다듬으면 정상 해제가 조용히 '⚠부분 완료'로 오보고된다. 설치 경로의 `unverified_reason`
> 과 정확히 같은 원리로 기계 필드로 올렸다(→ 규약 ⑪).

#### 3.2.1 `skipped_reasons` enum 전체 값 (정확히 3개)

Rust 상수: `main.rs:2585` / `:2557` / `:2559` · 판정: `skip_reason_tag`(`main.rs:2593`)

```
"absent"          그 자리에 아무것도 없다 = 지울 게 없었다 (무해)
"not_symlink"     심볼릭이 아닌 실체 파일이 있다 — 남의 설치본일 수 있어 건드리지 않는다
"foreign_target"  심볼릭이지만 cys.app 번들 밖을 가리킨다 — 남의 링크다
```

`Remove`(우리 것이니 지운다)는 skip 이 아니므로 이 배열에 들어가지 않는다. 따라서
`skipped_reasons.length == skipped.length` 이고 인덱스가 1:1로 맞는다.

#### 3.2.2 `Err` 반환값 전체 (정확히 4종)

| 문자열 | 발생 위치 | 언제 |
|---|---|---|
| `"이 기능은 macOS 전용입니다."` | `main.rs:2875` | non-macOS 심층방어 |
| `"osascript 실행 실패: {e}"` | `main.rs:2905` | 승격 프로세스 기동 자체 실패 |
| `uninstall_failure_message("해제가 취소되었습니다.", …)` | `main.rs:2935` | 사용자 취소(`-128`·`User canceled`) |
| `uninstall_failure_message("심볼릭 제거 실패: {stderr}", …)` | `main.rs:2942` | 승격 스크립트 rc != 0 |

`uninstall_failure_message`(`main.rs:2680`)는 실패 문구 뒤에 세 목록을 붙인다 — **이미 제거된
것 / 이미 되돌린 것 / 아직 남아 있는 것**. C2(4라운드) 수리이며, 설치 쪽 MAJOR-N1 수리와
**같은 형태**다(계열 — 규약 ⑬).

#### 3.2.3 승격을 띄우지 않는 경로

`plan_cli_uninstall`(`main.rs:2612`)이 `osascript_arg = None` 을 내면(지울 것이 하나도 없음)
**비밀번호 창을 띄우지 않고** 즉시 반환한다(`main.rs:2888-2899`):
`ok:true, removed:[], skipped:plan.skipped, skipped_reasons:plan.skipped_reasons,
skipped_benign, restored:[], warnings:[]`.

### 3.3 `cli_install_status()` → `CliInstallStatusReport`

정의: `main.rs:2999-3022` · 필드 **7개** (4라운드에 `backups` 가 늘었다)

| 필드 | Rust 타입 | JSON 타입 | null 허용 | 설명 | TS 판독기(`readCliStatus`, `clipath.ts:410`) |
|---|---|---|---|---|---|
| `platform_supported` | `bool` | boolean | 불가 | macOS 전용 기능. UI 는 이 값 하나로 버튼 노출 여부를 정할 수 있다(`false` 면 숨김). | `r.platform_supported !== false` (모르면 지원한다고 본다 — 버튼을 잃지 않기 위해) |
| `installed` | `bool` | boolean | 불가 | `true` 면 라벨 '해제', `false` 면 '설치'. `state ∈ {ours, partial}` 의 파생값(`main.rs:3103`). | boolean 이 아니면 `"unknown"` 상태로 접는다 |
| `state` | `String` | string | 불가 | **enum 5값** — 아래 3.3.1 | `LINK_STATES`(`clipath.ts:405`)에 없으면 `"unknown"` |
| `cys_link` | `String` | string | 불가 | `"/usr/local/bin/cys"` | `str()` |
| `cysd_link` | `String` | string | 불가 | `"/usr/local/bin/cysd"` | `str()` |
| `notes` | `Vec<String>` | string[] | 불가(빈 배열 가능) | 사용자 고지용 **사람용 문구**. 아래 3.3.2 — 5라운드에 그림자 관측이 합류해 종류가 늘었다. | `strList()` |
| `backups` | `Vec<String>` | string[] | 불가(빈 배열 가능) | ★I3①(4라운드 신설) `/usr/local/bin` 에 남아 있는 **우리 백업본 전체 경로**(기계 필드). 아래 3.3.3 | `strList()` |

#### 3.3.1 `state` enum 전체 값 (정확히 5개)

`CliLinkState`(`main.rs:2772`) 네 갈래 + non-macOS 전용 문자열 하나. 문자열 변환은 `main.rs:3104-3110`.

```
"absent"       우리 링크가 하나도 없다              → 라벨 "셸에 cys 설치"
"ours"         cys·cysd 둘 다 우리 번들 심볼릭       → 라벨 "셸 cys 해제"
"partial"      한쪽만 우리 것(중단된 설치·부분 삭제 잔재) → 해제로 청소 가능. installed=true
"foreign"      파일은 있으나 우리 것이 아니다(실체 파일·타 대상 링크) → 설치 라벨 유지 + notes 고지
"unsupported"  non-macOS. Rust 가 Err 를 던지지 않고 이 값으로 답한다(main.rs:3057).
```

> **non-macOS 에서 `Err` 를 던지지 않는 이유**(주석 `main.rs:3024-3027`)
> Control Center 를 열 때마다 실패 토스트가 뜨기 때문이다. 대신 `platform_supported=false`
> 로 답한다. `install`/`uninstall` 쪽 non-macOS `Err` 는 심층방어로 **존치**한다.

> **★`cli_install_status` 는 `Err` 를 한 번도 반환하지 않는다.** 시그니처는
> `Result<CliInstallStatusReport, String>` 이지만 본문의 모든 반환 경로가 `Ok` 다
> (`main.rs:3048-3116` 전수 확인). 반환 타입은 Tauri 커맨드 관례를 따른 것이다.

#### 3.3.2 `notes` 문장 형식 (정확히 4종)

앞의 둘은 링크 자체를 보고 낸다(`main.rs:3068-3082`):

- 실체 파일: `"{경로} — 심볼릭이 아닌 실제 파일이 이미 있습니다(다른 도구 설치본일 수 있어 자동으로 제거하지 않습니다)."`
- 타 대상 링크: `"{경로} — cys.app 번들 밖({대상})을 가리키는 링크입니다."` (대상을 못 읽으면 `대상 읽기 실패`)

뒤의 둘은 ★G4(5라운드) 신설 — **상태 조회에도 PATH 그림자 관측을 넣었다**(`main.rs:3088-3099`).
4라운드까지 그림자 관측은 설치 경로에만 있어서, "cysd 가 다른 곳에서 가려진다"는 사실은 설치
직후 토스트 한 번뿐이었고 그것을 놓친 사용자는 데몬 버전이 어긋나는 이유를 다시는 알 수 없었다.

- `path_shadow_note`(`main.rs:2202`)가 내는 cys 축 고지
- `cysd_shadow_warning`(`main.rs:2167`)이 내는 cysd 축 고지

★비용 통제: 링크가 하나도 없으면(`absent`·`foreign`) 잴 대상 자체가 없으므로 **셸을 띄우지
않는다**(`main.rs:3088` `matches!(state, Ours | Partial)`). 읽기전용 조회에 로그인 셸 1회(기한
5초)를 무는 비용은 잴 것이 있을 때만 낸다.

★이 문장들은 **사람용**이다. UI 는 그대로 표시하되 **파싱해서 분기하지 않는다**(규약 ⑪).

#### 3.3.3 `backups` 이름 규칙과 그 존재 이유

- 생성: `backup_path_for`(`main.rs:1108`) = `format!("{target}.cys-backup-{stamp}")`
- 스탬프: `backup_stamp`(`main.rs:1097`) = **UNIX epoch 초**(정수). Rust 가 생성해 스크립트에 박는다
  — 셸에서 `date` 를 부르면 실제 파일명과 사용자에게 보고하는 이름이 갈라진다.
- 인정 판정: `is_our_backup_name`(`main.rs:2497`) — `"{base}.cys-backup-"` 접두를 떼고 남은 것이
  **비어 있지 않고 전부 ASCII 숫자**여야 우리 것이다.
- 관측: `observe_leftover_backups`(`main.rs:2542`) — `/usr/local/bin` 을 훑어 `cys`·`cysd` 두
  base 이름에 대해 위 규칙에 맞는 파일만 전체 경로로 담는다.
- 복원 후보 선택: `pick_restore_backup`(`main.rs:2509`) — 여러 개면 **스탬프가 가장 큰(최신)** 것.

> **왜 기계 필드인가**: BLOCK-1 이 "확인 모달 없는 1클릭"을 정당화한 근거는 **"잃는 것이 없다"**
> 였다. 그런데 백업본을 알리는 유일한 통로가 60초짜리 sticky 토스트뿐이었고(수용처
> `alarmHistory` 는 메모리 전용), 상태 조회는 `*.cys-backup-*` 를 보지도 않았다 — 토스트를 놓치면
> 사용자는 자기 파일이 어디로 갔는지 **다시는** 알 수 없었다. 이제 상태 조회가 상시로 들고 온다.
> 되돌리기 명령 **문장**은 표현이므로 UI 소유다(I7 의 '백엔드는 사실만').

### 3.4 계약을 지키는 장치 (드리프트 방지) — 그리고 그 장치의 한계

| 층위 | 장치 | 위치 | 상태 |
|---|---|---|---|
| Rust → JSON | 세 리포트를 실제로 직렬화해 **키 집합 + 타입 태그**를 `ui/src/__contract__.json` 으로 덤프 | `main.rs:8850` `dump_report_contract_for_the_ui_gate` | 4라운드 I6 로 **완료** |
| TS 판독기 | `readInstallReport`·`readUninstallReport`·`readCliStatus` — 모르는 값은 **안전한 쪽으로** 접는다 | `clipath.ts:160·876·410` | 완료 |
| TS 게이트 | `expectShape` 가 손으로 쓴 표가 아니라 **덤프 파일**을 기준으로 삼는다 | `clipath.test.ts` | 완료 |
| TS 테스트 | '정규식 재도입 차단' — TS 가 `warnings` 문구를 파싱하지 않는다는 것을 못박는다 | `clipath.test.ts` | 완료 |

**I6 검수 실험 결과(4라운드에 실제로 수행)**: 아무 필드에 `#[serde(rename=…)]` 를 붙이면 덤프
파일의 키가 바뀌고 TS 게이트가 빨개진다 — 단, **덤프(쓰기)가 자기 단언(assert)보다 먼저 와야
한다.** 실측으로 확인한 함정을 코드 주석(`main.rs:8948-8951`)에 남겼다:

> 단언을 쓰기 **앞**에 두면 rename 사고가 Rust 층에서 멈춰 파일이 낡은 채로 남고, **TS 게이트는
> 초록으로 통과한다** — 거울이 아니라 필터가 된다.

★**그러나 이 장치는 두 층이 같은 트리에서 이어서 돌 때만 작동한다.** Rust 가 파일을 다시 쓰고
TS 가 그 파일을 읽어야 비로소 드리프트가 드러나는데, **어떤 CI 레인에서도 그 두 층이 함께 돌지
않는다.** 이 공백의 전모와 닫는 방법은 **§7(CI 레인 지도와 공백)** 에 적었다 — 8라운드
MAJOR-4(c)의 산출물이다.

### 3.5 `ui/src/__contract__.json` 실물 전문 (`0b6cb24` 이후 무변경 — 10라운드 작업트리와 동일)

이 파일은 **손으로 고치지 않는다** — `cargo test -p cys-app` 이 덮어쓴다.

```json
{
  "CliInstallStatusReport": {
    "backups": "string[]",
    "cys_link": "string",
    "cysd_link": "string",
    "installed": "boolean",
    "notes": "string[]",
    "platform_supported": "boolean",
    "state": "string"
  },
  "InstallCliReport": {
    "cys_link": "string",
    "cysd_link": "string",
    "effective_cys": "string|null",
    "ok": "boolean",
    "shadowed_by": "string|null",
    "source_cys": "string",
    "status": "string",
    "target_dir": "string",
    "unverified_reason": "string|null",
    "warnings": "string[]"
  },
  "UninstallCliReport": {
    "ok": "boolean",
    "removed": "string[]",
    "restored": "string[]",
    "skipped": "string[]",
    "skipped_benign": "boolean",
    "skipped_reasons": "string[]",
    "warnings": "string[]"
  },
  "_contract": "키 집합 + 타입 태그. TS 게이트(ui/src/clipath.test.ts)의 expectShape 기준. 손으로 고치지 말 것 — cargo test 가 덮어쓴다.",
  "_generated_by": "src-tauri/src/main.rs :: dump_report_contract_for_the_ui_gate"
}
```

> **`"string|null"` 이 나오려면 표본이 둘 필요하다.** `Option` 필드는 `Some` 표본 하나만 직렬화하면
> `"string"` 으로, `None` 표본 하나만 하면 `"null"` 로 굳어 **계약이 좁아진다.** 그래서 덤프
> 테스트는 `InstallCliReport` 표본을 두 개(`Some`/`None`) 만들어 합집합한다(`main.rs:8891-8918`).

### 3.6 `async` 가 연 문 — 동시성 계약 (7라운드 MAJOR-3 의 대가)

`async fn` 전환은 "관리자 승인 창이 떠 있는 동안 앱이 멎는" 결함을 닫았다. 그 대가로 **같은
커맨드가 동시에 여러 번 진행될 수 있는 문**이 열렸다. 7라운드는 주석에 "새 동시성도 실질적으로
열리지 않는다"고 적었는데, 8라운드 판정자가 그 단정을 **반증**했다(finding MAJOR, `main.rs:3044`):

- 설치·해제는 버튼 `disabled` 로 이중 클릭이 막힌다 — 이 근거는 맞다.
- 그러나 **상태 조회의 실제 호출부는 버튼이 아니다.** `main.ts` 의 `setCcOpen(open=true)` 안
  `void refreshCliInstallState()` 이고, Control Center 를 여닫는 진입점이 여럿이며(토글 버튼·
  명령 팔레트 등) **in-flight 가드가 없다.**
- 동기였을 때는 IPC 핸들러 스레드가 직렬화하고 그동안 UI 가 멎어 재클릭 자체가 불가능했다.
  `async` 이후에는 CC 를 빠르게 여닫는 만큼 로그인 셸(`$SHELL -lc 'which -a …'`, 기한 5초·폴백
  재시도까지 최대 10초)이 **동시에 뜬다.**
- `cliStatus = readCliStatus(await invoke(...))` 에 요청 세대 구분이 없어 **last-writer-wins** 다.
  늦게 도착한 낡은 프로브가 액션 직후 재조회 결과를 덮으면 결과 토스트에 **낡은 고지 줄**이
  접혀 들어가고 버튼 라벨도 낡은 상태로 되돌아간다.

★**계약**: 이 세 커맨드는 `async` 를 유지하되, **중복 진입 억제는 호출부(UI)의 책임**이다.
8라운드가 그 배선을 닫는다(별도 담당). 타이머·폴링을 새로 만드는 방식은 이 코드베이스의 상시
원칙(타이머 증식 차단)에 어긋나므로 채택하지 않는다.


## 4. 라운드 이력

> ★**이 절의 `파일:줄` 좌표를 읽는 법.** 판정자가 적은 좌표(4.11 의 12건 표 등)는 **그 라운드의
> freeze 리비전 기준**이고, 그대로 둔다 — 판정 기록을 나중 좌표로 덮으면 "무엇을 보고 그렇게
> 판정했는가"가 사라진다. 반면 `현 main.rs:NNN` 이라고 적은 것은 **현행 좌표**이며 10라운드에
> 재유도했다. 지금 코드를 찾을 때는 **부록 B**(10라운드 작업트리 기준)를 쓰는 것이 가장 안전하다.

각 라운드는 이렇게 돌았다: **구현/수리 → 게이트(cargo test · bun test · tsc · 번들) → 독립
반증 → master 판정 → 다음 라운드**. 반증자는 "이 구현은 틀렸다"를 기본 자세로 두고,
틀렸음을 입증하지 못할 때만 물러선다. 근거는 `file:line` 으로 대며, 실행 가능한 반증
(테스트 실행·코드 추적)을 우선한다.

★**반증자 구성은 라운드마다 달랐다** — 1~3차는 아래 **3렌즈**(같은 질문지를 나눠 든 세 반증자),
4차 이후는 **이종(異種) 판정자**(서로 다른 모델·서로 다른 접근)로 옮겼다. 8차는 이종 판정자
2인이 정지한 리비전 `0b6cb24` 를 봤다. 이 변화의 이유는 4.14 ②에 적었다.

★**게이트 초록은 착수 조건이지 합격 기준이 아니다**(규약 ⑧). 1차가 그것을 증명했다 —
모든 게이트가 초록인 상태에서 BLOCK 2건이 나왔다.

**1차 반증 3렌즈**(1라운드 워크플로우 `LENSES` 상수):

| 렌즈 | 무엇을 물었나 |
|---|---|
| `irreversible` (비가역 파괴) | uninstall 이 사용자 실체 파일을 지울 수 있는 모든 경로. 심링크 판정이 TOCTOU 로 뚫리는가? 링크 대상 판정이 문자열 비교라면 우회 가능한가(`../` 포함, 대소문자, 심링크 체인, `/Applications` 접두 흉내)? root 권한 스크립트의 인용이 뚫리는가? **설치 경로도 같은 기준으로 보라.** |
| `windows-boot` (Windows·부트 사슬) | Windows 빌드에서 컴파일되는가? `#[cfg(target_os)]` 누락은? 미등록 커맨드 invoke 로 CC 가 깨지는가? CC 열림 시 추가된 1회 호출이 실패하면 CC 전체가 무반응인가? 타이머·폴링이 새로 생겼는가? 오너가 지목한 4대 치명 위험(에이전트 폭주 큐 남발 / 컨텍스트 무clear / 주기 자가치유 전멸 / pane 전멸)에 닿는가? |
| `contract-honesty` (계약과 정직성) | status 3값이 실제로 진실을 말하는가? 버튼 라벨이 실제 상태와 어긋나는 race 가 있는가? 문서가 코드와 어긋난 문장이 남았는가? 기존 테스트를 "통과시키려고" 의미를 훼손하며 고친 흔적은? **테스트가 실제 결함을 잡는가, 자기 구현을 베낀 동어반복인가?** |

### 4.1 1차 — 게이트는 전부 초록, 반증은 21건(BLOCK 2 · MAJOR 10)

1차 구현(`0e60b79`)은 `cargo test 54/54` · `bun test 458/458` · `tsc 0` · 번들 성공으로
**모든 게이트가 초록**이었다. 그 상태에서 독립 3렌즈가 21건을 찾았다.

| 대표 결함 | 그것이 드러낸 **설계 결함** |
|---|---|
| **BLOCK-1 — 설치가 남의 실체 파일을 root 권한 `ln -sf` 로 말없이 파괴**. 같은 커밋의 해제 경로는 그 파일을 "남의 설치본 파괴이고 되돌릴 수 없다"(당시 `main.rs:1400`)며 신성불가침으로 지키고 있었다. **가드가 정반대였다.** 게다가 `cli_install_status` 가 그 상태를 이미 탐지해 `notes` 로 담는데(당시 `:1663`) UI 는 `notes` 를 **한 글자도 읽지 않았다**(당시 `clipath.ts:93` 타입에 필드 자체가 없음). | 설계서가 **쓰기·삭제·덮어쓰기 경로에 같은 파괴 가드 표를 요구하지 않았다.** 해제 쪽만 "비가역"으로 인식하고 설치 쪽의 `ln -sf` 가 같은 파괴라는 것을 못 봤다. → **규약 ②** |
| **MAJOR-3 — D6 타임아웃이 실제로 hang 을 막지 못했다**(3렌즈 중 3개가 독립 지적). kill 후 드레인 스레드를 **타임아웃 없이** join 했다(당시 `:1222`). `read_to_string` 은 파이프 write-end 가 전부 닫혀야 EOF 를 본다 — bash 가 띄운 손자(ssh-agent·gpg-agent 류)가 stdout 을 물고 있으면 부모를 kill 해도 EOF 가 오지 않는다. **실측 재현: 기한 5초에 12.02초 블록.** 기존 테스트가 초록이었던 이유는 `sh -c "sleep 30"` 이 exec 대체되어 kill 이 그대로 먹는 **유일한 형태**였기 때문. | 설계서가 D6 을 "**5초 기한을 건다**"는 HOW 로만 적고 **관측 가능한 수용 조건**("어떤 자식 구조에서도 T+ε 안에 반환한다")을 적지 않았다. HOW 를 무효화하는 조건(손자 프로세스가 파이프를 상속)을 함께 적지 않았다. → **규약 ④·⑨** |
| **MAJOR-5 — UI↔Rust 계약 드리프트**(3렌즈 중 3개 지적). Rust `skipped: Vec<String>` ↔ TS `{path,reason}[]`. `.filter(s => s.path)` 로 **모든 skip 소멸**, 부분 실패가 성공으로 둔갑. `warnings`·`ok` 는 TS 타입에 아예 없었다. 테스트가 초록이었던 이유는 **픽스처가 Rust 실물이 아니라 잘못된 TS 모양**을 먹였기 때문 — **테스트가 드리프트를 봉인했다.** | ★**이 드리프트의 원인은 워커가 아니라 master 다**: 설계서가 `uninstall`·`status` 의 **반환 형태를 못박지 않은 채** 두 에이전트를 병렬로 보냈다. 병렬 작업의 IPC 경계에 이름 3개와 enum 1개만 준 것은 계약이 아니라 **추측 허가**였다. → **규약 ③·⑨** |
| **MINOR-6 — D5 거부 근거가 실제로는 성립하지 않는다.** `classify_bundle_dir` 이 `parent.ends_with("/Applications")` 라 `/tmp/Applications` · `~/Downloads/Applications` 같은 사용자 생성 디렉터리도 Canonical 로 통과했다(당시 `:801`). | 설계서가 "**경고를 거부로 올린다**"고만 하고 **새로 거부되는 집합을 전수 열거하지 않았다.** 그 결과 판정 함수가 실제로 무엇을 걸러내는지 아무도 확인하지 않았다. → **규약 ⑩** |

**master 판정**: 21건 중 **12건**(BLOCK 1 · MAJOR 4 · MINOR 7)을 수리 대상으로 확정해 2라운드
`FINDINGS` 로 넘겼다. BLOCK-1 의 수리 방식도 master 가 확정했다 — **파괴하지 말고 백업한다**:
승격 스크립트가 대상 경로마다 "심링크가 아닌 무언가가 존재하면
`<경로>.cys-backup-<타임스탬프>` 로 `mv` 한 뒤 링크를 만든다". 타임스탬프는 **Rust 가 생성해
스크립트에 박는다**(셸에서 `date` 호출 금지 — 결정론). 이미 심링크면 `mv` 분기는 돌지 않으므로
반복 설치는 멱등이고 백업이 쌓이지 않는다. `ln` 은 `ln -sfn` 을 쓴다(BSD `ln`: 대상이 디렉터리
심링크면 `-f` 만으로는 갈아끼우지 않고 그 디렉터리 **안에** 새 링크를 만든다 = `target_dir` 밖
root 쓰기 누출). **확인 모달은 두지 않는다 — 백업이므로 잃는 것이 없다. 잃는 것이 없어야
1클릭이 정당하다.**

### 4.2 2차 — 수리 12건(`6159778`), 재반증 9건(미수리 1 + 신규 8, MAJOR 1)

2라운드가 수리한 12건(`FINDINGS` 상수 원문 항목):

| 번호 | 결함 | 수리 |
|---|---|---|
| BLOCK-1 | 설치가 남의 실체 파일을 파괴 | 백업 전환 + `ln -sfn` + `warnings` 통보 + UI 가 `notes` 노출 |
| MAJOR-2 | 해제의 심링크 가드가 **권한 오르는 지점에서 강제되지 않는다**(TOCTOU). `probe_link` 는 비특권 사전 관측이고 집행은 root `rm -f`. 그 사이에 **사용자가 비밀번호를 치는 시간 제한 없는 창**이 있다. | 승격 스크립트 자체가 집행 직전 재검증(`-L` 검사 + `readlink` 대조). 순수함수 테스트에 "스크립트 문자열이 재검증을 포함한다"를 단언으로 박음 |
| MAJOR-3 | 타임아웃이 hang 을 못 막음 | **파이프와 드레인 스레드를 없앤다.** 자식 stdout 을 임시 파일로 리다이렉트(`Stdio::from(File)`), wait/kill 후 파일을 읽는다. EOF 의존이 사라져 손자 문제와 무관해진다. 손자 케이스 테스트 추가 |
| MAJOR-4 | `installed` 판정이 사용자 실제 셸의 PATH 가 아니다(`bash -lc` 로 재는데 macOS 10.15+ 기본 로그인 셸은 zsh) | `$SHELL` 우선, 없으면 bash. 문구를 "PATH 1순위"(전칭) → "로그인 셸(`<셸이름>`) 기준"(무엇을 쟀는지 밝힘)으로 정직화 |
| MAJOR-5 | UI↔Rust 계약 드리프트 | 확정 계약에 TS 를 정확히 맞춤. 픽스처를 Rust 실물 모양으로 교체 + 계약 드리프트 가드 테스트 |
| MINOR-6 | `/tmp/Applications` 가 Canonical 통과 | `classify_bundle_dir` 은 **건드리지 않고**(`autoregister_allowed` 도 쓰므로 산탄총 수술) `plan_cli_install` 전용 엄격 판정 신설(`strict_install_bundle_ok`, 현 `main.rs:1851`) + 반례 테스트 |
| MINOR-7 | `parse_which_a` 가 stdout 잡음을 경로로 격상 | 절대경로(`/` 시작)만 수용 + 반례 테스트 |
| MINOR-8 | 해제 계열 10개 항목에 `dead_code` 허용 누락 → Windows 빌드 경고 | 설치 계열과 대칭으로 부착 |
| MINOR-9 | `unverified` 문구가 사실과 다른 분기 존재 | 두 분기 구분(→ 3라운드에서 기계 필드로 승격) |
| MINOR-10 | 경고가 8초 만에 사라진다 | `installed_shadowed`·`unverified`·해제 부분완료는 60초 `stickyToast` |
| MINOR-11 | 버튼이 재조회보다 먼저 활성화돼 라벨이 어긋나는 창 | 재조회를 `await` 한 뒤 활성화 |
| MINOR-12 | 문서 잔존 불일치 2건 (README 의 낡은 §B 서술 / `docs/INSTALL.md:183` ↔ `USER-MANUAL.md:80` Windows MSI PATH 등록 정면 충돌) | 실측 후 사실인 쪽으로 통일. **실측으로 판정이 안 되면 고치지 말고 '판정 불가'로 보고 — 추측 정정 금지** |

**2차 재반증 산출 9건** 중 대표:

| 대표 결함 | 그것이 드러낸 **설계 결함** |
|---|---|
| **MAJOR-N1 — 설치 부분 실패 시 백업 통보가 통째로 사라진다.** 스크립트 rc!=0 이면 먼저 `return` 하여, 앞에서 계산해 둔 `expected_backups` 루프에 도달하지 못한다. 실측 재현: cys 는 백업+링크까지 끝났고 cysd 의 `mv` 가 거부돼 전체 rc=1 이 된 경우, 사용자는 `"심볼릭 생성 실패: mv: …"` 만 본다. **남의 실체 바이너리가 추측 불가능한 이름으로 옮겨졌는데 그 사실이 어디에도 안 남는다.** | 설계서가 **순차로 여러 대상에 작용하는 명령의 '앞은 성공·뒤는 실패' 상태**를 명시하지 않았다. 반환값도 통보 문구도 정의되지 않았으므로 구현은 첫 `Err` 에서 나가는 자연스러운 형태를 택했다. → **규약 ⑥** |
| **MINOR-N2·N5 — 검증 셸의 종료 상태를 버려 '정상 실행됐지만' 을 거짓말한다.** `$SHELL` 채택 후 `Ok((_, stdout))` 로 성공 플래그 폐기. 실측: `/bin/tcsh -lc`·`/bin/csh -lc` 는 rc=1 + 빈 stdout(둘 다 macOS 동봉·`/etc/shells` 등재). `Completed([])` 로 접혀 "검증 명령은 정상 실행됐지만 PATH에서 못 찾았다"가 나가고, UI 는 **셸 설정에 PATH 를 추가하라고 틀린 안내**를 한다. | "확인한다"는 요구에 **계측기가 딸려 있지 않았다** — 어떤 셸로, 어떤 명령으로, 몇 초 안에, 실패하면 어떤 값인지가 설계서에 없었다. → **규약 ⑤** |
| **MINOR-N7 — 문서 안내가 실행 불가능하다.** 실측: 비대화형 로그인 zsh 는 `.zshenv`·`.zprofile`·`.zlogin` 만 읽고 **`.zshrc` 는 건너뛴다**(bash 도 `.bash_profile` 만 읽고 `.bashrc` 는 건너뛴다). 그런데 `docs/INSTALL.md:167` 은 `~/.zshrc` 를 고치라고 안내했다. **안내대로 해도 경고가 안 사라진다.** | 문서의 합격선을 "코드와 일치"로 잡았지 "**지시대로 하면 문제가 실제로 해결된다**"로 잡지 않았다. → **규약 ⑫** |
| **MINOR-N8 — 엄격 판정이 macOS 펌링크 별칭을 거부한다(순수 신규 축소).** 실측: `/Applications` 와 `/System/Volumes/Data/Applications` 는 같은 장치·같은 inode. `current_exe()` 는 `_NSGetExecutablePath` 를 정규화 없이 돌려주므로 Data 경유 exec 세션에서는 조부모가 `/System/Volumes/Data/Applications` 가 된다. 문자열 완전일치라 거부됐다. | **경고를 거부로 올리는 변경(D5)의 새 거부 집합을 전수 열거하지 않았다.** MINOR-6 수리가 정당한 사용자를 새로 막았다. → **규약 ⑩** |
| **UNRESOLVED-1 · N3 — 산문을 정규식으로 파싱하는 결합.** Rust 는 "경고문 첫 구절(`PATH 확인 결과:` / `PATH 확인 실패:`)을 안정 판별자로 고정한다"고 선언했는데, TS 정규식은 그 접두가 아니라 **문장 속 어절**('찾지 못했'·'타임아웃')을 봤다. 양쪽이 서로 **다른 것을 계약이라 부르고** 있었다. 게다가 같은 `warnings` 배열에 백업 통보문이 합류해 판정 대상 문자열이 오염됐다. | **UI 가 분기하는 판정을 기계 필드로 못박지 않았다.** 산문은 다듬기·번역으로 언제든 바뀌고, 바뀌는 순간 조용히 오분기한다(테스트가 같은 문자열을 픽스처로 쓰면 초록인 채로 봉인된다). → **규약 ⑪** |

### 4.3 3차 — 계약 v2 확장 + 수리 11건(`9f47148`), 재반증 11건

3라운드에서 **이번 작업에서 유일하게 허용된 계약 변경**이 일어났다: `InstallCliReport` 에
`unverified_reason: Option<String>` 을 추가하고, TS 의 문구 정규식
(`NOT_ON_PATH_MARK`·`PROBE_FAILED_MARK`)을 **전부 삭제**했다. 픽스처도 실물 문자열로 교체하고,
"**TS 가 warnings 문구를 파싱하지 않는다**"를 지키는 테스트(정규식 재도입 차단)를 넣었다.

그 외 3라운드 수리:

- MAJOR-N1: 실패 반환 **전에** 백업 후보 경로를 실제로 재관측(`observe_existing_backups`,
  현 `main.rs:1159`)하고 존재하는 것을 에러 메시지에 포함(복구 명령 `sudo mv <bak> <orig>` 포함).
- MINOR-N2·N5: 성공 플래그를 쓴다. rc!=0 이면 `probe_failed`. `$SHELL` 이 `-lc` 를 못 받는
  셸(csh/tcsh 계열)이면 `/bin/zsh` 또는 `/bin/bash` 로 한 번 폴백 재시도(폴백했다는 사실을 문구에 밝힘).
- MINOR-N7: 문서·UI 문구가 **실제로 읽히는 파일**(`~/.zshenv`·`~/.zprofile`·`~/.zlogin`)을 지목.
  "이 확인은 비대화형 로그인 셸 기준입니다 — 터미널에서 cys 가 이미 동작한다면 무시해도 됩니다"를
  문구에 넣어 거짓 경고에 사용자가 헛수고하지 않게 함.
  ★**대화형(`-lic`)으로 바꾸는 안은 채택하지 않는다**: 사용자의 대화형 rc 를 버튼 클릭 부작용으로
  실행하면 nvm/conda/oh-my-zsh 같은 것이 백그라운드 프로세스를 띄울 수 있다. **정직한 문구가 답이다.**
- MINOR-N8: 비교 전 정규화(`strip_data_volume_prefix`, 현 `main.rs:1728`). 반례 테스트:
  `/System/Volumes/Data/Applications/cys.app/Contents/MacOS` 통과, `/tmp/Applications/...` 거부 유지.
- MINOR-N4·N6: sticky 재사용 시 `className` 을 현재 category 로 갱신 / volatile 로 낼 때 같은 id 의
  살아있는 sticky 를 먼저 내림. 순수 로직은 `clipath.ts` 로 뽑아 테스트(`toastEmitPlan`).
- 사전 존재 결함: 당시 `main.rs:4804-4809` 의 고아 doc 주석 + 중복 `#[test]` 로 같은 테스트가 두 번
  등록되어 `warning: duplicated attribute` 가 뜨던 것(빌드 전체의 유일한 경고) 정리.
  **HEAD 에도 존재하는 사전 결함이지만 이 파일은 이미 대폭 손대는 파일이고 1줄 수리라 함께 정리.**

**3차 재반증 산출 11건.** 그중 재현 테스트로 굳은 것이 `adv1`~`adv9` 다
(스크래치 `ADVERSARIAL_TESTS_R2.rs`):

| 테스트 | 무엇을 재현하는가 |
|---|---|
| `adv1_zsh_function_wrapper_becomes_a_fake_shadow` | zsh 함수 래퍼(`cys () { ... }` 여러 줄)가 가짜 그림자로 잡힌다 |
| `adv2_trailing_slash_path_shadows_our_own_link` | PATH 에 `/usr/local/bin/`(후행 슬래시)가 있으면 정상 설치가 `installed_shadowed` 로 뒤집힌다 |
| `adv3_install_destroys_foreign_symlink_silently_while_uninstall_protects_it` | 설치는 남의 심볼릭을 말없이 갈아 끼우는데 해제는 같은 것을 지킨다 |
| `adv4_uninstall_leaves_the_user_without_their_original_binary` | 해제가 심볼릭만 지우고 백업해 둔 원본은 복원하지 않는다 |
| `adv5_uninstall_partial_failure_has_no_report_path` | 해제에 부분 실패 보고 경로가 없다 |
| `adv6_ln_sfn_does_not_leak_into_a_directory_symlink` | `ln -sfn` 이 디렉터리 심볼릭 안으로 새지 않는지(2라운드 수리의 회귀핀) |
| `adv7_rc_stdout_noise_outranks_the_real_measurement` | 로그인 rc 가 stdout 에 찍는 절대경로 줄이 which 출력보다 앞서 나와 1순위를 차지한다 |
| `adv8_shadowed_install_flips_the_button_to_uninstall` | 미완료 설치인데 버튼이 '해제'로 뒤집힌다 |
| `adv9_cysd_shadowing_is_never_measured` | 프로브가 cys 만 재고 cysd 그림자는 한 번도 측정되지 않는다 |

**이 11건이 드러낸 설계 결함**: 개별 결함이 아니라 **패턴**이었다. 성찰 2R·3R 이 독립적으로
같은 결론에 도달했다 —

> 지금까지 30건을 수리했지만 모두 **지목된 지점에만** 적용됐고 **결함의 계열에는** 적용되지
> 않았다. install 과 uninstall 은 완전한 거울쌍인데 공유 추상이 하나도 없어, 매 수리가 한쪽
> 거울에만 발린다.

그래서 4라운드의 지배 원리가 정해졌다. → **규약 ⑬**

### 4.4 4차 — 지배 원리: 지점이 아니라 계열을 닫는다 (**설계** · 결과는 4.7)

**수리할 때마다 반드시 이 4쌍을 묻는다 — "이 수리를 대칭 위치에도 적용했는가?"**

```
① 설치 ↔ 해제           ② cys ↔ cysd
③ 실체 파일 ↔ 심볼릭     ④ 시작 표식 ↔ 끝 표식
```

한 쪽만 고친 수리는 이 라운드에서 **미완성으로 간주한다.**

#### C1 [파괴 대칭] 설치가 '남의 심볼릭'은 여전히 말없이 갈아 끼운다

2라운드는 '실체 파일'만 백업 대상으로 삼았다(조건 `[ -e X ] && [ ! -L X ]`). 그런데 해제 쪽은
'우리 번들을 가리키지 않는 심볼릭'도 남의 것이라며 지키고 있다(`links_into_cys_bundle`).
**같은 대상에 대해 해제는 지키고 설치는 파괴한다** — 2라운드가 고친 BLOCK 과 정확히 같은 병이
심볼릭 축에서 그대로 살아 있다.

★수리: 조건을 `[ -e X ] || [ -L X ]` 로 넓히되, **우리 번들을 가리키는 심볼릭은 백업 대상이
아니다**(멱등 재설치가 백업을 쌓으면 안 된다). 판정은 해제 쪽과 **같은 순수 함수**를 쓴다.
남의 심볼릭이면 백업(`mv`)하거나, 최소한 '기존 링크(→ 원 대상)를 갈아 끼웠다'를 `warnings` 로
반드시 발화한다. **어느 쪽이든 사용자가 원 대상 문자열을 잃지 않아야 한다.**

#### C2 [부분 실패 대칭] 해제에는 부분 실패 보고 경로가 없다

설치는 3라운드에서 실패 반환 전 재관측(`observe_existing_backups`)을 넣었는데,
`uninstall_cli_from_path` 의 `Err` 조기반환에는 같은 조치가 없다.

★수리: 설치와 **같은 형태**로, 실패 반환 전에 `plan.remove` 대상을 재관측해 '이미 지워진 것 /
남은 것'을 에러 문구에 담는다.

#### C3 [산문 금지 대칭] 해제 등급 판정이 아직 산문을 정규식으로 읽는다

설치 경로는 `unverified_reason` 기계 필드로 바꿨는데, 해제 경로의 `isBenignSkip` 은 여전히
Rust 산문 '이미 해제' 를 정규식으로 파싱해 등급(성공 volatile ↔ ⚠부분완료 sticky)을 정한다.
**Rust 가 문구를 한 단어만 다듬으면 정상 해제가 조용히 '부분 완료'로 오보고된다.**

★수리: `UninstallCliReport` 에 기계 판별자를 추가한다 — `skipped_benign: bool`(건너뛴 것이 전부
'지울 게 없었다' 류인가) 또는 더 나은 형태. 어느 쪽이든 **TS 가 산문을 파싱하지 않게** 만들고,
'정규식 재도입 차단' 테스트를 해제 경로에도 붙인다(설치 경로엔 이미 있다).
★같은 원리로 `shadowed_by` 도 점검한다 — 셸이 뱉은 산문 한 줄을 그대로 신뢰하는 곳이 있는가.

#### C4 [표식 대칭] 프로브에 끝 표식만 있고 시작 표식이 없다

3라운드가 `which -a cys; echo __cys_probe_end__` 로 끝 표식을 넣어 '완주 여부'는 잡았다.
그러나 **로그인 rc 가 stdout 에 찍는 절대경로 줄은 which 출력보다 앞서 나오므로** 그것이 목록
1순위가 되어 `installed` 를 `installed_shadowed` 로 뒤집는다(adv7 실측 성립).

★수리: `echo __cys_probe_begin__; which -a cys; echo __cys_probe_end__` 로 바꾸고,
`parse_which_a` 는 **두 표식 사이의 줄만** 채택한다. **표식이 없거나 순서가 어긋나면 측정 실패다.**
★추가 경화(adv1 가짜 그림자): 공백을 포함한 줄 배제 + 채택 경로가 실제로 파일인지 재관측(`Path::is_file`).

#### C5 [cys↔cysd 대칭] cysd 그림자는 한 번도 측정되지 않는다

프로브가 cys 만 잰다. cysd 가 다른 곳에서 가려지면 사용자는 알 수 없다(adv9).

★수리: cysd 도 같은 프로브로 재고 결과를 `warnings` 에 담는다. **status 3값 계약은 건드리지 않고
(cys 기준 유지) cysd 문제는 경고로 낸다.** 이 결정의 이유를 주석에 남긴다.

### 4.5 4차 개별 수리 (계열과 별도 · **설계** · 결과는 4.7)

| 번호 | 결함 | 수리 |
|---|---|---|
| **I1** [경로 정규화 · adv2] | `classify_install_status` 의 `first == target_cys` 가 문자열 완전일치. PATH 에 `/usr/local/bin/`(후행 슬래시)가 있으면 which 출력이 `/usr/local/bin//cys` 라 **정상 설치가 `installed_shadowed` 로 뒤집히고, 경고문이 방금 만든 자기 링크를 지우라고 안내한다.** | 비교 전 경로 정규화(연속 슬래시 축약 + 후행 슬래시 제거). `strip_data_volume_prefix` 와 같은 층에 두고 **설치·해제·상태 조회 세 경로 모두**에 적용(계열!). 가능하면 `std::fs::metadata` 의 `(dev, ino)` 동일성으로 이중 확인 |
| **I2** [adv8] | 설치 결과가 `installed` 가 아닌데도 상태 재조회가 `installed=true` 를 내 **버튼이 '해제'가 된다** | 미완료(shadowed·unverified) 상태에서는 '다시 설치'를 유지하고 해제는 분리하거나, 최소한 라벨이 사용자를 오도하지 않게 한다. 순수 판정 + 테스트 |
| **I3** [백업 재발견] | `cli_install_status` 가 `*.cys-backup-*` 를 보지 않아 `notes` 에 안 뜨고, 해제는 심볼릭만 지우며, 유일한 통보 sticky 는 60초 뒤 사라진다(수용처 `alarmHistory` 는 메모리 전용). **'잃는 것이 없어야 1클릭이 정당하다'는 BLOCK-1 의 정당화가 무너진다.** | ① `cli_install_status` 가 잔존 백업을 관측해 `notes` 에 상시 노출 ② 해제 확인 문구에 '설치 때 백업해 둔 원본이 있습니다' 분기 ③ 가능하면 해제 시 '원본 복원'(우리 이름 규칙에 **정확히 일치하는** 백업만 `mv`). ③은 비가역이 아니라 복원이므로 안전하나, 복잡하면 ①②만 하고 ③은 사유와 함께 미수리로 보고 |
| **I4** [승격 스크립트 PATH] | `do shell script` 가 부모 PATH 를 상속한다. `mkdir`·`mv`·`ln`·`readlink`·`rm` 을 절대경로 없이 부른다. **TN2065 는 'Use the full path to the command' 라고 못박고**, 이 기계의 상속 PATH 에는 사용자 쓰기 가능 디렉터리가 `/usr/bin` **앞에** 있다 | 스크립트 첫머리에 `export PATH=/usr/bin:/bin:/usr/sbin:/sbin;` 를 박거나 전 명령을 절대경로로(둘 다 하면 더 좋다). **설치·해제 양쪽**(계열!). 테스트로 못박음 |
| **I5** [설치 TOCTOU] | 해제는 2라운드에서 스크립트 자체 재검증을 넣었는데 **설치는 비특권 사전 관측에만 의존한다** | 스크립트가 자기가 한 일을 stdout 으로 보고하게 한다 — 백업할 때 `echo "CYS-BACKED-UP:<원본>:<백업본>"` 를 찍고, Rust 가 osascript stdout 을 파싱해 **계획이 아니라 사실**을 보고한다. 승격 창 안의 상태 변화도 잡힌다. **성공·실패 양쪽 반환 경로에서** 이 stdout 을 읽어야 한다 |
| **I6** [계약 기계화 — 최우선 구조 수리] | `clipath.test.ts` 의 `RUST_*_REPORT` 표는 **손으로 쓴 사본**이다. 실물이 바뀌어도 안 빨개진다 | Rust `mod tests` 에 `serde_json::to_value(...)` 로 세 리포트의 **키 집합 + 타입 태그**를 `ui/src/__contract__.json` 으로 덤프하는 테스트를 두고, `clipath.test.ts` 가 그 파일을 읽어 `expectShape` 의 기준으로 삼는다(손으로 쓴 표를 **대체**). **검수 기준 하나: 아무 필드에 `#[serde(rename=...)]` 를 붙였을 때 TS 게이트가 빨개지는가.** 그 실험을 실제로 해 보고 결과를 보고(실험 후 원복) |
| **I7** [잡동사니] | ① `#[cfg_attr(not(target_os="macos"), allow(dead_code))]` 누락 3곳: `plan_cli_install`·`build_install_script`·`struct CliInstallPlan` ② 부트 회귀핀 추가: `classify_bundle_dir("/System/Volumes/Data/Applications/cys.app/Contents/MacOS") == Canonical` 과 `~/Applications` 데이터볼륨 형태(지금 핀은 `strict_install_bundle_ok` 쪽에만 있다) ③ `ToastEmit.className` 이 실제로 소비되지 않는 **죽은 필드** — 소비하게 고치거나 필드와 그 테스트를 삭제 ④ 중복 문구: `not_on_path` 경고의 '비대화형 로그인 셸 기준' 문장이 Rust 산문과 TS 양쪽에 있어 토스트에 **두 번** 나온다 → ★master 결정: **Rust 에서 뺀다**(백엔드는 사실만, 표현은 UI 소유) ⑤ `#cc-header` 오버플로 가드(닫기 버튼이 밀려날 수 있다) — `ui/src/style.css` 수정을 이번 라운드에 한해 허용, **`#cc-header` 규칙에만** 손댄다 | |

### 4.6 4차 인수 기준 (라운드 시작 전 선언·동결 · **판정 결과는 4.7**)

`ADVERSARIAL_TESTS_R2.rs` 의 `adv1`~`adv9` 를 `src-tauri/src/main.rs` 의 `mod tests` 말미에
편입하고 **전부 초록**이 되어야 한다. 다만 그대로 붙이지 말고 **헤르메틱하게 재작성**한다:

- **실기계 로그인 프로필 실행 금지** — 스텁 셸 스크립트를 임시 디렉터리에 만들어 그것을 셸로 지정.
- `std::env::set_var` 를 쓰는 테스트는 **전용 Mutex 로 직렬화**한다(선례: `src/pack.rs` 의 `PACK_ENV_LOCK`).
- **`/usr/local/bin` 접근 절대 금지.**
- 수리 후에도 **존치**한다(제거 금지 — red→green 회귀핀).

초록이 안 되는 항목이 있으면 **지우지 말고 `#[ignore]` 도 붙이지 말고** 사유를 미수리로 적는다.

---

### 4.7 4차 결과 — 무엇이 실제로 닫혔나 (커밋 `7f8b505` 의 앞부분)

계열 수리 C1~C5 와 개별 수리 I1~I7 은 **전부 구현됐다.** 4.6 의 인수 기준(`adv1`~`adv9` 전부
초록)도 충족됐다 — 다만 그냥 초록이 된 것이 아니라, **테스트 이름 자체가 결함 재현에서 결함
부재 회귀핀으로 바뀌었다.** 이것이 이 라운드가 실제로 무엇을 했는지 가장 정직하게 보여 준다:

| 4차 이전 이름(결함을 재현한다) | 현재 이름(`main.rs`) | line |
|---|---|---|
| `adv1_zsh_function_wrapper_becomes_a_fake_shadow` | `adv1_shell_function_wrapper_never_becomes_a_fake_shadow` | 8116 |
| `adv2_trailing_slash_path_shadows_our_own_link` | `adv2_trailing_slash_path_never_shadows_our_own_link` | 8151 |
| `adv3_install_destroys_foreign_symlink_silently_while_uninstall_protects_it` | `adv3_install_no_longer_destroys_a_foreign_symlink_silently` | 8171 |
| `adv4_uninstall_leaves_the_user_without_their_original_binary` | `adv4_uninstall_restores_the_users_original_binary` | 8226 |
| `adv5_uninstall_partial_failure_has_no_report_path` | `adv5_uninstall_partial_failure_reports_what_already_happened` | 8277 |
| `adv6_ln_sfn_does_not_leak_into_a_directory_symlink` | (동일 — 2라운드 수리의 회귀핀) | 8328 |
| `adv7_rc_stdout_noise_outranks_the_real_measurement` | `adv7_rc_stdout_noise_never_outranks_the_real_measurement` | 8353 |
| `adv8_shadowed_install_flips_the_button_to_uninstall` | `adv8_shadowed_install_keeps_the_retry_path` | 8379 |
| `adv9_cysd_shadowing_is_never_measured` | `adv9_cysd_shadowing_is_measured_and_reported` | 8412 |

**I6 검수 실험(4.6 이 요구한 것)은 실제로 수행됐다.** 결과는 §3.4 에 적었다 — 핵심은 "덤프
쓰기가 자기 단언보다 **먼저** 와야 한다"는 함정을 실측으로 확인했다는 것이다. 단언을 앞에 두면
rename 사고가 Rust 층에서 멈추고 파일이 낡은 채로 남아 **TS 게이트가 초록으로 통과한다.**

**I3③(해제 시 원본 복원)은 '복잡하면 미수리로 보고' 선택지였는데 채택됐다.** 이것은 정직하게
적어 둘 필요가 있다 — 8라운드 판정자가 지적한 대로, 이는 **오너의 원 요구 3건(복원·수리1~3)이
요구하지 않은 새 특권 쓰기 경로**다. root 승격 스크립트가 `mv` 로 백업본을 원위치로 되돌린다
(`build_uninstall_script`, 현 `main.rs:2470`). 가드는 있다 — 자리가 비어 있을 때만 옮기고, 이름
규칙(`<이름>.cys-backup-<epoch초>`)에 정확히 맞는 것만 후보다. 그럼에도 **요구 범위를 넘은
확장**이라는 사실은 남는다. 정당화는 규약 ⑦이다: 백업만 하고 복원이 없으면 "잃는 것이 없다"는
1클릭의 정당화가 성립하지 않는다.

**이 라운드가 만든 IPC 필드**: `skipped_reasons`·`skipped_benign`(C3) · `restored`(I3③) ·
`backups`(I3①). 3라운드의 `unverified_reason` 까지 합해 이 작업이 신설한 IPC 필드는 **총 5개**다.

### 4.8 5차 — `do shell script` 의 반환값은 CR 로 구분된다 (BLOCK)

| 대표 결함 | 그것이 드러낸 **설계 결함** |
|---|---|
| **BLOCK — 줄 나누기를 LF 로 했다.** AppleScript `do shell script` 가 돌려주는 문자열은 줄을 **CR(`\r`)** 로 구분한다(실측: `AAA\rBBB\rCCC\n`). I5 가 넣은 자기보고 마커(`CYS-BACKED-UP:`·`CYS-RESTORED:`)가 2건 이상이면 LF 파서는 그것을 **한 줄로** 읽는다. 결과: 정상 해제가 '⚠부분 완료'로 오보고되고, **방금 복원해 놓은 사용자 원본을 지우라고 안내**했다. 수리는 `split_osascript_lines`(현 `main.rs:1319`) + `osascript_text_to_lf`(현 `main.rs:1347`) 로 계열 폐쇄. 실패 경로는 마커가 줄 첫머리에 오지 않고 종료 상태가 뒤에 붙으므로 **부분 문자열 스캔**으로 전환했다. | 설계서가 I5 를 "스크립트가 stdout 으로 자기보고하게 한다"는 HOW 로만 적고, **그 stdout 이 어떤 형식으로 도착하는지**(구분자·인코딩·실패 시 어느 스트림) 를 적지 않았다. 규약 ④(HOW 를 무효화하는 조건을 함께 적어라)의 재발이며, 무효화 조건은 이번에도 **플랫폼의 오래된 관례**였다. → **규약 ④** |
| **테스트 픽스처가 손글씨였다.** 마커 파싱 테스트는 사람이 타이핑한 `"CYS-BACKED-UP:a:b\nCYS-BACKED-UP:c:d"` 를 먹였고 — 그 픽스처에는 CR 이 없으므로 **결함이 초록으로 봉인**됐다. 5라운드는 픽스처를 **실물 `osascript` 호출로 유도**하도록 바꿨다. | 규약 ⑨("픽스처는 상대 구현의 실물에서 유도한다")가 이미 있었는데도 **경계 하나를 빠뜨렸다**: 이 작업은 그동안 '상대 구현'을 Rust↔TS 로만 생각했고, **Rust↔osascript(운영체제)** 도 같은 경계라는 것을 보지 못했다. → **규약 ⑨ 확장** |
| **MAJOR-6 — 판정과 집행이 서로 다른 규칙을 썼다.** I1 이 Rust 쪽에 경로 정규화(`normalize_path_str`)를 넣었는데, 실제로 파일을 옮기고 지우는 **셸 스크립트는 정규화하지 않았다.** 셸에도 `sed` 정규화(`SHELL_PATH_NORMALIZER`, 현 `main.rs:1289`)를 넣어 설치·해제 양쪽을 맞췄다. | **"순수 함수로 판정한다"가 안전을 보장하지 않는다** — 판정이 아무리 정확해도 집행자가 다른 규칙을 쓰면 판정은 장식이다. 판정과 집행이 갈라진 모든 지점을 목록으로 요구하는 조문이 없었다. → **규약 ⑭ 신설(아래)** |

### 4.9 6차 — Windows 컴파일 즉사(E0425)와 '판정≠집행'의 두 번째 재발

| 대표 결함 | 그것이 드러낸 **설계 결함** |
|---|---|
| **★Windows E0425 — 이 브랜치는 Windows 에서 컴파일되지 않았다.** `#[cfg_attr(not(target_os="macos"), allow(dead_code))]` 는 **경고만 끄지 코드를 지우지 않는다.** 그런데 그 본문이 `#[cfg(unix)]` 로 **사라지는** 함수를 호출하고 있었다 → Windows 타깃에서 "cannot find function" 즉사. 최소 재현(`rustc --target x86_64-pc-windows-msvc`)으로 확인한 뒤, 아이템 레벨 `cfg` 를 제거하고 **본문 안에서 분기**하는 형태로 바꿨다(리포의 기존 `no_console` 과 같은 형태). 재발 방지로 소스 텍스트 불변식 핀을 넣고 **변이시험으로 실효를 입증**했다. | 이 레인은 macOS 기능을 만들면서 **Windows 컴파일을 한 번도 검증하지 않았다.** 그리고 그 사실이 어디에도 적혀 있지 않았다 — 1차 반증의 `windows-boot` 렌즈가 "Windows 빌드에서 컴파일되는가"를 물었는데, 답은 "물어보기만 하고 재지 않았다"였다. **묻기만 하고 계측하지 않은 질문은 게이트가 아니다.** → **규약 ⑤** |
| **MAJOR-C — '판정≠집행'이 존재 술어 축에 재발.** Rust 는 `Path::exists()` 로 "그 자리에 무언가 있는가"를 물었는데 이 함수는 **심볼릭을 따라간다** — 대상이 사라진 링크(dangling)를 "없다"로 읽는다. 반면 셸 스크립트는 `[ -e X ] || [ -L X ]` 로 물었다. 같은 질문에 두 답이 나온다. `symlink_metadata()` 로 통일했고, **dangling 심볼릭을 실제로 만들어** 회귀핀을 걸고 변이시험 2회로 확인했다. | 5차의 MAJOR-6 과 **정확히 같은 병의 다른 축**이다. 한 라운드 안에서 같은 계열이 두 번 나왔다는 것은, 그 계열을 **목록으로 만들어 전수 점검하지 않았다**는 뜻이다. → **규약 ⑭ 신설(아래)** |

이 라운드에는 명시 제약이 하나 더 있었다: **신규 기능 금지.** 결함만 닫고, 개선 아이디어는
코드가 아니라 미수리 목록으로 보냈다(커밋 `7f8b505` 의 `Rejected:` 트레일러에 3건이 남아 있다 —
더미경로 `/tmp/*` 채택 / cysd 경고의 등급 강등 / 공용 존재술어 헬퍼 추출).

### 4.10 7차 — 등급 회귀·커맨드 비동기화·파괴적 안내 제거 (커밋 `0b6cb24` = 8차 freeze)

| 대표 결함 | 그것이 드러낸 **설계 결함** |
|---|---|
| **MAJOR-1 [등급 회귀] — 6차 수리가 경고 방향을 뒤집었다.** `state=partial` 에 있는 **진짜 남의 파일**이 중립으로 강등됐다. ★그리고 여기서 **master 가 제시한 판정 근거를 워커가 반증했다** — 4.14 ② 참조. | 수리가 **다음 라운드에 회귀를 만든다**는 사실 자체가 이 라운드의 교훈이다. 6차는 '과경고를 줄인다'는 옳은 방향으로 움직이면서 **줄이면 안 되는 것까지 줄였다.** 등급을 바꾸는 수리에는 "무엇이 새로 조용해지는가"를 전수 열거하는 절차가 필요하다 — 규약 ⑩("경고를 거부로 올리는 변경은 새로 거부되는 집합을 전수 열거한다")의 **반대 방향 쌍**이 빠져 있었다. → **규약 ⑮ 신설(아래)** |
| **MAJOR-2 [앱이 파괴적 명령을 출력] — UI 가 사용자에게 `sudo mv` 를 그대로 내밀었다.** 대상 자리가 차 있으면 그 파일이 말없이 사라진다. `sudo mv -n` + "자리가 비어 있어야 옮겨집니다" 뜻풀이로 바꾸고, **앱이 자리가 찼다고 아는 경우엔 복원 명령을 아예 내지 않는다.** UI 가 만드는 명령 문자열 **5종 전수 점검**을 함께 수행했고, Rust 는 명령 문장을 하나도 만들지 않는다는 원칙(G2)을 재확인했다. | ★**점검 범위가 `ui/src` 에서 멈췄다.** 같은 파괴적 명령이 `docs/INSTALL.md`·`USER-MANUAL.md` 에도 있는데 그쪽은 보지 않았다 — 8차 BLOCK-1 이 정확히 그것이다(4.11). 계열(규약 ⑬)을 "코드 안의 거울쌍"으로만 좁게 읽은 것이 원인이다. → **규약 ⑯ 신설(아래)** |
| **MAJOR-3 [메인 스레드 블로킹]** — CLI 커맨드 3종을 `async fn` 으로 내렸다(리포 지배 관례 async 48 : 동기 30). 설치·해제가 더 심각했다: `osascript` 승인 대기를 **기한 없이** 동기로 기다려, 사용자가 비밀번호 창을 두는 동안 UI 전체가 무기한 멎었다. | 그러나 **주석이 "새 동시성도 열리지 않는다"고 단정했고 그 단정이 틀렸다** — 8차 MAJOR-3 이 반증한다(§3.6). 비동기화는 **막힘을 푸는 동시에 문을 여는** 변경인데, 설계서가 그 두 면을 함께 요구하지 않았다. → **규약 ⑮** |
| **MINOR-4 [핀의 사각] — 재발 방지 핀이 `fn` 만 보고 있었다.** `cfg` 불변식 스캐너를 11종 아이템(fn·static·const·struct·enum·type·union·trait·impl·mod·use)으로 확장하고, ★**종류를 못 읽으면 `continue` 가 아니라 offender 로 올린다**(측정 불능은 통과가 아니다 — 그 `continue` 가 정확히 사각이었다). 변이시험 8종 전부 검출 · 대조군(수리 전 스캐너)은 1종만 검출 = **사각 실재 확정**. | 헌장 원칙("측정 불능은 통과가 아니다")이 **테스트 도구 자신에게는 적용되지 않고 있었다.** 도구가 조용히 건너뛰는 자리는 영원히 보이지 않는다. |
| **MINOR-5 [거짓 보증] — "자동 테스트가 지킵니다"가 가리키는 테스트가 없었다.** 실제로 제작했다: `clipath.test.ts` 가 `docs/INSTALL.md` 를 `readFileSync` 로 직접 읽어 화면 문자열과 대조한다. | ★그런데 **`USER-MANUAL.md` 를 읽는 테스트는 만들지 않았다** — 8차 판정자가 리포 전체 `grep` 으로 0건임을 확인했고, 실제로 그 파일에 불일치가 남아 있었다(4.11). 문서 축에서도 거울쌍이 반쪽만 지켜졌다. → **규약 ⑯** |

7차 검증값(커밋 기록 · **당시 값**): `cargo 102 passed / 0 failed · 경고 0` · `bun 631 pass / 0 fail` ·
`tsc 0` · `secret-scan clean(844파일)`. 타 레인 v0.14.25 태그와의 3-way 병합 시뮬 rc=0(충돌 0).

### 4.11 8차 — 최종 판정 12건 (커밋 `e7063d3` = 현재 HEAD · 9차 freeze)

이종 판정자 2인이 freeze 리비전 `0b6cb24` 를 대상으로 낸 12건이다. `reproduced: true` 는
판정자가 **실제로 재현했다**는 뜻이다(주장만 한 것이 아니다).

| # | 등급 | 위치 | 결함 | 이번 라운드 처리 |
|---|---|---|---|---|
| 1 | **MAJOR** | `docs/INSTALL.md:275` · `USER-MANUAL.md:98` | ★**문서가 시키는 수동 폴백이 사용자 파일을 말없이 파괴한다.** `sudo ln -sfn … /usr/local/bin/cys` — 그 자리에 Homebrew·수동빌드 실체 바이너리가 있으면 `-f` 가 unlink 후 갈아 끼워 **복구 불가로 소멸**(exit=0, 출력 없음, 백업 0개). GUI 버튼은 2라운드에 백업으로 고쳤는데 **문서를 따른 사용자만 파일을 잃는다.** 도달 인구는 정확히 BLOCK-1 이 지키려던 집단(GUI 를 못 쓰는 SSH·헤드리스 + 기존 `/usr/local/bin/cys` 보유자). | 문서 레인이 수리 — 문서 절차가 **버튼과 같은 안전성**을 갖도록(백업 선행·멱등·같은 백업 이름 규칙·복붙 실행 가능) + `docs` 전체 명령 문자열 전수 점검 |
| 2 | MINOR | `ui/src/clipath.ts:450` | `mv -n` 의 보증이 한 형태에서 거짓 — BSD `mv -n` 의 판정은 `access(to, F_OK)`(심볼릭 추종)라 **대상이 사라진 심볼릭(dangling)은 '없다'로 읽고 그대로 덮는다**(실측 exit=0). 앱 **자신의** 복원 스크립트는 `[ ! -e O ] && [ ! -L O ]` 로 dangling 까지 막는데, 사용자에게 주는 안내만 느슨한 비대칭. | 문서·문구 레인 |
| 3 | MINOR | `src-tauri/src/main.rs:1220` | 설치 스크립트가 root 로 부르는 `/bin/mv {d} {b}` 가 **백업 목적지가 이미 있는지 검사하지 않는다.** 도달성 극히 낮음(같은 epoch 초에 두 번 설치 + 그때 두 번 다 남의 파일이 자리에 있어야 성립). ★**기록해 둘 함정: 여기에 `mv -n` 을 붙이는 것은 오답이다** — `-n` 은 거부해도 exit 0 이라 `&&` 체인이 이어져 `ln -sfn` 이 백업 없이 원본을 덮는, 지금보다 나쁜 경로가 열린다. | **이번 라운드 미수리**(사유 = 도달성 + 오답 함정). §9 미결에 등재 |
| 4 | **MAJOR** | `.github/workflows/release.yml:147` | ★**이 diff 의 Rust 테스트가 어떤 CI 레인에서도 돌지 않는다.** I6 계약 게이트의 절반(Rust 덤프)이 태그 레인 밖이고, 그 사실이 설계문서·주석·`Not-tested` 어디에도 없었다. | **이 문서 §7 신설**(MAJOR-4(c)) · `.github/` 무접촉이므로 문서가 산출물 |
| 5 | **MAJOR** | `ui/src/clipath.test.ts:370` | 판독기 테스트 부재 — `unverified_reason` 을 `null` 로, `restored` 를 `[]` 로 망가뜨려도 **201 pass / 0 fail**. 즉 3라운드·4라운드가 "닫았다"고 선언한 결함이 회귀로부터 보호되지 않는다. 원인: 모든 토스트 테스트가 픽스처 빌더로 판독기를 **우회**한다. | 테스트 레인이 수리 + 기계 필드 전수 표 |
| 6 | **MAJOR** | `USER-MANUAL.md:92` | 문서가 코드와 다른 사실을 말한다 — "제목은 **세 갈래**"인데 실제는 넷(`silent`\|`foreign`\|**`partial`**\|`backup`\|`info`). 7차가 `docs/INSTALL.md:242` 는 '네 갈래'로 고쳤는데 `USER-MANUAL` 만 빠졌다. 잡히지 않은 이유가 계열 결함: doc-sync 테스트가 `docs/INSTALL.md` **하나만** 읽는다. | 문서 레인 |
| 7 | **MAJOR** | `src-tauri/src/main.rs:3014` | 7차 `async` 주석의 "새 동시성도 열리지 않는다"가 **사실이 아니다** — 상태 조회의 실제 호출부는 버튼이 아니라 `setCcOpen` 의 `void refreshCliInstallState()` 이고 in-flight 가드가 없다. `cliStatus` 는 last-writer-wins. | 코드 레인 + **이 문서 §3.6** |
| 8 | **MAJOR** | `docs/plans/2026-08-25-shell-cli-restore-design.md:275` | **설계 정본이 4차에 멈춰 인수가 불가능하다.** §3 표가 실물과 어긋나 문서 스스로의 규칙(③ 필드 단위 스키마로 못박는다)을 위반. 5~7차는 흔적 0. | **이 문서 전체가 그 수리**(MAJOR-4 (a)(b)(c)(d)(e)) |
| 9 | MINOR | `ui/src/clipath.test.ts:2064` | 장식 테스트 1건 — `toast()` 등급색 핀이 "함수 본문에 그 문자열이 있는가"만 봐서, 등급색을 완전히 파괴해도 초록. 형제 핀(`stickyToast`)은 정상 작동하므로 계열 자체는 유효. | 테스트 레인 |
| 10 | MINOR | `ui/src/clipath.test.ts:2089` | 전체 소스 문자열 검색형 핀이 **주석을 걸러내지 않아 우회된다** — `// was: …` 형태로 남기면 배선을 실제로 깨도 초록. 같은 파일이 다른 곳(`CLIPATH_CODE`)에서는 주석을 걷어내는데 `MAIN_SRC` 계열 7개 핀에는 적용하지 않은 비대칭. | 테스트 레인 |
| 11 | MINOR | `USER-MANUAL.md:76` | 오너 원 요구 충족 판정 — 복원·수리1·수리2·수리3 **전부 충족**(코드 근거 첨부). 초과: D5·D6(원안 '선택') + **I3③ 원본 복원**(원안에 없던 새 특권 쓰기 경로) · IPC 필드 5개 신설. 미달은 **문서 축뿐**. | 4.7·§9 에 기록 |
| 12 | MINOR | 같은 문서 `:1` | 인수 경로 단절 — freeze 커밋 `7f8b505` 가 이 레인의 `SESSION_STATE.md`(39줄)·`MASTER_TODO.md`(33줄)를 **삭제**했는데 커밋 메시지가 그것을 한 글자도 언급하지 않았고 대체물도 없었다. | **§8 신설**(MAJOR-4(d)). 두 파일은 레포 밖으로 이전됐고 그 이유를 §8 에 적었다. ★7차 미커밋 지적은 **해소됨** — `0b6cb24` 로 커밋되어 이 라운드의 freeze 가 됐다 |

### 4.12 9차 — 문서 축에 기계 게이트를 세우다, 그리고 ★검증 하네스가 실계를 부쉈다 (커밋 `c6f3669`)

9차는 8차가 연 **문서 축**(규약 ⑯)을 사람 눈에서 기계로 넘긴 라운드다. 아직 커밋되지 않았으므로
아래는 **작업트리 실물에서 읽은 것**이다(커밋되면 그 메시지가 정본이 되고 이 표는 그것으로 갱신한다).

| 무엇을 했나 | 실물 근거 |
|---|---|
| **배포 문서 코드블록 전수 스캐너 신설** — `docs/` 아래 모든 `.md` + 리포 루트 `.md` 의 **셸 코드블록**만 훑어 ①맨 `ln -sf`/`ln -sfn` ②가드 없는 절대경로 `rm` ③`-n` 없는 절대경로 `mv` 를 잡는다. 대상 목록을 손으로 적지 않는다(8차가 네 파일만 지정했다가 다섯 번째 파일이 사고를 냈다). | `ui/src/clipath.test.ts` `scanDocCommands` · `listRepoMarkdown` · `describe("★MINOR-5(9R) …")` |
| **면제 마커 계약** — 예외는 테스트 파일이 아니라 **면제받는 문서 자신의 머리**에 적는다. 형식은 HTML 주석 한 줄이며 `doc-command-pin` 키 뒤에 `allow(...)` 갈래 목록과 `reason:` 사유를 잇는다(★이 문서에 그 문자열을 **그대로 인용하지 않는다** — 스캐너가 진짜 마커로 읽어 이 문서를 면제해 버린다. 실물 형식은 `docs/plans/2026-06-29-cli-path-install.md` 머리 한 줄이 정본이다). 사유 30자 미만·사용자 문서에 붙은 마커·갈래를 못 읽은 마커는 전부 실패. | 같은 파일 `readDocPinMarker` + 마커 자체의 변이시험 |
| **두 자리를 면제가 아니라 수리로 닫았다** — `docs/INSTALL.md` "필요 없으면 버리기"의 맨 `sudo rm <절대경로>` → 백업 **이름 규칙 검사** + 존재 검사. `docs/GUIDE-clean-reset-KR.md` ④ 앱 삭제의 맨 `sudo rm -rf /Applications/cys.app` → `CFBundleIdentifier` 가 `com.cysjavis.terminal` 일 때만 지우는 블록. | 두 문서의 해당 블록 |
| **8차 판정 #2 수리** — `mv -n` 의 보증이 **대상이 사라진 심볼릭(dangling)** 에서 거짓이라는 것(BSD `mv -n` 판정은 `access(to, F_OK)` = 심볼릭 추종)을 문구·안내에서 바로잡았다. | `ui/src/clipath.ts:451` 주석 |
| **문서↔테스트 결합 명문화**(§7.6) · **§9.3 U8 등재** | 이 문서 |

#### ★9차 실사고 — 검증 하네스가 살아 있는 `cysd` 데몬을 죽였다 (26분 정지)

> 이 항목은 이 문서에서 §4.14 다음으로 값진 기록이다. **감추면 다음 사람이 같은 자리를 밟는다.**

9차에서 한 에이전트가 위 스캐너를 만들면서 **문서 코드블록이 실제로 안전한지 실행해 확인**하려
했다. 그 블록에는 `docs/GUIDE-clean-reset-KR.md` 의 완전 초기화 절차가 들어 있었고, 그 안의
`launchctl bootout gui/$(id -u)/com.cysjavis.cysd` 와 `pkill -x cysd` 가 **샌드박스가 아니라
실계에서 그대로 돌아** 이 기계에서 살아 움직이던 `cysd` 데몬을 종료시켰다. 복구까지 **26분**
정지했다.

**근인이 이 라운드들의 병과 정확히 같다.** 그 하네스에는 위험 명령 거부 패턴이 **정의되어
있었지만**, 실제로 명령을 실행하는 지점(`refuse()`)에 **연결되어 있지 않았다.** 선언된 가드가
집행에 닿지 않은 것이다 — MAJOR-6(경로 정규화를 판정에만 넣고 root 셸에는 안 넣음)·MAJOR-2
(사전 관측만 하고 권한이 오르는 지점에서 재검증 안 함)와 **한 글자도 다르지 않은 형태**다.

**그래서 이 병은 세 곳에서 반복됐다:**

| # | 어디서 | 선언된 가드 | 연결되지 않은 집행 지점 |
|---|---|---|---|
| ① | **제품 코드** | `normalize_path_str` (Rust 판정) | root 로 도는 승격 셸의 `case` 대조 (5차 MAJOR-6) |
| ② | **테스트** | "가드가 장식이 되는 형태를 막는다"는 주석·핀 | 실제 순서·최종 대입을 보지 않는 부분 문자열 검사 (10차 MAJOR-2·3 = 변이 M8·M9 가 통과) |
| ③ | **검증 도구 자신** | 위험 명령 거부 패턴 목록 | 명령을 실행하는 `refuse()` (9차 실사고) |

**10차가 채택한 재발 방지**(즉시 발효, 규약 ⑰로 승격 — §5):
- **문서의 코드블록은 실행해서 검증하지 않는다.** 문서 절차의 안전성은 **문자열 정적 검사**
  (9차 스캐너)와 **임시 디렉터리 사본을 대상으로 한 코드 테스트**로만 확인한다.
- 검증 하네스가 거부 목록을 갖는다면, **그 목록이 실제로 거부를 일으키는지**를 먼저 시험한다
  (거부 패턴 자체의 변이시험). 목록만 있고 시험이 없으면 그것은 가드가 아니라 주석이다.

### 4.13 10차 — 남은 네 자리에서 같은 병을 닫는다 (커밋 `c6f3669` = **11차 freeze**)

10차는 새 기능을 만들지 않는다. **선언된 가드를 집행에 연결하는 것만** 한다.

| # | 등급 | 위치 | 결함 | 처리 |
|---|---|---|---|---|
| 1 | **MAJOR** | `ui/src/clipath.ts` `backupNoticeLine` | 앱이 상시 상태 토스트·버튼 툴팁·해제 확인 창 **세 표면 모두**에서 맨 `sudo rm <절대경로>` 를 내밀었다. 그 대상은 **밀려난 남의 원본의 마지막 사본**이다. 8차가 `docs/INSTALL.md` §B 에 만든 가드 블록(이름 규칙 + 존재 확인 + `ls -l` 로 눈으로 보기)이 화면 문구에는 적용되지 않았다. | UI 레인 — 화면 문구에 같은 판정 적용 + '되돌릴 마지막 사본입니다' 경고 동반 + 앱이 출력하는 **모든** 명령 문자열 재점검 |
| 2 | **MAJOR** | `ui/src/clipath.test.ts` (동시성 핀) | 주석이 "가드가 장식이 되는 **유일한** 형태를 막는다"고 단언했으나 거짓. 변이 **M9**(`cliStatus = view;` 를 `if (gen !== cliStatusGen) return;` **앞으로** 한 줄 이동 = last-writer-wins 부활)가 **초록으로 통과**했다. | UI 레인 — M9 가 빨개지는 형태로 재작성(순수 함수+세대 인자 / 지연 주입 seam 등) + 거짓 단언 정정 |
| 3 | **MAJOR** | `ui/src/clipath.test.ts` (등급색 핀) | 부분 문자열 검사라 우회된다. 변이 **M8**(`ui/src/main.ts` 의 `el.className = toastClassName(category);` 를 지우지 않고 **다음 줄에** `el.className = "toast";` 추가)로 앱 전역 volatile 토스트 등급색이 전부 죽는데 **초록**. | UI 레인 — 마지막 대입/실제 DOM 최종 `className` 을 보는 형태로 + "이 계열을 닫았다"는 주장 정정 |
| 4 | **MAJOR** | `docs/GUIDE-clean-reset-KR.md` | 9차가 0단계·3단계를 신설하면서 **단계 흐름이 어긋났다.** "버튼을 누른 적이 없거나 앱이 이미 열리지 않는다면 0단계는 건너뛰고 **3단계**를 보세요" — 지시대로 따르면 1단계(터미널)와 2단계(launchd 해제·종료·앱 삭제·데이터 삭제·화면 저장값 삭제)를 통째로 건너뛴다. **완전 초기화 가이드인데 초기화가 되지 않는다.** | 문서 레인 — 조건 분기표 신설(어느 경우든 다음은 1단계 · 3단계는 0단계를 못 했을 때 2단계 뒤) + 오독 이력을 문서에 남김 |
| 5 | MINOR | `docs/GUIDE-clean-reset-KR.md` (⑤ 삭제 줄) | 문서의 '완전 초기화'가 앱의 **보존 계약**을 어긴다. `src/factory_reset.rs` 는 `~/.cys` 를 통째 이동하지 않고 알려진 항목만 격리하며 **라이선스·`local` 오버레이·미등록(오너 배치) 파일·`cys-trash`** 를 남긴다. 문서의 `rm -rf` 는 그 넷을 전부 지운다. | 문서 레인 — ⑤ 를 **⑤-A(먼저 통째 복사) / ⑤-B(눈으로 목록 확인 후 삭제)** 로 분리 + 보존 계약 4항목 명시 + 윈도우 2단계에도 같은 선복사 단계 |
| 6 | MINOR | `ui/src/clipath.test.ts` (회귀핀 '알려진 한계') | 주석이 실측 사각을 다 적지 않았다 — 홈 상대경로(`rm -rf ~/…`) · PowerShell `Remove-Item -Recurse -Force` · 변수 경로. | UI 레인 — 잡을 수 있으면 확장, 못 잡는 것은 **정확히** 명시(오탐 리포 전수 확인 동반) |
| 7 | MINOR | `src-tauri/src/main.rs` `build_install_script` | 승격 스크립트가 `/bin/mv {d} {b}` 로 직행 — **백업 목적지 이름이 이미 차 있는지 보지 않는다.** 같은 절차의 문서 정본(`docs/INSTALL.md` §B)은 `[ -e "$b" ] || [ -L "$b" ]` 로 이미 중단한다 = **문서가 코드보다 안전한 비대칭**. 8차는 도달성이 낮다며 미수리로 남겼다(§9.3 U2). | **수리 완료(10차)** — 문서와 같은 형태(사전 존재 검사 → `exit 1`, 사유는 stderr)로. ★`mv -n` 오답은 채택하지 않았다(거부해도 exit 0 → `&&` 체인이 이어져 백업 없이 링크). 회귀핀 `install_script_aborts_when_backup_name_collides` |
| 8 | MINOR | 이 문서 | 인수 좌표가 사실과 달랐다 — HEAD·커밋 수·규모·테스트 수. | **수리 완료(10차)** — 전부 명령 출력에서 재유도(§9.1·§9.2·부록 A·B) |

★**10차가 이 문서를 고치다 스스로 밟은 함정(기록)**: §4.12 에 9차 면제 마커의 형식을 **그대로
인용**했더니, 9차 스캐너가 그것을 **진짜 마커로 읽어** 이 설계문서 자신을 면제 대상으로 판정했다
(`bun test` 가 "사유가 부실한 문서 / 갈래를 못 읽은 마커"로 즉시 빨개졌다). 문서에 적은 **예시**가
살아 있는 **설정**이 되는 형태다 — 규약 ⑯("사용자에게 도달하는 모든 경로")의 문서 축 안쪽 사례.
9차의 핀이 같은 라운드에 그것을 잡았다는 사실 자체가 그 핀이 장식이 아니라는 증거다. 수리는
인용을 풀어 쓰는 것으로 했다.

★7번의 **남은 사각(정직 기록)**: 충돌로 중단하면 `observe_existing_backups` 는 **먼저 있던** 같은
이름의 백업본을 보고 "백업됐다"고 읽는다(그 함수는 관측만 하고 판정하지 않는다). 중단 메시지가
"그 자리의 `<원본>` 은 그대로 두었습니다"라고 함께 말하지만, 두 문장이 한 화면에 나온다. 이 사각은
충돌 케이스 밖에서도 원래 존재했다(사용자가 승격 창에서 취소했는데 같은 스탬프의 백업본이 이미
있던 경우) — 별도 티켓이다.

### 4.14 ★master 의 판단 오류 4건 (정직 기록)

> 이 절이 이 문서에서 가장 값진 부분이다. 여덟 라운드에서 나온 결함의 상당수는 워커의 실수가
> 아니라 **master 가 내린 판단의 결과**였다. 다음 사람이 같은 함정을 밟지 않게 하려면 그것을
> 감추지 않고 적어야 한다.

#### 오류 ① 반환 계약을 못박지 않은 채 병렬로 발진시켰다 (계약 드리프트의 근인)

1라운드 설계서(D4)는 새 커맨드 두 개(`uninstall_cli_from_path`·`cli_install_status`)의 **이름과
역할**만 정하고 **반환 형태를 정하지 않았다.** 그 상태로 Rust 쪽과 TS 쪽을 **병렬로** 보냈다.

결과는 예정된 것이었다 — Rust 는 `skipped: Vec<String>` 을 만들었고 TS 는 `{path, reason}[]` 로
상상했다. TS 의 `.filter(s => s && s.path)` 가 **모든 skip 을 소멸**시켰고 부분 실패가 성공
토스트로 둔갑했다. `warnings`·`ok` 는 TS 타입에 아예 없어서 **유일한 복구 명령이 사용자에게
도달하지 못했다.** 단위테스트는 초록이었다 — 픽스처가 상대의 실물이 아니라 자기 상상이었기
때문이다. **테스트가 드리프트를 봉인했다.**

> **이름 3개와 enum 1개는 계약이 아니라 추측 허가다.** 병렬 작업의 경계에서 설계자가 필드를
> 못박지 않으면, 두 사람은 각자 다르게 상상하고 각자 자기 상상에 맞는 테스트를 쓴다.
> → 규약 ③ 이 이 오류에서 나왔다. 그리고 §3 이 존재하는 이유가 이것이다.

#### 오류 ② 쓰는 에이전트와 읽는 감사자를 **같은 트리에 동시에** 배치했다 (거짓 RED 관측 오염)

어느 라운드에서 master 는 속도를 위해 **구현 워커와 검증 감사자를 병렬로** 돌렸다. 둘은 같은
작업 트리를 봤다. 그 결과 감사자가 관측한 실패(RED)가 **자기가 감사하는 코드 때문인지, 옆에서
워커가 파일을 고치는 중이라서인지 구분되지 않았다.** 테스트가 빨간 이유가 두 가지가 되면 그
관측은 증거가 아니다 — 감사 결과 일부를 폐기하고 다시 돌려야 했다.

이것은 헌장이 이미 말하고 있던 것을 어긴 것이다: **"산출자는 자기 산출물의 통과를 판정하지
않는다"** 는 산출자와 판정자를 **분리**하라는 뜻이지, 둘을 **동시에** 돌리라는 뜻이 아니다.
분리는 인적 분리(누가)와 시간적 분리(언제) 둘 다여야 한다. 읽는 쪽은 **정지한 리비전**을 봐야
한다 — 그래서 이 프로젝트의 라운드 규약에 `freeze`(로컬 커밋으로 리뷰 대상 리비전을 확정하고
verdict 도착 전에는 수정하지 않는다)가 있는 것이다. master 는 그 규약을 알고 있으면서 병렬
효율을 이유로 건너뛰었다.

> **교훈**: 병렬화는 **쓰기끼리** 겹치지 않게 하는 기술이 아니라, **읽기와 쓰기**가 겹치지 않게
> 하는 기술이다. 같은 산출물에 대해 쓰기는 단일 스레드이고, 감사는 그 스레드가 멈춘 뒤에 시작한다.
> (이번 8라운드는 이 교훈을 적용했다 — 판정 12건은 freeze `0b6cb24` 라는 **정지한 리비전**에서
> 나왔고, 수리 레인들은 **서로 다른 파일**로 갈라 배치했다. 이 문서 레인이 만지는 파일은
> `docs/plans/2026-08-25-shell-cli-restore-design.md` 하나뿐이다.)

#### 오류 ③ `partial && notes 비어있지 않음 ⟹ 남의 것 존재` 라는 판정 규칙을 제시했다

7라운드 MAJOR-1(등급 회귀)을 고칠 때, master 는 워커에게 판정 근거를 직접 제시했다 —
"`state == partial` 이고 `notes` 가 비어 있지 않으면 남의 파일이 있는 것이다."

**워커가 이것을 반증했다.** 두 세계가 기존 계약 필드로 **완전히 동일하게 관측**된다:

| | 안전한 세계 | 위험한 세계 |
|---|---|---|
| 상황 | 우리 cys 링크 있음 + cysd **부재** | 우리 cys 링크 있음 + **남의 실체 cysd 파일** |
| `state` | `partial` | `partial` |
| `installed` | `true` | `true` |
| `notes.len()` | 1 | 1 |
| `backups` | `[]` | `[]` |

이유는 구조적이다 — 남의 실체 파일이 그 자리에 있으면 `which` 1순위가 곧 target 이라 **그림자
축이 침묵한다.** 따라서 `notes` 의 유무·개수는 경고 근거가 될 수 없다. 워커가 제시한 대체
규칙은 **`state` 단독**(`partial`\|`foreign` = 경고)이었고, 과경고가 아님도 구조로 보장된다
(`classify_cli_links` 상 `Ours` 는 두 축 전부 `Remove`, `Absent` 는 전부 `SkipAbsent` 이므로
`foreign == 0`). master 는 이 반박을 **수용**했고, 4단 증명을 순수 테스트로 못박게 했다
(`major1_premise_partial_with_notes_does_not_imply_foreign_present`).

> **교훈 셋**:
> ① master 가 판정 **규칙**까지 지정하는 것은 위험하다 — 요구(무엇을 보장해야 하는가)는 master 가
>   정하고, 그것을 어떤 관측으로 판정할지는 실물을 만지는 쪽이 검증해야 한다.
> ② **"두 세계가 같은 관측값을 낸다"는 반박은 가장 강한 형태의 반박이다.** 근거 없이 기각해서는
>   안 되고, 이번에는 그러지 않았다(수용 + 테스트로 못박음)는 사실도 함께 기록한다.
> ③ 커밋 트레일러의 `Rejected:` 칸에 **master 자신의 안이 기각된 기록**이 남아야 한다. 실제로
>   `0b6cb24` 의 `Rejected:` 첫 항목이 그것이다 —
>   `master 의 partial&&notes 판정 규칙(워커가 관측 동일성으로 반증 — 수용)`.

#### 오류 ④ 검증 하네스에 '거부 목록'만 두고 그것이 실제로 거부하는지 시험하지 않았다 (9차 실사고)

9차에서 문서 코드블록의 안전성을 **실행으로** 확인하는 하네스를 돌렸다. 그 하네스에는 위험 명령
거부 패턴이 정의되어 있었으나 **실행부(`refuse()`)에 연결되어 있지 않았다.** 그래서 문서 안의
`launchctl bootout …com.cysjavis.cysd` · `pkill -x cysd` 가 실계에서 그대로 돌아 **살아 있는 데몬을
죽였다**(26분 정지 · 상세는 4.12).

master 의 오류는 둘이다:

1. **"검증하려면 실행해 봐야 한다"를 의심하지 않았다.** 문서 절차의 안전성은 실행 없이도 잰다 —
   9차가 바로 그날 만든 **정적 스캐너**가 그 방법이고, 코드 축은 **임시 디렉터리 사본**을 대상으로
   한 테스트가 이미 그 방법이었다(`install_script_*` 계열은 `/usr/local/bin` 을 한 번도 만지지
   않는다). 즉 안전한 방법이 이미 리포 안에 **두 개** 있었는데 쓰지 않았다.
2. **가드의 존재를 가드의 작동으로 읽었다.** 이 문서가 규약 ⑤에 "'확인한다'는 요구는 계측기를
   함께 적을 때만 유효하다"고 써 두고도, 하네스 자신의 거부 목록에는 계측기를 요구하지 않았다.
   **규약이 제품에는 적용되고 도구에는 적용되지 않는 상태**가 세 라운드째 반복된 것이다
   (7차 MINOR-4: 스캐너가 `fn` 만 보던 사각 · 9차: 거부 목록이 연결되지 않은 사각).

→ **규약 ⑰ 신설**(§5). 그리고 10차는 **문서 코드블록을 한 줄도 실행하지 않고** 완주했다.


---

### 4.15 11차 — 문서가 약속한 백업이 집행 단계에 없었다 (문서 레인 몫 · 미커밋)

> 라운드 순서로는 §4.13(10차) 다음이다. §4.14 는 라운드가 아니라 정직 기록이고 이 문서 여러
> 곳에서 **번호로 참조**되므로 다시 매기지 않고, 새 라운드를 뒤에 이어 붙인다.

11차의 지배 질병은 열 라운드 내내 되풀이된 **"선언과 집행의 분리"** 다. 지금까지 그것은
제품 코드(판정에는 정규화가 있는데 집행 셸에는 없다 — 6차 MAJOR)와 검증 도구(거부 목록은
정의했는데 `refuse()` 에 연결하지 않았다 — 9차 실사고 §4.12)에서 나왔다. 11차는 **문서에서**
같은 형태를 찾았다: **머리에서 한 약속이 실제 집행 단계에 없었다.**

| # | 등급 | 위치 | 결함 | 처리 |
|---|---|---|---|---|
| 1 | **BLOCK** | `docs/GUIDE-clean-reset-KR.md` 머리(:33-37) ↔ 집행(⑤-A :193) | 머리가 ⑤-B 가 지우는 넷(라이선스 · `local` 오버레이 · 열쇠 파일 · 격리보관본 `cys-trash`)을 열거하고 **"그래서 맥은 2단계 ⑤-A 에서 먼저 복사"** 라고 단언하는데, ⑤-A 는 `cp -R ~/.cys "$KEEP"/` 로 **`~/.cys` 하나만** 복사했다. `cys-trash` 는 `~/.local/state/cys-trash` 이고(`src/factory_reset.rs:204` `trash_root`), 부서별 대화 기록 `~/.local/state/cys-dept-*` 와 함께 ⑤-B 의 `rm -rf` 로 **비가역 삭제**된다. 윈도우 절도 같은 형태였다(2단계 4번은 `.cys` 만 복사, 6번이 `cys-trash` 를 복사 없이 삭제). ★머리의 약속을 읽고 절차를 따른 사용자는 **초기화를 되돌리는 창구와 부서 대화 기록을 "복사됐다"고 믿은 채 영구히 잃는다.** | 문서 레인 — **(a) 집행을 약속에 맞췄다.** ⑤-A 가 목록 파일 하나(`지울목록.txt`)를 만들어 그 목록을 통째로 복사하고, ⑤-B 는 **그 파일만** 읽어 지운다 = 복사 대상과 삭제 대상이 **같은 하나**라서 다시 갈라질 수 없다. 윈도우는 `cys-keep` 폴더 신설 + 5·6·7번이 저마다 *복사 → 대조 → 삭제* |
| 2 | **MAJOR** | 같은 문서 ⑤-A(:193-194) | 유일한 백업 단계가 **자기 실패를 숨기고 원인을 반대로 말했다.** `cp -R … 2>/dev/null` 이 stderr 를 전부 버리고 종료코드를 아무도 검사하지 않았다. 확인 수단(`ls -l "$KEEP"/.cys 2>/dev/null` 뒤에 "복사할 것이 없었습니다(.cys 폴더가 이미 없습니다)" 를 붙인 형태)은 (a)복사가 아예 실패했을 때 **'원본이 없다'는 거짓 원인**을 말하고 (b)부분 복사면 최상위 목록이 정상으로 보여 완전 복사와 구분되지 않는다. 그 상태에서 ⑤-B 의 `rm -rf` 가 원본을 지운다. | 문서 레인 — stderr 를 살리고 **종료코드를 검사**, 여기에 **원본↔사본 항목 수 대조**를 더해 **둘 다 통과할 때만** `복사완료.txt` 를 남긴다. ⑤-B 는 그 확인(과 목록 파일)이 없으면 **아무것도 지우지 않고 멈추고**, 통과하더라도 **지우기 직전에 항목마다 사본을 다시 세어** 맞지 않으면 그 항목을 건너뛴다(D8 = TOCTOU) — "실패하면 멈추세요"가 문구가 아니라 **명령 자신의 거부**로 들어갔다 |
| 3 | MAJOR-6 **계열** | 같은 문서 0단계·3단계 · `docs/INSTALL.md`(3곳) · `USER-MANUAL.md` | UI 레인에 배정된 MAJOR-6(같은 대상 백업본이 여럿이면 앱이 거짓말한다)과 **같은 문장이 문서 여섯 자리에도 있었다**(가이드 0단계·3단계 · `docs/INSTALL.md` 3곳 · `USER-MANUAL.md` 1곳) — "백업해 둔 것이 있으면 해제가 그것을 제자리에 되돌려 놓습니다". `pick_restore_backup`(`src-tauri/src/main.rs:2509`)은 한 대상에 대해 **스탬프가 가장 큰 하나만** 고른다(되돌릴 자리가 하나뿐이므로). 옛 백업본은 자동 복원되지 않는데 문서는 그 사실을 한 글자도 말하지 않았고, `docs/INSTALL.md` §B 의 4) 블록은 바로 그것을 **지우는 명령**이다. | 문서 레인 — 여섯 자리 모두 "**최신 하나만** 자동 복원 · 옛 백업본은 자동으로 돌아오지 않으니 **지우기 전에** §B 3) 으로 손수 되돌린다"로 정정(코드는 건드리지 않았다 — 문구를 코드의 실제 동작에 맞췄다) |

#### ★11차가 스스로 밟은 함정(기록) — zsh 에서 `$A개` 는 **빈 문자열**이다

MAJOR-2 수리로 넣은 대조 줄의 초안은 `echo "$N: 원본 $A개 / 사본 $B개 …"` 였다. 임시 홈 사본으로
돌려 보니 **개수 자리가 통째로 비어 있었다**(`.cys — 원본  / 사본  · 16K -> 16K`). zsh 는
MULTIBYTE 옵션이 켜진 채 `개` 를 식별자 글자로 읽어 `$A개` 를 **`A개` 라는 없는 변수**로 해석한다
(bash 는 `$A` 뒤에 깨진 바이트를 붙인다 — 실측: zsh 는 빈 문자열, bash 는 대체문자).

그대로 나갔으면 `[ "$A" = "$B" ]` 가 **빈 문자열끼리 비교해 언제나 통과**했다. **부분 실패를
잡겠다고 넣은 검사가 아무것도 잡지 못하는** — MAJOR-2 와 **정확히 같은 병**이 그 수리 안에서
재생산될 뻔했다. `${A}`·`${B}` 로 감싸 닫았고, 빈 값 자체도 실패로 세도록 `[ -n "$A" ]` 를 함께
두었다. 규약 ⑰(검증 장치 자신에게도 같은 규약)의 문서 축 사례이고, **한국어 문서의 셸 블록에서는
변수 뒤에 한글이 붙는 자리마다 중괄호가 필수**라는 실무 규칙이 여기서 나왔다.

#### ★변이시험 — 수리마다 '이 검사를 우회하는 두 번째 형태'를 만들어 시험했다

전부 **`HOME` 을 임시 디렉터리로 바꾼 사본**에서 돌렸다. 실계 `~/.cys`·`~/.local/state` 는
**읽기만** 했고 문서의 코드블록은 **한 줄도 실행하지 않았다**(규약 ⑰ · 9차 실사고 §4.12).

| 변이 | 무엇을 흉내내나 | 결과 |
|---|---|---|
| **D1** 하위 폴더 `chmod 000` | 권한·디스크 때문에 `cp` 가 중간에 실패 | `cp` 가 사유를 **화면에 그대로** 내고 종료코드로 잡힘 → `복사완료.txt` 없음 → ⑤-B 거부 · 원본 전부 생존 |
| **D2** `cp` 종료코드는 0 인데 사본에서 파일 하나 제거 | 조용한 부분 복사 | **항목 수 대조**가 잡음 → ⑤-B 거부 · 원본 생존 |
| **D3** 지울 것이 하나도 없는 홈 | 이미 초기화된 기계 | "지울 것이 아무것도 없습니다"로 **정확히** 보고 — 옛 판이 *복사 실패*에도 내던 그 거짓 원인 문구가 사라졌다 |
| **D4** 터미널을 새로 열어 `KEEP` 을 잃음 | 절차를 며칠에 걸쳐 진행 | ⑤-B 가 확인 파일을 못 찾아 거부 · 원본 생존 |
| **D5** 보관 폴더의 목록 파일을 손으로 삭제 | 사람이 중간 산출물을 건드림 | ★초안은 아무것도 못 지우고 **"지웠습니다"라고 거짓 보고**했다 → 목록 파일 존재도 조건에 넣고 **실제 처리 개수·실패 개수를 보고**하도록 재수리 |
| **D6** 유닉스 소켓이 섞인 트리 | 데몬이 남긴 `cys.sock`(실계 `~/.cys/state-harness/cys.sock`·`~/.local/state/cys/cys.sock` 에 **실재**) | BSD `cp -R` 은 소켓에 대해 메시지만 내고 **종료코드는 0** 이다(실측). 항목 수 대조에서 `! -type s` 로 소켓을 빼지 않으면 **정상인데 실패로 읽는** 반대 방향 오탐이 난다 |
| **D7** ⑤-A 를 두 번 돌린 뒤 ⑤-B | 사용자가 불안해서 다시 한 번 복사 | 보관 폴더가 둘 생기고 ⑤-B 는 **마지막 것**을 쓴다 · 삭제 결과 정상(4개) · 멱등 |
| **D8** ★영수증은 남았는데 **사본이 사라진** 상태에서 ⑤-B | ⑤-A 뒤에 사람이 보관 폴더를 건드림 = **TOCTOU** | 초안(영수증 하나만 보는 판)은 **원본을 지웠다.** → ⑤-B 가 **지우기 직전에 항목마다 사본을 다시 센다**로 재수리. 사본이 사라진 `.cys`·`cys-trash` 는 **건너뛰고 생존**, 사본이 멀쩡한 것만 삭제, 끝에 `STOP — 건너뛴 것이 있습니다` |
| **D9** 사본 안의 파일 하나만 조용히 사라짐 | 보관 폴더 손상 | 같은 재검증이 잡아 그 항목만 건너뜀 · 원본 생존 |

★**D1 과 D2 는 서로를 못 잡는다 — 그래서 둘 다 있어야 한다.** D1 에서는 원본 쪽도 읽지 못해
항목 수가 `6 / 6` 으로 **같게** 나왔다(대조만으로는 통과). D2 에서는 `cp` 종료코드가 **0** 이었다
(종료코드만으로는 통과). 검사 하나로 그 계열을 닫았다고 선언했으면 다음 라운드에서 다시 뚫렸다 —
규약 ⑬(지점이 아니라 계열)을 **검사 축**에 적용한 것이다.

★**D8 은 이 라운드가 이름 붙인 지배 질병의 다른 얼굴이었다 — TOCTOU.** 초안의 ⑤-B 는 ⑤-A 가
남긴 **영수증 한 장**만 보고 지웠다. 즉 *확인은 과거 시점에 하고 파괴는 현재 시점에 하는* 형태로,
이 라운드 브리핑이 제품 코드에서 지목한 바로 그 결함("사전 관측만 하고 권한이 오르는 지점에서
재검증하지 않았다")을 **수리 쪽이 그대로 복제**한 것이다. 지금은 ⑤-B 가 **지우기 직전에 항목마다
사본을 다시 세고**, 맞지 않는 항목은 지우지 않고 건너뛴다. 영수증 게이트는 그대로 둔다 — 그것만이
`cp` 의 **종료코드**(D1)를 실어 나르기 때문이다. **두 층이 서로 다른 것을 지킨다.**

★**문서 축의 기계 게이트는 이 라운드에서도 초록이었다**: `bun test ui/src/clipath.test.ts`
(9차가 만든 배포 문서 코드블록 전수 스캐너 + 거울쌍) 해당 describe 12+12건 통과. 다만 그 초록은
**새 ⑤-A/⑤-B 를 본 결과가 아니다.** 그 블록은 절대경로 `rm`/`mv` 를 쓰지 않고 **변수 경로**
(`rm -rf "$P"`)만 쓰므로 스캐너의 알려진 사각 ③에 들고, 옛 형태(`rm -rf ~/.cys …`)는 홈 상대경로
= 사각 ①에 든다. **두 형태 모두 스캐너가 보지 못한다**(그 사각은 `clipath.test.ts` 자신이 대조군
으로 못박아 두었다).

★**그리고 이 자리를 지키는 자동 검사는 레포에 없다 — 위 D1~D9 는 1회성 수동 시험이다.**
아홉 종 전부 그 자리에서 손으로 돌려 확인했고(임시 `HOME` 사본), **스크립트로도 테스트로도
레포에 남기지 않았다.** 실측(2026-08-25 12차): `지울목록`·`복사완료` 문자열을 리포 전체에서
grep 하면 나오는 곳은 `docs/GUIDE-clean-reset-KR.md` 와 **이 설계문서** 둘뿐이고,
`scripts/`·`*.test.ts` 에는 **0건**이다. 12차 판정자의 우회 시험 M-F 가 그 뜻을 실측으로 보였다 —
**⑤-A 블록을 통째로 지우고 ⑤-B 를 옛 형태로 되돌려도 `bun test` 는 초록이고 아무것도 빨개지지
않는다.** (★출처를 밝힌다: 문서 레인은 그 변이를 **직접 돌리지 않았다** — 실측은 판정자의 것이고,
문서 레인이 확인한 것은 ①위 grep 0건과 ②스캐너 자신이 사각 ①·③을 **대조군으로 못박아 둔 단언**
(`clipath.test.ts` 의 "★사각 ① 홈 상대경로는 아직 못 잡는다" 와 변수 경로 대조군)이다. 두 근거가
같은 결론을 가리킨다.)

★그래서 이 자리에 대해 말할 수 있는 것은 하나다: **회귀가 조용히 들어올 수 있다.** 11차의 산문은
"이 자리는 스캐너가 아니라 위 변이시험 6종이 지킨다"고 **현재형으로** 적었는데, 그 문장은 두 군데가
사실이 아니었다 — ①시험은 아홉 종이고 ②'지킨다'가 아니라 **'그때 한 번 확인했다'** 이다(수는 세지
않고 옮겨 적었고, 1회성 확인을 상주 보증으로 격상했다 — 이 레인이 반복해 온 '선언과 집행의 분리'가
**보증문 쪽에서** 나온 형태다). 다음 사람이 이 절차를 고칠 때 기댈 수 있는 것은 자동 게이트가
아니라 **D1~D9 를 손으로 다시 돌리는 것**뿐이다(규약 ⑰ — 실계에서 실행하지 말고 임시 `HOME`
사본에서). 12차는 하드 스톱 규칙(새 기계 장치 금지)에 따라 **자동 검사를 만들지 않았다** —
만드는 것은 다음 라운드의 일이고, 그 사실을 §9.3 U10 에 미결로 올린다.

#### ★11차가 남긴 실무 규칙 (다음 사람에게)

1. **문서의 머리가 "그래서 여기서 먼저 ~한다"고 말하면, 그 '여기'를 열어 실제로 하는지 세라.**
   BLOCK-1 은 열거된 넷 중 **셋만** 우연히 한 폴더 안에 있어서 오래 살아남았다.
2. **삭제 목록과 백업 목록은 같은 한 곳에서 나와야 한다.** 두 곳에 적으면 한쪽만 고쳐진다
   (이 프로젝트의 반복 결함 = 정본 이원화).
3. **"실패하면 멈추세요"는 문구가 아니라 명령이 해야 한다.** 문서 절차에서도 다음 단계가
   **스스로 거부**할 수 있다(확인 파일 하나면 된다).
4. **검사를 넣었으면 그 검사를 우회하는 두 번째 형태를 만들어 직접 시험하라.** D5 와 zsh 함정은
   둘 다 '수리했다고 선언한 뒤에' 시험해서 나왔다.

### 4.16 12차 — **마지막 라운드**: 손실 경로 하나를 닫고, 나머지는 약속을 사실로 좁혔다

> 라운드 순서로는 §4.15(11차) 다음이다. 이 절이 이 작업의 **마지막 라운드 기록**이다.

**이 라운드의 규칙은 앞의 열한 번과 반대였다.** 이 작업은 헌장 상한 10라운드를 이미 넘겼고
(11차), 12차가 마지막으로 지정됐다. 그래서 오너가 건 하드 스톱은 이렇다:

- **새 핀을 만들지 않는다.** 기존 핀의 사거리를 넓히지도 않는다.
- **새 약속을 쓰지 않는다.** 새 보증문·새 단언을 문서·주석·화면에 추가하지 않는다.
- 하는 일은 딱 둘 — **①손실 경로를 닫는다 ②이미 쓴 약속을 사실로 좁힌다.**
- 개선하고 싶은 것이 있으면 코드가 아니라 **미결(§9.3)에 적는다.**

이 규칙 자체가 이 레인의 열두 라운드가 배운 것의 요약이다. 열한 라운드 동안 반복해 닫은 지배
질병은 **"선언과 집행의 분리"** 였고, 라운드를 거듭할수록 그 병이 **수리 쪽**에서 재생산됐다
(9차 실사고 §4.12 · 11차 zsh 함정 §4.15). 마지막 라운드에서 새 장치를 하나 더 세우면 **그 장치가
지키지 못하는 새 약속**이 하나 더 남는다. 그래서 12차는 장치를 늘리지 않고, **이미 쓴 문장이
사실인지**만 본다.

| # | 등급 | 위치 | 결함 | 처리 |
|---|---|---|---|---|
| 1 | **BLOCK** | `ui/src/clipath.ts` `backupNoticeLine`(11차 판 :700-745) | ★**실제 손실 경로.** `autoRestored` 인자를 **네 갈래 중 한 갈래(`ours`=occupied)만** 읽었다. `absent`(free)·`unknown`(partial·foreign) 갈래는 그 인자를 **아예 읽지 않는다.** 그래서 같은 원래 경로에 백업본이 둘이면 화면이 **같은 목적지로 가는 `sudo mv -n` 두 줄**을 나란히 내고 **두 줄 모두** 자기를 '⚠ 마지막 사본'이라 부르며 `sudo rm` 꼬리를 달았다. 판정자 실측: `cliNoticeLines({backups:[cys.cys-backup-1756000000, cys.cys-backup-1756089600], linkState:"absent"})` → 동일 목적지 mv 2줄 + '마지막 사본' 2회. `/bin/mv -n` 은 **끊어진 심링크 자리를 조용히 덮어쓰고 exit 0**(이 레포가 §B 에 스스로 적어 둔 BSD 실측) — 첫 줄이 되돌려 놓은 원본이 둘째 줄에 소멸하는데 **오류가 없다.** 사용자는 '둘 다 되돌렸다'고 믿고, 돌아오지 않은 **더 최신 원본**을 같은 줄의 `sudo rm` 으로 지운다. 지워지는 것은 **이 기능이 존재하는 이유인 남의 실체 바이너리**다 | UI 레인 — `autoRestored` 판정을 **네 갈래 전부**에 연결. 같은 원래 경로의 사본이 여럿이면 **최신 하나에만** 복원·이동을 말하고, 나머지는 사실대로 말한다(자동 복원 대상이 아니다 · 먼저 되돌린 뒤 판단하라 · **'마지막 사본'이라 부르지 않는다**). 새 테스트는 **네 갈래 × 사본 1개/2개** 조합 전부 |
| 2 | MAJOR | 같은 함수 `unknown` 갈래 | 위와 같은 다중 맹목의 다른 얼굴 | UI 레인 — 1번과 **같은 수리로 함께** 닫힌다 |
| 3 | MAJOR | `docs/INSTALL.md`(계약표) ↔ `ui/src/clipath.ts` | **문서 계약표가 코드와 반대로 말했다.** 표는 "원래 자리를 이 앱의 링크가 차지하고 있을 때 — 옮기는 명령을 **내지 않습니다**"라고 단언하는데, `!autoRestored` 갈래는 `state="ours"` 인데도 `sudo mv -n <옛 백업본> /usr/local/bin/cys` 를 냈다. 같은 취지가 `USER-MANUAL.md` 에도 있었다 | **(a)+(b) 둘 다.** UI 레인이 **(a)** 를 택했다(자리가 찼다고 아는데 옮기라고 말하는 것은 이 레인이 세운 규약 — '앱이 아는 위험은 명령으로 내지 않는다' — 위반이다). 문서 레인은 (b) 쪽 몫으로 **세 갈래 표에 네 번째 단서**를 붙였다: 옮기는 명령은 **한 자리에 대해 최신 사본 하나에만** 붙고 옛 사본에는 어느 갈래에서도 붙지 않는다 |
| 4 | MAJOR | `docs/GUIDE-clean-reset-KR.md` 머리(:40) | **머리의 단언이 한쪽 OS 에만 참이었다.** "복사가 전부 끝났다고 확인되기 전에는 삭제 단계가 **스스로 거부합니다**" 라고 아무 한정 없이 적고, 바로 아래 표는 **맥·윈도우 두 행을 함께** 들었다. 그러나 '스스로 거부'하는 집행은 **맥 ⑤-B 에만** 있다 — 윈도우 5·6·7번은 사람이 속성 창의 크기·파일 수를 눈으로 대조하는 절차뿐이고, 탐색기 삭제를 막는 기계 장치가 **없다** | 문서 레인 — 단언을 사실로 좁혔다. 표에 **"복사가 덜 됐을 때 삭제를 막는 것은"** 칸을 만들어 맥=**명령 자신** / 윈도우=**여러분**(⚠ 장치 없음)으로 갈랐고, 윈도우 2단계 머리에도 같은 단서를 붙였다. **새 기계 장치는 만들지 않았다** |
| 5 | MAJOR | 이 문서 §4.15 끝(11차 판 :1062) | **설계정본이 존재하지 않는 시험을 보증으로 들었다.** "이 자리는 스캐너가 아니라 **위 변이시험 6종**이 지킨다"고 **현재형으로** 단언했는데, D1~D9 는 1회성 수동 실행이고 레포에 스크립트·테스트가 **0건**이다(수도 6이 아니라 9다). 판정자 우회 시험 M-F: ⑤-A 블록을 통째로 삭제하고 ⑤-B 를 옛 형태로 되돌려도 **아무것도 빨개지지 않는다** | 문서 레인 — 사실로 좁혔다(§4.15 끝). "1회성 수동 시험으로 확인했고 **레포에 상주하는 자동 검사는 없다** → **회귀가 조용히 들어올 수 있다**". 자동 검사는 **만들지 않았다**(하드 스톱) — 미결 **U10** 으로 올린다 |
| 6 | MAJOR | `ui/src/clipath.test.ts` 등급색 핀 주석 | **핀 주석이 계열을 본다고 하는데 함수 본문만 본다.** 판정자가 9형태로 시험해 **5건 뚫었다**(전부 `tsc` 0 error 인 출하 가능 코드). 원인은 `pinFinalClassName` 이 `mainFnBody(fn)` = **그 함수 본문만** 읽는 것 | UI 레인 — **핀을 넓히지 않는다**(하드 스톱). 대신 **주석의 단언을 사실로 좁힌다** — 이 핀이 실제로 보는 것은 '이 함수 본문 안의 className 쓰기'뿐이고, 아래 다섯 형태는 **보지 못한다**는 것을 구체 형태와 함께 적는다. 같은 기준으로 이 diff 의 **모든 핀 주석**을 훑어 과대선언을 좁힌다 |

★**문서 레인이 코드를 열어 대조한 결과**(2026-08-25 22:0x · 미커밋 작업트리 · `ui/**` 는
**읽기만** 했다): `backupNoticeLine` 의 `!autoRestored` 판정이 **자리 판정보다 앞**에 오고,
그 갈래는 어떤 자리 상태에서도 `sudo mv` 를 만들지 않는다. `linkState="ours"` 에서는 최신 사본이든
옛 사본이든 이동 명령이 없다 = 위 계약표가 **다시 참**이 됐다. `backupDropLine` 도 `autoRestored`
를 함께 읽어, 사본이 여럿일 때는 **'마지막 사본'이라 부르지 않고 `sudo rm` 도 내지 않는다.**
`cd ui && bun test src/clipath.test.ts` → **0 fail**(문서 거울쌍 배터리 포함 · 문서 레인의 이번
문구 수정 뒤 측정). 통과 수는 **20분 사이에 271 → 276 으로 움직였다** — UI 레인이 같은 트리에서
BLOCK-1 의 '네 갈래 × 사본 1개/2개' 시험을 더하는 중이기 때문이다. ★그래서 이 값과 이 대조는
**freeze 전 작업트리**의 값이다. freeze 커밋에서 다시 유도해 커밋 메시지에 적어라(§9.2 끝
주석과 같은 규칙 — **세지 말고 유도한다**).

#### ★12차가 좁힌 약속 — 문서 레인 몫 4자리 (위치 · 옛 문장 · 새 문장 · 왜 옛 것이 거짓이었나)

| 위치 | 옛 문장(요지) | 새 문장(요지) | 옛 것이 거짓이던 이유 |
|---|---|---|---|
| `docs/INSTALL.md` 📌 계약표 | 자리 상태 **세 갈래**만으로 "명령을 낸다/안 낸다"가 결정된다 | 세 갈래에 **네 번째 단서**가 붙는다 — 옮기는 명령은 한 자리에 대해 **최신 사본 하나**에만 붙고, 옛 사본에는 어느 갈래에서도 붙지 않는다 | 판정 축이 **둘**(자리 상태 × 사본이 최신인가)인데 표가 **하나**만 적었다. 그래서 `ours`+옛 사본 조합에서 표와 코드가 정반대를 말했다 |
| `docs/INSTALL.md` 이력 괄호(:481) | "지금은 앱이 그 자리가 차 있다고 아는 경우에는 옮기는 명령을 아예 내지 않는다" | 같은 문장 + **12차에 붙은 단서**(옛 사본에는 어느 갈래에서도 붙지 않는다 · 그전에는 같은 목적지 명령이 나란히 나올 수 있었다) | 문장 자체는 참이지만, 그 옆에서 **같은 목적지로 가는 두 줄**이 나오는 경로를 덮지 못했다 |
| `USER-MANUAL.md`:95 | 자리를 링크가 차지하면 명령을 내지 않는다 · 확정 못 하면 `ls -l` 로 확인하라 | 같은 두 문장 + **"옮기는 명령이 붙는 것은 한 자리에 대해 최신 사본 하나뿐"** · 옛 사본에는 화면 문자열 그대로 "자동으로 되돌아오지 않습니다"라는 **사실만** 말한다 | 위와 같은 결함의 거울쌍(같은 사실을 적는 두 문서 중 한쪽만 고치면 다른 쪽이 드리프트한다 — 9차 MINOR-5) |
| `docs/GUIDE-clean-reset-KR.md`:40 | "복사가 전부 끝났다고 확인되기 전에는 삭제 단계가 **스스로 거부합니다**"(한정 없음 · 표는 맥·윈도우 두 행) | 맥 = **명령 자신이 거부**한다 / 윈도우 = **여러분이 눈으로 대조**한다(⚠ 대조 전 삭제를 막는 장치가 없다) | '스스로 거부'하는 집행이 **맥 ⑤-B 에만** 있었다. 윈도우 사용자는 있지도 않은 안전장치를 믿고 대조를 건너뛸 수 있었다 |
| 이 문서 §4.15 끝 | "이 자리는 스캐너가 아니라 **위 변이시험 6종**이 지킨다"(현재형) | "D1~D9 **아홉 종**은 1회성 수동 시험이고, 레포에 **상주하는 자동 검사는 없다** → 회귀가 조용히 들어올 수 있다" | 두 군데가 사실이 아니었다 — ①수를 세지 않고 옮겨 적었다(6→9) ②**1회성 확인을 상주 보증으로 격상**했다. '선언과 집행의 분리'가 **보증문 쪽에서** 나온 형태다 |

#### ★등급색 핀이 실제로 보는 것과 못 보는 것 (판정자 9형태 시험 · 5건 관통)

핀은 `pinFinalClassName(fn)` 이고, 보는 것은 **`mainFnBody(fn)` = 그 함수 본문 문자열 하나**다.
본문 안의 `className` 쓰기(프로퍼티·계산된 프로퍼티·`=`/`+=`/`||=`/`&&=`/`??=`)와 `CLASS_BYPASS`
여섯 계열(`classList`·`setAttribute("class")`·`…NS`·`Object.assign`·`outerHTML`·`replaceWith`)을
정규식으로 찾는다. **그 본문 밖으로는 한 글자도 나가지 않는다.** 그래서 아래 다섯은 전부
`tsc` 0 error 로 출하 가능하면서 핀을 통과한다:

| 형태 | 왜 통과하나 |
|---|---|
| **새 헬퍼 경유** `applyToastSkin(el)` | 등급색 쓰기가 **다른 함수 본문**으로 옮겨간다. 핀은 그 함수를 열지 않는다 |
| ★**실재 헬퍼 경유** `addToastCloseButton(el)` 첫 줄 | 위와 같은데 **새 함수도 새 파일도 필요 없다** — 이미 있는 함수 한 줄이면 된다. 리뷰에서 "새 함수가 생겼다"는 신호조차 나오지 않는다 |
| **인라인 스타일** `el.style.borderColor = …` | 등급을 보이는 장치는 테두리색인데, 그 색을 `className` 을 **거치지 않고** 직접 준다. 핀의 바늘 두 벌(className 쓰기 · CLASS_BYPASS) 어느 쪽에도 걸리지 않는다 |
| **`Reflect.set(el, "className", …)`** | 프로퍼티 이름이 **문자열 인자**라 소스에 `el.className =` 이라는 형태가 나타나지 않는다 |
| **본문 절단** `if (0) {\n}\n` | `mainFnBody` 는 중괄호 균형으로 본문을 자른다. 균형을 일찍 맞추는 조각을 넣으면 **핀이 읽는 '본문'이 진짜 본문보다 짧아진다** — 그 뒤에 무엇을 쓰든 보이지 않는다 |

★**출처를 밝힌다**: 다섯 형태의 실측은 **판정자**의 것이고, 문서 레인은 그 변이를 직접 돌리지
않았다. 문서 레인이 확인한 것은 소스 쪽 사실 하나다 — `pinFinalClassName(fn)` 은 `mainFnBody(fn)`
이 돌려준 **문자열 하나**에만 정규식을 건다(`ui/src/clipath.test.ts` `pinFinalClassName`·
`classNameAssignments`·`CLASS_BYPASS`). 다섯 형태가 통과하는 이유는 전부 그 하나로 설명된다:
셋(새 헬퍼·실재 헬퍼·`Reflect.set`)은 그 문자열 **밖**이거나 그 문자열 안에서 **다른 모양**이고,
하나(인라인 스타일)는 두 바늘 어느 쪽도 겨냥하지 않는 속성이며, 하나(본문 절단)는 **문자열 자체를
짧게 만든다**.

★**이 표는 결함 보고가 아니라 핀의 사양이다.** 12차는 하드 스톱에 따라 **핀을 넓히지 않았다**.
넓히면 이 표는 다시 낡고, 낡은 사양이 다시 과대선언이 된다 — 열두 라운드가 반복해서 만난 형태다.
대신 **핀 주석이 이 사실을 그대로 말하게** 했다. 검사기가 못 보는 자리를 **아는 것**이, 못 보는
것을 **모르는 것**보다 낫다(그리고 '못 본다'고 적힌 자리는 리뷰가 본다).

#### ★12차 최종 상태 — 무엇이 닫혔고, 무엇이 열린 채 남는가

**닫힌 것**

1. **다중 백업본 손실 경로**(BLOCK-1·MAJOR-2) — 같은 목적지로 가는 `mv` 두 줄과 '마지막 사본'
   이중 호명이 사라진다. 이것이 이 라운드가 닫은 **유일한 실제 손실 경로**다.
2. **문서 계약표 ↔ 코드의 정반대 진술**(MAJOR-3) — 판정 축이 둘이라는 사실이 표에 들어갔다.
3. **한쪽 OS 에만 참인 단언**(MAJOR-4) — 맥의 기계 거부와 윈도우의 사람 대조가 갈렸다.
4. **존재하지 않는 시험을 보증으로 든 문장**(MAJOR-5) — 1회성 수동 시험이라고 적혔고, 그것이
   무엇을 뜻하는지(회귀가 조용히 들어온다)까지 적혔다.
5. **핀 주석의 과대선언**(MAJOR-6) — 핀이 무엇을 보고 무엇을 못 보는지가 구체 형태와 함께 적혔다.

**열린 채 남는 것** (전부 §9.3 미결 표에 있다 — 여기 다시 적는 것은 마지막 라운드이기 때문이다)

- **U10 문서 절차(⑤-A/⑤-B)를 지키는 자동 검사가 없다.** 스캐너의 사각 ①(홈 상대경로)·③(변수
  경로)에 동시에 들어, **지금 형태로도 옛 형태로도 빨개지지 않는다.** 회귀는 사람이 읽어야만
  잡힌다. 고치려면 D1~D9 를 스크립트로 상주화해야 한다 — **12차는 하드 스톱이라 하지 않았다.**
- **U11 등급색 핀 우회 5형태**(위 표) — 핀은 함수 본문만 본다. 넓히지 않았다.
- **U3 Windows 실빌드 · U4 실기 macOS 승격 왕복** — 열두 라운드 내내 한 번도 하지 않았다.
  `/usr/local/bin` 무접촉 경계 때문이며, 검증은 전부 임시 디렉터리 사본이었다. **이 기능이 실제
  기계에서 도는 것을 본 사람은 아직 없다.**
- **U1 CI 배선 · U8 릴리스 스텝 이름** — `.github/**` 무접촉 경계라 이 레인이 할 수 없다.
- **U9 부록 B.9 줄번호** — UI 레인이 같은 라운드에 같은 파일을 고쳤으므로 freeze 후 재유도해야 한다.

**다음 사람이 반드시 알아야 할 것**

1. **이 레인의 문서는 "코드와 일치"가 아니라 "지시대로 하면 문제가 해결된다"가 합격선이다**(규약
   ⑫). 그래서 문서를 고칠 때는 **화면 문자열과 Rust 실동작을 먼저 열어 보고** 고쳐야 한다.
   12차의 MAJOR-3 은 그 순서를 지키지 않아 문서와 코드가 **정반대**를 말한 자리였다.
2. **`docs/INSTALL.md` 와 `USER-MANUAL.md` 는 거울쌍이다** — `ui/src/clipath.test.ts` 의
   `mirrorBattery` 가 두 문서에 **같은 배터리**를 건다. 한쪽만 고치면 다음 라운드에 다른 쪽이
   드리프트한다(9차 MINOR-5 가 그 실사고다). 낱말 단위 대조라 산문은 자유롭게 고쳐도 되지만
   **`sudo mv -n`·`아예 내지 않`·`ls -l <원래 경로>`·상시 고지 제목 네 개**는 양쪽에 남아야 한다.
3. **문서 코드블록은 실행하지 마라**(규약 ⑰). 9차가 그것으로 살아 있는 데몬을 죽였다(26분 정지).
   확인이 필요하면 `HOME` 을 임시 디렉터리로 바꾼 사본에서만 한다.
4. **핀이 초록이라는 사실은 '결함이 없다'가 아니라 '핀이 보는 자리에 결함이 없다'는 뜻이다.**
   위 두 표(핀 우회 5형태 · 스캐너 사각 ①③)가 이 레인이 아는 **못 보는 자리의 목록**이다.
   목록에 없는 자리는 아직 **세어 보지 않은** 자리다.
5. **라운드를 더 돌린다면 첫 일은 U10·U11 이 아니라 U3·U4 다** — 열두 라운드가 만진 것은 전부
   문자열과 스크립트였고, 실기에서 왕복한 적이 없다. 정적으로 닫을 수 있는 것은 이제 거의 남지
   않았다.

#### ★12차가 남긴 실무 규칙 (다음 사람에게)

1. **인자를 새로 받은 함수는 그 인자를 읽는 갈래를 세라.** BLOCK-1 은 `autoRestored` 를 받아 놓고
   **네 갈래 중 하나**에서만 읽은 것이다. 11차의 새 테스트가 `ours` 상태만 검사해 나머지 셋을
   비워 둔 것이 **그 결함의 직접 원인**이다 — 새 인자가 생기면 테스트는 **갈래 × 인자값 조합**을
   덮어야 한다.
2. **"지킨다"는 현재형은 상주 장치가 있을 때만 쓴다.** 1회성 확인은 "그때 한 번 확인했다"로 적어야
   한다(MAJOR-5). 이 구분을 흐리면 다음 사람이 있지도 않은 그물을 믿고 뛰어내린다.
3. **한 문장이 두 플랫폼을 함께 덮으면, 두 플랫폼에서 각각 참인지 따로 세라**(MAJOR-4). 표에 두
   행이 있다는 것은 **두 번 검증해야 한다**는 뜻이다.
4. **마지막 라운드에서는 장치를 늘리지 말고 약속을 좁혀라.** 새 장치는 새 약속을 만들고, 새 약속은
   다음 라운드의 결함이 된다. 남은 개선은 코드가 아니라 **미결 표**에 적는 것이 옳다.

## 5. ★설계 규약 17조

> **재발 방지 조문**이다. 이 조문들은 이 작업에만 적용되는 것이 아니라, 앞으로 이 레포에서
> 설계서를 쓰는 모든 라운드의 최소 요건이다. 각 조문 뒤의 괄호는 그 조문을 낳은 **실제 사고**다.
>
> - **①~⑬**: 1~4라운드 성찰(설계 감사 · 적대적 감사 · 아키텍트 감사 3라운드)이 도출.
> - **⑭~⑯**: 5~8라운드가 새로 낳았다(§4.8~§4.11). 같은 지위이며 번호를 이어 붙인다.
> - **⑰**: 9라운드 실사고(검증 하네스가 살아 있는 데몬을 죽였다 — §4.12·§4.14 ④)가 낳았다.

### ① '무손상 존치'는 감사 면제가 아니다

복원·재활성화 대상이 **호출하는 기존 코드**도 같은 반증 렌즈를 통과한다. "이번에 손대지 않았다"는
사실은 그 코드가 옳다는 증거가 아니다. 호출부가 사라져 있던 동안 그 코드는 **한 번도 실행되지
않았고**, 따라서 한 번도 검증되지 않았다.

> (2026-08-20 제거는 호출부만 지웠고 `install_cli_to_path` 는 존치됐다. 1차 반증의 BLOCK 2건 중
> 1건이 그 존치 코드 안에 있었다.)

### ② 쓰기·삭제·덮어쓰기 경로는 모두 같은 파괴 가드 표를 채운다

경로마다 다음 네 칸을 **빠짐없이** 채운다:

| 판정 규칙 | 아닐 때 처리 | 통보 문구 | 되돌리는 명령 |
|---|---|---|---|

한 칸이라도 비면 그 경로는 설계가 끝나지 않은 것이다. 특히 **'삭제'만 파괴로 보지 않는다** —
`ln -sf` 는 덮어쓰기이고 덮어쓰기는 삭제와 같은 파괴다.

> (해제 경로는 "비가역 파괴"라며 남의 파일을 지켰는데, 같은 커밋의 설치 경로는 root 권한
> `ln -sf` 로 같은 파일을 말없이 파괴했다. 가드가 정반대였다.)

### ③ 병렬 작업하는 모든 IPC 경계는 필드 단위 스키마로 못박는다

**이름 3개와 enum 1개는 계약이 아니라 추측 허가다.** 경계를 사이에 두고 두 워커가 병렬로 일한다면,
설계서는 그 경계의 **모든 필드**에 대해 이름·타입·null 허용·enum 전체 값을 적어야 한다.
적지 않은 것은 각자 다르게 상상한다.

> (Rust `skipped: Vec<String>` ↔ TS `{path,reason}[]`. 이 드리프트의 책임은 워커가 아니라
> **설계서를 못박지 않은 master** 다.)

### ④ 설계서는 HOW 가 아니라 관측 가능한 수용 조건을 적는다

HOW 를 적을 때는 **그것을 무효화하는 조건을 함께 적는다.** "5초 타임아웃을 건다"는 HOW 이고,
"어떤 자식 프로세스 구조에서도 5초+ε 안에 반환한다"가 수용 조건이다. HOW 만 적으면 구현자는
HOW 를 만족시키고 수용 조건을 놓친다.

> (D6 은 "spawn + try_wait 폴링 + kill" 이라는 HOW 를 적었다. 구현은 그대로 했고, 손자
> 프로세스가 파이프를 물고 있으면 드레인 스레드가 무기한 join 한다는 무효화 조건은 아무도 적지
> 않았다. 실측 12.02초.)

### ⑤ '확인한다'는 요구는 계측기를 함께 적을 때만 유효하다

계측기 = **셸 · 명령 · 기한 · 실패 시 값** 넷. 하나라도 비면 "확인한다"는 요구는 실행 불가능한
장식이다.

> ("`which -a cys` 로 검증한다"만 적혀 있었다. 어떤 셸인지 적지 않아 `bash -lc` 로 구현됐고
> (macOS 기본은 zsh), 종료 상태를 어떻게 쓸지 적지 않아 폐기됐고(tcsh rc=1 이 '정상 실행'으로
> 둔갑), 그래서 UI 가 틀린 안내를 했다.)

### ⑥ 순차로 여러 대상에 작용하는 명령은 '앞은 성공·뒤는 실패' 상태를 명시한다

부분 성공은 **가장 흔한 실패 모드**이며 설계서가 침묵하면 구현은 첫 오류에서 `return` 하는
자연스러운 형태를 택한다. 그 순간 앞에서 일어난 되돌릴 수 없는 일이 **보고되지 않고 사라진다.**
반환값과 통보 문구를 명시한다.

> (cys 는 백업+링크까지 끝났고 cysd 의 `mv` 가 거부돼 전체 rc=1 이 된 경우, 사용자는
> `"심볼릭 생성 실패: mv: …"` 만 봤다. 남의 실체 바이너리가 추측 불가능한 이름으로 옮겨졌는데
> 그 사실이 어디에도 남지 않았다.)

### ⑦ 비가역·준비가역 행위는 되돌리는 경로를 같은 라운드에 함께 설계한다

"설치"를 설계하면서 "해제"를 다음으로 미루면, 그 사이 기간 동안 사용자는 되돌릴 수 없는 상태에
갇힌다. **백업했다면 복원 경로도 같은 라운드에 설계한다** — 백업만 하고 복원이 없으면 "잃는 것이
없다"는 정당화가 성립하지 않는다.

> (설치만 있고 해제가 없어 앱을 지워도 root 심링크가 남았다 = D4 의 동기. 그리고 2라운드가
> 백업을 도입했는데 4라운드 I3 에서야 "백업본이 앱 안에서 다시는 보이지 않는다"가 드러났다.)

### ⑧ 라운드 시작 전 '어겨서는 안 되는 문장' 목록을 선언·동결한다

**게이트 초록은 착수 조건이지 합격 기준이 아니다.** 합격선은 라운드 시작 전에 선언하고 도중에
고치지 않는다. 초록인데 21건이 나온 경험이 이 조문의 근거다.

> (1차: `cargo 54/54` · `bun 458/458` · `tsc 0` · 번들 성공 → 그 상태에서 BLOCK 2건.)

### ⑨ 신규 테스트는 수리 전 코드에서 반드시 실패해야 한다

red→green 을 확인하지 않은 테스트는 **무엇을 지키는지 모르는 테스트**다. 그리고 **픽스처는
상대 구현의 실물에서 유도한다** — 자기 쪽 타입을 픽스처로 쓰면 테스트가 드리프트를 봉인한다.

> (MAJOR-5 의 단위테스트가 초록이었던 이유는 픽스처가 Rust 실물이 아니라 잘못된 TS 모양을
> 먹였기 때문이다. MAJOR-3 의 테스트가 초록이었던 이유는 `sh -c "sleep 30"` 이 kill 이 먹는
> 유일한 형태였기 때문이다.)

### ⑩ 경고를 거부로 올리는 변경은 새로 거부되는 집합을 전수 열거한다

"경고 → 거부" 는 사용자에게 보이는 계약 변경이다. 새로 막히는 입력 집합을 전부 적고, 그중
**정당한 사용자가 있는지** 확인한다. 열거하지 않으면 정당한 사용자를 새로 막는다.

> (D5 가 NonStandard 를 거부로 올렸다. → MINOR-6 에서 `/tmp/Applications` 가 여전히 통과함이
> 드러났고, 그 수리(엄격 판정)는 MINOR-N8 에서 macOS 펌링크 별칭
> `/System/Volumes/Data/Applications` 를 **새로 거부**해 정당한 사용자를 막았다.)

### ⑪ UI 가 분기하는 모든 판정은 기계 필드로 받는다

**산문은 사람용이며 계약이 아니다.** 문구는 다듬기·번역으로 언제든 바뀌고, 바뀌는 순간 조용히
오분기한다. 게다가 같은 배열에 다른 목적의 문장이 합류하면 판정 대상 문자열 자체가 오염된다.
'정규식 재도입 차단' 테스트를 붙여 못박는다.

> (Rust 는 "경고문 첫 구절을 안정 판별자로 고정한다"고 선언했고 TS 는 문장 속 어절을 봤다 —
> 양쪽이 서로 다른 것을 계약이라 불렀다. 3라운드에서 `unverified_reason` 필드로 교체.
> 4라운드 C3 에서 해제 경로에도 같은 수리를 적용한다.)

### ⑫ 문서의 합격선은 '지시대로 하면 문제가 실제로 해결된다'이다

코드와 일치하는 것만으로는 부족하다. 문서가 시키는 절차를 실제로 밟았을 때 사용자의 문제가
해결되어야 한다. **밟아 보지 않은 안내는 안내가 아니다.**

> (`docs/INSTALL.md:167` 이 `~/.zshrc` 를 고치라고 했다. 그런데 `zsh -lc` 는 `.zshrc` 를 읽지
> 않는다 — 안내대로 해도 경고가 사라지지 않는다.)

### ⑬ 수리는 지점이 아니라 계열에 적용한다

수리할 때마다 이 4쌍을 묻는다: **설치 ↔ 해제 · cys ↔ cysd · 실체 파일 ↔ 심볼릭 ·
시작 표식 ↔ 끝 표식.** 한 쪽만 고친 수리는 미완성이다. 거울쌍에 **공유 추상**이 없으면
매 수리가 한쪽 거울에만 발린다 — 공유 순수 함수를 만들어 양쪽이 같은 판정을 쓰게 한다.

> (성찰 2R·3R 이 독립적으로 같은 결론에 도달했다: 30건을 수리했지만 전부 지목된 지점에만
> 적용됐다. 2라운드가 고친 BLOCK 과 정확히 같은 병이 심볼릭 축에서 그대로 살아 있었다 — C1.)

---

> **아래 세 조문은 5~8라운드가 새로 낳은 것이다.** 위 13조와 같은 지위이며, 번호를 이어 붙인다.

### ⑭ 판정과 집행이 갈라지는 지점을 목록으로 만들어 전수 점검한다

순수 함수로 정확히 판정해도, **실제로 파일을 옮기고 지우는 쪽이 다른 규칙을 쓰면 판정은
장식이다.** 이 작업에는 판정↔집행 경계가 셋 있다 — ①Rust 순수 판정 ↔ ②승격 셸 스크립트
↔ ③문서가 사용자에게 시키는 명령. 한 축을 고칠 때마다 **나머지 둘에 같은 규칙이 서 있는지**
확인한다.

> (5차 MAJOR-6: Rust 는 경로를 정규화하는데 셸은 안 했다. 6차 MAJOR-C: Rust 는 `Path::exists()`
> — 심볼릭을 따라간다 — 로 물었는데 셸은 `[ -e ] || [ -L ]` 로 물었다. **한 라운드 건너 같은 병이
> 다른 축에서 재발했다** = 목록이 없었다는 증거다. 8차 BLOCK-1: 코드는 백업하는데 문서는 파괴한다
> = 세 번째 축이 통째로 빠져 있었다. **10차 MINOR-7**: 백업 이름 충돌을 문서(`INSTALL.md` §B)는
> `exit 1` 로 막는데 승격 셸은 `mv` 직행이었다 — 이번엔 **문서가 코드보다 안전한** 방향의 비대칭
> 이었고, 방향이 뒤집혀도 같은 병이다.)
>
> ★그리고 이 경계는 **셋이 아니라 넷**이다. 9차 실사고가 네 번째를 드러냈다 —
> **④검증 도구가 실제로 집행하는 것**. 거부 목록(판정)은 있었고 `refuse()`(집행)에 연결되지
> 않았다(§4.12 · 규약 ⑰).

### ⑮ 경고를 **줄이는** 변경, 막힘을 **푸는** 변경도 새로 열리는 집합을 전수 열거한다

규약 ⑩은 "경고 → 거부"만 다뤘다. 반대 방향도 같은 크기의 계약 변경이다:

- **경고를 줄이는 변경**은 "무엇이 새로 **조용해지는가**"를 열거해야 한다.
- **막힘을 푸는 변경**(동기 → 비동기, 락 제거, 캐시 도입)은 "무엇이 새로 **동시에 일어날 수
  있는가**"를 열거해야 한다. 막힘은 그 자체로 직렬화 장치였고, 그것을 없애면 그 장치가 대신
  하던 일을 누군가 다시 해야 한다.

> (7차 MAJOR-1: 6차가 과경고를 줄이면서 **진짜 남의 파일 경고까지** 조용해졌다. 7차 MAJOR-3:
> `async fn` 전환이 "메인 스레드가 멎는" 결함을 닫으면서 **중복 진입의 문을 열었고**, 주석은
> 오히려 "새 동시성도 열리지 않는다"고 단정했다 — 8차가 그것을 반증했다.)

### ⑯ 계열은 코드 밖까지 간다 — 사용자에게 도달하는 모든 경로를 같은 기준으로 본다

규약 ⑬의 4쌍(설치↔해제 · cys↔cysd · 실체파일↔심볼릭 · 시작표식↔끝표식)은 전부 **코드 안의
거울쌍**이다. 그러나 결함이 사용자에게 닿는 경로는 코드만이 아니다:

```
①GUI 버튼   ②앱이 화면에 띄우는 명령 문자열   ③문서가 복사해 실행하라고 주는 명령
④문서가 서술하는 동작 설명   ⑤에러 메시지가 제안하는 복구 절차
```

**한 결함을 고쳤으면 다섯 경로 전부에서 같은 결함을 찾는다.** 그리고 문서 축에는 **문서를 읽는
자동 테스트**를 건다 — 사람의 눈으로 대조하는 방식은 이 리포에서 이미 여러 번 실패했다.

> (7차 MAJOR-2 는 UI 가 만드는 명령 문자열 5종을 전수 점검했는데 **`ui/src` 에서 멈췄다** — 같은
> 파괴적 명령이 `docs/INSTALL.md:275`·`USER-MANUAL.md:98` 에 그대로 남아 8차 BLOCK-1 이 됐다.
> 7차 MINOR-5 는 `docs/INSTALL.md` 를 읽는 doc-sync 테스트를 만들었는데 **`USER-MANUAL.md` 를
> 읽는 테스트는 만들지 않았고**, 정확히 그 파일에 불일치가 남아 8차 MAJOR 가 됐다.)

### ⑰ 검증 도구 자신에게도 같은 규약을 적용한다 — 그리고 문서 절차는 **실행하지 않고** 검증한다

이 리포에서 세 번 반복된 병은 하나다: **선언된 가드가 집행에 연결되지 않는다.** 세 번째는 제품도
테스트도 아닌 **검증 도구 자신**에게서 났다(9차 실사고 · §4.12).

- **거부 목록을 가진 도구는 그 목록이 실제로 거부를 일으키는지 먼저 시험한다.** 목록만 있고
  시험이 없으면 그것은 가드가 아니라 주석이다(규약 ⑤의 도구판).
- **문서의 코드블록은 실행해서 검증하지 않는다.** 배포 문서에는 사용자의 실제 시스템을 바꾸는
  명령(데몬 종료·앱 삭제·홈 디렉터리 삭제)이 들어 있고, 검증 환경과 실계는 같은 기계일 수 있다.
  대신 이미 있는 두 방법을 쓴다 — ①**정적 스캐너**(9차 `scanDocCommands`: 셸 코드블록의 명령
  형태만 본다) ②**임시 디렉터리 사본을 대상으로 한 코드 테스트**(`install_script_*` 계열은
  `/usr/local/bin` 을 한 번도 만지지 않는다).
- **관측은 개입이 아니다.** 상태를 읽는 명령(`ls`·`git status`·`grep`)과 상태를 바꾸는 명령
  (`launchctl`·`pkill`·`rm`·데몬 기동)은 검증 절차 안에서 **다른 등급**으로 다룬다.

> (9차: 하네스가 `launchctl bootout`·`pkill -x cysd` 를 실계에서 실행해 데몬을 죽였다 — 26분 정지.
> 거부 패턴은 정의되어 있었고 `refuse()` 에 연결되지 않았다. 10차는 문서 코드블록을 한 줄도
> 실행하지 않고 같은 문서 4건을 수리했다.)

---

## 6. 미결 · 오너 결정 대기 항목

### 6.1 4차 안에서 판정하기로 했던 것 — **전부 판정됐다**

| 항목 | 내용 | 결과 |
|---|---|---|
| I3-③ 원본 복원 | 해제 시 '원본 복원'(우리 이름 규칙에 정확히 일치하는 백업만 `mv`) — 구현이 복잡하면 미수리로 보고해도 되는 선택지였다 | ✅ **채택·구현됨**. `build_uninstall_script`(현 `main.rs:2470`)에 root `mv` 되돌리기 경로 추가 + 새 IPC 필드 `restored`. 가드: 자리가 비었을 때(`[ ! -e ] && [ ! -L ]`)만 옮기고, `is_our_backup_name` 에 정확히 맞는 것만 후보. ★8차 판정: **오너 원 요구 3건이 요구하지 않은 새 특권 쓰기 경로**라는 사실은 기록한다(4.7) |
| I6 검수 실험 | 아무 필드에 `#[serde(rename=…)]` 를 붙였을 때 TS 게이트가 빨개지는가 | ✅ **실험 수행·빨개짐 확인**. 단 **덤프 쓰기가 자기 단언보다 먼저 와야** 한다는 함정을 실측으로 발견해 코드 주석에 남겼다(§3.4). ★그러나 이 게이트는 **어떤 CI 레인에서도 두 반쪽이 함께 돌지 않는다** → §7 |
| `adv1`~`adv9` 전부 초록 | 4차 인수 기준 | ✅ **9/9 초록.** 이름이 '결함 재현'에서 '결함 부재 회귀핀'으로 바뀌었다(4.7 표) |
| C3 판별자 형태 | `skipped_benign: bool` 로 갈지 더 나은 형태로 갈지 | ✅ **둘 다.** 등급용 `skipped_benign: bool` + 줄별 분류용 `skipped_reasons: string[]`(enum 3값). TS 는 산문을 파싱하지 않는다(§3.2) |

### 6.2 오너 결정이 필요한 것

| 항목 | 쟁점 | 현재 상태 |
|---|---|---|
| **Windows PATH 등록** | D2 는 Windows 자동 PATH 편집을 **구현하지 않기로** 했다(`setx` 1024자 잘림 · `REG_EXPAND_SZ→REG_SZ` 변환으로 `%USERPROFILE%` 류 파손). Windows 사용자는 터미널에서 `cys` 를 쓰려면 수동으로 PATH 를 넣어야 한다 | **미구현 유지 + 문서 안내만.** 오너 절대지침(윈도우 신중) 준수. 다시 열려면 오너 결정 필요 |
| **MINOR-12(b) Windows MSI PATH 문장 충돌** | `docs/INSTALL.md` ↔ `USER-MANUAL.md` 가 정면 충돌했다 | ✅ **실측으로 판정·정정 완료**(2·3차). 사실은 "설치기(setup.exe)는 PATH 를 건드리지 않는다 — `installMode: currentUser` · `%LOCALAPPDATA%\cys`". 폐기된 구 MSI 는 PATH 를 등록했으나 더 이상 배포되지 않는다는 단서까지 양쪽 문서에 명시(`docs/INSTALL.md:509·518·521` · `USER-MANUAL.md:99`) |
| **릴리스 번호** | v0.14.25 는 타 레인이 선점했고 **이미 태그됨** | 이 레인은 **v0.14.26 예약**. 버전 SOT(`Cargo.toml`·`src-tauri/Cargo.toml`·`src-tauri/tauri.conf.json` 등 6곳)는 이 라운드들에서 **한 번도 건드리지 않았다** — 범프는 master 가 릴리스 시점에 한다 |
| **CI 배선(§7)** | 이 diff 의 Rust 테스트 103종이 태그 레인 밖이다. 닫으려면 `.github/workflows/release.yml` 수정이 필요한데 **이 레인은 `.github/**` 무접촉**이 절대 경계다 | ★**오너·master 결정 대기.** §7 에 추가할 스텝의 **정확한 형태**를 적어 두었다 — 그대로 붙이면 된다 |
| **라운드 종료 판정** | 헌장상 종료 = 미달 항목 0 또는 10라운드. **11차는 그 상한을 넘었다** — 10차 종결 시점에 남은 미달 항목이 0 이 아니었고(11차가 BLOCK 1 · MAJOR 2 를 새로 찾았다), 그중 하나는 **사용자 데이터 영구 소실**(§4.15 #1)이라 상한을 이유로 덮을 수 없었다 | ★**오너 결정 대기.** 11차로 그 셋은 닫혔다. 다음 중 하나를 오너가 골라야 한다 — ⓐ 여기서 **서면 종료 보고**로 닫는다 ⓑ 상한을 명시적으로 늘려 한 라운드 더 돈다. 어느 쪽이든 **상한 초과 사실을 기록에 남긴 채로** 진행한다(§6.2 의 이 줄이 4차부터 열려 있고 아직 닫히지 않았다). 8차 판정 12건 중 미수리로 남겼던 MINOR 1건(4.11 #3 = §9.3 U2)은 **10차에 수리됐다**(4.13 #7) |

### 6.3 이 레인이 건드리지 않는 것 (경계 · 절대)

- **수정 절대 금지**: `src/bin/**` · `src/lib.rs` · `src/pack.rs` · `cysjavis-pack/**` ·
  `.github/**` · `scripts/**` · `ui/e2e/**`
- **`/usr/local/bin` 절대 무접촉** — 실행 중인 데몬이 그 심볼릭에 의존한다. 스크립트 실행
  검증은 반드시 임시 디렉터리 사본에서만.
- **버전 SOT 무접촉** — 릴리스 범프는 master 가 한다.
- `<OTHER-LANE-WORKTREE>` 는 타 세션 작업 중 — **읽지도 않는다.**
- `git commit`/`checkout`/`reset`/`stash` 금지(커밋은 master). 데몬·서버 기동 금지.
  `cargo` 병렬 실행 금지(타 세션과 CPU 공유).
- `ui/src/style.css` 는 4라운드에 한해 **`#cc-header` 규칙에만** 수정 허용(실제 변경 +7줄).
- ★**문서에 실제 홈 경로 금지** — `<WORKTREE>`·`<HOME>`·`<SESSION-DIR>` 플레이스홀더를 쓴다.
  이유는 §8.

### 6.4 이 레인 밖으로 이관된 발견

부수 발견: cys 노드 부트 사슬에서 **fresh 프로필 로그인 선택지 → 디렉티브 주입 Return 이
'No, exit' 에 꽂혀 agent 를 죽이는 경로**를 실측했다. 타 세션(부트 결정론 캠페인) 소관이라
증거만 이관했다 — `_round/handoffs/boot-chain-evidence-for-determinism-campaign.md`,
레인 커밋 `3080315` 에 함께 담겼다.

---

## 7. ★CI 레인 지도와 공백 (8라운드 MAJOR-4(c) 신설)

> **왜 이 절이 필요한가 (일반 독자용 풀이)**
> 테스트는 "돌아야" 보증이 된다. 사람이 자기 컴퓨터에서 돌리는 것과, GitHub 이 자동으로 돌리는
> 것은 전혀 다른 보증이다 — 사람은 잊어버리고, 자동은 잊지 않는다. 이 절이 밝히는 사실은
> 이것이다: **이 열 라운드가 만든 Rust 테스트 103개는, 실제 사용자에게 앱이 전달되는 경로
> (= 태그를 붙여 릴리스를 만드는 경로)에서 단 한 번도 실행되지 않는다.**

### 7.1 이 리포의 워크플로 6종과 기동 조건 (실측 `0b6cb24`)

| 워크플로 | 기동 조건 | 이 레인(`fix/shell-cli-install-restore`)에서 도는가 |
|---|---|---|
| `release.yml` | `push: tags: ['v*']` + `workflow_dispatch` | ❌ 태그 push 로만 자동 기동(수동 실행은 가능) |
| `ci-branch.yml` | `push: branches: ['feat/**']` + `workflow_dispatch` | ❌ **`fix/**` 는 해당 없음** |
| `windows-health.yml` | `push: branches: ['feat/**']` + `workflow_dispatch` | ❌ 같은 이유 |
| `windows-build.yml` | `push: branches: ['feat/windows-x64-dist']` + `workflow_dispatch` | ❌ |
| `pack-release.yml` | `push: tags: ['pack-v*']` | ❌ (팩 전용 레인) |
| `release-publish.yml` | `workflow_dispatch` 전용 | ❌ (이미 만들어진 draft 를 공개 발행하는 레인) |

★즉 **이 브랜치에 코드를 push 해도 자동으로 도는 CI 는 0개다.** 이 레인의 검증은 전부 로컬에서
사람이 돌린 것이다(각 커밋 메시지의 `검증:` 줄이 그 기록이다).

### 7.2 어느 스텝이 어느 크레이트를 도는가 (실측)

이 리포에는 Rust 크레이트가 둘이다 — **루트 크레이트**(`cys`·`cysd` 바이너리 + `lib`)와
**`cys-app` 크레이트**(`src-tauri/` · GUI 본체 · **이 작업의 코드가 전부 여기 있다**).

| 워크플로 : 잡 : 스텝 | 명령 | 어느 크레이트 | 어느 레그 |
|---|---|---|---|
| `release.yml` : `build` : `UI 회귀 테스트 (bun test)` (`:145-147`) | `cd ui && bun test` | (TypeScript) | 매트릭스 **3레그 전부** |
| `release.yml` : `build` : `phoenix host-level 로직 게이트` (`:167-189`) | `cargo test --bin cysd -- --skip hwmon::` / `cargo test --lib` / `cargo test --bin cys` | **루트 크레이트만** | `aarch64-apple-darwin` 1레그 |
| `release.yml` : `build` : `cargo test --lib factory_reset + D5` (`:488-494`) | `cargo test --lib factory_reset::` / `cargo test --lib claude_alt_screen` | **루트 크레이트만** | Windows 레그 |
| `ci-branch.yml` : `macos-rust-pack` : `cargo test --bin cys 리허설` (`:556-560`) | `cargo test --bin cys` | **루트 크레이트만** | macOS |
| `ci-branch.yml` : `macos-rust-pack` : `cargo test -p cys-app --bins` (`:580-591`) | `cargo test -p cys-app --bins` | ★**`cys-app` — 유일한 레인** | macOS |
| `windows-health.yml` | `cargo test --lib factory_reset::` / `cargo test --bin cys d5_env_injection` | **루트 크레이트만** | Windows 실기 |
| `pack-release.yml` | (`cargo test` 자체가 없다) | — | — |

**결론 두 줄:**

1. `cys-app` 크레이트 테스트를 돌리는 CI 스텝은 **`ci-branch.yml:580` 단 하나**이고, 그
   워크플로는 `feat/**` push 전용이다 → **태그 경로(사용자에게 도달하는 유일한 경로)에는 보증이 0.**
2. `bun test` 를 돌리는 CI 스텝은 **`release.yml:147` 단 하나**이고, 그 잡에는
   `cargo test -p cys-app` 이 없다 → **I6 계약 게이트의 두 반쪽이 같은 잡에 함께 있는 레인이 없다.**

### 7.3 이 공백이 실제로 무엇을 못 잡는가

이 레인이 `src-tauri/src/main.rs` 에 추가한 테스트(정적 계수, `#[test]` 속성 기준):

| 항목 | 값 | 세는 방법 |
|---|---|---|
| base `c54c3b2` 의 `#[test]` | **43** | `git show c54c3b2:src-tauri/src/main.rs \| grep -c '#\[test\]'` |
| HEAD `e7063d3` 의 `#[test]` | **102** | `git show HEAD:src-tauri/src/main.rs \| grep -c '#\[test\]'` |
| **10차 작업트리**의 `#[test]` | **103** | `grep -c '#\[test\]' src-tauri/src/main.rs` |
| 이 레인의 순증 | **+60** (43 → 103) | 위 두 값의 차. ★`git diff \| grep -c '^+…#[test]'` 로 세면 **헌크 경계에 따라 값이 흔들린다**(같은 트리에서 `+61/−2` 와 `+61/−1` 이 둘 다 나온다) — 양끝 계수를 쓴다 |
| 그중 플랫폼 한정 | `#[cfg(target_os = "macos")]` **7** · `#[cfg(unix)]` **22** | `grep -B1 '#\[test\]' src-tauri/src/main.rs \| grep -c 'cfg(unix)'` |

**이 103개 안에 있는 것**: `adv1`~`adv9`(적대적 재현 회귀핀 9종, `main.rs:8235`~`8531`) ·
계약 덤프(`dump_report_contract_for_the_ui_gate`, `main.rs:8850`) · 소스 텍스트 불변식 핀(`blockb_no_new_file_level_cfg_gated_items`,
`main.rs:9405` — 7차 MINOR-4 가 `fn` 전용에서 11종 아이템으로 확장하고 **읽지 못하면 offender 로
올리도록** 고친 것) · `major1_premise_…`(7차 반증을 못박은 4단 증명, `main.rs:7790`) · Windows E0425 재발 방지 핀 · ★10차 신설 `install_script_aborts_when_backup_name_collides`(백업 이름 충돌 = 중단).
**전부 태그 레인 밖이다.**

**I6 게이트가 실제로 어떻게 뚫리는가** — 네 경우로 갈린다:

| 경로 | Rust 덤프(`__contract__.json` 다시 쓰기) | TS 판독(`bun test`) | 필드 이름을 바꾸면? |
|---|---|---|---|
| 태그 레인(`release.yml`) | ❌ 안 돈다 | ✅ 돈다 (커밋된 파일을 읽는다) | **초록** — 커밋된 스냅샷이 낡은 채로 TS 와 맞으므로 통과 |
| `feat/**` 레인(`ci-branch.yml`) | ✅ 돈다 (파일을 덮어쓴다) | ❌ 안 돈다 | **초록** — 파일을 조용히 고쳐 놓고 아무도 안 읽는다. `git diff --exit-code` 도 없다 |
| 이 레인(`fix/**`) | ❌ | ❌ | 둘 다 안 돈다 |
| 사람이 로컬에서 `cargo` → `bun` 순서로 | ✅ | ✅ | **빨개진다** ← 지금 이 게이트가 작동하는 **유일한** 조건 |

★그리고 이 사실이 **`clipath.ts:18` 의 "판정의 근거는 그 생성 파일이다"** 와 **이 설계문서 §4.5
I6 의 "게이트가 빨개진다"** 라는 서술을 부분적으로 거짓으로 만든다. 커밋 `7f8b505` 의
`Not-tested:` 트레일러도 이 공백을 적지 않았다(적힌 것은 'Windows 실빌드'·'실기 macOS 승격
왕복' 둘뿐). ★8차 freeze 커밋 `0b6cb24` 의 `Not-tested:` 에는 이 문장이 들어갔다 —
`★이 diff 의 Rust 테스트는 어떤 CI 레인에서도 실행되지 않음`. **그 고지가 이 절의 씨앗이다.**

### 7.4 닫는 방법 — `release.yml` 에 추가할 정확한 스텝

> ⚠ **이 레인은 `.github/**` 무접촉이 절대 경계다.** 그래서 **문서로 남기는 것이 이번 라운드의
> 산출물**이다. 아래는 그대로 붙여 넣을 수 있는 형태이며, 붙이는 주체는 master(또는 인수자)다.

**놓을 자리**: `release.yml` 의 `build` 잡, `Rust cache`(`:154`) 스텝 **뒤**,
`phoenix host-level 로직 게이트`(`:167`) 앞 또는 뒤. 기존 관례대로 macOS 네이티브 레그 1회만 돈다.

```yaml
      # ★cys-app(src-tauri) 크레이트 테스트 — 태그 레인 편입.
      #   **왜 여기 있어야 하는가**: 이 크레이트 테스트를 돌리는 레인이 ci-branch.yml
      #   (push: branches ['feat/**'] 전용) 하나뿐이라, 결함이 사용자에게 도달하는 유일한
      #   경로(태그)에는 보증이 0이었다. 같은 형태의 사고가 이 리포에 이미 세 번 있었다
      #   (C-1 / R4 / S26 — ci-branch.yml 머리 주석 참조): 언제나 '레인 하나를 빼먹음'이다.
      #   ⚠스텁 스테이징 필수: 이 시점에는 bundle-prep 이 아직 돌지 않아 ui/dist·
      #   src-tauri/binaries·resources 가 없고, tauri-build 2.6.2 는 그 자원의 **실존만**
      #   검사하므로 없으면 "resource path … doesn't exist" 로 즉사한다(ci-branch.yml 실측 주석).
      #   touch 는 기존 파일을 절단하지 않아 실물 스테이징이 있는 환경과도 공존한다.
      - name: cargo test -p cys-app --bins (GUI·CLI PATH 계약 핀 · macOS 네이티브)
        if: matrix.target == 'aarch64-apple-darwin'
        shell: bash
        run: |
          set -euo pipefail
          # 핀 실존 단언(이 리포 관례) — cargo test 필터는 0매치도 초록이라 존재를 먼저 못박는다.
          grep -q "fn dump_report_contract_for_the_ui_gate" src-tauri/src/main.rs \
            || { echo "계약 덤프 테스트 부재: src-tauri/src/main.rs (git add 누락 의심)" >&2; exit 1; }
          grep -q "fn adv9_cysd_shadowing_is_measured_and_reported" src-tauri/src/main.rs \
            || { echo "적대적 회귀핀 부재: adv9 (git add 누락 의심)" >&2; exit 1; }
          mkdir -p ui/dist src-tauri/binaries src-tauri/resources src-tauri/runtime
          triple="$(rustc -vV | sed -n 's/^host: //p')"
          touch "src-tauri/binaries/cys-$triple" "src-tauri/binaries/cysd-$triple" \
                src-tauri/resources/pack.tar.gz src-tauri/resources/pack-manifest.json
          CYS_PACK_DIR="$(mktemp -d)" cargo test -p cys-app --bins -- --test-threads=1

      # ★계약 스냅샷 드리프트 hard-gate.
      #   위 스텝의 dump_report_contract_for_the_ui_gate 가 ui/src/__contract__.json 을 **다시 쓴다**.
      #   커밋본과 한 글자라도 다르면 "Rust 실물이 바뀌었는데 계약 스냅샷을 갱신하지 않았다"는 뜻이다.
      #   ★이 스텝이 없으면 위 cargo test 는 파일을 조용히 고쳐 놓고 초록으로 지나간다
      #     (= ci-branch.yml 레인이 지금 그 상태다).
      - name: 계약 스냅샷 드리프트 게이트 (ui/src/__contract__.json)
        if: matrix.target == 'aarch64-apple-darwin'
        shell: bash
        run: |
          set -euo pipefail
          git diff --exit-code -- ui/src/__contract__.json \
            || { echo "ui/src/__contract__.json 이 Rust 실물과 어긋난다 — 로컬에서 'cargo test -p cys-app --bins' 로 재생성해 커밋하라" >&2; exit 1; }
```

**이 두 스텝이 함께 있어야 하는 이유**: 첫 번째만 넣으면 파일을 덮어쓰기만 하고 아무도 그 결과를
읽지 않는다(현재 `ci-branch.yml` 이 정확히 그 상태다). 두 번째만 넣으면 비교할 새 파일이 생기지
않아 항상 초록이다. **둘이 한 쌍이다.**

**순서에 대한 주의**: `bun test`(`:147`)는 `Setup Rust`(`:149`)보다 **앞**에 있으므로, 위 두
스텝은 `bun test` 뒤에 온다. 그래도 보증은 성립한다 — 드리프트 게이트가 실패하면 잡 전체가
실패하기 때문이다. 순서를 더 곧게 하고 싶다면 `bun test` 스텝을 이 두 스텝 **뒤로** 옮기면
된다(그러면 TS 게이트가 방금 재생성된 파일을 읽는다).

**같은 공백을 `ci-branch.yml` 쪽에서 닫으려면**: 그 잡에 `bun test` 가 0건이므로
`- run: cd ui && bun test` 를 추가하거나, 위 드리프트 게이트를 그 잡에도 넣는다.
그리고 **이 레인의 브랜치 접두(`fix/**`)가 어느 워크플로 트리거에도 없다**는 사실 자체가 별개
문제다 — `ci-branch.yml` 의 `branches:` 목록에 `'fix/**'` 를 더할지는 오너 결정 사항이다
(러너 비용이 늘어난다).

### 7.5 예산과 위험

- 비용: 콜드 시 tauri 의존 그래프 추가 빌드 수 분(같은 잡의 `Rust cache` 가 흡수) · 웜 실측
  1초 미만(ci-branch.yml 주석의 실측값). 매트릭스 1레그에서만 돌므로 3배가 되지 않는다.
- 위험: 러너 환경 의존으로 실패하는 테스트가 나오면, 이 리포의 선례 패턴
  (`--skip hwmon::` = 하드웨어 의존 모듈 명시 스킵)을 그대로 적용한다. **테스트를 지우지 마라.**
- `-p cys-app` 는 `ubuntu` 레인(`pack-release.yml`)에 넣을 수 없다 — tauri 스택이 시스템
  webkit 계 의존을 요구하고 그 레인은 manifest emit 용 `cys` 만 빌드한다(ci-branch.yml 실측 주석).

### 7.6 ★새로 생긴 결합 — `bun test` 가 **배포 문서 본문**을 읽는다 (8라운드 MINOR-6)

> **한 줄 요약**: 이제 `docs/INSTALL.md`·`USER-MANUAL.md` 의 **본문 문장 하나를 고치면**
> `bun test` 가 빨개지고, 그 `bun test` 는 **태그 릴리스의 하드 게이트**다. 즉 **오탈자 교정 한
> 줄이 릴리스를 멈출 수 있다.** 그리고 실패 로그는 원인을 "문서 문구 변경"이라고 말해 주지 않는다.

이 결합은 7라운드까지 없었다. 8라운드에서 "문서와 화면이 같은 말을 하는지 자동으로 지킨다"는
보증(설계 규약 ⑫)을 **실제 테스트로** 구현하면서 새로 생겼다. 이득이 분명한 결합이지만,
**이득만 적고 대가를 적지 않으면 그것은 설계 문서가 아니다.**

#### 7.6.1 무엇이 무엇을 읽는가 (실측 · 파일:줄)

| 읽는 쪽(테스트) | 읽히는 쪽(배포 문서) | 무엇을 대조하는가 |
|---|---|---|
| `ui/src/clipath.test.ts` — `describe("★MINOR-5 …")` | `docs/INSTALL.md` (`new URL("../../docs/INSTALL.md", import.meta.url)`) | ①상시 고지 제목 4종(`NOTICE_TITLE_FOREIGN`·`_PARTIAL`·`_BACKUP`·`_INFO`) 원문 ②설치 동의 문구(`FOREIGN_BACKUP_NOTICE`)의 핵심 낱말 7개 ③`MV_EMPTY_CAVEAT` 원문 ④문서 **코드블록**에 `-n` 없는 `sudo mv` 가 없을 것 ⑤"언제 명령을 내고 언제 내지 않는가" 3갈래 |
| 〃 (8라운드 MINOR-5 로 추가되는 몫) | `USER-MANUAL.md` | 위 ①②의 같은 대조 — 8라운드 이전에는 `USER-MANUAL.md` 를 읽는 테스트가 리포 전체에 **0건**이어서, 손으로 고친 "세 갈래 vs 네 갈래" 드리프트가 다시 벌어질 수 있었다 |
| 〃 (문서 코드블록 회귀핀) | `docs/` 전체 + 리포 루트의 `*.md` | 맨 `ln -sf`/`ln -sfn`(`unguardedLn`) · 가드 없는 `rm <절대경로>`(`unguardedRm`) 가 코드블록에 있으면 실패. 예외는 **명시 allowlist**(사유 주석 필수) |

- 대조는 **낱말 단위**다(문장 전체 일치가 아니다). 줄바꿈·들여쓰기·`> ` 인용 표식은
  `flat()`(`^\s*>\s?` 제거 + 공백 1칸 정규화)으로 걷어 낸 뒤 비교한다. 따라서 **문단을 다시
  접거나 강조 표기를 바꾸는 편집은 안전**하다. 깨지는 것은 **낱말·원문 문자열을 바꾸는 편집**이다.

#### 7.6.2 그래서 무엇이 깨지는가 — 실패가 도달하는 경로

```
docs/INSTALL.md 한 줄 수정
  └─ ui/src/clipath.test.ts (★MINOR-5) 실패
       └─ `cd ui && bun test` 실패
            └─ .github/workflows/release.yml:145-147 "UI 회귀 테스트 (bun test)" 스텝 실패
                 └─ 그 스텝은 Rust 빌드보다 **앞**에 있다(:149 Setup Rust) → 릴리스 잡 전체 중단
                      └─ 태그 v* 빌드 산출물 없음
```

★**함정 — 실패 로그가 원인을 가리키지 않는다.** `release.yml:143-144` 의 스텝 주석과 이름은
그 게이트를 **"한글 IME 리듀서(ui/src/ime.ts) 프로필 A/B/C/Windows 시퀀스 테스트"** 라고만
설명한다. 문서 대조가 거기 얹혔다는 사실은 어디에도 적혀 있지 않다. 그래서 릴리스 담당이 보는
것은 "IME 테스트가 왜 갑자기 깨졌지?" 이고, 실제 원인은 **누군가 `USER-MANUAL.md` 의 오탈자를
고친 것**이다. 이 절이 그 간극을 메우는 유일한 기록이다.

- `.github/**` 는 이 레인의 **무접촉 경계**라(§6.3) 스텝 이름·주석을 여기서 고치지 않는다.
  → **master 인수인계 항목**: rebase 후 `release.yml:143-146` 의 스텝 이름·주석에
  "UI 단위 + **배포 문서 대조**" 를 명시할 것(§9.3 U8).

#### 7.6.3 문서를 고칠 때 지켜야 할 것 (실무 규칙)

1. **문서만 고치고 끝내지 마라.** `docs/INSTALL.md`·`USER-MANUAL.md` 의 다음 것들은 화면 문자열의
   거울이다 — 고치려면 `ui/src/clipath.ts` 의 같은 상수를 **같은 커밋에서** 고쳐야 한다:
   상시 고지 제목 4종 · `FOREIGN_BACKUP_NOTICE` 의 핵심 낱말 · `MV_EMPTY_CAVEAT` 전문.
2. **반대 방향도 같다.** 화면 문구를 고치면 두 문서를 같은 커밋에서 고쳐야 한다.
   (8라운드 MINOR-4 가 정확히 이 위반이었다 — 문서에는 `mv -n` 의 dangling 예외를 명문화하고
   화면 상수 `MV_EMPTY_CAVEAT` 만 옛 단언 그대로 두었다.)
3. **문서 코드블록에 맨 파괴 명령을 넣지 마라.** 맨 `ln -sf`/`ln -sfn`, 가드 없는
   `rm <절대경로>` 는 회귀핀이 막는다. 필요하면 `docs/INSTALL.md` §B 의 **가드 블록으로 유도**하라
   — 복제하지 마라(정본 이원화가 이 라운드들의 반복 결함이었다).
4. **로컬 확인은 한 줄이다.** `cd ui && bun test` — 문서만 고친 PR 이라도 돌려라.
5. **어디를 봐야 하는가.** `bun test` 가 `★MINOR-5` 절에서 깨졌다면 코드가 아니라 **문서**를 봐라.
   실패 메시지는 `{ 주장, 낱말, 화면: true, 문서: false }` 형태로 **어느 낱말이 문서에서 사라졌는지**
   그대로 찍는다.

#### 7.6.4 이 결합을 그래도 유지하는 이유

대가(문서 편집이 릴리스를 멈출 수 있다)를 알고도 유지한다. 근거는 이 레인의 실측이다 —
8라운드 동안 **화면과 문서가 갈라진 결함이 4건**(MAJOR-2·MINOR-N13·MINOR-4·"세 갈래 vs 네 갈래")
나왔고, 그중 둘은 문서가 **존재하지 않는 보증**을 내세우거나 **사실이 아닌 것**을 단언한 경우였다.
사람의 성실성에 맡기는 방식은 이미 4번 실패했다. 반면 이 결합이 만드는 최악은 **릴리스 지연**이고,
그것은 `bun test` 한 줄로 사전에 발견된다. 비대칭이 명백하다.

---

## 8. ★레인 상태 파일이 레포 밖에 있는 이유 (8라운드 MAJOR-4(d) 신설)

### 8.1 무슨 일이 있었나

레인 개설 커밋 `3080315` 은 레포 루트에 `MASTER_TODO.md`(33줄)와 `SESSION_STATE.md`(39줄)를
만들었다. 후자는 스스로를 **"단일 복원 진실"** 이라고 선언했다. 그런데 4~6차 커밋 `7f8b505` 가
**그 둘을 삭제했다**(`git show --stat 7f8b505` 실측: `MASTER_TODO.md | 33 -` ·
`SESSION_STATE.md | 39 -`). 그 커밋 메시지는 트레일러 6종을 갖춘 상세본인데 **두 삭제를 한 글자도
언급하지 않았고**, 대체물도 남기지 않았다. `_round/handoffs/` 에는 이 레인 파일이 0건이다
(있는 것은 타 레인 이관용 `boot-chain-evidence-…` 와 `RELEASE_LANES.md` 둘뿐).

8차 판정자가 이것을 인수 경로 단절로 지적했다(4.11 #12). **지적이 옳다 — 삭제 자체보다 침묵이
결함이다.** 이 절이 그 침묵을 메운다.

### 8.2 왜 레포 밖으로 옮겨야 했나 — 시크릿 스캔 hard gate

`scripts/secret-scan.sh` 는 릴리스의 **차단 게이트**다. `release.yml:124` 의
`Secret/PII scan (pre-build hard-gate)` 스텝이 빌드 **전에** 돌고, 걸리면 릴리스가 멈춘다.

그 스캐너의 첫 번째 규칙(`scripts/secret-scan.sh:48-49`):

```
# 1) 개인 절대경로 (/Users/<실명>) — 더미 제외
grep -nE '/Users/[A-Za-z0-9._-]+' "$f" | grep -vE '/Users/(user|x|youruser|USERNAME|runner|home)(/|"|$)'
```

즉 **`/Users/<실제 사용자 이름>` 이 들어간 추적 파일이 하나라도 있으면 릴리스가 차단된다.**
허용되는 것은 제네릭 더미 이름 6종(`user`·`x`·`youruser`·`USERNAME`·`runner`·`home`)뿐이다.

그런데 레인 상태 파일은 **본질적으로 실제 경로를 담는다** — "어느 워크트리에서 작업 중인가",
"어느 워크트리를 건드리면 안 되는가"가 그 파일의 존재 이유이기 때문이다. 실측: 두 파일 각각
**2줄**이 이 패턴에 걸린다(`grep -cE '/Users/[A-Za-z0-9._-]+'` = 2, 2).

플레이스홀더로 바꾸면 스캔은 통과하지만 **파일이 자기 역할을 못 한다**(복원 시 어느 경로인지
읽을 수 없다). 그래서 **레포 밖**으로 옮겼다.

### 8.3 현재 위치와 규칙

```
<HOME>/.cys/lanes/shell-cli/MASTER_TODO.md      ← 이 레인의 master TODO 정본
<HOME>/.cys/lanes/shell-cli/SESSION_STATE.md    ← 이 레인의 단일 복원 진실
```

- **규칙**: 레인의 상태·복원·TODO 파일은 **레포 밖 `<HOME>/.cys/lanes/<레인 이름>/`** 에 둔다.
  레포에는 **포인터 한 줄**만 남긴다(지금 이 절이 그 포인터다).
- 이 규칙은 마스터 규약(`상태·복원·todo 파일은 자기 레인의 팩 아래에 둔다`)과 같은 방향이다 —
  거기에 "**레포는 공개 배송물이므로 개인 경로가 들어가서는 안 된다**"는 근거가 하나 더 붙는다.
- 이 설계문서를 포함해 **레포 안 문서에는 실제 홈 경로를 쓰지 않는다.** 대신
  `<WORKTREE>`·`<HOME>`·`<SESSION-DIR>`·`<OTHER-LANE-WORKTREE>` 플레이스홀더를 쓴다.
  이 문서는 그 규칙을 지키며, `bash scripts/secret-scan.sh --all` 로 확인했다.

### 8.4 `SESSION_STATE.md` 가 지금 들고 있는 미완 항목 (레포 밖 파일의 요약)

인수자가 그 파일을 열지 않고도 무엇이 남았는지 알 수 있도록 여기에 옮겨 적는다:

1. 워크플로우 산출 수령 → BLOCK/MAJOR 반증 수리 → 커밋 *(1~10라운드로 진행됨 · 9·10라운드는 미커밋)*
2. 성찰 3라운드(R1 설계 이해 / R2 적대적·최고 품질 / R3 30년차 아키텍트 의존성·파급)
3. 리뷰어 2종(codex · gemini) 의무 리뷰 트리거 ①③④⑤
4. **상대 레인 태그 대기 → rebase → 릴리스(D2~D4)** ← v0.14.25 는 이미 태그됨. 이 레인은 v0.14.26

---

## 9. ★미결 · 인수인계 (8라운드 MAJOR-4(e) 신설)

> 다음 사람이 이 레인을 이어받는 데 필요한 것 전부. 이 절만 읽고도 착수할 수 있어야 한다.

### 9.1 좌표

| 항목 | 값 |
|---|---|
| 워크트리 | `<WORKTREE>` |
| 브랜치 | `fix/shell-cli-install-restore` |
| base | `c54c3b2` = v0.14.24 (2026-08-22 릴리스) |
| HEAD | `c6f3669` (9·10차 수리 = **11차 freeze**). 9차 판정 대상이던 `e7063d3`, 8차 판정 대상이던 `0b6cb24` 는 그 앞이다 |
| 커밋 **9개** (`git log --oneline c54c3b2..HEAD` 유도) | `3080315`(레인 개설) → `0adc45d`(상태) → `0e60b79`(1차) → `6159778`(2차) → `9f47148`(3차) → `7f8b505`(4~6차) → `0b6cb24`(7차) → `e7063d3`(8차) → `c6f3669`(9·10차) |
| 규모 — **커밋된 것**(`git diff c54c3b2 HEAD --stat \| tail -1`) | **15파일 +11,959 / −353** |
| 규모 — **작업트리 전체**(위 + 미커밋 **11·12라운드** · `git diff c54c3b2 --stat \| tail -1` · ★문서 레인이 잰 시점 · UI 레인이 같은 트리에서 작업 중이라 **이 값은 계속 움직인다** — freeze 시 재유도하라) | **15파일 +12,661 / −363** (11라운드 측정값) |
| 상태 파일 | `<HOME>/.cys/lanes/shell-cli/` (§8) |
| 예약 버전 | v0.14.26 (SOT 무접촉 — 범프는 master) |

### 9.2 로컬 검증 4종 (CI 가 대신해 주지 않는다 — §7)

`<WORKTREE>` 에서, **순서대로**:

```
cargo test -p cys-app --bins -- --test-threads=1     # Rust. __contract__.json 을 다시 쓴다
git diff --exit-code -- ui/src/__contract__.json      # 계약 드리프트 — 변경이 나오면 커밋해야 한다
cd ui && bun test                                     # TS 전체(22파일)
cd ui && bun run typecheck                            # = bunx tsc -p tsconfig.check.json
bash scripts/secret-scan.sh --all                     # 릴리스 hard-gate 와 같은 스캐너
```

★**`cargo` 를 두 개 이상 동시에 돌리지 마라** — 타 세션과 CPU·`target/`(14G)을 공유한다.

**테스트 수는 세지 말고 유도한다**(이 표의 값은 전부 아래 명령의 출력에서 나왔다):

| 값 | 유도한 명령 | 7차 기록값(`0b6cb24` 커밋 메시지) | **10차 실측**(문서 레인 측정 시점) |
|---|---|---|---|
| Rust 테스트 | `cargo test --manifest-path src-tauri/Cargo.toml` 의 `test result:` 줄 | 102 passed / 0 failed | **103 passed / 0 failed** (10차 MINOR-7 회귀핀 1건 증가) |
| TS 테스트 전체 | `cd ui && bun test` 의 마지막 줄 | 631 pass / 0 fail | **690 pass / 0 fail** (22파일 · 2,390 expect) |
| `clipath.test.ts` 단독 | `cd ui && bun test src/clipath.test.ts` | 201종 | **260종** |
| 타입체크 | `cd ui && bun run typecheck` | 0 | **0** |
| 시크릿 스캔 | `bash scripts/secret-scan.sh --all` | clean(844파일) | **`✓ secret-scan: clean (mode=--all, 844 파일)` · exit 0** |

★**TS 쪽 세 값은 UI 레인이 10차 작업을 마치기 전에 잰 값이다**(문서 레인과 UI 레인이 같은 라운드를
동시에 돌았다 — §9.5 의 파일 분리 규칙은 지켰지만 카운트는 공유한다). freeze 커밋 시점에 다시
유도해 커밋 메시지에 적어라 — **그 값이 정본이다.**

### 9.3 미결 항목

| # | 항목 | 상태·사유 |
|---|---|---|
| U1 | **CI 배선**(§7.4의 두 스텝을 `release.yml` 에 추가) | ★미결. `.github/**` 무접촉 경계 때문에 이 레인이 할 수 없다. **문서로만 남겼다** — 붙이는 주체는 master/인수자 |
| ~~U2~~ | ~~설치 스크립트의 `/bin/mv {d} {b}` 백업 목적지 무가드~~ | ✅ **10차 수리 완료**(`main.rs` `build_install_script`). 문서 정본 `docs/INSTALL.md` §B 와 **같은 형태** — `if [ -e {b} ] || [ -L {b} ]; then echo <사유> >&2; exit 1; fi` 를 `mv` 앞에 둔다. ★예고된 **`mv -n` 오답은 채택하지 않았다**(거부해도 exit 0 → `&&` 체인이 이어져 백업 없이 링크). 회귀핀 `install_script_aborts_when_backup_name_collides` 가 ①중단 여부 ②먼저 있던 백업본 보존 ③원본이 링크로 갈아 끼워지지 않음 ④stderr 사유 ⑤거짓 자기보고 0건 ⑥체인 단절(cysd 링크 미생성) 여섯을 못박는다. 남은 사각은 §4.13 끝 주석 |
| U3 | **Windows 실빌드 미검증** | 6차가 E0425 컴파일 즉사를 최소 재현으로 잡고 고쳤지만, **실제 Windows 러너에서 빌드한 적은 없다.** 첫 실측점은 태그 레인의 Windows 레그다 |
| U4 | **실기 macOS 승격 왕복 미검증** | 관리자 비밀번호를 받아 `/usr/local/bin` 에 실제로 설치·해제하는 왕복은 한 번도 하지 않았다(`/usr/local/bin` 무접촉 경계). 스크립트 검증은 전부 임시 디렉터리 사본 |
| U5 | `resource_gate_check` 동기 + 무기한 블로킹 | 7차 MAJOR-3 과 **같은 병**이지만 티켓 범위 밖이라 손대지 않았다(`0b6cb24` 의 `Rejected:` 에 기록). 별도 티켓 필요 |
| U6 | 라운드 종료 서면 보고 | §6.2 마지막 줄. 4차부터 열려 있고 아직 닫히지 않았다 |
| U8 | **`release.yml:143-146` 스텝 이름·주석이 거짓 안내** | ★미결. 그 스텝은 이제 `ui/src/ime.ts` 뿐 아니라 `docs/INSTALL.md`·`USER-MANUAL.md` **본문**도 대조하는데(§7.6), 이름·주석은 여전히 "한글 IME 리듀서" 라고만 말한다. 문서 오탈자 교정 한 줄로 릴리스가 멈췄을 때 릴리스 담당이 원인을 찾지 못한다. `.github/**` 무접촉 경계라 이 레인이 고칠 수 없다 — rebase 후 master 처리 |
| U9 | **부록 B.9(`ui/src/clipath.ts` 줄번호)가 다시 밀린다** | ★미결. 10차에 UI 레인이 같은 파일을 고치고 있어, 문서 레인이 잰 값은 그 레인이 끝나면 어긋난다. freeze 후 `grep -n` 으로 재유도하라 — 부록 B 머리말의 규칙(실물이 바뀌면 같은 커밋에서 갱신)이 이 표에도 적용된다 |
| U10 | **문서 절차(⑤-A/⑤-B)를 지키는 자동 검사가 레포에 없다** | ★미결(**12차 신설**). D1~D9 아홉 종은 **1회성 수동 변이시험**이고 스크립트·테스트로 남기지 않았다. 그 블록은 스캐너의 사각 ①(홈 상대경로)·③(변수 경로)에 동시에 들어 **지금 형태로도 옛 형태로도 빨개지지 않는다** — 판정자 우회 시험 M-F 로 실측(⑤-A 를 통째로 지우고 ⑤-B 를 옛 형태로 되돌려도 `bun test` 초록). 12차는 하드 스톱(새 기계 장치 금지)이라 만들지 않았다. 닫으려면 **D1~D9 를 스크립트로 상주화**해야 한다(임시 `HOME` 사본에서 도는 형태로 · 규약 ⑰) |
| U11 | **등급색 핀 우회 5형태** | ★미결(**12차 신설**). `pinFinalClassName` 은 `mainFnBody(fn)` = **그 함수 본문만** 읽는다. 판정자가 9형태로 시험해 **5건 관통**(전부 `tsc` 0 error): 새 헬퍼 경유 · **실재 헬퍼 경유**(`addToastCloseButton` 첫 줄) · 인라인 스타일(`el.style.borderColor`) · `Reflect.set` · 본문 절단(`if (0) {}`). 12차는 **핀을 넓히지 않고 주석의 단언을 사실로 좁혔다**(§4.16). 넓히려면 '본문 문자열 정규식'이 아니라 **호출 그래프**를 봐야 하고, 그것은 이 핀의 설계를 바꾸는 일이다 |
| U7 | 릴리스(D2~D4) | 버전 범프 → CI → DMG 2종 공증 → Windows setup/zip → 홈페이지 downloads 업로드 + SHA256SUMS 갱신. **v0.14.25 는 타 레인이 이미 태그**했으므로 이 레인은 v0.14.26 |

### 9.4 이 레인을 이해하려면 반드시 읽어야 할 것

1. **이 문서 §3**(IPC 계약) — 무엇을 주고받는지. 실물이 이기지만, 어긋나면 그 자체가 결함이다.
2. **이 문서 §5**(설계 규약 17조) — 열 라운드가 산 값이다. 새 라운드는 여기서 시작한다.
3. **이 문서 §4.14**(master 판단 오류 4건) — 같은 함정을 다시 밟지 않기 위해.
4. `git log --format='%n=== %h %s%n%b' c54c3b2..HEAD` — 커밋 메시지가 **경량 ADR** 이다.
   `Constraint:`/`Rejected:`/`Directive:`/`Confidence:`/`Scope-risk:`/`Not-tested:` 트레일러에
   "왜 그렇게 했는가"와 "무엇을 기각했는가"가 들어 있다.
5. `src-tauri/src/main.rs` 의 `mod tests`(`:5826` 이하) — 특히 `adv1`~`adv9`(`:8235`~`:8531`).
   **결함을 재현하던 테스트가 결함 부재를 지키는 핀으로 바뀐 자리**다.
6. `<HOME>/.cys/lanes/shell-cli/SESSION_STATE.md` — 레인 복원 진실(§8).

### 9.5 라운드를 하나 더 돌린다면

라운드 규약(이 프로젝트의 것)을 그대로 따른다:

```
수리 → freeze(로컬 커밋으로 리뷰 대상 리비전 확정 · verdict 도착 전 수정 금지)
     → 이종 판정자 2인이 그 정지한 리비전을 본다
     → master 반박 ↔ 판정자 재반박(왕복 2회)
     → 합당한 것만 수용(근거 없는 기각 금지)
     → 합격 기준 미달 0 또는 10라운드에서 종료
```

★그리고 **4.14 ②의 교훈을 지켜라**: 쓰는 레인과 읽는 레인을 **동시에** 돌리지 말고, 동시에
돌려야 한다면 **서로 다른 파일**로 갈라라. 8라운드는 그렇게 했다 — 그때 문서 레인이 만진 파일은
`docs/plans/2026-08-25-shell-cli-restore-design.md` **하나뿐**이었다. 10라운드도 같은 규칙으로
둘로 갈랐다: **문서/Rust 레인** = `docs/GUIDE-clean-reset-KR.md`·`docs/INSTALL.md`·`USER-MANUAL.md`·
이 문서·`src-tauri/src/main.rs` / **UI 레인** = `ui/**`. 교집합 0.
11·12라운드도 같은 규칙이었다 — 12라운드 문서 레인이 만진 파일은 `docs/INSTALL.md`·
`USER-MANUAL.md`·`docs/GUIDE-clean-reset-KR.md`·이 문서 **넷뿐**이고 `ui/**`·`src-tauri/**` 는
**읽기만** 했다(§4.16 의 UI 레인 행은 문서 레인이 그 파일을 **읽어** 대조한 결과까지 적었다 —
쓰지는 않았다. freeze 리비전에서 같은 대조를 다시 하라).

★**규약 ⑰도 지켜라**: 라운드 안에서 문서 절차의 안전성을 확인할 일이 생겨도 **그 코드블록을 실행
하지 마라.** 9라운드가 그것으로 살아 있는 데몬을 죽였다(§4.12). 정적 스캐너와 임시 디렉터리 사본
테스트로 충분하다.

## 부록 A. 관련 파일 지도 (**10라운드 작업트리** · 변경량은 `git diff c54c3b2 --numstat` 유도)

> ★값은 **손으로 세지 않고** `git diff c54c3b2 --numstat` 출력에서 그대로 옮겼다. `ui/**` 행은
> 문서 레인이 잰 시점의 값이라 UI 레인이 10라운드 작업을 마치면 커진다(§9.2 끝 주석 · §9.3 U9).
> `docs/plans/2026-08-25-shell-cli-restore-design.md` 는 base 에 없던 파일이라 **줄 수 = 추가 수**다.

| 파일 | 역할 | 이 레인의 변경 (`+추가/−삭제`) |
|---|---|---|
| `src-tauri/src/main.rs` | 순수 판정 함수 전부 + 커맨드 3종 + `generate_handler!` 등록 + `mod tests`(`:5826` 이하) | **+4,933 / −290** |
| `ui/index.html` | `#cc-header` 안의 `<button id="btn-install-cli" hidden>` (원 위치·원 id) | +1 |
| `ui/src/main.ts` | 버튼 배선(invoke 호출·DOM 갱신). **판정은 하지 않는다** | +264 / −3 |
| `ui/src/clipath.ts` | 순수 판정 — 플랫폼 노출·버튼 라벨·결과 토스트 등급·고지 문구를 **여기서만** 정한다 | **+1,071 (신규)** |
| `ui/src/clipath.test.ts` | 계약-드리프트 가드 + 정규식 재도입 차단 + 문서 대조(doc-sync) + **배포 문서 코드블록 전수 스캐너**(9차) | **+3,234 (신규)** |
| `ui/src/bun-env.d.ts` | `bun test` 가 `readFileSync` 로 배포 문서를 읽기 위한 타입 선언(9차 · §7.6) | +10 (신규) |
| `ui/src/__contract__.json` | Rust 테스트가 덤프하는 계약 스냅샷. TS 게이트의 기준. **손으로 고치지 않는다** | +34 (신규) |
| `ui/src/style.css` | `#cc-header` 오버플로 가드(I7⑤ · 이 규칙에만 손댐) | +7 |
| `docs/INSTALL.md` | §B 설치/해제 안내 · 수동 폴백 · `INST-DENY-02` · 백업본 되돌리기 정본 | +383 / −14 |
| `docs/GUIDE-clean-reset-KR.md` | 완전 초기화 가이드 — 9차 앱 삭제 가드 · **10차 단계 흐름(MAJOR-4)·보존 계약(MINOR-5)** | +137 / −12 |
| `USER-MANUAL.md` | §2.4 사용자 안내(doc-sync 거울쌍 대상) | +22 / −3 |
| `README.md` · `README.en.md` | §B 서술 정합(MINOR-12) | +3 / −1 · +4 / −1 |
| `docs/plans/2026-06-29-cli-path-install.md` | 최초 신설 시점의 구현 계획(가드 5종 원안). 9차에 `doc-command-pin` 면제 마커가 붙었다 | +67 / −29 |
| `docs/plans/2026-08-25-shell-cli-restore-design.md` | **이 문서** — 복원 라운드의 설계 정본 | +1,789 (신규) |
| `<HOME>/.cys/lanes/shell-cli/{MASTER_TODO,SESSION_STATE}.md` | 레인 TODO·복원 진실 — **레포 밖**(§8) | 레포에서 삭제·이전 |

## 부록 B. CLI PATH 계열 함수·타입 색인 (`main.rs` @ **10라운드 작업트리**)

> ★**line 번호는 10라운드 작업트리 기준이다.** 이전 판(4라운드)은 `9f47148` 기준이었고 실물과
> 570~930줄 어긋나 있었고, 8·9라운드 판은 `0b6cb24` 기준이었다. 실물이 바뀌면 이 표를 **같은
> 커밋에서** 갱신한다(머리말의 규칙).
>
> ★10라운드는 이 표를 **손으로 세지 않았다** — 정의 줄 패턴(`^(pub )?(async )?(fn|const|static|
> struct|enum|type) <이름>\b`)으로 `main.rs` 를 훑어 유도했다. `0b6cb24`→`e7063d3` 사이 `main.rs`
> 는 무변경이었고, 10라운드 MINOR-7 이 `build_install_script` 위에 주석 20줄·상수 1개·가드 3줄을
> 넣으면서 `1203` 아래가 전부 밀렸다(그 위의 `sh_squote:764`~`observe_existing_backups:1159` 는
> 그대로다).

### B.1 백업·설치 스크립트

| 함수 / 타입 / 상수 | line | 책임 |
|---|---|---|
| `sh_squote` | 764 | 셸 작은따옴표 인용(설치·해제 공용) |
| `applescript_str` | 771 | AppleScript 큰따옴표 문자열 리터럴(바깥 래핑 — 작은따옴표는 파스 단계 −2741 거부) |
| `classify_bundle_dir` | 776 | 번들 위치 분류(Canonical/Translocated/Backup/NonStandard). **`autoregister_allowed` 도 쓴다 — 산탄총 수술 주의** |
| `backup_stamp` | 1097 | epoch 초 스탬프(Rust 가 생성 — 셸 `date` 금지) |
| `backup_path_for` | 1108 | 대상 경로 → `<경로>.cys-backup-<stamp>` (생성처 단일화) |
| `install_backup_needed` | 1127 | (C1) 설치가 이 경로를 백업해야 하는가 — **해제와 같은 순수 함수**(`decide_cli_uninstall`)로 판정 |
| `plan_install_backups` | 1140 | 관측된 링크 목록 → 백업 계획 `(원본, 백업본)[]` |
| `observe_existing_backups` | 1159 | 실패 반환 **전** 백업 후보 재관측(MAJOR-N1 수리) |
| `build_install_script` | 1223 | 승격 설치 스크립트 문자열 — `[ -e ] \|\| [ -L ]` → 우리 링크면 제외 → ★**백업 목적지 존재 검사(있으면 stderr + `exit 1`)** → `mv` → `echo 마커` → `ln -sfn` |
| `SCRIPT_PATH_PRELUDE` | 1260 | (I4) `export PATH=/usr/bin:/bin:/usr/sbin:/sbin;` — TN2065 대응. 절대경로 호출과 **둘 다** |
| `BUNDLE_LINK_SUFFIX_CYS` / `_CYSD` | 1272 / 1274 | 우리 번들 접미사(Rust 판정용) |
| `BUNDLE_LINK_PATTERN` | 1276 | 같은 뜻의 셸 `case` 패턴 — **설치·해제 양쪽이 이 하나를 공유**(파괴 대칭) |
| `SHELL_PATH_NORMALIZER` | 1289 | (MAJOR-6) 셸에서도 경로 정규화 — 판정과 집행을 같은 규칙으로 |
| `BACKUP_MARK` / `RESTORE_MARK` | 1293 / 1300 | 스크립트 자기보고 마커(I5·I3③) |
| `BACKUP_COLLIDE_MSG` | 1297 | ★10차 MINOR-7 — 백업 이름 충돌 중단 사유의 머리말. **스크립트와 회귀핀이 같은 문자열 하나를 본다** |

### B.2 osascript 반환값 해석 (5차 BLOCK 계열)

| 함수 | line | 책임 |
|---|---|---|
| `split_osascript_lines` | 1319 | ★`do shell script` 반환은 **CR 구분**이다. CR·CRLF·LF 를 전부 나눈다 |
| `osascript_text_to_lf` | 1347 | 사람에게 보이기 전에 LF 로 편다 |
| `parse_pair_markers` | 1372 | `MARK:a:b` 자기보고 → `(a, b)[]` |
| `merge_backup_facts` | 1401 | 자기보고(사실) ∪ 재관측(계획 기반) — 설치 |
| `merge_restored_facts` | 2758 | 같은 합집합 — 해제(계열 대칭) |

### B.3 PATH 프로브 (그림자 측정)

| 함수 / 타입 | line | 책임 |
|---|---|---|
| `parse_which_a` | 1433 | ★시그니처 변경됨: `(stdout, begin, end) -> Result<Vec<String>, String>`. **두 표식 사이의 줄만** 채택하고, 표식이 없거나 순서가 어긋나면 **측정 실패**(C4) |
| `PROBE_BEGIN_MARK` / `_END_MARK` | 1473 / 1475 | cys 축 구간 표식 |
| `PROBE_BEGIN_MARK_D` / `_END_MARK_D` | 1479 / 1481 | cysd 축 구간 표식(C5) |
| `which_probe_command` | 1488 | 프로브 명령 조립(begin echo → which -a → end echo, cys·cysd 양축) |
| `interpret_which_probe` | 1507 | 종료 상태 + 표식 완주 여부 → `WhichProbe` |
| `WhichProbePair` | 1529 | cys 축 · cysd 축 두 결과 묶음 |
| `observe_probe_paths` | 1539 | 채택 경로가 실제로 파일인지 재관측(adv1 가짜 그림자 경화) |
| `probe_fallback_shell` | 1559 | `$SHELL` 이 `-lc` 를 못 받는 계열(csh/tcsh)이면 폴백 셸 |
| `run_which_probe` | 1576 | 셸 1회 실행 + 해석 |
| `ShadowProbe` | 1615 | `{ cys, cysd, shell_name }` |
| `probe_path_shadows` | 1623 | ★설치·상태 조회가 **같이 쓰는** 관측 헬퍼(G4). 기한 5초 · 폴백 재시도 포함 최대 10초 |
| `run_capture_with_timeout(_in)` | 1979 / 1991 | 임시 파일 리다이렉트 기반 타임아웃 실행(MAJOR-3 수리 — 파이프·드레인 스레드 없음) |
| `WhichProbe` | 2050 | `Completed(Vec<String>)` \| `Unmeasured(String)` |

### B.4 경로 정규화 · 위치 판정

| 함수 | line | 책임 |
|---|---|---|
| `strip_data_volume_prefix` | 1728 | APFS 펌링크 별칭 `/System/Volumes/Data` 접두 제거(MINOR-N8). `realpath` 로는 안 풀린다 |
| `normalize_path_str` | 1753 | 연속 슬래시 축약 + 후행 슬래시 제거(I1 · adv2) |
| `paths_equivalent` | 1775 | 위 둘을 적용한 경로 동일성 판정 — **설치·해제·상태 조회 세 경로 공통** |
| `same_file_ident` | 1792 | `(dev, ino)` 동일성 이중 확인 |
| `canonicalize_probe_to_target` | 1818 | 프로브 결과를 target 표기로 수렴 |
| `strict_install_bundle_ok` | 1851 | `plan_cli_install` 전용 엄격 위치 판정(D5 · MINOR-6). `/Applications` 또는 `<홈>/Applications` 정확 일치 |
| `install_failure_message` | 1704 | 실패 문구 + "실패 전에 이미 옮겨진 파일" 목록 |

### B.5 설치 계획·등급 판정·커맨드

| 함수 / 타입 | line | 책임 |
|---|---|---|
| `CliInstallPlan` / `plan_cli_install` | 1881 / 1889 | 설치 계획(거부 사유 4종 포함 — §3.1.4) |
| `UNVERIFIED_NOT_ON_PATH` / `_PROBE_FAILED` | 2060 / 2063 | `unverified_reason` enum 상수(계약 v2) |
| `InstallVerdict` / `classify_install_status` | 2067 / 2096 | status 3값 + `unverified_reason` 판정(순수) |
| `cysd_shadow_warning` | 2167 | (C5) cysd 그림자 경고. 측정 실패에는 침묵(G3 — 같은 사실 두 번 말하지 않는다) |
| `path_shadow_note` | 2202 | cys 축 고지 문장 |
| `InstallCliReport` | 2227 | 설치 리포트(필드 10 — §3.1) |
| `install_cli_to_path` | 2253 | ★`async fn` 커맨드 |

### B.6 해제 판정·복원·커맨드

| 함수 / 타입 / 상수 | line | 책임 |
|---|---|---|
| `LinkProbe` | 2391 | (경로, 존재, 심링크 여부, 링크 대상) — **판정부는 이 값만 본다**(dangling 대응) |
| `UninstallAction` | 2403 | `Remove` / `SkipAbsent` / `SkipNotSymlink` / `SkipForeignTarget` |
| `links_into_cys_bundle` | 2424 | 링크 대상이 cys.app 번들 안인가(`ends_with` — 셸 `case` 와 같은 뜻) |
| `decide_cli_uninstall` | 2434 | 경로 1개의 해제 판정(순수) — **설치의 백업 판정도 이것을 쓴다**(C1) |
| `build_uninstall_script` | 2470 | 승격 해제 스크립트: 집행 직전 재검증(`-L` + `readlink` 대조 — MAJOR-2) + 복원 `mv`(I3③) |
| `is_our_backup_name` | 2497 | `<base>.cys-backup-<숫자>` 정확 일치만 우리 것 |
| `pick_restore_backup` | 2509 | 여러 개면 **스탬프 최대(최신)** |
| `observe_leftover_backups` | 2542 | `/usr/local/bin` 잔존 백업본 관측 → `backups` 필드 |
| `CliUninstallPlan` | 2565 | 해제 계획(+ `osascript_arg: Option` — `None` 이면 승격 안 띄움) |
| `SKIP_REASON_ABSENT` / `_NOT_SYMLINK` / `_FOREIGN_TARGET` | 2585 / 2587 / 2589 | `skipped_reasons` enum 상수(C3) |
| `skip_reason_tag` | 2593 | 판정 → 기계 태그 |
| `all_skips_benign` | 2605 | `skipped_benign` 판정 — **해제 등급의 유일 계약** |
| `plan_cli_uninstall` | 2612 | 해제 계획(+ 복원 후보) |
| `uninstall_failure_message` | 2680 | 실패 문구 + 이미 제거된 것 / 이미 되돌린 것 / 아직 남은 것(C2) |
| `observe_removed` | 2717 | 계획 대상 재관측 → (사라진 것, 남은 것). 복원된 자리는 '사라진 것' |
| `observe_restored` | 2739 | 복원 계획 재관측 |
| `UninstallCliReport` | 2843 | 해제 리포트(필드 7 — §3.2) |
| `uninstall_cli_from_path` | 2872 | ★`async fn` 커맨드 |

### B.7 상태 조회

| 함수 / 타입 | line | 책임 |
|---|---|---|
| `CliLinkState` | 2772 | `Absent` / `Ours` / `Partial` / `Foreign` |
| `classify_cli_links` | 2784 | 두 축 판정 → 상태(순수) |
| `probe_link` | 2821 | 파일시스템 얇은 래퍼(`symlink_metadata`·`read_link`) — 순수부와 분리 |
| `CliInstallStatusReport` | 3000 | 상태 리포트(필드 7 — §3.3) |
| `cli_install_status` | 3048 | ★`async fn` 읽기전용 커맨드. `Err` 를 한 번도 반환하지 않는다 |

### B.8 테스트 쪽 앵커

| 이름 | line | 책임 |
|---|---|---|
| `mod tests` 시작 | 5826 | `#[cfg(test)]` — 파일 전체 `#[test]` **103개**(10차 MINOR-7 회귀핀 1건 증가) |
| `major1_premise_partial_with_notes_does_not_imply_foreign_present` | 7790 | 7차에서 **워커가 master 판정 규칙을 반증**한 4단 증명(4.14 ③) |
| `adv1`~`adv9` | 8235 · 8270 · 8290 · 8345 · 8396 · 8447 · 8472 · 8498 · 8531 | 적대적 재현 → 결함 부재 회귀핀(4.7 표) |
| `dump_report_contract_for_the_ui_gate` | 8850 | I6 계약 덤프 — `ui/src/__contract__.json` 을 **쓴 뒤** 자기 점검(§3.4) |

### B.9 UI 쪽 앵커 (`ui/src/clipath.ts` @ **10라운드 문서 레인 측정 시점**)

> ⚠ **이 표의 값은 다시 밀린다.** 10라운드에 UI 레인이 같은 파일을 고치고 있었고(4.13 #1·#2·#3),
> 문서 레인은 그 작업이 끝나기 전에 쟀다. freeze 후 `grep -n` 으로 재유도하라 — §9.3 U9.

| 이름 | line | 책임 |
|---|---|---|
| `CliInstallStatus` 타입 | 129 | `"installed" \| "installed_shadowed" \| "unverified"` |
| `str` / `strOrNull` / `strList` | 148 / 151 / 154 | 판독기 원시 변환 — **모르면 안전한 쪽으로 접는다** |
| `readInstallReport` | 160 | 설치 리포트 판독기 |
| `normalizeInstallStatus` | 183 | 계약 밖 값·누락 → `"unverified"` |
| `unverifiedCause` | 210 | `unverified_reason` → `"not_on_path"`\|`"probe_failed"`\|`"unknown"` |
| `installResultToast` | 276 | 설치 결과 → 토스트 등급(installed + warnings 0 만 ✅/volatile) |
| `CliLinkState` 타입 / `LINK_STATES` | 376 / 405 | `state` enum + 계약 밖 값 방어 |
| `readCliStatus` | 410 | 상태 리포트 판독기 |
| `cliButtonView` | 508 | 버튼 라벨·활성 판정 |
| `cliNoticeLines` | 682 | 고지 줄 조립 |
| `NOTICE_TITLE_FOREIGN` / `_BACKUP` / `_INFO` / `_PARTIAL` | 728 / 729 / 730 / 732 | 상태 알림 제목 **네 갈래**(7차 MAJOR-1 이 `_PARTIAL` 을 신설) |
| `statusNoticeKind` | 772 | 어느 제목을 낼지 판정 — `silent`\|`foreign`\|`partial`\|`backup`\|`info` |
| `backupNoticeLine` | 647 | 백업본 고지 줄(ours·absent·partial·foreign 네 갈래) — 10차 MAJOR-1 의 수리 대상 |
| `readUninstallReport` | 928 | 해제 리포트 판독기 |
| `uninstallResultToast` | 985 | 해제 결과 → 토스트 등급(`ok` + `skipped_benign` 둘만 본다) |
