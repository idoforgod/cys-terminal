// drainverify.ts 순수 함수 회귀 테스트 (bun test — 신규 의존성 0).
//
// [F5] drain_verify 폴백 사유 분기: 구버전 미지원과 크래시/하드캡을 구분해 UI가 정직한 문구를 고르게 한다.
// 거동(plain drain 폴백)은 양쪽 동일하고 분류·문구만 다르다.
import { describe, it, expect } from "bun:test";
import { classifyDrainVerifyFallback, drainVerifyFallbackToast, drainVerifyUnresponsiveLines } from "./drainverify";

describe("classifyDrainVerifyFallback — drain_verify 폴백 사유 분기", () => {
  it("구버전 미지원(unsupported 접두) → 'unsupported'", () => {
    expect(
      classifyDrainVerifyFallback("unsupported: cys drain --verify 미지원(구버전 바이너리) (stderr: ...)"),
    ).toBe("unsupported");
  });
  it("크래시/하드캡(verify_failed 접두) → 'verify_failed'", () => {
    expect(
      classifyDrainVerifyFallback("verify_failed: drain --verify 실행 실패(exit=Some(3)...) (stderr: )"),
    ).toBe("verify_failed");
  });
  it("알 수 없는 에러 → null(폴백 아님·상위 rethrow)", () => {
    expect(classifyDrainVerifyFallback("some other tauri error")).toBeNull();
  });
});

describe("drainVerifyFallbackToast — 사유별 정직 문구", () => {
  it("미지원은 '미지원' 문구, '무손실' 표현 없음", () => {
    const t = drainVerifyFallbackToast("unsupported");
    expect(t.title).toContain("미지원");
    expect(t.body).toContain("기존 방식");
    expect(t.body).not.toContain("무손실");
  });
  it("검증 실패는 '실패·점검 권고' 문구, 미지원과 구별", () => {
    const t = drainVerifyFallbackToast("verify_failed");
    expect(t.title).toContain("실패");
    expect(t.body).toContain("점검");
    expect(t.body).not.toContain("무손실");
    // 두 문구가 실제로 다른지(정직성 교정의 핵심)
    expect(t.body).not.toBe(drainVerifyFallbackToast("unsupported").body);
  });
});

describe("drainVerifyUnresponsiveLines — U4-B2② 응답 없는 데몬 표기", () => {
  it("unresponsive 만 사유로 나열하고 down(데몬 없음)은 제외한다", () => {
    const lines = drainVerifyUnresponsiveLines([
      { dept: "gone", department: "없는 부서", kind: "down", detail: "connect: No such file" },
      { dept: "hung", department: "무응답 부서", kind: "unresponsive", detail: "rpc_timeout: org.status" },
    ]);
    expect(lines.length).toBe(1);
    expect(lines[0]).toContain("무응답 부서");
    expect(lines[0]).toContain("응답 없음");
    expect(lines.join("\n")).not.toContain("없는 부서");
  });
  it("표시명이 없으면 부서 키로 적는다", () => {
    expect(drainVerifyUnresponsiveLines([{ dept: "d7", kind: "unresponsive" }])[0]).toContain("d7");
  });
  it("필드 부재(구버전 cys)·빈 목록은 빈 결과 — 종전 문구 무변경", () => {
    expect(drainVerifyUnresponsiveLines(undefined)).toEqual([]);
    expect(drainVerifyUnresponsiveLines([])).toEqual([]);
  });
});
