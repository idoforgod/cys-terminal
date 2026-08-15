// selfdiag.ts CEO 팔레트 노출 게이트 회귀 테스트 (스펙 D4 · 결정 D4 — 상호 배타·pending 우선 고정).
//
// 파일 실측(.pre-ceo·md·CEO_TEMPLATE 비교)은 Rust 쪽 순수 핀(src-tauri ceo_drift_verdict_gate_matrix)이
// 담당하고, 여기는 그 bool 신호 2개 → 팔레트 항목 결정의 계약만 고정한다.
import { describe, it, expect } from "bun:test";
import { ceoPaletteEntries } from "./selfdiag";

describe("CEO 팔레트 항목 결정 — 상호 배타 매트릭스", () => {
  it("드리프트만 참 → '재실행' 단독 노출 (D4 의 유일한 신규 노출 케이스)", () => {
    expect(ceoPaletteEntries({ pending: false, drift: true })).toEqual(["repromote"]);
  });
  it("PENDING만 참 → 기존 '승격 진행' 단독 (기존 R8 동선 무회귀)", () => {
    expect(ceoPaletteEntries({ pending: true, drift: false })).toEqual(["pending"]);
  });
  it("동시 참 → pending 우선·재실행 숨김 (최초 승격 미완에 '재실행' 권유 금지)", () => {
    expect(ceoPaletteEntries({ pending: true, drift: true })).toEqual(["pending"]);
  });
  it("둘 다 거짓(invoke 실패 폴백 포함) → 노출 0 (조용한 기본값)", () => {
    expect(ceoPaletteEntries({ pending: false, drift: false })).toEqual([]);
  });
});
