# 진단 계약 (DIAGNOSTIC-CONTRACT)

> v0.13.21 브라우저 품질 개혁(WS-8)의 정본 계약 문서. **"거부의 이유를 말하는 능력"을 보안 설계의
> 일부로 격상한다** — fail-closed 거부가 사람에게 침묵으로 보이면, 그 침묵은 보안이 아니라 장애다.
> 설계 SOT: `_round/DESIGN_BROWSER_QUALITY_OVERHAUL.md` v1.4 §5-1·§5-0-A·§9-4.
> 코드 인용은 브랜치 `feat/browser-quality-overhaul` 실물에서 직접 읽어 file:line을 병기한다.

관련 파일:
- `src/bin/cysd/authority_broker/runtime_launcher.rs` — supervisor 사인 채집·분류(StderrTap), 런치 실패 code
- `src/bin/cysd/authority_broker/mod.rs` — 소유권 판정·부서 정책·`BrokerStatus` 스키마
- `src/bin/cysd/handlers.rs` — 부서(dept) 데몬 런치 verb 게이트
- `src-tauri/src/main.rs` — `RpcFailure`·배너 합성(`banner()`)·`include_str!` 트립와이어
- `ui/src/webpane.ts` / `ui/src/main.ts` — 배너 절단 예산(`castFailureReason` maxLen)·이벤트 수신
- 상호 참조: [`docs/BROWSER-ACTIVATION-CONTRACT.md`](BROWSER-ACTIVATION-CONTRACT.md) (활성화 경로 계약),
  `_round/RCA_BROWSER_DISABLED_SAFE.md` (재발 시 점검 절차)

> **라인 번호 규약**: 이 문서의 `file:line`은 `feat/browser-quality-overhaul`(베이스 `c232a1e`)
> 실측값이며, 편집으로 이동한다. **정본 조회는 항상 `grep -rn '"<CODE>"' src/bin/cysd/`**,
> 함수·상수 이름은 `grep -n 'fn <이름>'`으로 한다. 라인이 어긋났다고 계약이 폐기된 것은 아니다.

---

## 1. 3조 규칙 (불변식 — 수정자 필독)

이 문서가 강제하는 것은 표가 아니라 **세 개의 규칙**이다. 표는 규칙의 현재 스냅샷일 뿐이다.

### 규칙 ① — 모든 fail-closed 거부는 기계 판독 `error.code`를 갖는다

브로커가 요청을 거부할 때는 반드시 `BrokerFailure::new("<CODE>", <message>)` 형태로 **코드와 메시지를
분리**해 반환한다. `code` 없이 산문 message만 올리는 거부는 금지다.

- 근거: `code`가 없으면 GUI가 **분기할 수 없다**. 자가 회복(재등록·재시도)은 message 부분열 매칭이라는
  깨지기 쉬운 폴백에 의존하게 되고, 문구 한 글자만 바뀌면 무증상으로 죽는다(v0.13.10 실사고).
- 종단 전파 경로: `BrokerFailure{code,message}` → NDJSON `error` 객체 → `rpc_oneshot_full`
  (`src-tauri/src/main.rs:522`, 전체 `Value` 보존) → `RpcFailure` → `banner()`
  (`src-tauri/src/main.rs:436-438`) → GUI pane 배너.
- **`rpc_full` 직사용 금지** — 반드시 `rpc_on_full`(`:463`) / `rpc_coded`(`:475`)를 경유한다.
  구 `rpc_oneshot`(`:511`)은 message만 반환하는 래퍼로 강등됐고, 기존 소비자 호환용으로만 남는다.
- 조기 return 경로도 code를 실어야 한다. `browserd_state`는 `{alive, code, message}`를 내려보낸다
  (`src-tauri/src/main.rs:1253-1256`). **잔여 부채**: 같은 함수의 조기 return 2곳(`:1244`, `:1248`)은
  아직 bare `{alive:false}`다 — passive 경로(pane 복원·5s 재시도)에서 사유가 유실된다.

### 규칙 ② — 원인이 message **선두 120자 안**에 온다 (배너 절단 예산)

GUI 배너는 `castFailureReason`의 `maxLen`에서 잘린다. 기본 예산은 **120자**이며, cast 실패 경로만
200자로 상향돼 있다(`ui/src/webpane.ts`). 따라서 **사인은 문장 맨 앞에** 둔다.

배너 합성 형식(고정):

```
BROWSER_DISABLED_SAFE [<CODE>]: <message>
└─────────── 접두 오버헤드 = 23 + len(CODE) + 1 + 2 ───────────┘
```

- 접두 상수부 `BROWSER_DISABLED_SAFE [` = 23자, `]` = 1자, `: ` = 2자 → **오버헤드 = 26 + len(CODE)**.
- code가 26자면 오버헤드 52자 → message에 남는 예산 **68자**.
- message 조립 규칙: **`<SIGN> (exit n)`** — 산문 해설과 `cys-browserd: ` 접두는 **로그 전용**이며
  배너에서 제거한다(`normalize_sign` `runtime_launcher.rs:302`,
  `supervisor_death_message` `:330`).
- 자기유발 SIGKILL(우리가 격상시킨 signal 9)은 `(exit …)` 주석에서 억제한다
  (`exit_annotation` `:310`) — 우리가 죽인 사실은 사인이 아니다.
- 회귀 핀: `supervisor_sign_leads_the_message_and_fits_the_banner`
  (`runtime_launcher.rs:1881`, ≤120자 단언), `banner_code_survives_ui_truncation_budget`
  (`src-tauri/src/main.rs:5202`), `assert_banner_budget` 헬퍼(`mod.rs:2439` — 실제 생산된
  실패값으로 code ≤26 + 배너 ≤120 동시 단언).

> **왜 이 규칙이 생겼나**: 설계 시뮬레이션이 실제 배너를 합성해보니 192자가 나왔고, 120자 절단이
> 사인(`RUNTIME_ALREADY_RUNNING`)을 **통째로 삭제**했다. 사용자에게는 "실패했다"만 남고 "왜"가
> 사라졌다. 절단은 UI 사고가 아니라 진단 설계의 실패다.

### 규칙 ③ — 계약 문자열은 `include_str!` 트립와이어 테스트 쌍으로 고정한다

프로세스 경계를 넘는 문자열(브로커 code, 브로커 message 마커, 배너 형식)은 컴파일러가 검사하지
않는다. 한쪽에서 개명하면 **무증상으로** 분기가 죽는다. 그래서 소비자 쪽 크레이트가 생산자 소스를
`include_str!`로 임베드해 컴파일 시점에 대조한다.

**트립와이어는 반드시 쌍(양성 + 음성 대조군)으로 쓴다.** 음성 대조군이 없으면 `contains`가 공허
참이 돼도 아무도 모른다.

템플릿:

```rust
#[test]
fn gui_branch_codes_exist_in_broker_source() {
    let broker_src = include_str!("../../src/bin/cysd/authority_broker/mod.rs");
    // 양성: GUI가 분기하는 code가 브로커 소스에 실재하는가
    for code in [GUI_NOT_REGISTERED_CODE, GUI_IDENTITY_MISMATCH_CODE] {
        assert!(
            broker_src.contains(&format!("\"{code}\"")),
            "브로커 소스에 {code} 가 없음 — GUI 자가 회복 분기가 죽는다(코드 개명·삭제 의심)"
        );
    }
    // 음성 대조군: 실재하지 않는 code는 잡히면 안 된다(부분열 오탐 방지 포함)
    for absent in ["\"GUI_NOT_REGISTERED_V2\"", "\"GUI_IDENTITY_MISMATCHED\""] {
        assert!(!broker_src.contains(absent), "음성 대조군 {absent} 검출 — 판정이 무의미해짐");
    }
}
```

현재 배선된 트립와이어 4종(전부 `src-tauri/src/main.rs`):

| 테스트 | 고정하는 계약 | 음성 대조군 |
|---|---|---|
| `broker_registration_lost_marker_matches_broker_source` (`:5086`) | `"GUI peer has no broker-owned registration"` 원문 (자가 재등록 판별 마커) | 마커가 브로커 message의 부분열임을 역방향 단언 |
| `gui_branch_codes_exist_in_broker_source` (`:5104`) | `GUI_NOT_REGISTERED` / `GUI_IDENTITY_MISMATCH` code 실재 | `GUI_NOT_REGISTERED_V2`·`GUI_IDENTITY_MISMATCHED` 부재 |
| `banner_bracket_convention_matches_ui_fixture` (`:5124`) | `BROWSER_DISABLED_SAFE [<CODE>]: <msg>` 대괄호 규약 ↔ UI 픽스처 | code 없는 구형 `BROWSER_DISABLED_SAFE: ` 회귀 금지 |
| `banner_code_survives_ui_truncation_budget` (`:5202`) | 120자 절단 후에도 code가 생존 | — (경계값 단언) |

**불가침 문자열**: `"GUI peer has no broker-owned registration"`
(`src/bin/cysd/authority_broker/mod.rs:410`). 변경하려면 트립와이어와 GUI 마커 상수를 함께 바꿔야
하며, 그 전에 위 테스트가 막는다.

---

## 2. 코드 길이 상한 계약 — 26자

**신설·개명되는 모든 진단 code는 26자를 넘지 않는다.**

- 근거: 규칙 ②의 예산 산식. 26자를 넘으면 message 예산이 68자 밑으로 내려가 사유가 절단되기
  시작한다. `RUNTIME_SELF_OWNED_UNVALIDATED`(30자)는 실제로 배너 127자를 만들어 사유 후반
  `(재시도 시 재판정)`이 소실됐다 — 이것이 이 상한을 강제 규칙으로 승격시킨 실측 사고다.
- 상한은 **신설·개명 시점**에 적용한다. 기존 code(`RUNTIME_START_FAILED` 등)는 이미 상한 이하다.
- code 명명 규칙: `SCREAMING_SNAKE_CASE`, 도메인 접두(`SUPERVISOR_`·`RUNTIME_`·`GUI_`·`BROWSER_`),
  약어는 의미가 유지되는 선에서만(`READINESS`→`READY`, `EXITED_BEFORE_READY`→`EXIT_PRE_READY`).
- message는 **영문 간결형**을 기본으로 한다(리포 관행). 한국어 산문은 로그 전용이다.

**이 상한은 자동 강제된다** — 리뷰 관행이 아니라 테스트다.

| 게이트 | 위치 | 성격 |
|---|---|---|
| `every_error_code_literal_in_this_module_fits_the_26_char_cap` | `runtime_launcher.rs:2133`, `mod.rs:2459` | `include_str!` **소스 스캔 트립와이어** — 26자 초과 `SCREAMING_SNAKE_CASE` 리터럴을 모듈 전체에서 잡는다. 긴 code를 새로 넣으면 즉시 깨진다 |
| `assert_banner_budget(&failure)` 헬퍼 | `mod.rs:2439` | **실제 생산된 실패값**으로 code ≤26 + 배너 ≤120(문자 수 기준)을 동시 단언 |

> 소스 스캔 방식은 "새 code를 넣고 표에 등재하는 것을 잊는" 인간 실수를 막는다 — 등재를 잊어도
> 길이 위반은 컴파일 직후 터진다. 등재 자체는 여전히 §6 체크리스트의 사람 몫이다.

---

## 3. 자기유발 사인 필터 규정

**우리가 파이프를 닫아서 supervisor가 내는 소리는 사인이 아니다.**

브로커는 실패 경로에서 control 파이프를 drop해 supervisor에게 EOF를 유도한다(WS-2 종료기). 그러면
supervisor는 자기 stderr에 "broker liveness pipe closed" 류의 마지막 말을 남기고 죽는다. 이것을
사인으로 채집하면 **모든 실패의 원인이 "브로커가 파이프를 닫았다"로 수렴**한다 — 진단이 자기
꼬리를 무는 것이다.

계약 상수 (`runtime_launcher.rs:200-205`, `SELF_INFLICTED_SIGNS`):

```rust
const SELF_INFLICTED_SIGNS: [&str; 4] = [
    "broker liveness pipe closed",
    "broker liveness pipe failed",
    "unexpected command on startup-only broker channel",
    "AUTHORITY_REJECTED: broker liveness",
];
```

- 출처: `browser-runtime/supervisor/main.rs:64-66`. `SupervisorError`의 Display 형식은
  `{code}: {message}`이므로 접두형(`AUTHORITY_REJECTED: …`)도 함께 등재한다.
- 목록에 항목을 추가할 때는 **supervisor 소스의 해당 문자열을 실측 인용**하고, 그 라인이 정말
  브로커 유발인지(= 우리가 닫아서 생기는지) 확인한다. 진짜 실패 사인을 여기 넣으면 그 실패는
  영구히 침묵한다.
- 자기유발 SIGKILL도 같은 원리로 표시에서 제외한다(§1 규칙 ②).

---

## 4. filter-then-last 추출 규칙

사인 추출 순서는 **"필터를 통과하는 마지막 완결 라인"**이다. "마지막 완결 라인을 뽑은 뒤 필터"가
**아니다**. 순서가 뒤집히면 우리가 닫은 파이프의 자기유발 라인이 항상 마지막 줄이므로, 필터가
그것을 버리는 순간 **진짜 사인까지 함께 사라진다**(사인이 빈 상태로 수렴).

구현 계약 (`StderrTap`, `runtime_launcher.rs:209-293`):

| 단계 | 규칙 | 근거 |
|---|---|---|
| 링 용량 | **8 KiB** (`STDERR_TAP_CAPACITY` `:194`) | 다중 라인 보존 — 자기유발 라인 하나에 진짜 사인이 밀려나면 안 된다 |
| 완결 라인만 | 마지막 개행 **이후**의 조각은 아직 끝나지 않은 라인이므로 배제 (`last_sign` `:267`, `rfind('\n')` `:271`) | 반쪽 라인을 사인으로 보고하면 오독 유발 |
| 링 롤 시 첫 라인 폐기 | `rolled` 플래그가 서면 첫 라인은 앞이 잘렸을 수 있으므로 제거 (`:276-279`) | 잘린 앞머리는 사인이 아니다 |
| 필터 → last | `.filter(!is_self_inflicted_sign).map(normalize_sign).next_back()` (`:280-285`) | **filter-then-last** |
| EOF 유계 대기 | `sync_channel(1)` 신호 + `recv_timeout(200ms)` (`await_eof` `:251`, `STDERR_TAP_EOF_WAIT` `:197`) | 데이터는 커널 파이프에 있는데 tap 스레드가 아직 스케줄되지 않은 창 → 사인이 **확률적으로 빔**. 무한 join은 금지(ensure 경로의 broker Mutex 안에서 데드락) |
| 스레드 수명 | detach. supervisor stderr **EOF(= supervisor 사망)** 로 스스로 끝난다 | cysd측 drop이 아니라 supervisor측 종료가 인과 |
| `stderr` | `Stdio::piped()` (`:461`) — `Stdio::null()` 금지 | stderr를 버리면 실패 원인이 침묵 속에 소멸(진단 블랙홀) |
| `.env_remove("PATH")` | **불가침** (`:456`) | 보안 봉인. tap 배선이 이 줄을 건드리면 안 된다 |

회귀 핀: `self_inflicted_liveness_lines_are_not_mistaken_for_a_sign` (`:1942`),
`oversized_stderr_keeps_the_last_complete_line` (`:1929`),
`incomplete_trailing_line_is_not_reported_as_a_sign` (`:1957`),
`tap_eof_synchronisation_collects_the_sign_every_time` (100회 반복),
`a_slow_living_supervisor_never_blocks_the_sign_snapshot`.

---

## 5. 코드 레지스트리 (전수)

> **실측 기준**: 브랜치 `feat/browser-quality-overhaul`, 베이스 `c232a1e`(v0.13.20).
> 라인 번호는 편집으로 이동한다 — **정본 조회는 항상 `grep -rn '"<CODE>"' src/bin/cysd/` 로 한다.**
> 함수명은 라인보다 안정적이므로 함께 적었다.

### 5-1. 본 브랜치가 신설한 code

| # | code | 길이 | 의미 | 발생 조건 | 생산 지점 (함수 / file:line) | 사용자 조치 |
|---|---|---|---|---|---|---|
| 1 | `SUPERVISOR_EXIT_PRE_READY` | 25 | supervisor가 readiness 프레임을 내기 **전에** 죽었다. **가장 흔한 즉사 실패** | readiness 채널이 `(0, _)` 반환 = stdout EOF | `RealSupervisorLauncher::launch` / `runtime_launcher.rs:533` | message에 stderr 사인 + exit status가 실린다. 사인을 그대로 읽어라 — 그것이 근본 원인이다 |
| 2 | `SUPERVISOR_READY_OVERFLOW` | 25 | readiness 프레임이 상한을 넘었다 | 프레임 길이 > 64 KiB | `RealSupervisorLauncher::launch` / `runtime_launcher.rs:541` | 런타임 바이너리·매니페스트 손상 의심. `cys doctor` 후 재설치 |
| 3 | `SUPERVISOR_READY_TIMEOUT` | 24 | 25초 안에 readiness 프레임이 오지 않았다 | `wait_for_supervisor_readiness` 데드라인 초과 | `wait_for_supervisor_readiness` / `runtime_launcher.rs:887` | 엔진 부팅 지연(콜드 스타트·디스크 IO 포화) 또는 supervisor 행. `cysd.log`의 같은 시각 구간을 읽어라 |
| 4 | `RUNTIME_UNSUPPORTED` | 19 | 이 데몬은 브라우저 런타임을 **구조적으로 소유하지 않는다**(부서 데몬). 고장이 아니라 **정책** | 부서 소켓에서 런치 verb(`ensure`·`prepare_embed`·`operation`) 호출 | 게이트 `handlers.rs:992` / 브로커 `mod.rs:736` (사유 SOT = `DEPT_RUNTIME_UNSUPPORTED_REASON` `mod.rs:226`) | 정상 동작이다. 브라우저는 **본부 cysd**에서 열어라. 부서 데몬을 kill하지 말 것(부서 pane 전멸) |
| 5 | `RUNTIME_FOREIGN_OWNER` | 21 | 살아있는 정품 supervisor가 이 런타임을 소유 중으로 판정됐다 — **일시 상태**. 단일 데몬 환경에서는 회수 불가한 **자기 고아 런타임**일 수도 있으므로 message는 양쪽을 연다 | `UNAUTHENTICATED_STATE` fall-through에서 3중 신호(v2 파싱 && supervisor pid 생존 && 정규화 절대경로 완전일치) + `started_at ↔ start_time` **부호 있는 범위 `[-2s, +25s]`** 충족 | `ensure_shared` 소유권 분기 / `mod.rs:724` (판정 `classify_live_owner`) | 재시도하면 재판정한다. **kill 금지**(X2) — 자동 kill은 설계상 금지돼 있다 |
| 6 | `RUNTIME_SELF_UNVALIDATED` | 24 | 이 데몬이 방금까지 소유하던 런타임이 살아 있으나 재검증에 실패했다 — 두 번째 supervisor를 얹지 않는다 | 위 fall-through의 **자기소유 선검사**(`recently_owned` 일치 + pid 생존 + 동일 범위 대조 — pid 재사용 방어) | `ensure_shared` 소유권 분기 / `mod.rs:714` (판정 `classify_disk_state_owner`) | 재시도하면 재판정한다. 반복되면 supervisor·engine 프로세스를 확인(`ps aux \| grep -E "[c]ys-browserd\|[c]ys-browser-engine"`) |
| 7 | `BrokerStatus::Unsupported { reason }` (**code 아님 — 스키마 변형**) | — | 부서 데몬의 `probe` 응답. **오류가 아니라 정상 result로 사실 보고** → `{"status":"UNSUPPORTED","reason":…}` | 부서 소켓에서 `probe` 호출 | 정의 `mod.rs:215` / 생성 `dept_unsupported_status()` `mod.rs:230-234` / 응답 `handlers.rs:996-1008` | 진단 정보다. `#[serde(tag="status")]` 태그 계약 준수 — `{unsupported:true}` 같은 평면 키 금지 |

**개명 이력(추적용)** — 26자 상한(§2) 적용으로 본 브랜치 안에서 개명됐다. 옛 이름으로 검색하는
문서·로그가 있으면 이 표로 대응시켜라:

| 옛 이름 | 길이 | → 현재 이름 | 길이 |
|---|---|---|---|
| `SUPERVISOR_EXITED_BEFORE_READY` (설계 원안) | 30 | `SUPERVISOR_EXIT_PRE_READY` | 25 |
| `SUPERVISOR_READINESS_OVERFLOW` | 29 | `SUPERVISOR_READY_OVERFLOW` | 25 |
| `SUPERVISOR_READINESS_TIMEOUT` | 28 | `SUPERVISOR_READY_TIMEOUT` | 24 |
| `RUNTIME_SELF_OWNED_UNVALIDATED` | 30 | `RUNTIME_SELF_UNVALIDATED` | 24 |
| `ENGINE_ENDPOINT_UNAVAILABLE` | 27 | `ENGINE_ENDPOINT_FAILED` (`mod.rs:852`, `:928`, `runtime_launcher.rs:663`) | 22 |
| `DIRECT_USER_CONFIRMATION_REQUIRED` | 33 | `DIRECT_USER_CONFIRM_NEEDED` (`mod.rs:1344`) | 26 |

### 5-2. `RUNTIME_START_FAILED` 잔존 생산 지점 (전수 — 11곳)

`RUNTIME_START_FAILED`는 **폐기되지 않았다**. readiness 3분기만 세분됐을 뿐, 그 밖의 런치 실패는
전부 이 코드로 남는다. 이 코드를 받으면 **message가 유일한 판별 수단**이므로 전 지점을 등재한다.

| # | file:line | 함수 | message | 조건 |
|---|---|---|---|---|
| 1 | `runtime_launcher.rs:466` | `launch` | `supervisor control pipe unavailable` | `child.stdin.take()` 실패 |
| 2 | `runtime_launcher.rs:472` | `launch` | `supervisor readiness pipe unavailable` | `child.stdout.take()` 실패 |
| 3 | `runtime_launcher.rs:492` | `launch` | 사인이 있으면 `<SIGN> (exit n)`, 없으면 `private broker handshake failed: {error}` | 세션키·hello 라인 write/flush 실패 (핸드셰이크 grace 적용) |
| 4 | `runtime_launcher.rs:552` | `launch` | `supervisor process identity unavailable` | `process_start_time(supervisor_pid)` = None (identity 실패 — 자식은 반드시 회수) |
| 5 | `runtime_launcher.rs:562` | `launch` | `engine process identity unavailable` | `process_start_time(engine_pid)` = None |
| 6 | `runtime_launcher.rs:846` | `spawn_supervisor_cancellable` | `verified supervisor spawn failed: {error}` | `Command::spawn` 실패(ENOENT·권한·서명 검증 등) |
| 7 | `runtime_launcher.rs:896` | `wait_for_supervisor_readiness` | `supervisor readiness read failed: {error}` | readiness 채널이 IO 에러를 전달 |
| 8 | `runtime_launcher.rs:906` | `wait_for_supervisor_readiness` | `supervisor readiness channel disconnected` | 리더 스레드 소멸(채널 disconnect) |
| 9 | `runtime_launcher.rs:1171` | `random_session_key` | CSPRNG 오류 원문 | `random_token_hex()` 실패 |
| 10 | `runtime_launcher.rs:1175` | `random_session_key` | `CSPRNG returned malformed bytes` | hex 파싱 실패 |
| 11 | `mod.rs:761` | `ensure_shared` | `supervisor returned an incompatible live state: {status:?}` | launch 성공 후 상태가 `Compatible`이 아님 |

> 설계 §5-1은 7곳으로 예측했고 감사(`_round/AUDIT_IMPL_v0.13.21.md` §4-2)는 8곳으로 셌다.
> **실측 전수는 11곳**이다 — 9·10번(`random_session_key`)과 11번(`mod.rs`)이 두 계수 모두에서
> 누락됐다. 이 표가 정본이다.

### 5-3. 인접 code (본 브랜치 신설 아님 — 같은 배너를 공유하므로 참조용)

| code | 길이 | 생산 지점 | 성격 |
|---|---|---|---|
| `BROWSER_CANCELLED` | 17 | `runtime_launcher.rs` `ensure_launch_not_cancelled` / `wait_for_supervisor_readiness` | 취소(요청 연결 소멸 포함 — WS-0-3) |
| `PROTOCOL_MISMATCH` | 17 | `runtime_launcher.rs` (legacy state) / `mod.rs` (Incompatible) | 세대 불일치 |
| `LEGACY_ACTIVE` | 13 | `mod.rs` | 구 런타임 활성 — **자동 kill 금지** |
| `RUNTIME_DISABLED` / `RUNTIME_REVOKED` / `RUNTIME_INTEGRITY_FAILED` / `RUNTIME_NOT_FOUND` | ≤26 | `runtime_launcher.rs` `verify_release_metadata` 외 | 릴리스·정책·무결성 게이트 |
| `GUI_NOT_REGISTERED` / `GUI_IDENTITY_MISMATCH` | 18 / 21 | `mod.rs` GUI peer 대장 | GUI 자가 회복 분기의 대상 (트립와이어 고정) |
| `UNAUTHENTICATED_STATE` | 21 | `mod.rs` `status_for_v2` | fall-through 협착의 입구 (§5-1의 5·6번 분기가 여기서 갈린다) |
| `NATIVE_ACTIVATION_REQUIRED` / `USER_GESTURE_REQUIRED` / `RUNTIME_START_TIMEOUT` | ≤26 | `src-tauri/src/main.rs` | GUI측 자체 생산 배너 |

---

## 6. 신설 시 체크리스트 (code를 하나 추가할 때)

1. 이름이 **26자 이하**인가 (§2). `SCREAMING_SNAKE_CASE` + 도메인 접두인가.
2. 배너 합성 결과가 **120자 이하**인가 — `26 + len(CODE) + len(message) ≤ 120` (§1 규칙 ②).
   message는 영문 간결형인가(산문·한국어 해설은 로그로).
3. 사인이 **선두**에 오는가 — `<SIGN> (exit n)` 형식인가.
4. GUI가 이 code로 **분기**하는가? 그렇다면 `include_str!` 트립와이어를 **양성+음성 쌍**으로 추가했는가 (§1 규칙 ③).
5. supervisor stderr에서 오는 문자열이라면, 그것이 **자기유발**이 아닌지 확인했는가 (§3).
6. **이 문서 §5 표에 등재**했는가 — {의미, 발생 조건, file:line, 사용자 조치} 4열 전부.
7. `docs/browser-v2/REQUIREMENTS-LEDGER.md`의 진단 code 절에 반영했는가.

---

## 7. 알려진 잔여 부채 (릴리스 전 판단 대상)

이 계약이 아직 완전히 관철되지 않은 지점을 **정직하게** 남긴다. 감사
(`_round/AUDIT_IMPL_v0.13.21.md`) 실측 근거. 상태는 커밋 `5215463`(적대 검증 REVISE 반영) 기준.

| # | 부채 | 위치 | 상태 / 영향 |
|---|---|---|---|
| D1 | `browserd_state` 조기 return 2곳이 bare `{alive:false}` | `src-tauri/src/main.rs:1244`, `:1248` | **잔존** — passive 경로에서 사유 유실 (규칙 ① 미관철) |
| D2 | `DEPT_RUNTIME_UNSUPPORTED_REASON`이 **한국어 산문** | `mod.rs:226` | **잔존** — 배너 112자로 절단은 면했으나 리포 영문 관행과 불일치 (§2 message 규칙) |
| D3 | post-readiness 사망 사인 소비처가 `eprintln!`뿐 — 단언 없음 | `runtime_launcher.rs` 워처 / `validate_live`의 `ENGINE_EXITED` | **잔존** — 정상 운영에서 더 흔한 post-readiness 사망의 사인이 회귀 보호를 못 받는다 |
| D4 | `SUPERVISOR_READY_OVERFLOW`·`SUPERVISOR_READY_TIMEOUT` 분기 단위 테스트 부재 | — | **잔존** — 두 code의 생산 경로 자체를 태우는 테스트가 없다(길이·배너 예산은 `5215463`의 소스 스캔 트립와이어로 덮였으나, 그것은 **분기 실행 검증이 아니다**) |
| D5 | `RUNTIME_FOREIGN_OWNER`의 `started_at ↔ start_time ±2s` 신호가 프로덕션 의미론과 불일치 | `mod.rs` `classify_live_owner` | **해소(`5215463`)** — supervisor는 엔진 기동 완주 뒤 `started_at`을 찍으므로 실제 drift는 수 초~수십 초다. ±2s로는 정품도 상시 탈락해 협착이 늘 열려 있었다. **부호 있는 범위 `[-2s, +25s]`**로 교체(프로세스는 `started_at`보다 먼저 시작됐어야 하고, 그 간격은 브로커 readiness 예산 25s 안이어야 한다). 기각 대안: state 파일 mtime — 누구나 `touch`로 갱신 가능한 위조 표면이며 복사·백업·시계 되감기로 깨진다 |
| D6 | evict는 **종료 보장이 아니라 종료 요청**이다 | `evict_managed_runtime` / `terminate_evicted_runtime` | **한계로 박제됨(`5215463`)** — control 파이프만 닫고 끝나며 `try_wait`·SIGKILL 격상이 없다(`Child` 핸들을 워처 스레드가 단독 소유해 구조적으로 불가). EOF를 무시하는 자식은 evict 후에도 생존한다. 이 사실을 관측하는 테스트가 계약으로 존재한다 — **고치지 않고 정직하게 기록한 것**이다 |
