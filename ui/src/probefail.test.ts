// probefail.ts 순수 판정 회귀 테스트 + main.ts 배선 핀 (bun test — 신규 의존성 0).
// 0.14.41 · U4 A5 ②(경보 배지: 조회 실패가 '경보 0'으로 보임) · ③(승인 자동 전환: feed 조회 실패를 '결재됨'으로 접음).
//
// 근거: _evidence/impl-9items-20260923/phase1/U4c-silentpass-pack-ui{,.refute}.md F9·F10(R9 유지 · ① 재예약 상한 1 동의)
// 실패 방향(이 파일이 막는 것): 조회가 실패했는데 화면이 '없음'(경보 0 · 이미 결재됨)을 그려 사람이 봐야 할 것이
//   조용히 묻히는 것. 반대로 한두 번의 일시 실패로 배지가 깜빡이거나(상한 3), 재시도가 무한히 도는 것(상한 1)도 막는다.
import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import {
  ALERTS_FAIL_LIMIT,
  FEED_LOOKUP_RETRY_MS,
  alertsListOf,
  alertsView,
  feedSwitchStep,
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
  test("재시도 상한 1 — 계속 실패하는 조회를 흉내 내면 정확히 1회 재시도 뒤 open(무한 재예약 0)", () => {
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
    expect(FEED_LOOKUP_RETRY_MS).toBeLessThanOrEqual(10_000);
  });
});

// ---------- 배선 핀 ----------
const src = readFileSync(new URL("./main.ts", import.meta.url), "utf-8");
const css = readFileSync(new URL("./style.css", import.meta.url), "utf-8");
const code = src
  .split("\n")
  .map((l) => (l.trimStart().startsWith("//") ? "" : l.replace(/\s\/\/.*$/, "")))
  .join("\n");
const j = (...p: string[]) => p.join("");

describe("배선 핀 — main.ts", () => {
  test("probefail 을 들여온다", () => {
    expect(/from "\.\/probefail"/.test(code)).toBe(true);
  });
  test("경보: 실패 계수 · 미측정 판독 경유 · 옛 '없으면 빈 목록' 접기 부재", () => {
    const a = code.indexOf("async function refreshControlCenter");
    const b = code.indexOf("function updateCcStale", a);
    expect(a).toBeGreaterThan(0);
    const body = code.slice(a, b);
    expect(body.includes("alertsListOf(")).toBe(true);
    expect(body.includes("alertsFailStreak")).toBe(true);
    expect(code.includes("alertsView(")).toBe(true);
    expect(code.includes(j("a?.alerts ?", "? []"))).toBe(false);
  });
  test("승인 전환: feedSwitchStep 경유 · 옛 'pending 아니면 return' 접기 부재 · 재시도는 짧은 간격", () => {
    const a = code.indexOf("function scheduleFeedSwitchIfStillPending");
    const b = code.indexOf("let ftOpen", a);
    expect(a).toBeGreaterThan(0);
    expect(b).toBeGreaterThan(a);
    const body = code.slice(a, b);
    expect(body.includes("feedSwitchStep(")).toBe(true);
    expect(body.includes("FEED_LOOKUP_RETRY_MS")).toBe(true);
    expect(code.includes(j('if (item?.status !== "pending") ', "return;"))).toBe(false);
  });
  test("style.css — 회색 '조회 불가' 배지·행 클래스", () => {
    expect(/\.cc-alert-badge\.unknown\s*\{/.test(css)).toBe(true);
    expect(/\.cc-alert-row\.unknown\s*\{/.test(css)).toBe(true);
    // [hidden] 짝 규칙은 이미 있다(보존 확인 — display 명시 셀렉터에 hidden 을 쓰는 규약).
    expect(/\.cc-alert-badge\[hidden\]\s*\{\s*display:\s*none/.test(css)).toBe(true);
  });
});
