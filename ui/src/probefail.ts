// ui/src/probefail.ts — '조회 실패'를 '없음'으로 접지 않는 화면 판정(0.14.41 · U4 A5 ②③).
//
// 근거: _evidence/impl-9items-20260923/phase1/U4c-silentpass-pack-ui{,.refute}.md F9·F10(R9 유지).
//   ② 경보 배지: 종전 renderAlerts 는 `a?.alerts ?? []` 라 필드 누락이 '경보 0'이 됐고, control_alerts 호출
//      실패는 catch 에서 무음이라 연속 실패해도 배지는 숨겨진 채(=경보 없음)였다. CC stale 배너(ccFailStreak)는
//      control_dashboard 실패만 센다 — 대시보드는 되는데 경보 조회만 실패하는 경우가 비었다.
//   ③ 승인 자동 전환: 종전 scheduleFeedSwitchIfStillPending 은 feed_list 실패(null)를 `item?.status !== "pending"`
//      으로 '이미 결재됨'과 같이 접어 조용히 끝냈다 — 사람 승인이 필요한 항목이 전환 없이 묻힌다.
// 원칙: 표시·전환 시점만 바꾼다(새 RPC 0 · 새 주기 0). 재시도 상한 1(타이머 1개) — 폭주 경로 없음.
// 모듈 최상위 부수효과 0 · DOM/Tauri 무관(bun test 대상).

/** 경보 조회 연속 실패 이 횟수부터 '경보 조회 불가' 배지(CC stale 배너 ccFailStreak ≥3 과 같은 문턱). */
export const ALERTS_FAIL_LIMIT = 3;
/** 승인 전환 판정의 feed_list 조회가 실패했을 때 한 번 더 볼 때까지의 간격(유예를 통째로 다시 기다리지 않는다). */
export const FEED_LOOKUP_RETRY_MS = 5_000;

export type AlertSev = "warn" | "crit" | "unknown";
export type AlertRow = { sev: AlertSev; icon: string; msg: string };
export type AlertsView = { hidden: boolean; text: string; cls: AlertSev; title: string; rows: AlertRow[] };

/** control_alerts 응답에서 경보 배열을 꺼낸다. 배열이 아니면 null(미측정) — 빈 목록으로 접지 않는다. */
export function alertsListOf(a: any): any[] | null {
  const l = a?.alerts;
  return Array.isArray(l) ? l : null;
}

/**
 * 경보 배지·스트립 표시. list=null 은 이번 조회 실패(호출 실패 또는 미측정 응답).
 * 반환 null = 아무것도 바꾸지 않는다(실패가 아직 ALERTS_FAIL_LIMIT 미만 — 직전 표시 유지, 일시 실패로 깜빡이지 않음).
 */
export function alertsView(list: any[] | null, failStreak: number): AlertsView | null {
  if (list === null) {
    if (!(failStreak >= ALERTS_FAIL_LIMIT)) return null;
    const msg = `경보 조회 불가 — control.alerts 연속 ${failStreak}회 실패 · 경보가 없는 것이 아니라 확인하지 못한 상태입니다(자동 재시도 중)`;
    return {
      hidden: false,
      text: "⚠ 경보 조회 불가",
      cls: "unknown",
      title: msg,
      rows: [{ sev: "unknown", icon: "⚪", msg }],
    };
  }
  const crit = list.filter((x) => x?.severity === "crit").length;
  return {
    hidden: list.length === 0,
    text: list.length ? `⚠ ${list.length}` : "",
    cls: crit > 0 ? "crit" : "warn",
    title: "",
    rows: list.map((x) => ({
      sev: x?.severity === "crit" ? "crit" : "warn",
      icon: x?.severity === "crit" ? "🔴" : "🟠",
      msg: String(x?.message ?? x?.kind ?? ""),
    })),
  };
}

export type FeedSwitchStep = "settled" | "pending" | "retry" | "open";
/**
 * 유예 뒤 feed_list 결과로 다음 걸음을 정한다.
 *   settled = 결재됨/목록에 없음(종전 — 전환 없음) · pending = 아직 대기(종전 전환 경로)
 *   retry   = 조회 실패(null·items 비배열) 첫 회 → FEED_LOOKUP_RETRY_MS 뒤 1회만 다시 본다
 *   open    = 재시도도 실패 → 승인 Feed 를 연다(보여 주는 쪽으로 실패)
 */
export function feedSwitchStep(r: any, requestId: string, lookupRetried: boolean): FeedSwitchStep {
  const items = r?.items;
  if (!Array.isArray(items)) return lookupRetried ? "open" : "retry";
  const item = items.find((i: any) => i?.request_id === requestId);
  return item?.status === "pending" ? "pending" : "settled";
}
