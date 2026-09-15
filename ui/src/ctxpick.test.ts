// ctxpick.ts 순수 함수 회귀 테스트 (bun test — 신규 의존성 0). ★WP6-2.
//
// 60% 임계를 읽는 소비자 전부가 같은 축(실측 > 신선한 자기보고 · 결측 null)을 쓰는지,
// 그리고 결측이 0 으로 위장돼 목록에서 조용히 빠지는 옛 결함(`?? 0`)이 재발하지 않는지 못박는다.
import { expect, test } from "bun:test";
import { pickCtx, isHotCtx } from "./ctxpick";

test("실측이 있으면 자기보고를 덮는다", () => {
  expect(pickCtx({ usage: { ctx_pct: 78 }, status: { context_pct: 95, age_secs: 1 } }))
    .toEqual({ pct: 78, src: "measured" });
});
test("신고가 없으면 0 이 아니라 결측이다 — 60% 목록에서 조용히 빠지면 안 된다", () => {
  expect(pickCtx({ usage: null, status: null })).toEqual({ pct: null, src: "none" });
  expect(isHotCtx({ usage: null, status: null })).toBe(false);
  // 회귀 박제: 옛 코드는 `?? 0` 이라 이 노드를 "0%"로 세었다.
  expect(pickCtx({ usage: null, status: null }).pct).not.toBe(0);
});
test("낡은 자기보고는 판정에 쓰지 않는다", () => {
  expect(isHotCtx({ usage: null, status: { context_pct: 90, age_secs: 301 } })).toBe(false);
  expect(isHotCtx({ usage: null, status: { context_pct: 90, age_secs: 10 } })).toBe(true);
});
test("실측 60% 는 자기보고가 낮아도 잡힌다", () => {
  expect(isHotCtx({ usage: { ctx_pct: 61 }, status: { context_pct: 20, age_secs: 1 } })).toBe(true);
});
test("낡은/나이 미상 자기보고는 src=stale · pct=null — 판정은 불가하되 그 사실은 보인다", () => {
  expect(pickCtx({ usage: null, status: { context_pct: 90, age_secs: 301 } })).toEqual({ pct: null, src: "stale" });
  expect(pickCtx({ usage: null, status: { context_pct: 90 } })).toEqual({ pct: null, src: "stale" });
  expect(isHotCtx({ usage: null, status: { context_pct: 90, age_secs: 301 } })).toBe(false);
  // 자기보고 자체가 숫자가 아니면 stale 이 아니라 none(빈 칸 유지)
  expect(pickCtx({ usage: null, status: { state: "working" } })).toEqual({ pct: null, src: "none" });
  // 실측이 있으면 낡은 자기보고는 무관 — measured 가 이긴다
  expect(pickCtx({ usage: { ctx_pct: 40 }, status: { context_pct: 90, age_secs: 999 } })).toEqual({ pct: 40, src: "measured" });
});
