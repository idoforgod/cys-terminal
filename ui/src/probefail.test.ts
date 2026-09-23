// probefail.ts 순수 판정 회귀 테스트 + main.ts 배선 핀 (bun test — 신규 의존성 0).
// 0.14.41 · U4 A5 ②(경보 배지: 조회 실패가 '경보 0'으로 보임) · ③(승인 자동 전환: feed 조회 실패를 '결재됨'으로 접음).
//
// 근거: _evidence/impl-9items-20260923/phase1/U4c-silentpass-pack-ui{,.refute}.md F9·F10(R9 유지 · ① 재예약 상한 1 동의)
// 실패 방향(이 파일이 막는 것): 조회가 실패했는데 화면이 '없음'(경보 0 · 이미 결재됨)을 그려 사람이 봐야 할 것이
//   조용히 묻히는 것. 반대로 한두 번의 일시 실패로 배지가 깜빡이거나(상한 3), 재시도가 무한히 도는 것(상한 1)도 막는다.
// ★리뷰1 #1(배선 공허 — main.ts 재예약 인자 true→false 가 54/54 녹색으로 살아남았다) 이후: 계수·재예약·연장 결정은
//   전부 probefail.ts 로 올라왔고, 여기서 **실제 스케줄러**를 가짜 타이머로 전수 시뮬레이션해 타이머 수·총 경과 상한을
//   단언한다. main.ts 쪽은 본문 구간·정확한 호출형 핀(의존성 주입 한 벌 · 위임 한 줄)만 남는다.
import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import {
  ALERTS_FAIL_LIMIT,
  FEED_LOOKUP_RETRY_MS,
  FEED_SWITCH_GRACE_AUTO_MS,
  FEED_SWITCH_GRACE_MS,
  FEED_SWITCH_MAX_MS,
  alertsListOf,
  alertsView,
  feedSwitchGraceMs,
  feedSwitchMayExtend,
  feedSwitchPlan,
  feedSwitchStep,
  makeAlertsTracker,
  makeFeedSwitchScheduler,
  nextAlertsFailStreak,
  readAlerts,
} from "./probefail";

describe("U4 A5② — control_alerts 응답 판독", () => {
  test("alerts 배열이면 그대로", () => {
    const l = [{ severity: "warn", message: "m" }];
    expect(alertsListOf({ alerts: l })).toBe(l);
    expect(alertsListOf({ alerts: [] })).toEqual([]);
  });
  test("필드 누락·비배열·null 은 빈 목록이 아니라 '미측정'(null) — 경보 0 으로 접지 않는다", () => {
    expect(alertsListOf({})).toBeNull();
    expect(alertsListOf({ alerts: "x" })).toBeNull();
    expect(alertsListOf({ alerts: null })).toBeNull();
    expect(alertsListOf(null)).toBeNull();
    expect(alertsListOf(undefined)).toBeNull();
  });
});

describe("U4 A5② — 경보 배지 표시", () => {
  test("성공 · 0건 → 숨김(종전)", () => {
    const v = alertsView([], 0)!;
    expect(v.hidden).toBe(true);
    expect(v.rows).toEqual([]);
  });
  test("성공 · warn/crit → ⚠ N · crit 우선 색(종전)", () => {
    const v = alertsView([{ severity: "warn", message: "a" }, { severity: "crit", kind: "k" }], 0)!;
    expect(v.hidden).toBe(false);
    expect(v.text).toBe("⚠ 2");
    expect(v.cls).toBe("crit");
    expect(v.rows.map((r) => r.sev)).toEqual(["warn", "crit"]);
    expect(v.rows[1].msg).toBe("k");
    expect(alertsView([{ severity: "warn", message: "a" }], 0)!.cls).toBe("warn");
  });
  test("실패가 상한 미만이면 null — 직전 표시 유지(한 번의 일시 실패로 깜빡이지 않는다)", () => {
    for (let n = 1; n < ALERTS_FAIL_LIMIT; n++) expect(alertsView(null, n)).toBeNull();
  });
  test("실패 3회 연속 → 회색 '경보 조회 불가' 배지(숨기지 않는다) + 스트립 1행", () => {
    expect(ALERTS_FAIL_LIMIT).toBe(3);
    const v = alertsView(null, ALERTS_FAIL_LIMIT)!;
    expect(v).not.toBeNull();
    expect(v.hidden).toBe(false);
    expect(v.cls).toBe("unknown");
    expect(v.text).toContain("경보 조회 불가");
    expect(v.title).toContain(String(ALERTS_FAIL_LIMIT));
    expect(v.rows.length).toBe(1);
    expect(v.rows[0].sev).toBe("unknown");
    expect(alertsView(null, 9)!.cls).toBe("unknown");
  });
  test("성공하면 실패 계수와 무관하게 정상 표시로 돌아온다", () => {
    expect(alertsView([], 5)!.hidden).toBe(true);
  });
});

describe("U4 A5③ — 승인 자동 전환: feed_list 조회 실패", () => {
  const items = [
    { request_id: "r1", status: "pending" },
    { request_id: "r2", status: "approved" },
  ];
  test("pending 이면 pending(종전 전환 경로)", () => {
    expect(feedSwitchStep({ items }, "r1", false)).toBe("pending");
  });
  test("이미 결재됨·목록에 없음 → settled(종전)", () => {
    expect(feedSwitchStep({ items }, "r2", false)).toBe("settled");
    expect(feedSwitchStep({ items }, "zz", false)).toBe("settled");
  });
  test("조회 실패(null)·items 비배열은 '결재됨'이 아니다 — 첫 실패는 재시도", () => {
    expect(feedSwitchStep(null, "r1", false)).toBe("retry");
    expect(feedSwitchStep({}, "r1", false)).toBe("retry");
    expect(feedSwitchStep({ items: "x" }, "r1", false)).toBe("retry");
  });
  test("재시도도 실패하면 open — 보여 주는 쪽으로 실패", () => {
    expect(feedSwitchStep(null, "r1", true)).toBe("open");
  });
  // (순수 판정 층만 본다 — 실제 재예약 경로의 상한은 아래 '스케줄러 전수 시뮬레이션'이 makeFeedSwitchScheduler 로 잰다.)
  test("(순수 판정) 재시도 상한 1 — 계속 실패하는 조회에 feedSwitchStep 은 정확히 1회 retry 뒤 open", () => {
    let retried = false;
    const trail: string[] = [];
    for (let i = 0; i < 10; i++) {
      const step = feedSwitchStep(null, "r1", retried);
      trail.push(step);
      if (step !== "retry") break;
      retried = true;
    }
    expect(trail).toEqual(["retry", "open"]);
  });
  test("재시도 간격은 짧다(유예를 통째로 다시 기다리지 않는다)", () => {
    expect(FEED_LOOKUP_RETRY_MS).toBeGreaterThan(0);
    expect(FEED_LOOKUP_RETRY_MS).toBeLessThan(10_001);
  });
});

describe("U4 A5② — 경보 조회·계수기(리뷰1 #1: main.ts 가 하던 계수·리셋을 여기서 잰다)", () => {
  test("readAlerts — 거부·동기 throw·미측정 응답은 null, 배열은 그대로", async () => {
    expect(await readAlerts(() => Promise.reject(new Error("down")))).toBeNull();
    expect(
      await readAlerts(() => {
        throw new Error("sync");
      }),
    ).toBeNull();
    expect(await readAlerts(async () => ({}))).toBeNull();
    expect(await readAlerts(async () => ({ alerts: [] }))).toEqual([]);
    const l = [{ severity: "crit" }];
    expect(await readAlerts(async () => ({ alerts: l }))).toBe(l);
  });
  test("nextAlertsFailStreak — 실패면 +1, 성공(빈 목록 포함)이면 0", () => {
    expect(nextAlertsFailStreak(0, null)).toBe(1);
    expect(nextAlertsFailStreak(2, null)).toBe(3);
    expect(nextAlertsFailStreak(7, [])).toBe(0);
    expect(nextAlertsFailStreak(7, [{}])).toBe(0);
  });
  test("계수기 — 실패 1·2회는 직전 표시 유지(null), 3회째 '조회 불가', 성공하면 즉시 정상·계수 0", () => {
    const t = makeAlertsTracker();
    expect(t.step(null)).toBeNull();
    expect(t.streak()).toBe(1);
    expect(t.step(null)).toBeNull();
    const v = t.step(null)!;
    expect(v).not.toBeNull();
    expect(v.cls).toBe("unknown");
    expect(v.text).toContain("경보 조회 불가");
    expect(t.streak()).toBe(ALERTS_FAIL_LIMIT);
    expect(t.step(null)!.cls).toBe("unknown"); // 계속 실패하면 계속 '조회 불가'
    const ok = t.step([])!;
    expect(ok.hidden).toBe(true);
    expect(t.streak()).toBe(0);
    // 성공 뒤의 일시 실패 1회는 곧바로 배지가 아니다(리셋이 실제로 일어났다)
    expect(t.step(null)).toBeNull();
  });
  test("계수기 reset — CC 를 다시 열 때 옛 계수를 푼다(리뷰1 #7)", () => {
    const t = makeAlertsTracker();
    t.step(null);
    t.step(null);
    t.reset();
    expect(t.streak()).toBe(0);
    expect(t.step(null)).toBeNull(); // reset 이 없으면 여기서 3 → '조회 불가'
  });
  test("계수기 둘은 서로 독립(모듈 전역 상태 0)", () => {
    const a = makeAlertsTracker();
    const b = makeAlertsTracker();
    a.step(null);
    a.step(null);
    expect(b.streak()).toBe(0);
  });
});

describe("U4 A5③ — 승인 전환 한 틱의 결정(feedSwitchPlan)", () => {
  test("유예 — 재시도 5초 · auto 90초 · 비대상 30초 · 상한 300초(값 봉인)", () => {
    expect(FEED_SWITCH_GRACE_MS).toBe(30_000);
    expect(FEED_SWITCH_GRACE_AUTO_MS).toBe(90_000);
    expect(FEED_SWITCH_MAX_MS).toBe(300_000);
    expect(feedSwitchGraceMs(false, false)).toBe(FEED_SWITCH_GRACE_MS);
    expect(feedSwitchGraceMs(true, false)).toBe(FEED_SWITCH_GRACE_AUTO_MS);
    expect(feedSwitchGraceMs(true, true)).toBe(FEED_LOOKUP_RETRY_MS);
    expect(feedSwitchGraceMs(false, true)).toBe(FEED_LOOKUP_RETRY_MS);
  });
  test("연장 가능 = pending ∧ auto ∧ 상한 이내 — 조회 2회 실패(open)는 연장 불가", () => {
    expect(feedSwitchMayExtend("pending", true, 0, 90_000)).toBe(true);
    expect(feedSwitchMayExtend("open", true, 0, 90_000)).toBe(false);
    expect(feedSwitchMayExtend("retry", true, 0, 90_000)).toBe(false);
    expect(feedSwitchMayExtend("settled", true, 0, 90_000)).toBe(false);
    expect(feedSwitchMayExtend("pending", false, 0, 30_000)).toBe(false);
    expect(feedSwitchMayExtend("pending", true, 210_000, 90_000)).toBe(false); // 정확히 상한 = 연장 불가
    expect(feedSwitchMayExtend("pending", true, 209_999, 90_000)).toBe(true);
  });
  test("결정 진리표", () => {
    const o = { autoRoute: true, elapsedMs: 90_000, grace: 5_000, ceoActive: true };
    expect(feedSwitchPlan("settled", o)).toEqual({ kind: "end" });
    // retry 는 재시도 표식을 **단 채** 재예약(상한 1)
    expect(feedSwitchPlan("retry", { ...o, grace: 90_000, elapsedMs: 0 })).toEqual({
      kind: "reschedule",
      autoRoute: true,
      elapsedMs: 90_000,
      lookupRetried: true,
    });
    // 성공 조회 뒤 연장은 재시도 표식을 푼다
    expect(feedSwitchPlan("pending", o)).toEqual({ kind: "reschedule", autoRoute: true, elapsedMs: 95_000, lookupRetried: false });
    expect(feedSwitchPlan("pending", { ...o, ceoActive: false })).toEqual({ kind: "open" });
    expect(feedSwitchPlan("open", o)).toEqual({ kind: "open" }); // CEO 가 활성이어도
    expect(feedSwitchPlan("pending", { ...o, autoRoute: false })).toEqual({ kind: "open" });
    expect(feedSwitchPlan("pending", { ...o, elapsedMs: 295_000 })).toEqual({ kind: "open" }); // 상한
  });
});

// ---------- 스케줄러 시뮬레이션: 실제 재예약 코드를 가짜 타이머로 끝까지 돌린다(리뷰1 #1 — ① 폭주 상한의 실행 증거) ----------
type Resp = "null" | "pending" | "settled" | "throw" | "reject" | "junk";
type SimOpts = { autoRoute: boolean; feed: (i: number) => Resp; ceo?: (i: number) => boolean | "throw"; requestId?: string };
type Sim = {
  clock: number;
  timers: number;
  opens: number;
  maxQueued: number;
  lookups: number;
  ceoCalls: number;
  graces: number[];
  consumed: Resp[];
  openAt: number | null;
  finished: boolean;
};
const TICK_CAP = 64;
async function simulate(o: SimOpts): Promise<Sim> {
  const q: { fn: () => Promise<void>; ms: number }[] = [];
  const sim: Sim = {
    clock: 0,
    timers: 0,
    opens: 0,
    maxQueued: 0,
    lookups: 0,
    ceoCalls: 0,
    graces: [],
    consumed: [],
    openAt: null,
    finished: false,
  };
  const rid = o.requestId ?? "r1";
  const schedule = makeFeedSwitchScheduler({
    setTimer: (fn, ms) => {
      q.push({ fn, ms });
      sim.timers++;
      sim.graces.push(ms);
      sim.maxQueued = Math.max(sim.maxQueued, q.length);
    },
    feedList: () => {
      const r = o.feed(sim.lookups++);
      sim.consumed.push(r);
      if (r === "throw") throw new Error("sync");
      if (r === "reject") return Promise.reject(new Error("down"));
      if (r === "null") return Promise.resolve(null);
      if (r === "junk") return Promise.resolve({ items: "x" });
      return Promise.resolve({ items: [{ request_id: rid, status: r === "pending" ? "pending" : "approved" }] });
    },
    ceoActive: async () => {
      const c = (o.ceo ?? (() => true))(sim.ceoCalls++);
      if (c === "throw") throw new Error("ceo");
      return c;
    },
    open: () => {
      sim.opens++;
      sim.openAt = sim.clock;
    },
  });
  schedule(rid, o.autoRoute);
  let n = 0;
  while (q.length && n < TICK_CAP) {
    const t = q.shift()!;
    sim.clock += t.ms;
    await t.fn();
    n++;
  }
  sim.finished = q.length === 0;
  return sim;
}
const seq = (xs: Resp[], tail: Resp = "null") => (i: number): Resp => xs[i] ?? tail;
/** 코드에 박힌 상한에서 유도한 최악값: 정상 유예 타이머는 elapsed<MAX 일 때만 생기고 매번 GRACE_AUTO 이상 전진 →
 *  floor((MAX-1)/GRACE_AUTO)+1 개, 각각 뒤에 재시도 1개. */
const MAX_TIMERS_AUTO = 2 * (Math.floor((FEED_SWITCH_MAX_MS - 1) / FEED_SWITCH_GRACE_AUTO_MS) + 1);
const MAX_CLOCK = FEED_SWITCH_MAX_MS + FEED_SWITCH_GRACE_AUTO_MS + FEED_LOOKUP_RETRY_MS;

describe("U4 A5③ — 스케줄러 시나리오(실제 makeFeedSwitchScheduler · 가짜 타이머)", () => {
  test("비대상: 조회가 계속 실패하면 30초 뒤 1회만 5초 재시도하고 open(총 35초 · 타이머 2)", async () => {
    const s = await simulate({ autoRoute: false, feed: seq([], "null") });
    expect(s.finished).toBe(true);
    expect(s.graces).toEqual([FEED_SWITCH_GRACE_MS, FEED_LOOKUP_RETRY_MS]);
    expect(s.opens).toBe(1);
    expect(s.openAt).toBe(35_000);
    expect(s.ceoCalls).toBe(0);
  });
  test("비대상: 재시도에서 결재됨이 보이면 전환 없음 · pending 이면 open", async () => {
    const a = await simulate({ autoRoute: false, feed: seq(["null", "settled"]) });
    expect(a.opens).toBe(0);
    expect(a.timers).toBe(2);
    const b = await simulate({ autoRoute: false, feed: seq(["null", "pending"]) });
    expect(b.opens).toBe(1);
    expect(b.openAt).toBe(35_000);
  });
  test("동기 throw·reject·items 비배열도 '조회 실패'(결재됨으로 접지 않는다)", async () => {
    for (const bad of ["throw", "reject", "junk"] as Resp[]) {
      const s = await simulate({ autoRoute: false, feed: seq([bad, bad]) });
      expect(s.graces).toEqual([FEED_SWITCH_GRACE_MS, FEED_LOOKUP_RETRY_MS]);
      expect(s.opens).toBe(1);
    }
  });
  test("auto: 조회 2회 연속 실패면 CEO 가 활성이어도 연장하지 않고 open(95초) — CEO 판정 호출 0", async () => {
    const s = await simulate({ autoRoute: true, feed: seq(["null", "null"]), ceo: () => true });
    expect(s.graces).toEqual([FEED_SWITCH_GRACE_AUTO_MS, FEED_LOOKUP_RETRY_MS]);
    expect(s.opens).toBe(1);
    expect(s.openAt).toBe(95_000);
    expect(s.ceoCalls).toBe(0);
  });
  test("auto: 첫 조회 실패 → 5초 재시도에서 pending · CEO 비활성 → open(95초)", async () => {
    const s = await simulate({ autoRoute: true, feed: seq(["null", "pending"]), ceo: () => false });
    expect(s.graces).toEqual([FEED_SWITCH_GRACE_AUTO_MS, FEED_LOOKUP_RETRY_MS]);
    expect(s.openAt).toBe(95_000);
    expect(s.ceoCalls).toBe(1);
  });
  test("auto: CEO 가 계속 활성이면 상한까지만 연장하고 open(90·180·270·360초 · 타이머 4)", async () => {
    const s = await simulate({ autoRoute: true, feed: seq([], "pending"), ceo: () => true });
    expect(s.graces).toEqual([90_000, 90_000, 90_000, 90_000]);
    expect(s.openAt).toBe(360_000);
    expect(s.opens).toBe(1);
  });
  test("auto: CEO 판정이 던지면 비활성으로(보여 주는 쪽으로 실패)", async () => {
    const s = await simulate({ autoRoute: true, feed: seq([], "pending"), ceo: () => "throw" });
    expect(s.openAt).toBe(90_000);
    expect(s.opens).toBe(1);
  });
  test("비대상 pending 은 CEO 판정 없이 30초 뒤 open(종전)", async () => {
    const s = await simulate({ autoRoute: false, feed: seq([], "pending") });
    expect(s.openAt).toBe(30_000);
    expect(s.ceoCalls).toBe(0);
  });
  test("request_id 가 비면 타이머 0", async () => {
    const s = await simulate({ autoRoute: true, feed: seq([], "null"), requestId: "" });
    expect(s.timers).toBe(0);
    expect(s.opens).toBe(0);
  });
});

describe("U4 A5③ — 스케줄러 전수 시뮬레이션: 어떤 응답 순서에도 타이머 수·총 경과가 코드 상한 안에서 끝난다(① 폭주 0)", () => {
  function checkInvariants(s: Sim, autoRoute: boolean) {
    expect(s.finished).toBe(true); // TICK_CAP(64) 안에 끝났다 — 무한 재예약 0
    expect(s.maxQueued).toBeLessThan(2); // 요청당 타이머는 언제나 1개
    expect(s.timers).toBeLessThan((autoRoute ? MAX_TIMERS_AUTO : 2) + 1);
    expect(s.clock).toBeLessThan(MAX_CLOCK + 1);
    // 재시도(5초)는 연속 1회뿐
    for (let i = 1; i < s.graces.length; i++)
      expect(s.graces[i] === FEED_LOOKUP_RETRY_MS && s.graces[i - 1] === FEED_LOOKUP_RETRY_MS).toBe(false);
    // 끝은 정확히 하나: 마지막 조회가 결재됨이면 전환 0, 아니면 open 1
    const last = s.consumed[s.consumed.length - 1];
    expect(s.opens).toBe(last === "settled" ? 0 : 1);
    if (!autoRoute) expect(s.ceoCalls).toBe(0);
  }
  test("상한 유도값(최악 8타이머 · 395초) — 값 봉인", () => {
    expect(MAX_TIMERS_AUTO).toBe(8);
    expect(MAX_CLOCK).toBe(395_000);
  });
  test("응답 {실패,pending}^9 전부 + 각 접두 뒤 결재됨 — CEO 항상 활성(최장 경로)", async () => {
    let worstTimers = 0;
    let worstClock = 0;
    for (const autoRoute of [true, false]) {
      for (let mask = 0; mask < 1 << 9; mask++) {
        const xs: Resp[] = [];
        for (let k = 0; k < 9; k++) xs.push(mask & (1 << k) ? "pending" : "null");
        for (let cut = 0; cut <= 9; cut++) {
          const pre = cut === 9 ? xs : [...xs.slice(0, cut), "settled" as Resp];
          const s = await simulate({ autoRoute, feed: seq(pre, "pending"), ceo: () => true });
          checkInvariants(s, autoRoute);
          worstTimers = Math.max(worstTimers, s.timers);
          worstClock = Math.max(worstClock, s.clock);
        }
      }
    }
    // 시뮬레이션이 공허하지 않다 — 적대 순서(실패·pending 교대)가 실제로 상한에 닿는다
    expect(worstTimers).toBe(MAX_TIMERS_AUTO);
    expect(worstClock).toBe(380_000);
  });
  test("응답 {실패,pending}^6 × CEO {활성,비활성}^6 전부", async () => {
    for (let fm = 0; fm < 1 << 6; fm++)
      for (let cm = 0; cm < 1 << 6; cm++) {
        const xs: Resp[] = [];
        for (let k = 0; k < 6; k++) xs.push(fm & (1 << k) ? "pending" : "null");
        const s = await simulate({ autoRoute: true, feed: seq(xs, "null"), ceo: (i) => !!(cm & (1 << (i % 6))) });
        checkInvariants(s, true);
      }
  });
});

// ---------- 배선 핀: main.ts 는 의존성을 꽂고 결과를 그리기만 한다(리뷰1 #1 — 본문 구간·정확한 호출형) ----------
const src = readFileSync(new URL("./main.ts", import.meta.url), "utf-8");
const css = readFileSync(new URL("./style.css", import.meta.url), "utf-8");
const code = src
  .split("\n")
  .map((l) => (l.trimStart().startsWith("//") ? "" : l.replace(/\s\/\/.*$/, "")))
  .join("\n");
const j = (...p: string[]) => p.join("");
/** 공백을 지운 비교 — 줄바꿈·들여쓰기 변경엔 둔감, 인자·호출형 변경엔 민감. */
const ns = (x: string) => x.replace(/\s+/g, "");
const count = (hay: string, needle: string) => hay.split(needle).length - 1;
/** code 안에서 start 로 시작해 end 직전까지(없으면 실패). */
function sliceBody(start: string, end: string): string {
  const a = code.indexOf(start);
  expect(a).toBeGreaterThan(0);
  const b = code.indexOf(end, a + start.length);
  expect(b).toBeGreaterThan(a);
  return code.slice(a, b);
}

describe("배선 핀 — main.ts · 경보", () => {
  test("probefail 을 들여온다", () => {
    expect(/from "\.\/probefail"/.test(code)).toBe(true);
  });
  test("계수기는 최상위 한 개 · setCcOpen 보다 위(첫 호출 전 초기화)", () => {
    const decl = "const alertsTracker = makeAlertsTracker();";
    expect(count(code, decl)).toBe(1);
    expect(code.indexOf(decl)).toBeLessThan(code.indexOf("function setCcOpen("));
  });
  test("refreshControlCenter: readAlerts 로 조회 → 계수기 step 결과를 그대로 그린다", () => {
    const body = ns(sliceBody("async function refreshControlCenter", "function updateCcStale"));
    expect(body.includes(ns('const alerts = await readAlerts(() => invoke("control_alerts"));'))).toBe(true);
    expect(body.includes(ns("renderAlerts(alertsTracker.step(alerts));"))).toBe(true);
  });
  test("계수기 step 은 main.ts 전체에서 한 번(이중 계수 금지) · 판정 함수를 main.ts 가 직접 부르지 않는다", () => {
    expect(count(code, "alertsTracker.step(")).toBe(1);
    expect(code.includes("alertsView(")).toBe(false);
    expect(code.includes("nextAlertsFailStreak(")).toBe(false);
    expect(/\balertsFailStreak\b/.test(code)).toBe(false); // 옛 main.ts 지역 계수기 부재
    expect(code.includes(j("a?.alerts ?", "? []"))).toBe(false);
  });
  test("renderAlerts 는 표시 값만 받는다", () => {
    expect(code.includes("function renderAlerts(v: AlertsView | null)")).toBe(true);
  });
  test("setCcOpen(true): 계수기 reset 이 refreshControlCenter() 보다 먼저(리뷰1 #7)", () => {
    const openBranch = sliceBody("function setCcOpen(", "} else {");
    const r = openBranch.indexOf("alertsTracker.reset();");
    const f = openBranch.indexOf("refreshControlCenter();");
    expect(r).toBeGreaterThan(0);
    expect(f).toBeGreaterThan(r);
  });
});

describe("배선 핀 — main.ts · 승인 전환", () => {
  test("스케줄러는 makeFeedSwitchScheduler 하나 — 실제 의존성을 꽂는다", () => {
    expect(count(code, "makeFeedSwitchScheduler(")).toBe(1);
    const block = ns(sliceBody("const feedSwitchScheduler = makeFeedSwitchScheduler({", "});"));
    expect(block.includes(ns("setTimer: (fn, ms) => setTimeout(fn, ms),"))).toBe(true);
    expect(block.includes(ns('feedList: () => invoke("feed_list", { status: null }),'))).toBe(true);
    expect(block.includes(ns("ceoActive: ceoIsActivelyGenerating,"))).toBe(true);
    expect(block.includes(ns("open: openFeed,"))).toBe(true);
  });
  test("scheduleFeedSwitchIfStillPending 는 위임 한 줄 — 자기 타이머·재예약·판정 0", () => {
    const body = sliceBody("function scheduleFeedSwitchIfStillPending(", "\n}\n");
    expect(ns(body)).toBe(
      ns("function scheduleFeedSwitchIfStillPending(requestId: string, autoRoute = false): void {\n  feedSwitchScheduler(requestId, autoRoute);"),
    );
  });
  test("판정·상수는 main.ts 에 없다(정의처 probefail.ts 하나)", () => {
    expect(code.includes("feedSwitchStep(")).toBe(false);
    expect(code.includes("feedSwitchPlan(")).toBe(false);
    expect(/const FEED_SWITCH_[A-Z_]+ =/.test(code)).toBe(false);
    expect(code.includes(j('if (item?.status !== "pending") ', "return;"))).toBe(false);
  });
  test("호출처는 승인 이벤트 하나 — auto_route 여부만 넘긴다", () => {
    expect(count(code, "scheduleFeedSwitchIfStillPending(")).toBe(2); // 정의 1 + 호출 1
    expect(code.includes('scheduleFeedSwitchIfStillPending(String(payload.request_id ?? ""), autoRoute);')).toBe(true);
  });
});

describe("style.css", () => {
  test("회색 '조회 불가' 배지·행 클래스", () => {
    expect(/\.cc-alert-badge\.unknown\s*\{/.test(css)).toBe(true);
    expect(/\.cc-alert-row\.unknown\s*\{/.test(css)).toBe(true);
    // [hidden] 짝 규칙은 이미 있다(보존 확인 — display 명시 셀렉터에 hidden 을 쓰는 규약).
    expect(/\.cc-alert-badge\[hidden\]\s*\{\s*display:\s*none/.test(css)).toBe(true);
  });
});
