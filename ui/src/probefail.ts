// ui/src/probefail.ts — ★RED 단계 전사본(0.14.41 · WP-A5).
// 현행 main.ts 인라인 판정(renderAlerts 의 `a?.alerts ?? []` · scheduleFeedSwitchIfStillPending 의
// `r?.items.find(...)` → pending 아니면 return)을 문자 그대로 옮긴 것이다. probefail.test.ts 가 '수정 전 동작'이
// 조회 실패를 '없음'으로 접는 자리를 단언 단위로 보이게 하려는 목적 — GREEN 커밋에서 교체된다.

export const ALERTS_FAIL_LIMIT = 3;
export const FEED_LOOKUP_RETRY_MS = 5_000;

export type AlertSev = "warn" | "crit" | "unknown";
export type AlertRow = { sev: AlertSev; icon: string; msg: string };
export type AlertsView = { hidden: boolean; text: string; cls: AlertSev; title: string; rows: AlertRow[] };

// 현행: `a?.alerts ?? []`
export function alertsListOf(a: any): any[] | null {
  return a?.alerts ?? [];
}

// 현행 renderAlerts: 실패 계수 없음(조회 실패는 catch 에서 무음 — 표시가 그대로 남는다)
export function alertsView(listIn: any[] | null, _failStreak: number): AlertsView | null {
  const list: any[] = listIn ?? [];
  const crit = list.filter((x) => x.severity === "crit").length;
  return {
    hidden: list.length === 0,
    text: list.length ? `⚠ ${list.length}` : "",
    cls: crit > 0 ? "crit" : "warn",
    title: "",
    rows: list.map((x) => ({
      sev: x.severity === "crit" ? "crit" : "warn",
      icon: x.severity === "crit" ? "🔴" : "🟠",
      msg: String(x.message ?? x.kind ?? ""),
    })),
  };
}

export type FeedSwitchStep = "settled" | "pending" | "retry" | "open";
// 현행: `const item = r?.items.find(...)`; `if (item?.status !== "pending") return;`
export function feedSwitchStep(r: any, requestId: string, _lookupRetried: boolean): FeedSwitchStep {
  const item = r?.items?.find?.((i: any) => i.request_id === requestId);
  return item?.status !== "pending" ? "settled" : "pending";
}
