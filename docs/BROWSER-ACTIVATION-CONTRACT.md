# 내부 브라우저 활성화 계약 (BROWSER-ACTIVATION-CONTRACT)

> v0.13.7 근본수정의 정본 계약 문서. 내부 브라우저 활성화 인터셉터가 tauri 스크립트 연결에서
> 통째로 삼켜져 영구 미설치되던 결함(v0.13.6 실사고: `NATIVE_ACTIVATION_REQUIRED` 전 클릭 실패)을
> 봉쇄한다. 코드 인용은 모두 현재 워크트리(`feat/browser-activation-v0.13.7`) 실물에서 직접 읽어
> 파일:라인을 병기한다.

관련 파일:
- `src-tauri/src/main.rs` — 활성화 스크립트 생성·주입·회귀 테스트(코드 SOT)
- `ui/src/webpane.ts` — 표식 기반 진단 보강 순수함수(`castActivationDiagnostics`)
- `ui/e2e/browser_activation_gate/` — WebKit 실구동 수동 게이트(sim_e2e·concat_parse_probe)

---

## 1. 결함 해부 — 연결·ASI·삼킴 메커니즘

### 1.1 주입 경로

활성화 스크립트는 tauri `Builder::append_invoke_initialization_script`로 주입된다
(`src-tauri/src/main.rs:3541-3543`):

```rust
let activation_script = browser_activation_initialization_script(&bootstrap_capability);
tauri::Builder::default()
    .append_invoke_initialization_script(activation_script)
```

이 API는 **문자열 `push_str` 연결**이다 — 뒤 스크립트를 선행 invoke 초기화 스크립트 뒤에
그대로 이어 붙인다. 파서·세미콜론 삽입을 대신 해주지 않는다.

### 1.2 선행 스크립트의 말미 바이트 (실측)

선행 기본 스크립트(tauri 내부 `ipc-protocol.js`, 2.11.2 실측)는 **식(expression)** 으로 끝난다 —
즉시실행함수 `(function(){…})()` 뒤에 개행. 실측 말미 5바이트:

```
7d 29 28 29 0a   →   }  )  (  )  \n   →   "})()\n"
```

(근거: 소스 주석 `src-tauri/src/main.rs:181-193`, 회귀 테스트 주석
`src-tauri/src/main.rs:3794-3796` "선행 기본 스크립트(ipc-protocol.js)는 `})()` + 개행(식)으로
끝난다(2.11.2 실측)". 바이트 `7d 29 28 29` = `})()` 는 이 문서 작성 시 `printf '%s' '})()' | xxd`로
재확인.)

### 1.3 ASI 미발동 → 호출 연쇄 삼킴

JavaScript 자동 세미콜론 삽입(ASI)은 **다음 토큰이 현재 식의 계속으로 파싱 가능하면 발동하지
않는다**. 선행 스크립트가 `})()`(값을 내는 식)로 끝나고 개행 뒤 뒤 스크립트가 `(`로 시작하면:

```js
})()          // 선행 ipc-protocol.js 말미(식)
(() => {…})()  // 뒤 활성화 스크립트가 IIFE로 시작하면…
```

파서는 이를 `})()(() => {…})()` — 즉 **앞 식의 반환값을 함수로 호출하는 연쇄**로 읽는다. 개행이
있어도 ASI가 끊어주지 않는다. 앞 식의 반환값은 함수가 아니므로 런타임에 `TypeError`가 나거나,
설령 그렇지 않더라도 **뒤 활성화 IIFE의 본문이 독립 실행되지 못하고 앞 식의 인자로 통째로
삼켜진다**. 결과: 클릭 인터셉터가 **영구 미설치**되고, 사용자는 브라우저 버튼을 눌러도 무반응이며
백엔드는 `NATIVE_ACTIVATION_REQUIRED`로 거부한다(v0.13.6 실사고).

### 1.4 수정 — `;`-시작 파스 분리

뒤 스크립트의 첫 바이트를 `;`(빈 문장 분리자)로 두면 연결 지점이
`})()\n;(() => {…})()` 가 되어 앞 식이 `;`에서 결정론적으로 종료된다. 삼킴 연쇄가 성립할 수
없다. 실물 생성 함수(`src-tauri/src/main.rs:194-223`)는 `;(() => {`로 시작한다:

```rust
fn browser_activation_initialization_script(capability: &str) -> String {
    format!(
        r##";(() => {{
  'use strict';
  const capability = '{capability}';
  const mark = (s) => {{ try {{ window.__CYS_BROWSER_ACTIVATION__ = s; }} catch (_) {{}} }};
  mark('installed');
  …
```

수정 요지(3층): ① `;` 시작 파스 분리 ② 진단 표식 `__CYS_BROWSER_ACTIVATION__`
(installed / armed-ok / arm-failed:) ③ arm 실패에도 클릭 재생(무반응 버튼 대신 백엔드 거부 사유가
UI로 노출).

### 1.5 v0.13.7 후일담 — 삼킴 해소가 노출시킨 2차 잠복 결함

삼킴을 `;`로 해소하자 스크립트는 비로소 **실행**됐지만, 이번엔 그 실행 첫머리에서 죽는 2차
결함이 드러났다. 활성화 스크립트는 tauri 2.11.2 실측 순서상 **invoke init(ipc-protocol) 단계**에서
실행되는데, `window.__TAURI_INTERNALS__.invoke`는 그보다 **뒤인 core.js에서야**
`Object.defineProperty`로 정의된다. 그런데 초기 수정본은 설치 시점(리스너 등록 전)에
`const invoke = window.__TAURI_INTERNALS__.invoke.bind(window.__TAURI_INTERNALS__);`를 실행했다 —
이 시점 `.invoke`는 `undefined`이므로 `undefined.bind`가 **TypeError**로 죽고, `mark('installed')`는
이미 찍힌 뒤라 표식은 `installed`에 정지, **클릭 리스너가 영영 등록되지 않는다**. 라이브 증상:
재연결 클릭 → `NATIVE_ACTIVATION_REQUIRED · (진단: 인터셉터 설치됨·이 클릭 미인터셉트)`.

근본 교정: **설치 시점 코드는 `__TAURI_INTERNALS__`를 일절 접촉하지 않고**, invoke 조회를 클릭
핸들러 안에서 **지연 수행**한다(`main.rs:212-219`). arm 불가(핸들러 시점에도 invoke 부재) 시에도
`arm-failed:`를 찍고 클릭을 재생해 기존 계약을 유지한다. 이 불변식은 §2 **C5**가 집행한다.

---

## 2. 경계 계약 (불변식 — 수정자 필독)

### C1. 이 스크립트는 반드시 `;`로 시작한다

`browser_activation_initialization_script`의 반환 문자열은 **trim 이전 실제 첫 바이트가 `;`** 여야
한다. 공백 시작조차 불허(결정론 완성 조건). 근거·강제:

- 소스 계약: `src-tauri/src/main.rs:181` "invoke 초기화 스크립트에 push_str 연결되므로 **반드시
  `;`로 시작**해야 한다".
- 회귀 테스트 `activation_script_survives_ipc_protocol_concatenation`
  (`src-tauri/src/main.rs:3807-3815`): `assert_eq!(script.as_bytes().first(), Some(&b';'));`
- 술어 테스트 `initialization_script_keeps_capability_lexical_and_requires_trusted_activation`
  (`src-tauri/src/main.rs:3789`): `assert!(script.starts_with(';'), "push_str 연결 경계 — 파스
  분리자 필수");`

### C2. concatenation 회귀 테스트 2종을 삭제하지 마라

다음 두 테스트는 경계 계약의 집행기다. **삭제·약화 금지**:

1. **`activation_script_survives_ipc_protocol_concatenation`** (`src-tauri/src/main.rs:3807-3815`)
   — 활성화 스크립트가 삼킴 위험이 없는지(첫 바이트 `;`) 검사. 술어
   `concatenation_swallow_hazard`(`main.rs:3799-3804`)로 `( [ ` . + - * / , ?` 등 식-연속 시작
   토큰을 위험으로 판정한다.
2. **`concatenation_detector_catches_the_original_swallow_bug`** (`src-tauri/src/main.rs:3818-3829`)
   — **음성 대조군(계측 타당성 게이트)**. 현행 스크립트에서 선두 `;`를 벗겨낸 v0.13.6 실사고 형태를
   탐지기가 실제로 잡는지 스스로 증명한다. 이게 없으면 탐지기 자체가 무력화돼도 알 수 없다.

계측기(탐지기)만 있고 음성 대조군이 없으면 "탐지기가 아무것도 안 잡아도 초록"인 위양성에 빠진다.
두 테스트는 쌍으로만 유효하다 — 하나라도 지우면 계약이 무너진다.

### C3. 표식은 권위가 아니다

`__CYS_BROWSER_ACTIVATION__` 표식은 **진단 전용**이며 클릭 차단·권한 부여를 하지 않는다. 활성화
판정자는 항상 백엔드 `consume`이다(`src-tauri/src/main.rs:185-187`). UI는 표식을 게이트가 아니라
실패 문구의 **재료**로만 쓴다(§3).

### C4. capability는 클로저 격리

주입 스크립트의 `capability`는 IIFE 클로저 안에만 있어야 한다(`window`/`globalThis` 유출 금지).
테스트가 `script.matches(&capability).count() == 1` 및 `!contains("window.browser")` /
`!contains("globalThis.browser")`로 강제한다(`src-tauri/src/main.rs:3786-3787`).

### C5. 설치 시점 `__TAURI_INTERNALS__` 접촉 금지 (invoke는 클릭 시점 지연 조회)

활성화 스크립트는 **core.js의 invoke 정의 이전에 실행된다**. tauri 2.11.2
`prepare_pending_webview` 초기화 순서(실측):

1. `isTauri`
2. `__TAURI_INTERNALS__ = { plugins: {} }` ← 이 시점엔 `.invoke` 없음
3. 메타(convertFileSrc 등)
4. **invoke init script = ipc-protocol + 본 활성화 스크립트** ← 활성화 스크립트가 여기서 실행
5. pattern
6. ipc
7. **core.js ← `Object.defineProperty(__TAURI_INTERNALS__, 'invoke', …)` = invoke 정의 지점**
8. …(plugin init 등)

따라서 설치 시점(리스너 등록 전)에 `window.__TAURI_INTERNALS__.invoke.bind(...)`처럼
`__TAURI_INTERNALS__`를 접촉하면 `.invoke`가 `undefined`이라 `undefined.bind` **TypeError**로 죽고,
`mark('installed')`는 이미 찍힌 뒤라 **클릭 리스너가 미등록**된다(v0.13.7 실사고). 불변식: 설치
시점 코드는 `__TAURI_INTERNALS__`를 일절 접촉하지 않고(`mark('installed')`→`nativeClick`
캡처→`addEventListener`만), invoke 조회는 **클릭 핸들러 안에서 지연 수행**한다
(`src-tauri/src/main.rs:212-219`). 집행 테스트 2종:

1. **`activation_script_defers_invoke_lookup_past_install_time`** (`src-tauri/src/main.rs:3846-3853`)
   — 술어 `install_time_internals_capture`(`main.rs:3837-3843`: `addEventListener` 첫 등장 이전
   구간에 `__TAURI_INTERNALS__`가 있으면 위험)로 현행 스크립트가 안전(false)인지 검사.
2. **`install_time_capture_detector_catches_the_v0137_bug`** (`src-tauri/src/main.rs:3856-3872`)
   — **음성 대조군**. `mark('installed');` 직후에 v0.13.7 실사고 라인
   (`const invoke = window.__TAURI_INTERNALS__.invoke.bind(...)`)을 합성 삽입한 형태를 탐지기가
   실제로 잡는지 증명한다. C2의 삼킴 탐지기 쌍과 같은 철학(계측 타당성 게이트)이다.

### C6. GUI peer 등록·검증 계약 (cysd codesign + Tauri 등록·TTL)

v0.13.8 실기: C5(브라우저 활성화 게이트)는 통과했으나 그 뒤 단계에서 4건이 연쇄 노출됐다(전부
이 머신 실기 재현). 계약과 근거:

**① codesign requirement 문법 (치명·전 맥 영구 실패였음)** — `authority_broker/mod.rs`의
`verify_gui_code_identity`는 GUI peer의 코드 정체성을 codesign `-R=`로 검증한다. `-R=`는 **단일
requirement** 문법만 받는다. v0.13.8은 requirement-set 접두(`designated => `)를 붙여
`format!("-R{req}")`로 넘겨, codesign이 `Requirement syntax error: unexpected token: designated`로
**모든 맥에서 항상 파스 실패**했다(실측). 결과: "GUI designated code requirement did not verify" →
GUI peer 등록 영구 실패 → ensure가 "Browser GUI peer registration is unavailable". **계약**:
requirement는 접두 없는 순수 형태(`const GUI_CODE_REQUIREMENT`)로 정의하고 `-R=`를 붙인다.

**② 번들 자기오염 vs strict 리소스 seal** — 앱은 첫 구동에서 번들 내
`Resources/runtime/python/**/__pycache__/*.pyc` 수백 개를 자가 생성해 리소스 seal을 스스로
깨뜨린다(실측: file added 170여 + modified 3). 문법을 고쳐도 기본 검증(리소스 워크 포함)은 설치 후
첫 사용부터 실패한다. **계약**: `--ignore-resources`를 추가한다 — Mach-O 코드·인증서 체인·identifier
검증은 유지되고, 앱이 스스로 깨는 리소스 seal만 제외한다(오염 번들·실행 중 pid 모두
`satisfies its Designated Requirement` 통과 실측).

**③ 대상 = pid(실행 중 peer)** — 검증 대상을 경로가 아니라 **pid 문자열**로 넘겨 살아있는 peer
프로세스 자체를 검증한다(경로 검증 후 바이너리 교체 TOCTOU 창 축소). 정규 경로 게이트
(/Applications 고정)는 별도로 유지한다.

**④ 등록 fail-closed-forever → 지연 재시도 1회** — 부팅 시 1회 `register_browser_gui_peer()`가
데몬 교체·부팅 경합으로 실패하면 재시도가 없어 영구 불능이었다(`main.rs`). **계약**: 등록은
**부팅 1회 + ensure 시 지연 재시도 1회**다 — `ensure_browserd_cast`에서 신뢰된 클릭 소모 직후
`BROWSER_APP_SESSION`이 비어 있으면 1회만 재시도하고, 실패하면 그 에러를 `BROWSER_DISABLED_SAFE`로
반환한다(fail-closed 의미 유지·무한루프 없음·OnceLock 이중 set 무해).

**⑤ 네이티브 활성화 TTL** — arm→consume 유효시간이 2s였으나, 부팅 직후 부하에서 첫 클릭의
arm→consume이 2s를 초과해 "trusted native Browser activation expired"를 유발했다(실측; 두 번째
클릭은 통과). **계약**: `NATIVE_ACTIVATION_TTL_MS = 10_000`. 여전히 one-time·window-bound 단일
소모라 보안 성질(재생 불가)은 불변이다.

**⑥ 등록은 갱신 가능 자원 — 데몬 재시작 시 대장 소실 → 낡은 세션 거부 시 자가 회복 (v0.13.10
실사고)** — 브로커(cysd) 등록 대장(`gui_peers.sessions`)은 **데몬 메모리**에 있어 데몬 재시작·업데이트
후 재가동 시 소실된다. 그런데 GUI는 부팅 시 1회 등록한 세션을 계속 제시하므로, 데몬만 재시작되면
브로커가 `issue_gesture`를 `GUI peer has no broker-owned registration`(코드 `GUI_NOT_REGISTERED`)으로
거부해 "BROWSER_DISABLED_SAFE"가 **간헐**(GUI 재시작하면 해소 → 됐다안됐다) 발생한다. ④의 지연
재시도는 세션이 비어 있을 때만 발동하므로 낡은 세션이 남아있는 이 경우엔 발동하지 못한다.
**계약**: 세션 저장소는 재설정 가능한 `RwLock<Option<String>>`이며(OnceLock 회귀 금지 — 재설정 불가로
v0.13.10 실사고 재발), `ensure_browserd_cast`는 issue_gesture가 등록 소실성 거부(`main.rs`
`BROKER_REGISTRATION_LOST_MARKER = "no broker-owned registration"` 부분열 매칭)로 실패하면 세션을
비우고 `register_browser_gui_peer()`를 **1회 재실행**한 뒤 issue_gesture를 **1회만 재시도**한다. 재시도도
실패하면 그 에러로 fail-closed(**무한루프 금지** — 재등록·재시도 각 1회 한정). one-time activation은
함수 초입에서 이미 consume된 뒤이므로 재시도 경로는 rpc만 다시 태우며 **새 클릭이 필요 없다**.

**집행 테스트** (`authority_broker/mod.rs` `#[cfg(all(test, target_os = "macos"))]`):
1. **양성** `gui_code_requirement_compiles` — `GUI_CODE_REQUIREMENT`(순수 형태)가 `csreq -r -t`로
   컴파일(exit 0)되고 codesign `-R=` 경로에서도 파스 에러가 아님을 확인.
2. **음성 대조군** `designated_prefix_requirement_is_a_codesign_parse_error` — `designated =>` 접두
   형태가 codesign `-R=`에서 **파스 에러**(`unexpected token: designated`)가 남을 증명(탐지 타당성
   게이트). ※ `csreq`는 두 형태를 모두 받아들여 결함을 숨기므로(실측), 음성 대조군은 반드시
   codesign 기반이어야 한다. TTL은 `native_browser_activation_is_window_bound_ttl_and_single_consume`
   (`main.rs`)가 상수 기준으로 만료 경계를 집행한다.
3. **판별 문자열 정합**(⑥) `broker_registration_lost_marker_matches_broker_source`(`main.rs`) —
   `include_str!`로 브로커 소스를 읽어 GUI 판별 부분열 `BROKER_REGISTRATION_LOST_MARKER`가 브로커
   메시지 원문(`GUI peer has no broker-owned registration`)의 부분열임을 컴파일 시점 임베드로 검증한다.
   브로커 메시지가 바뀌면 이 테스트가 깨져 자가 회복 미발동을 조기에 잡는다.

---

## 3. 표식 상태기계 (UI 진단 대응)

주입 스크립트가 `window.__CYS_BROWSER_ACTIVATION__`에 기록하는 표식과, `ui/src/webpane.ts`의
`castActivationDiagnostics(err, marker)`(`ui/src/webpane.ts:398-410`)가 각 상태를
`NATIVE_ACTIVATION_REQUIRED` 실패 문구로 보강하는 방식:

| 표식 상태 | 기록 지점 | 의미 | UI 진단 문구 (webpane.ts) |
|---|---|---|---|
| **(부재 / undefined·null)** | 표식이 한 번도 안 써짐 | 활성화 스크립트 **미설치**(삼킴 결함) — 원인 확정 | `활성화 스크립트 미설치(빌드 결함 — 재설치 필요) · {raw}` (`webpane.ts:401-402`) |
| **`installed`** | `mark('installed')` 주입 직후 (`main.rs:200`) | 인터셉터는 살아있으나 이 클릭이 미인터셉트 — 선택자 드리프트 **또는 설치 직후 코드가 죽어 리스너 미등록(C5 위반)** 의심 | `{raw} · (진단: 인터셉터 설치됨·이 클릭 미인터셉트 — 선택자 불일치 또는 설치 직후 크래시 의심)` (`webpane.ts:405-406`) |
| **`armed-ok`** | arm 성공 `.then` (`main.rs:218`) | arm 성공 이력인데 소모 실패 — TTL 만료/이중 소모 의심 | `{raw} · (진단: arm 성공 이력 — TTL 만료/이중 소모 의심)` (`webpane.ts:407-408`) |
| **`arm-failed: <e>`** | arm 거부 `.catch` (`main.rs:219`), 또는 클릭 시점 invoke 부재(C5) `main.rs:214` | arm 거부 — 원인 확정. 클릭은 재생됨 | `{marker.slice(0,80)} · {raw}` (`webpane.ts:404`, 80자 캡: 장문 사유가 백엔드 코드를 placeholder에서 축출 못 하게) |
| **`skip-*`** (skip-untrusted·skip-inactive-ua·skip-not-element·skip-nomatch:`<id/tag>`) | 클릭 핸들러 각 조기 return (`main.rs`) | **진단 전용·권위 없음**. 인터셉터는 살아있으나 클릭이 특정 가드에서 탈락 — `installed`(미인터셉트 통칭)를 탈락 지점별로 세분한다. skip-untrusted는 arm 성공 후 nativeClick 재생(합성 클릭)에서도 발생하므로 armed 접두면 기록을 보류해 armed-ok를 최종 상태로 보존 | `{marker.slice(0,80)} · {raw}` (`webpane.ts`, installed 분기보다 **앞**) |
| **(미지 값 / 위조)** | — | 무보강 — 오진 금지, 백엔드 원문 유지 | `{raw}` (`webpane.ts:409`) |

상태 흐름: `installed`(주입 시) → 신뢰 클릭 시 arm 시도 → 성공이면 `armed-ok`(그 후 nativeClick
재생), 실패면 `arm-failed:<사유>`(그래도 nativeClick 재생). arm 실패에도 클릭을 재생하는 이유는
무반응 버튼 대신 백엔드의 정확한 거부 사유를 `ensure_browserd_cast` 실패 경로로 흘려 UI에
노출하기 위함이다(`main.rs:186-187`).

`castActivationDiagnostics`의 분기 판단 근거는 `ui/src/webpane.ts:391-397` 주석에 명시되어 있다:
원인 확정 상태(부재·arm-failed)는 진단을 **선행**하고, 정황 상태(installed·armed-ok)는 백엔드
원문 뒤에 힌트를 **후행**하며, 미지 값(위조 포함)은 무보강한다.

---

## 4. 검증 사다리

이 결함은 문자열 층·논리 층·실구동 층에서 각각 다른 도구로 봉쇄한다. 아래에서 위로 갈수록
실기에 가깝고 비용이 크다.

1. **cargo 술어 테스트** (`src-tauri/src/main.rs`) — 결정론·무의존. `cargo test`로 실행.
   - `initialization_script_keeps_capability_lexical_and_requires_trusted_activation`
     (`:3763-3778`): 첫 바이트 `;`, capability 클로저 격리, 신뢰 활성화 술어, 표식 문자열 존재.
   - `activation_script_survives_ipc_protocol_concatenation` (`:3792-3801`): 삼킴 위험 부재.
   - `concatenation_detector_catches_the_original_swallow_bug` (`:3803-3815`): 음성 대조군.

2. **bun 순수함수 분기 테스트** — `ui/src/webpane.ts`의 `castActivationDiagnostics`·
   `castFailureReason` 분기를 `NATIVE_ACTIVATION_REQUIRED` 유무 × 표식 5상태(부재/installed/
   armed-ok/arm-failed/미지)로 교차 검증(7분기). 표식→문구 매핑이 §3 표와 일치하는지 확인.

3. **WKWebView 하네스** (`ui/e2e/browser_activation_gate/`, 수동) — 활성화 스크립트를 진짜
   WKWebView에 주입해 arm→armed-ok→ensure 연쇄와 arm 실패 진단이 실기 WebKit에서 도는지 확인.
   - `sim_e2e.swift` 시나리오 A(정상)/B(arm 실패)/C(연타 dup).
   - `concat_parse_probe.swift` 삼킴 결함 실증(음성 대조군: 수정 전 형태 재현).
   - 실행법·기대 출력은 `ui/e2e/browser_activation_gate/README.md` 참조.

4. **실기 E2E** — 실제 앱 빌드에서 브라우저 버튼 클릭 → 내부 브라우저 활성화까지의 사용자 경로.
   신선 머신(bun 미설치 등)·arm 실패·연타를 사람이 직접 재현해 §3 진단 문구가 실제로 뜨는지 확인.

각 단은 앞 단이 못 잡는 층을 덮는다: cargo=문자열 경계, bun=진단 로직, WKWebView=주입·연결 실행,
실기=전 경로 통합. 상위 단만으로 하위 단을 대체하지 마라(비용이 커 회귀를 늦게 잡는다).
