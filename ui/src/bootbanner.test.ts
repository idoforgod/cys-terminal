import { describe, expect, test } from "bun:test";
import { bootBannerDecision, isFormationKind, PARTIAL_BANNER_TEXT } from "./bootbanner";

describe("bootBannerDecision — 배너 수명 판정(kind 문자열 핀 = 회귀 0)", () => {
  test("formation-complete → dismiss(배너 소멸)", () => {
    expect(bootBannerDecision("formation-complete")).toEqual({ action: "dismiss" });
  });

  test("formation-partial → update(문구 갱신·제거 아님)", () => {
    const d = bootBannerDecision("formation-partial");
    expect(d.action).toBe("update");
    if (d.action === "update") expect(d.text).toBe(PARTIAL_BANNER_TEXT);
  });

  test("formation-pending → ignore(미완이므로 배너 유지·무변경)", () => {
    expect(bootBannerDecision("formation-pending")).toEqual({ action: "ignore" });
  });

  test("formation-failed → ignore(실패는 dismiss 하지 않는다 — 오소멸 방지)", () => {
    expect(bootBannerDecision("formation-failed")).toEqual({ action: "ignore" });
  });

  test("구팩 호환 kind 'formation'(상태 미분화) → ignore(오소멸 방지)", () => {
    expect(bootBannerDecision("formation")).toEqual({ action: "ignore" });
  });

  test("비-formation kind(승인 요청 등) → ignore", () => {
    expect(bootBannerDecision("ceo-promote-request")).toEqual({ action: "ignore" });
    expect(bootBannerDecision("bootstrap-fail")).toEqual({ action: "ignore" });
    expect(bootBannerDecision("")).toEqual({ action: "ignore" });
  });
});

describe("isFormationKind — 편성 신호 라우팅 판별", () => {
  test("formation-* 접두는 편성 신호", () => {
    expect(isFormationKind("formation-complete")).toBe(true);
    expect(isFormationKind("formation-partial")).toBe(true);
    expect(isFormationKind("formation-pending")).toBe(true);
    expect(isFormationKind("formation")).toBe(true);
  });

  test("그 외 kind 는 편성 신호 아님(승인 경로 유지)", () => {
    expect(isFormationKind("ceo-promote-request")).toBe(false);
    expect(isFormationKind("notification")).toBe(false);
    expect(isFormationKind("")).toBe(false);
  });
});
