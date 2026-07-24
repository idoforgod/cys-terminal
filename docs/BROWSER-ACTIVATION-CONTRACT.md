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
(`src-tauri/src/main.rs:3529-3531`):

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

(근거: 소스 주석 `src-tauri/src/main.rs:181-184`, 회귀 테스트 주석
`src-tauri/src/main.rs:3780-3782` "선행 기본 스크립트(ipc-protocol.js)는 `})()` + 개행(식)으로
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
없다. 실물 생성 함수(`src-tauri/src/main.rs:188-211`)는 `;(() => {`로 시작한다:

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

---

## 2. 경계 계약 (불변식 — 수정자 필독)

### C1. 이 스크립트는 반드시 `;`로 시작한다

`browser_activation_initialization_script`의 반환 문자열은 **trim 이전 실제 첫 바이트가 `;`** 여야
한다. 공백 시작조차 불허(결정론 완성 조건). 근거·강제:

- 소스 계약: `src-tauri/src/main.rs:181` "invoke 초기화 스크립트에 push_str 연결되므로 **반드시
  `;`로 시작**해야 한다".
- 회귀 테스트 `activation_script_survives_ipc_protocol_concatenation`
  (`src-tauri/src/main.rs:3792-3801`): `assert_eq!(script.as_bytes().first(), Some(&b';'));`
- 술어 테스트 `initialization_script_keeps_capability_lexical_and_requires_trusted_activation`
  (`src-tauri/src/main.rs:3775`): `assert!(script.starts_with(';'), "push_str 연결 경계 — 파스
  분리자 필수");`

### C2. concatenation 회귀 테스트 2종을 삭제하지 마라

다음 두 테스트는 경계 계약의 집행기다. **삭제·약화 금지**:

1. **`activation_script_survives_ipc_protocol_concatenation`** (`src-tauri/src/main.rs:3792-3801`)
   — 활성화 스크립트가 삼킴 위험이 없는지(첫 바이트 `;`) 검사. 술어
   `concatenation_swallow_hazard`(`main.rs:3785-3790`)로 `( [ ` . + - * / , ?` 등 식-연속 시작
   토큰을 위험으로 판정한다.
2. **`concatenation_detector_catches_the_original_swallow_bug`** (`src-tauri/src/main.rs:3803-3815`)
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
`!contains("globalThis.browser")`로 강제한다(`src-tauri/src/main.rs:3772-3774`).

---

## 3. 표식 상태기계 (UI 진단 대응)

주입 스크립트가 `window.__CYS_BROWSER_ACTIVATION__`에 기록하는 표식과, `ui/src/webpane.ts`의
`castActivationDiagnostics(err, marker)`(`ui/src/webpane.ts:398-410`)가 각 상태를
`NATIVE_ACTIVATION_REQUIRED` 실패 문구로 보강하는 방식:

| 표식 상태 | 기록 지점 | 의미 | UI 진단 문구 (webpane.ts) |
|---|---|---|---|
| **(부재 / undefined·null)** | 표식이 한 번도 안 써짐 | 활성화 스크립트 **미설치**(삼킴 결함) — 원인 확정 | `활성화 스크립트 미설치(빌드 결함 — 재설치 필요) · {raw}` (`webpane.ts:401-402`) |
| **`installed`** | `mark('installed')` 주입 직후 (`main.rs:194`) | 인터셉터는 살아있으나 이 클릭이 미인터셉트 — 선택자 드리프트 의심 | `{raw} · (진단: 인터셉터 설치됨·이 클릭 미인터셉트 — 선택자 불일치 의심)` (`webpane.ts:405-406`) |
| **`armed-ok`** | arm 성공 `.then` (`main.rs:206`) | arm 성공 이력인데 소모 실패 — TTL 만료/이중 소모 의심 | `{raw} · (진단: arm 성공 이력 — TTL 만료/이중 소모 의심)` (`webpane.ts:407-408`) |
| **`arm-failed: <e>`** | arm 거부 `.catch` (`main.rs:207`) | arm 거부 — 원인 확정. 클릭은 재생됨 | `{marker.slice(0,80)} · {raw}` (`webpane.ts:404`, 80자 캡: 장문 사유가 백엔드 코드를 placeholder에서 축출 못 하게) |
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
