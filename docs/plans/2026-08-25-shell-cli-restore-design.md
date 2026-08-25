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

- **레인**: `<WORKTREE>` @ `fix/shell-cli-install-restore`
- **기준점(base)**: `c54c3b2` = v0.14.24 (2026-08-22 릴리스)
- **작성 시점 HEAD**: `9f47148` (3라운드 수리 완료 · 4라운드 착수 직전)

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
| 2026-08-25 | `9f47148` | 3차 — 산문 계약 폐기(`unverified_reason` 기계 판별자)·부분실패 백업 통보·셸 종료상태 존중·펌링크 별칭. |
| 2026-08-25 | (진행 중) | 4차 — **계열(class) 수리**. 지점이 아니라 결함의 계열을 닫는다. |

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

> **작성 근거**: `src-tauri/src/main.rs` @ `9f47148` 실물. 아래 line 번호는 그 리비전 기준이다.
> **serde rename 없음** — Rust 필드명이 그대로 snake_case 로 JavaScript 에 노출된다.
> 세 커맨드 모두 Rust 시그니처는 `-> Result<T, String>` 이며, Tauri `invoke` 에서 `Err` 는
> **Promise reject** 로 도착한다(TS 는 반드시 try/catch 로 감싼다).

### 3.0 커맨드 3종 요약

| 커맨드 | 선언 위치 | 승격 | 호출 시점 |
|---|---|---|---|
| `install_cli_to_path()` | `main.rs:1649` | osascript 1회 | 버튼 클릭(설치 라벨) |
| `uninstall_cli_from_path()` | `main.rs:2009` | osascript 1회 (지울 것 있을 때만) | 버튼 클릭(해제 라벨) |
| `cli_install_status()` | `main.rs:2085` | **없음** (읽기 전용) | CC 열림 1회 + 액션 직후 1회 |

### 3.1 `install_cli_to_path()` → `InstallCliReport`

정의: `main.rs:1626-1647` (`#[derive(serde::Serialize)] struct InstallCliReport`)

| 필드 | Rust 타입 | JSON 타입 | null 허용 | 설명 |
|---|---|---|---|---|
| `ok` | `bool` | boolean | 불가 | `status == "installed"` 의 **파생값**(`main.rs:1779`). 두 개의 진실을 만들지 않는다. 부분 성공(그림자·측정불능)은 `false`. |
| `status` | `String` | string | 불가 | **enum 3값** — 아래 3.1.1 |
| `target_dir` | `String` | string | 불가 | 항상 `"/usr/local/bin"` |
| `cys_link` | `String` | string | 불가 | `"/usr/local/bin/cys"` |
| `cysd_link` | `String` | string | 불가 | `"/usr/local/bin/cysd"` |
| `source_cys` | `String` | string | 불가 | 링크가 가리키는 번들 안 실행파일 절대경로 |
| `effective_cys` | `Option<String>` | string \| null | **허용** | `which -a cys` 1순위. `unverified` 두 분기에서는 `null`. |
| `shadowed_by` | `Option<String>` | string \| null | **허용** | `/usr/local/bin/cys` 앞을 가리는 다른 cys. `installed_shadowed` 에서만 `Some`. |
| `unverified_reason` | `Option<String>` | string \| null | **허용** | **enum 2값 + null** — 아래 3.1.2. 계약 v2(2026-08-25) 확장. |
| `warnings` | `Vec<String>` | string[] | 불가(빈 배열 가능) | 사람용 설명 문장. **계약이 아니다** — 정규식 파싱 금지. |

#### 3.1.1 `status` enum 전체 값 (정확히 3개)

```
"installed"           심링크 생성 + 로그인 셸 기준 `which -a cys` 1순위 == /usr/local/bin/cys
"installed_shadowed"  심링크는 생겼으나 PATH 앞을 가리는 다른 cys 가 있다
"unverified"          확인을 못 했다(probe_failed) 또는 로그인 셸 PATH 에서 cys 를 못 찾았다(not_on_path)
```

계약 밖의 값·필드 누락은 TS 판독기가 전부 `"unverified"` 로 접는다
(`ui/src/clipath.ts` `normalizeInstallStatus`) — 구버전 백엔드와 붙어도 성공으로 둔갑하지 않게 하는 장치.

#### 3.1.2 `unverified_reason` enum 전체 값 (정확히 2개 + null)

Rust 상수 정의: `main.rs:1539` / `main.rs:1542`

```
"not_on_path"    검증 명령이 정상 종료했고 로그인 셸 PATH 에서 cys 를 못 찾았다 (원인 = PATH 구성)
"probe_failed"   검증 명령 자체를 못 돌렸다 — 실행 실패·비정상 종료·타임아웃
null             status 가 "unverified" 가 아니다
```

TS 는 값이 없거나 계약 밖이면 `"unknown"` 으로 접는다(`unverifiedCause`) — **모르면 모른다고 말한다.**

#### 3.1.3 상태 불변식 (`classify_install_status`, `main.rs:1575-1621`)

| `status` | `unverified_reason` | `effective_cys` | `shadowed_by` | `warnings` |
|---|---|---|---|---|
| `installed` | `null` | `Some(target)` | `null` | 빈 배열(설치 경로에서 백업 통보문이 합류할 수 있음) |
| `installed_shadowed` | `null` | `Some(first)` | `Some(first)` | 1건 이상 |
| `unverified` | `"not_on_path"` | `null` | `null` | 1건 이상 |
| `unverified` | `"probe_failed"` | `null` | `null` | 1건 이상 |

★`unverified_reason` 은 `status == "unverified"` 일 때만 `Some` 이고, 그 외에는 **반드시** `None`.

#### 3.1.4 Err 반환값 (Promise reject)

- `"이 기능은 macOS 전용입니다."` — non-macOS 심층방어
- `"번들 디렉토리 해석 실패"` — `current_exe().parent()` 실패
- translocation·백업 번들·비표준 위치 거부 메시지 (D5 — `plan_cli_install`, `main.rs:1368`)
- `"심볼릭 생성 실패: …"` — 승격 스크립트 rc != 0 (3라운드 이후 **잔존 백업 경로와 복구 명령을 동봉**)
- 사용자 취소

### 3.2 `uninstall_cli_from_path()` → `UninstallCliReport`

정의: `main.rs:1994-2003`

| 필드 | Rust 타입 | JSON 타입 | null 허용 | 설명 |
|---|---|---|---|---|
| `ok` | `bool` | boolean | 불가 | 계획한 제거가 **전부 실측으로 확인**됐는가. 지울 것이 없었던 경우도 `true`. |
| `removed` | `Vec<String>` | string[] | 불가(빈 배열 가능) | 사후 재관측으로 **정말 사라진** 경로만 담는다(산출자의 자기신고를 믿지 않는다). |
| `skipped` | `Vec<String>` | string[] | 불가(빈 배열 가능) | ★**문자열 배열이다.** `"경로 — 사유"` 형식. **객체 배열이 아니다.** |
| `warnings` | `Vec<String>` | string[] | 불가(빈 배열 가능) | 남아 있는 경로 + 유일한 복구 명령 `sudo rm <경로>` |

> **`skipped` 가 문자열인 이유와, 이 한 줄이 만든 사고**
> 이전 TS 는 이것을 `{path, reason}[]` 로 읽고 `.filter(s => s && s.path)` 로 걸렀다
> (당시 `clipath.ts:157,168`). 객체가 아니므로 `s.path` 는 항상 `undefined` → **모든 skip 이
> 소멸**했고, 부분 실패가 성공 토스트로 둔갑했다. `warnings`·`ok` 는 TS 타입에 아예 없었고
> (당시 `clipath.ts:158-162`), 그 결과 유일한 복구 명령이 사용자에게 **도달하지 못했다.**
> 단위테스트가 초록이었던 이유는 픽스처가 Rust 실물이 아니라 **잘못된 TS 모양**을 먹였기 때문이다.
> 지금은 `clipath.test.ts` 의 계약-드리프트 가드가 재발을 막는다.

> **⚠ 4라운드 예정 확장 (아직 실물에 없다)**
> C3(제4.4절)이 `UninstallCliReport` 에 **기계 판별자** 1개를 추가한다 —
> `skipped_benign: bool`(건너뛴 것이 전부 '지울 게 없었다' 류인가) 또는 그보다 나은 형태.
> 목적은 TS 의 `isBenignSkip` 이 Rust 산문('이미 해제')을 정규식으로 파싱하지 않게 하는 것이다
> (설치 경로의 `unverified_reason` 과 같은 원리 — 제5절 ⑪조). **4차가 끝나면 이 절의 표에
> 그 필드를 정식 행으로 올린다.**

#### 3.2.1 Err 반환값

- `"이 기능은 macOS 전용입니다."`
- `"osascript 실행 실패: {e}"` — 프로세스 기동 자체가 실패
- `"해제가 취소되었습니다."` — stderr 에 `-128` 또는 `User canceled`
- `"심볼릭 제거 실패: {stderr}"`

#### 3.2.2 승격을 띄우지 않는 경로

`plan_cli_uninstall` 이 `osascript_arg = None` 을 내면(지울 것이 하나도 없음) **비밀번호 창을
띄우지 않고** `ok:true, removed:[], skipped:plan.skipped, warnings:[]` 로 즉시 반환한다
(`main.rs:2019-2027`).

### 3.3 `cli_install_status()` → `CliInstallStatusReport`

정의: `main.rs:2066-2080`

| 필드 | Rust 타입 | JSON 타입 | null 허용 | 설명 |
|---|---|---|---|---|
| `platform_supported` | `bool` | boolean | 불가 | macOS 전용 기능. UI 는 이 값 하나로 버튼 노출 여부를 정할 수 있다(`false` 면 숨김). |
| `installed` | `bool` | boolean | 불가 | `true` 면 라벨 '해제', `false` 면 '설치'. `state ∈ {ours, partial}` 의 파생값. |
| `state` | `String` | string | 불가 | **enum 5값** — 아래 3.3.1 |
| `cys_link` | `String` | string | 불가 | `"/usr/local/bin/cys"` |
| `cysd_link` | `String` | string | 불가 | `"/usr/local/bin/cysd"` |
| `notes` | `Vec<String>` | string[] | 불가(빈 배열 가능) | 설치도 해제도 아닌 상태(실체 파일·타 대상 링크)의 사유 — **사용자 고지용** |

#### 3.3.1 `state` enum 전체 값 (정확히 5개)

`CliLinkState` 열거 + non-macOS 전용 문자열 하나:

```
"absent"       우리 링크가 하나도 없다            → 라벨 "셸에 cys 설치"
"ours"         cys·cysd 둘 다 우리 번들 심볼릭     → 라벨 "셸 cys 해제"
"partial"      한쪽만 우리 것(중단된 설치·부분 삭제 잔재) → 해제로 청소 가능
"foreign"      파일은 있으나 우리 것이 아니다(실체 파일·타 대상 링크) → 설치 라벨 유지 + notes 고지
"unsupported"  non-macOS. Rust 가 Err 를 던지지 않고 이 값으로 답한다.
```

> **non-macOS 에서 `Err` 를 던지지 않는 이유**(주석 `main.rs:2081-2084`)
> Control Center 를 열 때마다 실패 토스트가 뜨기 때문이다. 대신 `platform_supported=false`
> 로 답한다. `install`/`uninstall` 쪽 non-macOS `Err` 는 심층방어로 **존치**한다.

#### 3.3.2 `notes` 문장 형식 (`main.rs:2104-2119`)

- 실체 파일: `"{경로} — 심볼릭이 아닌 실제 파일이 이미 있습니다(다른 도구 설치본일 수 있어 자동으로 제거하지 않습니다)."`
- 타 대상 링크: `"{경로} — cys.app 번들 밖({대상})을 가리키는 링크입니다."` (대상을 못 읽으면 `대상 읽기 실패`)

★이 문장들은 **사람용**이다. UI 는 이것을 그대로 표시하되 **파싱해서 분기하지 않는다**(제5절 ⑪조).

### 3.4 계약을 지키는 장치 (드리프트 방지)

| 층위 | 장치 | 위치 |
|---|---|---|
| Rust → JSON | `serde_json::to_value` 스냅샷 단언 | `main.rs` `mod tests` (`:5839-5854` 등) |
| TS 판독기 | `readInstallReport` — 모르는 값은 **안전한 쪽으로** 접는다 | `ui/src/clipath.ts` |
| TS 테스트 | 계약-드리프트 가드 + "정규식 재도입 차단" 테스트 | `ui/src/clipath.test.ts` |
| **(4라운드 예정)** | Rust 테스트가 키 집합 + 타입 태그를 `ui/src/__contract__.json` 으로 덤프하고 TS 게이트가 그것을 읽는다 | I6 — 아래 4.5 |

---

## 4. 라운드 이력

각 라운드는 이렇게 돌았다: **구현/수리 → 게이트(cargo test · bun test · tsc · 번들) → 독립
반증 3렌즈 → master 판정 → 다음 라운드**. 반증자는 "이 구현은 틀렸다"를 기본 자세로 두고,
틀렸음을 입증하지 못할 때만 물러선다. 근거는 `file:line` 으로 대며, 실행 가능한 반증
(테스트 실행·코드 추적)을 우선한다.

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
| MINOR-6 | `/tmp/Applications` 가 Canonical 통과 | `classify_bundle_dir` 은 **건드리지 않고**(`autoregister_allowed` 도 쓰므로 산탄총 수술) `plan_cli_install` 전용 엄격 판정 신설(`strict_install_bundle_ok`, 현 `main.rs:1334`) + 반례 테스트 |
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
  현 `main.rs:1130`)하고 존재하는 것을 에러 메시지에 포함(복구 명령 `sudo mv <bak> <orig>` 포함).
- MINOR-N2·N5: 성공 플래그를 쓴다. rc!=0 이면 `probe_failed`. `$SHELL` 이 `-lc` 를 못 받는
  셸(csh/tcsh 계열)이면 `/bin/zsh` 또는 `/bin/bash` 로 한 번 폴백 재시도(폴백했다는 사실을 문구에 밝힘).
- MINOR-N7: 문서·UI 문구가 **실제로 읽히는 파일**(`~/.zshenv`·`~/.zprofile`·`~/.zlogin`)을 지목.
  "이 확인은 비대화형 로그인 셸 기준입니다 — 터미널에서 cys 가 이미 동작한다면 무시해도 됩니다"를
  문구에 넣어 거짓 경고에 사용자가 헛수고하지 않게 함.
  ★**대화형(`-lic`)으로 바꾸는 안은 채택하지 않는다**: 사용자의 대화형 rc 를 버튼 클릭 부작용으로
  실행하면 nvm/conda/oh-my-zsh 같은 것이 백그라운드 프로세스를 띄울 수 있다. **정직한 문구가 답이다.**
- MINOR-N8: 비교 전 정규화(`strip_data_volume_prefix`, 현 `main.rs:1306`). 반례 테스트:
  `/System/Volumes/Data/Applications/cys.app/Contents/MacOS` 통과, `/tmp/Applications/...` 거부 유지.
- MINOR-N4·N6: sticky 재사용 시 `className` 을 현재 category 로 갱신 / volatile 로 낼 때 같은 id 의
  살아있는 sticky 를 먼저 내림. 순수 로직은 `clipath.ts` 로 뽑아 테스트(`toastEmitPlan`).
- 사전 존재 결함: `main.rs:4804-4809` 의 고아 doc 주석 + 중복 `#[test]` 로 같은 테스트가 두 번
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

### 4.4 4차 — 지배 원리: 지점이 아니라 계열을 닫는다

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

### 4.5 4차 개별 수리 (계열과 별도)

| 번호 | 결함 | 수리 |
|---|---|---|
| **I1** [경로 정규화 · adv2] | `classify_install_status` 의 `first == target_cys` 가 문자열 완전일치. PATH 에 `/usr/local/bin/`(후행 슬래시)가 있으면 which 출력이 `/usr/local/bin//cys` 라 **정상 설치가 `installed_shadowed` 로 뒤집히고, 경고문이 방금 만든 자기 링크를 지우라고 안내한다.** | 비교 전 경로 정규화(연속 슬래시 축약 + 후행 슬래시 제거). `strip_data_volume_prefix` 와 같은 층에 두고 **설치·해제·상태 조회 세 경로 모두**에 적용(계열!). 가능하면 `std::fs::metadata` 의 `(dev, ino)` 동일성으로 이중 확인 |
| **I2** [adv8] | 설치 결과가 `installed` 가 아닌데도 상태 재조회가 `installed=true` 를 내 **버튼이 '해제'가 된다** | 미완료(shadowed·unverified) 상태에서는 '다시 설치'를 유지하고 해제는 분리하거나, 최소한 라벨이 사용자를 오도하지 않게 한다. 순수 판정 + 테스트 |
| **I3** [백업 재발견] | `cli_install_status` 가 `*.cys-backup-*` 를 보지 않아 `notes` 에 안 뜨고, 해제는 심볼릭만 지우며, 유일한 통보 sticky 는 60초 뒤 사라진다(수용처 `alarmHistory` 는 메모리 전용). **'잃는 것이 없어야 1클릭이 정당하다'는 BLOCK-1 의 정당화가 무너진다.** | ① `cli_install_status` 가 잔존 백업을 관측해 `notes` 에 상시 노출 ② 해제 확인 문구에 '설치 때 백업해 둔 원본이 있습니다' 분기 ③ 가능하면 해제 시 '원본 복원'(우리 이름 규칙에 **정확히 일치하는** 백업만 `mv`). ③은 비가역이 아니라 복원이므로 안전하나, 복잡하면 ①②만 하고 ③은 사유와 함께 미수리로 보고 |
| **I4** [승격 스크립트 PATH] | `do shell script` 가 부모 PATH 를 상속한다. `mkdir`·`mv`·`ln`·`readlink`·`rm` 을 절대경로 없이 부른다. **TN2065 는 'Use the full path to the command' 라고 못박고**, 이 기계의 상속 PATH 에는 사용자 쓰기 가능 디렉터리가 `/usr/bin` **앞에** 있다 | 스크립트 첫머리에 `export PATH=/usr/bin:/bin:/usr/sbin:/sbin;` 를 박거나 전 명령을 절대경로로(둘 다 하면 더 좋다). **설치·해제 양쪽**(계열!). 테스트로 못박음 |
| **I5** [설치 TOCTOU] | 해제는 2라운드에서 스크립트 자체 재검증을 넣었는데 **설치는 비특권 사전 관측에만 의존한다** | 스크립트가 자기가 한 일을 stdout 으로 보고하게 한다 — 백업할 때 `echo "CYS-BACKED-UP:<원본>:<백업본>"` 를 찍고, Rust 가 osascript stdout 을 파싱해 **계획이 아니라 사실**을 보고한다. 승격 창 안의 상태 변화도 잡힌다. **성공·실패 양쪽 반환 경로에서** 이 stdout 을 읽어야 한다 |
| **I6** [계약 기계화 — 최우선 구조 수리] | `clipath.test.ts` 의 `RUST_*_REPORT` 표는 **손으로 쓴 사본**이다. 실물이 바뀌어도 안 빨개진다 | Rust `mod tests` 에 `serde_json::to_value(...)` 로 세 리포트의 **키 집합 + 타입 태그**를 `ui/src/__contract__.json` 으로 덤프하는 테스트를 두고, `clipath.test.ts` 가 그 파일을 읽어 `expectShape` 의 기준으로 삼는다(손으로 쓴 표를 **대체**). **검수 기준 하나: 아무 필드에 `#[serde(rename=...)]` 를 붙였을 때 TS 게이트가 빨개지는가.** 그 실험을 실제로 해 보고 결과를 보고(실험 후 원복) |
| **I7** [잡동사니] | ① `#[cfg_attr(not(target_os="macos"), allow(dead_code))]` 누락 3곳: `plan_cli_install`·`build_install_script`·`struct CliInstallPlan` ② 부트 회귀핀 추가: `classify_bundle_dir("/System/Volumes/Data/Applications/cys.app/Contents/MacOS") == Canonical` 과 `~/Applications` 데이터볼륨 형태(지금 핀은 `strict_install_bundle_ok` 쪽에만 있다) ③ `ToastEmit.className` 이 실제로 소비되지 않는 **죽은 필드** — 소비하게 고치거나 필드와 그 테스트를 삭제 ④ 중복 문구: `not_on_path` 경고의 '비대화형 로그인 셸 기준' 문장이 Rust 산문과 TS 양쪽에 있어 토스트에 **두 번** 나온다 → ★master 결정: **Rust 에서 뺀다**(백엔드는 사실만, 표현은 UI 소유) ⑤ `#cc-header` 오버플로 가드(닫기 버튼이 밀려날 수 있다) — `ui/src/style.css` 수정을 이번 라운드에 한해 허용, **`#cc-header` 규칙에만** 손댄다 | |

### 4.6 4차 인수 기준 (라운드 시작 전 선언·동결)

`ADVERSARIAL_TESTS_R2.rs` 의 `adv1`~`adv9` 를 `src-tauri/src/main.rs` 의 `mod tests` 말미에
편입하고 **전부 초록**이 되어야 한다. 다만 그대로 붙이지 말고 **헤르메틱하게 재작성**한다:

- **실기계 로그인 프로필 실행 금지** — 스텁 셸 스크립트를 임시 디렉터리에 만들어 그것을 셸로 지정.
- `std::env::set_var` 를 쓰는 테스트는 **전용 Mutex 로 직렬화**한다(선례: `src/pack.rs` 의 `PACK_ENV_LOCK`).
- **`/usr/local/bin` 접근 절대 금지.**
- 수리 후에도 **존치**한다(제거 금지 — red→green 회귀핀).

초록이 안 되는 항목이 있으면 **지우지 말고 `#[ignore]` 도 붙이지 말고** 사유를 미수리로 적는다.

---

## 5. ★설계 규약 13조

> 이번 성찰(설계 감사 · 적대적 감사 · 아키텍트 감사 3라운드)이 도출한 **재발 방지 조문**이다.
> 이 조문들은 이 작업에만 적용되는 것이 아니라, 앞으로 이 레포에서 설계서를 쓰는 모든 라운드의
> 최소 요건이다. 각 조문 뒤의 괄호는 그 조문을 낳은 실제 사고다.

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

## 6. 미결 · 오너 결정 대기 항목

### 6.1 이번 라운드 안에서 판정될 것 (4차 진행 중)

| 항목 | 내용 | 상태 |
|---|---|---|
| I3-③ 원본 복원 | 해제 시 '원본 복원'(우리 이름 규칙에 정확히 일치하는 백업만 `mv`) 선택지. 비가역이 아니라 복원이므로 안전하나 구현이 복잡하다 | 구현이 복잡하면 ①②만 하고 ③은 사유를 적어 미수리로 보고 — 4차 산출물에서 확인 |
| I6 검수 실험 | 아무 필드에 `#[serde(rename=...)]` 를 붙였을 때 **TS 게이트가 빨개지는가** — 실제로 실험하고 결과를 보고(실험 후 원복) | 4차 산출물에서 확인 |
| `adv1`~`adv9` 전부 초록 | 4차 인수 기준. 초록이 안 되는 항목은 지우지도 `#[ignore]` 하지도 않고 사유를 적는다 | 4차 산출물에서 확인 |
| C3 판별자 형태 | `skipped_benign: bool` 로 갈지 "더 나은 형태"로 갈지 | 워커 재량 — 단 **TS 가 산문을 파싱하지 않을 것** 이 절대 조건 |

### 6.2 오너 결정이 필요한 것

| 항목 | 쟁점 | 현재 잠정 결정 |
|---|---|---|
| **Windows PATH 등록** | D2 는 Windows 자동 PATH 편집을 **구현하지 않기로** 했다(`setx` 1024자 잘림 · `REG_EXPAND_SZ→REG_SZ` 변환으로 `%USERPROFILE%` 류 파손). Windows 사용자는 터미널에서 `cys` 를 쓰려면 수동으로 PATH 를 넣어야 한다 | **미구현 유지 + 문서 안내만.** 오너 절대지침(윈도우 신중) 준수. 다시 열려면 오너 결정 필요 |
| **MINOR-12(b) Windows MSI PATH 문장 충돌** | `docs/INSTALL.md:183`("설치 시 PATH에 등록됩니다") ↔ `USER-MANUAL.md:80`("설치기는 외부 PATH를 등록하지 않음")이 정면 충돌. 실측(`tauri.conf.json` 번들 타겟 nsis/msi · `dist-win/` · `scripts/` 설치기 설정)으로 판정하되 **실측으로 판정이 안 되면 고치지 말고 '판정 불가'로 보고 — 추측 정정 금지** | 실측 결과가 아직 이 문서에 반영되지 않았다. 2차 산출물에서 확인 필요 |
| **릴리스 번호** | v0.14.25 는 타 레인이 선점. 이 레인은 **v0.14.26 예약** | 버전 SOT(`Cargo.toml`·`src-tauri/Cargo.toml`·`src-tauri/tauri.conf.json`)는 이번 라운드에서 **건드리지 않는다** |
| **라운드 종료 판정** | 헌장상 라운드 종료 = 미달 항목 0 또는 10라운드. 현재 4라운드 | 4차 결과에 따라 5차 필요 여부를 오너께 보고 |

### 6.3 이 레인이 건드리지 않는 것 (경계)

- **수정 절대 금지**: `src/bin/**` · `src/lib.rs` · `src/pack.rs` · `cysjavis-pack/**` ·
  `.github/**` · `ui/e2e/**`
- **`/usr/local/bin` 절대 금지** — 실행 중인 데몬이 그 심볼릭에 의존한다. 스크립트 실행 검증은
  반드시 임시 디렉터리 사본에서만.
- `<OTHER-LANE-WORKTREE>` 은 타 세션 작업 중 — **읽지도 않는다.**
- `git commit`/`checkout`/`reset`/`stash` 금지(커밋은 master). 데몬·서버 기동 금지.
  `cargo` 병렬 실행 금지(타 세션과 CPU 공유).
- `ui/src/style.css` 는 4라운드에 한해 **`#cc-header` 규칙에만** 수정 허용.

### 6.4 이 레인 밖으로 이관된 발견

부수 발견: cys 노드 부트 사슬에서 **fresh 프로필 로그인 선택지 → 디렉티브 주입 Return 이
'No, exit' 에 꽂혀 agent 를 죽이는 경로**를 실측했다. 타 세션(부트 결정론 캠페인) 소관이라
증거만 이관했다 — 증거 파일은 스크래치의
`boot-chain-evidence-for-determinism-campaign.md`, 레인 커밋 `3080315` 에 함께 담겼다.

---

## 부록 A. 관련 파일 지도

| 파일 | 역할 |
|---|---|
| `src-tauri/src/main.rs` | 순수 판정 함수 전부 + 커맨드 3종 + `generate_handler!` 등록 + `mod tests` |
| `ui/index.html` | `#cc-header` 안의 `<button id="btn-install-cli">` (hidden 시작) |
| `ui/src/main.ts` | 버튼 배선(invoke 호출·DOM 갱신). **판정은 하지 않는다** |
| `ui/src/clipath.ts` | 순수 판정 — 플랫폼 노출·버튼 라벨·결과 토스트 등급을 **여기서만** 정한다 |
| `ui/src/clipath.test.ts` | 계약-드리프트 가드 + 정규식 재도입 차단 테스트 |
| `ui/src/__contract__.json` | **(4차 예정)** Rust 테스트가 덤프하는 계약 스냅샷. TS 게이트의 기준 |
| `docs/INSTALL.md` | §B 설치/해제 안내 · `INST-DENY-02` |
| `USER-MANUAL.md` | §2.4 사용자 안내 |
| `docs/plans/2026-06-29-cli-path-install.md` | 최초 신설 시점의 구현 계획(가드 5종 원안) |
| `docs/plans/2026-08-25-shell-cli-restore-design.md` | **이 문서** — 복원 라운드의 설계 정본 |

## 부록 B. 주요 순수 함수 색인 (`main.rs` @ `9f47148`)

| 함수 / 타입 | line | 책임 |
|---|---|---|
| `sh_squote` | 764 | 셸 작은따옴표 인용(설치·해제 공용) |
| `classify_bundle_dir` | 776 | 번들 위치 분류(Canonical/Translocated/Backup/NonStandard). **`autoregister_allowed` 도 쓴다 — 산탄총 수술 주의** |
| `observe_existing_backups` | 1130 | 실패 반환 전 백업 후보 재관측(MAJOR-N1 수리) |
| `build_install_script` | 1157 | 승격 설치 스크립트 문자열 생성 |
| `parse_which_a` | 1188 | `which -a` stdout → precedence 순 절대경로 목록 |
| `strip_data_volume_prefix` | 1306 | `/System/Volumes/Data` 접두 제거(펌링크 별칭 정규화) |
| `strict_install_bundle_ok` | 1334 | `plan_cli_install` 전용 엄격 위치 판정(D5 · MINOR-6) |
| `CliInstallPlan` / `plan_cli_install` | 1361 / 1368 | 설치 계획(거부 사유 포함) |
| `run_capture_with_timeout(_in)` | 1458 / 1470 | 임시 파일 리다이렉트 기반 타임아웃 실행(MAJOR-3 수리) |
| `WhichProbe` | 1529 | `Completed(Vec<String>)` \| `Unmeasured(String)` |
| `InstallVerdict` / `classify_install_status` | 1546 / 1575 | status 3값 + `unverified_reason` 판정 |
| `InstallCliReport` / `install_cli_to_path` | 1627 / 1649 | 설치 리포트 / 커맨드 |
| `LinkProbe` | 1800 | (경로, 존재, 심링크여부, 링크대상) |
| `UninstallAction` | 1812 | Remove / SkipAbsent / SkipNotSymlink / SkipForeignTarget |
| `links_into_cys_bundle` | 1828 | 링크 대상이 cys.app 번들 안인가 |
| `decide_cli_uninstall` | 1839 | 경로 1개의 해제 판정 |
| `build_uninstall_script` | 1865 | 승격 해제 스크립트(집행 직전 재검증 포함 — MAJOR-2 수리) |
| `CliUninstallPlan` / `plan_cli_uninstall` | 1883 / 1890 | 해제 계획(+ `osascript_arg: Option` — None 이면 승격 안 띄움) |
| `classify_cli_links` | 1947 | absent / ours / partial / foreign |
| `probe_link` | 1973 | 파일시스템 얇은 래퍼(순수부와 분리) |
| `UninstallCliReport` / `uninstall_cli_from_path` | 1995 / 2009 | 해제 리포트 / 커맨드 |
| `CliInstallStatusReport` / `cli_install_status` | 2067 / 2085 | 상태 리포트 / 읽기전용 커맨드 |
