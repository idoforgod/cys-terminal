# browser_activation_gate — 내부 브라우저 활성화 WebKit 실구동 게이트 (수동 실행 — CI 미배선)

기존 `ui/e2e/*_gate.py`(wsbar_gate·cc_resize_gate·office_*_gate) 관례와 동렬의 **수동 게이트**다.
CI에 배선하지 않고, 활성화 배관을 손댈 때 사람이 직접 돌려 회귀를 잡는다. 파이썬 대신 Swift인
이유는 검증 대상이 브라우저 UI가 아니라 **WKWebView 실인스턴스 안에서의 스크립트 주입·연결·실행
연쇄**이기 때문이다 — 실제 WebKit 없이는 재현되지 않는다.

## 목적

v0.13.7이 고친 결함(내부 브라우저 활성화 인터셉터가 통째로 삼켜져 영구 미설치 → 클릭 무반응,
`NATIVE_ACTIVATION_REQUIRED` 전 클릭 실패)의 **실구동 회귀**를 잡는다. cargo 술어 테스트
(`concatenation_*`)와 bun 순수함수 분기 테스트가 문자열·논리 층을 봉쇄하는 반면, 이 게이트는
동일한 활성화 스크립트를 **진짜 WKWebView에 주입해** arm→armed-ok→ensure 연쇄와 arm 실패 시
진단 보강이 실기 WebKit에서 실제로 도는지를 확인한다(검증 사다리 3단 = WKWebView 하네스).

계약·결함 해부·표식 상태기계는 상위 문서 `docs/BROWSER-ACTIVATION-CONTRACT.md`가 정본이다.

## 파일

- **`sim_e2e.swift`** — 활성화 전 경로(설계 확정 D1 주입 스크립트 + D2 진단 순수함수)를 동일
  WebKit에서 구동하는 3시나리오 하네스. tauri `append_invoke_initialization_script`의 `push_str`
  연결(선행 ipc-protocol.js 동형 말미 `})()\n` + 뒤 `;`-시작 활성화 스크립트)을 그대로 재현한다.
  - **A = 정상**: arm 성공 → `armed-ok` 표식 → `ensure_browserd_cast` 성공.
  - **B = arm 실패**: arm 거부 → `arm-failed:<사유>` 표식 → 클릭 재생 → enriched 진단 노출.
  - **C = 연타**: 두 번째 클릭은 dup 판정(focus-only) → 이중 `ensure` 금지.
- **`concat_parse_probe.swift`** — 결함 자체의 실증 프로브. tauri `push_str` 연결에서 뒤 스크립트가
  IIFE(`(() =>`)로 시작하면 선행 `})()\n`과 호출 연쇄로 묶여 **통째로 삼켜지는** 현상을 진짜 WebKit
  에서 재현한다(수정 전 형태 = 음성 대조군). 두 스크립트를 이어 붙였을 때 뒤 IIFE가 실행되지 않아
  `window.__X__`가 정의되지 못함을 `evaluateJavaScript`로 확인한다.

## 실행법

```bash
cd ui/e2e/browser_activation_gate
xcrun swiftc -O sim_e2e.swift -o sim_e2e && ./sim_e2e A   # 시나리오 A/B/C 중 하나
xcrun swiftc -O concat_parse_probe.swift -o concat_parse_probe && ./concat_parse_probe
```

macOS·Xcode 커맨드라인 툴 필요(WebKit·Cocoa 링크). 인자 없이 `./sim_e2e` 실행 시 기본 `A`.

## 기대 출력

- **A (정상)**: `interceptor-marker=installed` → 클릭 → `invoke:arm_browser_native_activation` →
  표식 `armed-ok` → `invoke:ensure_browserd_cast` → `ensure OK (calls=1)`.
- **B (arm 실패)**: 표식 `arm-failed: ...`(백엔드 거부 사유) → **클릭 재생됨**(무반응 버튼 아님) →
  `ensure FAIL enriched=arm-failed: ... · ...NATIVE_ACTIVATION_REQUIRED...`(진단이 표식으로 보강됨).
- **C (연타)**: 첫 클릭 정상 처리 후 둘째 클릭에서 `openCastPane: dup -> focus-only` → `ensure`
  재호출 없음(`ensureCalls`가 늘지 않음).
- **concat_parse_probe**: 결함 재현 프로브는 `EVAL: __X__ typeof = undefined`를 출력한다 — 뒤 IIFE가
  선행 식에 삼켜져 실행되지 못함을 뜻한다. 이 형태가 v0.13.6 실사고이며, `;`-시작 분리자가 이를 없앤다.

## 주의 — 스크립트 수정 시 실물에서 재추출

`sim_e2e.swift`의 주입 스크립트(변수 `d1`)와 진단 함수(`castActivationDiagnostics`)는 실물에서
**손으로 옮겨온 사본**이다. 실물(`src-tauri/src/main.rs`의
`browser_activation_initialization_script`, `ui/src/webpane.ts`의 `castActivationDiagnostics`)이
바뀌면 이 하네스도 실물에서 **재추출**해 동기화하라. 재추출 시 치환 규칙:

- Rust `format!` 이스케이프 `{{` `}}` → JS 리터럴 `{` `}`.
- `{capability}` 플레이스홀더 → 64자리 hex(하네스는 `String(repeating: "c", count: 64)`로 대입).

동기화가 깨지면 이 게이트는 실물이 아니라 낡은 사본을 검증하게 되어 회귀를 놓친다.
