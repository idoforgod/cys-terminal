// ctxthreshold.ts 순수 함수 회귀 테스트 (bun test — 신규 의존성 0). REVISE-3a/R-6 문구 분기 잠금.
//
// 잠그는 사실:
//  ① `observed-uncertain` 만 불확실 꼬리표를 받는다. 다른 출처·필드 부재는 **종전 문구 그대로**
//     (구 데몬 스큐에서 갑자기 모든 알림이 "불확실"로 오염되면 꼬리표가 의미를 잃는다).
//  ② 꼬리표는 **병기**다 — 기존 본문(role·surface·임계·action)이 하나도 잘리지 않는다.
//  ③ 불확실이라고 알림을 삼키거나 OS 배너 임계(≥80)를 바꾸지 않는다. C1 고위험 폴백 발화가
//     UI 단에서 조용히 사라지면 "100% 방치"라는 원 결함으로 되돌아간다.
import { describe, it, expect } from "bun:test";
import {
  isUncertainAttribution,
  contextThresholdTitle,
  contextThresholdBody,
  shouldOsBanner,
  UNCERTAIN_NOTE,
} from "./ctxthreshold";

const ACTION = "cycle-agent(저장→검증→clear→복원) 집행 대상";

describe("isUncertainAttribution — 출처 판정(엄격 리터럴)", () => {
  it("observed-uncertain 만 참", () => {
    expect(isUncertainAttribution("observed-uncertain")).toBe(true);
  });
  it("observed(귀속 확실)는 거짓", () => {
    expect(isUncertainAttribution("observed")).toBe(false);
  });
  it("self-report·statusline 은 거짓", () => {
    expect(isUncertainAttribution("self-report")).toBe(false);
    expect(isUncertainAttribution("statusline")).toBe(false);
  });
  it("필드 부재·null(구 데몬)은 거짓 — 종전 어조 유지(스큐 안전)", () => {
    expect(isUncertainAttribution(undefined)).toBe(false);
    expect(isUncertainAttribution(null)).toBe(false);
  });
  it("모르는 미래 출처도 거짓(확신 어조 오염 금지)", () => {
    expect(isUncertainAttribution("observed-something-new")).toBe(false);
  });
});

describe("contextThresholdBody — 문구 병기", () => {
  const base = {
    context_pct: 90,
    threshold: 60,
    role: "worker",
    surface_ref: "surface:7",
    action: ACTION,
  };

  it("귀속 확실(observed) — 종전 문구 그대로, 꼬리표 없음", () => {
    const body = contextThresholdBody({ ...base, source: "observed" });
    expect(body).toBe(`worker surface:7 ≥ 60% — ${ACTION}`);
    expect(body).not.toContain(UNCERTAIN_NOTE);
  });

  it("자기보고(self-report) — 종전 문구 그대로", () => {
    expect(contextThresholdBody({ ...base, source: "self-report" })).toBe(
      `worker surface:7 ≥ 60% — ${ACTION}`,
    );
  });

  it("source 부재(구 데몬) — 종전 문구 그대로", () => {
    expect(contextThresholdBody(base)).toBe(`worker surface:7 ≥ 60% — ${ACTION}`);
  });

  it("observed-uncertain — 꼬리표 병기(기존 본문 무손실)", () => {
    const body = contextThresholdBody({ ...base, source: "observed-uncertain" });
    expect(body).toBe(`worker surface:7 ≥ 60% — ${ACTION} ${UNCERTAIN_NOTE}`);
    expect(body).toContain("worker surface:7 ≥ 60%");
    expect(body).toContain(ACTION);
  });

  it("role·surface 누락 payload 도 종전과 동일하게 빈칸으로 떨어진다(구 문구 바이트 동일)", () => {
    // 종전 main.ts 인라인 템플릿과 **같은 결과**여야 한다: role/surface 가 없으면 앞이 빈칸 2개다.
    expect(contextThresholdBody({ context_pct: 90, threshold: 60 })).toBe("  ≥ 60% — ");
  });

  it("payload 자체가 없어도 던지지 않는다", () => {
    expect(() => contextThresholdBody(null)).not.toThrow();
    expect(() => contextThresholdBody(undefined)).not.toThrow();
  });
});

describe("contextThresholdTitle — 제목은 출처와 무관", () => {
  it("불확실 여부와 상관없이 퍼센트만 찍는다", () => {
    expect(contextThresholdTitle({ context_pct: 90, source: "observed-uncertain" })).toBe(
      "🔋 컨텍스트 90%",
    );
    expect(contextThresholdTitle({ context_pct: 90, source: "observed" })).toBe("🔋 컨텍스트 90%");
  });
});

describe("shouldOsBanner — ≥80 계약 불변(불확실이 알림을 삼키지 않는다)", () => {
  it("80 미만은 토스트만", () => {
    expect(shouldOsBanner({ context_pct: 79 })).toBe(false);
  });
  it("80 이상은 배너 — 출처가 불확실이어도 그대로 올린다", () => {
    expect(shouldOsBanner({ context_pct: 80, source: "observed-uncertain" })).toBe(true);
    expect(shouldOsBanner({ context_pct: 100, source: "observed-uncertain" })).toBe(true);
  });
  it("퍼센트 부재는 배너 없음", () => {
    expect(shouldOsBanner({})).toBe(false);
    expect(shouldOsBanner(null)).toBe(false);
  });
});
