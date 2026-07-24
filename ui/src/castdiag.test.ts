// castActivationDiagnostics 순수 함수 회귀 테스트 (bun test — 신규 의존성 0).
//
// NATIVE_ACTIVATION_REQUIRED 실패의 진단 보강 — 표식(window.__CYS_BROWSER_ACTIVATION__)은
// 게이트가 아니라 문구 재료다. 판정자는 항상 백엔드 consume. 7분기 전수 검증(표식 상태별 진단 +
// 비활성화 무간섭 + 미지 표식 무보강 + 빈 입력).
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

  it("5) 'armed-ok' → 원문 + 'TTL 만료/이중 소모 의심' 힌트 후행", () => {
    expect(castActivationDiagnostics(NATIVE, "armed-ok")).toBe(
      `${NATIVE} · (진단: arm 성공 이력 — TTL 만료/이중 소모 의심)`,
    );
  });

  it("6) 미지 표식('hacked-by-page') → 무보강 원문(위조 오진 금지)", () => {
    expect(castActivationDiagnostics(NATIVE, "hacked-by-page")).toBe(NATIVE);
  });

  it("7) 빈 문자열·null 에러 → '' (빈 결과)", () => {
    expect(castActivationDiagnostics("", "installed")).toBe("");
    expect(castActivationDiagnostics(null, undefined)).toBe("");
  });
});
