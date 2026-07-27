// castActivationDiagnostics 순수 함수 회귀 테스트 (bun test — 신규 의존성 0).
//
// NATIVE_ACTIVATION_REQUIRED 실패의 진단 보강 — 표식(window.__CYS_BROWSER_ACTIVATION__)은
// 게이트가 아니라 문구 재료다. 판정자는 항상 백엔드 consume. 8분기 전수 검증(표식 상태별 진단 +
// 비활성화 무간섭 + 미지 표식 무보강 + 빈 입력 + skip-* 가드 탈락).
import { describe, it, expect } from "bun:test";
import { castActivationDiagnostics } from "./webpane";

const NATIVE =
  "BROWSER_DISABLED_SAFE [NATIVE_ACTIVATION_REQUIRED]: trusted native Browser activation is required";

describe("castActivationDiagnostics — 표식 기반 진단 보강", () => {
  it("1) 비활성화 에러는 무간섭 통과 — marker 있어도 원문 그대로", () => {
    const err = "bun 미설치 — https://bun.sh";
    expect(castActivationDiagnostics(err, "installed")).toBe(err);
  });

  it("2) 표식 부재(undefined) + NATIVE → '활성화 스크립트 미설치' 접두", () => {
    expect(castActivationDiagnostics(NATIVE, undefined)).toBe(
      `활성화 스크립트 미설치(빌드 결함 — 재설치 필요) · ${NATIVE}`,
    );
  });

  it("3) 'arm-failed: …' → 접두 + 80자 캡(150자 사유는 marker부 80자로 절단)", () => {
    const marker = "arm-failed: " + "x".repeat(138); // 총 150자
    expect(marker.length).toBe(150);
    const capped = marker.slice(0, 80);
    expect(capped.length).toBe(80); // 캡이 실제 80자로 잘림을 명시
    expect(castActivationDiagnostics(NATIVE, marker)).toBe(`${capped} · ${NATIVE}`);
  });

  it("4) 'installed' → 원문 + '선택자 불일치 또는 설치 직후 크래시 의심' 힌트 후행", () => {
    expect(castActivationDiagnostics(NATIVE, "installed")).toBe(
      `${NATIVE} · (진단: 인터셉터 설치됨·이 클릭 미인터셉트 — 선택자 불일치 또는 설치 직후 크래시 의심)`,
    );
  });

  it("5) 'armed-ok' → 원문 + '직전 활성화가 이미 사용됨' 힌트 후행", () => {
    expect(castActivationDiagnostics(NATIVE, "armed-ok")).toBe(
      `${NATIVE} · (진단: 직전 활성화가 이미 사용됨 — 한 번 더 클릭하면 연결됩니다)`,
    );
  });

  it("6) 미지 표식('hacked-by-page') → 무보강 원문(위조 오진 금지)", () => {
    expect(castActivationDiagnostics(NATIVE, "hacked-by-page")).toBe(NATIVE);
  });

  it("7) 빈 문자열·null 에러 → '' (빈 결과)", () => {
    expect(castActivationDiagnostics("", "installed")).toBe("");
    expect(castActivationDiagnostics(null, undefined)).toBe("");
  });

  it("8) 'skip-*'(가드 탈락) → 접두 + 80자 캡(installed보다 앞서 세분 진단)", () => {
    const marker = "skip-nomatch:btn-close";
    expect(castActivationDiagnostics(NATIVE, marker)).toBe(`${marker} · ${NATIVE}`);
  });

  // ★WS-1 매핑 갱신: 백엔드가 데몬 error.code를 대괄호 규약으로 실어 올린 뒤에도, 활성화 진단은
  // NATIVE_ACTIVATION_REQUIRED **한 code에만** 반응해야 한다. 다른 사인에 활성화 힌트를 덧대면
  // "한 번 더 클릭하면 연결됩니다" 같은 오진이 진짜 원인(등록·런타임 실패)을 덮는다.
  it("9) 다른 code의 대괄호 배너는 무보강 원문 — 활성화 진단이 사인을 가로채지 않는다", () => {
    const coded = [
      "BROWSER_DISABLED_SAFE [GUI_NOT_REGISTERED]: GUI peer has no broker-owned registration",
      "BROWSER_DISABLED_SAFE [GUI_IDENTITY_MISMATCH]: GUI PID incarnation, canonical executable, code signature, or digest changed",
      "BROWSER_DISABLED_SAFE [GUI_REGISTRATION_LIMIT]: too many GUI peers are registered",
      "BROWSER_DISABLED_SAFE [RUNTIME_START_TIMEOUT]: broker did not answer in 40s",
    ];
    for (const err of coded) {
      expect(err.startsWith("BROWSER_DISABLED_SAFE [")).toBe(true); // 규약 자체의 회귀 핀
      expect(castActivationDiagnostics(err, "armed-ok")).toBe(err);
      expect(castActivationDiagnostics(err, undefined)).toBe(err);
    }
  });
});
