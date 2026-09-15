// ctxpick.ts 순수 함수 회귀 테스트 (bun test — 신규 의존성 0). ★WP6-2.
//
// 60% 임계를 읽는 소비자 전부가 같은 축(실측 > 신선한 자기보고 · 결측 null)을 쓰는지,
// 그리고 결측이 0 으로 위장돼 목록에서 조용히 빠지는 옛 결함(`?? 0`)이 재발하지 않는지 못박는다.
import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
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
// ⑦(0.14.31 감사) 종료 좌석 — 데몬 수집기(usage.rs collect_tick)는 exited 좌석을 건너뛰어 usage 가 마지막 값에
//   동결된다. 실패 방향: 값 쪽으로 접히면 죽은 좌석의 동결 실측이 60% 목록에 남는다.
test("종료 좌석(exited)의 동결 실측은 산 값이 아니다 — pct null · src stale · isHotCtx false", () => {
  expect(pickCtx({ exited: true, usage: { ctx_pct: 82 }, status: { context_pct: 70, age_secs: 1 } }))
    .toEqual({ pct: null, src: "stale" });
  expect(isHotCtx({ exited: true, usage: { ctx_pct: 82 } })).toBe(false);
  // 음성 대조 — exited 가 false/null/부재면 게이트가 열리지 않는다(구버전 데몬 페이로드 무해).
  expect(pickCtx({ exited: false, usage: { ctx_pct: 82 } })).toEqual({ pct: 82, src: "measured" });
  expect(pickCtx({ exited: null, usage: { ctx_pct: 82 } })).toEqual({ pct: 82, src: "measured" });
  expect(pickCtx({ usage: { ctx_pct: 82 } })).toEqual({ pct: 82, src: "measured" });
});
// ⑧ 에이전트 사망(agent_alive=false · pane 은 살아 exited=false) — 워치독이 확정한 사망(3상 · false 만).
//   실패 방향: null 을 false 로 접으면 미관측 좌석(부팅 직후·구버전 데몬) 전부가 판정 불가가 된다 — null 은 "모른다".
test("에이전트 사망(agent_alive=false)의 동결 실측도 산 값이 아니다 — null/부재/true 는 게이트를 열지 않는다", () => {
  expect(pickCtx({ exited: false, agent_alive: false, usage: { ctx_pct: 82 }, status: { context_pct: 70, age_secs: 1 } }))
    .toEqual({ pct: null, src: "stale" });
  expect(isHotCtx({ agent_alive: false, usage: { ctx_pct: 82 } })).toBe(false);
  expect(pickCtx({ agent_alive: null, usage: { ctx_pct: 82 } })).toEqual({ pct: 82, src: "measured" });
  expect(pickCtx({ exited: false, usage: { ctx_pct: 82 } })).toEqual({ pct: 82, src: "measured" });
  expect(pickCtx({ agent_alive: true, usage: { ctx_pct: 82 } })).toEqual({ pct: 82, src: "measured" });
});

// ★소스 계약 핀(리뷰어 지적) — main.ts 의 60% 소비자가 이 한 벌을 쓰고, 결측을 0 으로 접던 옛 형태가
//   되돌아오지 않았는지를 소스로 못박는다. 실패 방향: 소비자 하나가 헬퍼를 우회하거나 `?? 0` 이 부활하면
//   여기가 먼저 깨진다(두 구현이 조용히 갈라져 같은 좌석이 두 화면에서 다른 숫자를 내는 것을 막는다).
const MAIN_URL = new URL("./main.ts", import.meta.url);
test("main.ts 소스 계약 — context_pct 의 `?? 0` 부재 · pickCtx/isHotCtx 호출 수 ≥ WP6-2 이동 소비자 수", () => {
  const src = readFileSync(MAIN_URL, "utf-8");
  // 바늘은 조각으로 조립한다 — 이 파일 자체가 `git grep` 수용 기준에 잡히지 않게.
  const oldForm = ["context_pct", " ?? ", "0"].join("");
  expect(src.includes(oldForm)).toBe(false);
  const calls =
    (src.match(/\bpickCtx\(/g) ?? []).length + (src.match(/\bisHotCtx\(/g) ?? []).length;
  // WP6-2 가 옮긴 소비자(2026-09-15 소스 계수 · 하한): ①업무행 막대 ②nodeSig.ctx_pct ③60% cycle 토스트
  //   ④팔레트 점프행 ⑤60% cycle 목록(isHotCtx 필터 + pickCtx 부제) = 5 자리(호출은 6).
  const MOVED = 5;
  expect(calls >= MOVED).toBe(true);
  expect(calls).toBeGreaterThan(MOVED - 1);
});
