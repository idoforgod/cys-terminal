// ui/src/probefail.ts — '조회 실패'를 '없음'으로 접지 않는 화면 판정(0.14.41 · U4 A5 ②③).
//
// 근거: _evidence/impl-9items-20260923/phase1/U4c-silentpass-pack-ui{,.refute}.md F9·F10(R9 유지).
//   ② 경보 배지: 종전 renderAlerts 는 `a?.alerts ?? []` 라 필드 누락이 '경보 0'이 됐고, control_alerts 호출
//      실패는 catch 에서 무음이라 연속 실패해도 배지는 숨겨진 채(=경보 없음)였다. CC stale 배너(ccFailStreak)는
//      control_dashboard 실패만 센다 — 대시보드는 되는데 경보 조회만 실패하는 경우가 비었다.
//   ③ 승인 자동 전환: 종전 scheduleFeedSwitchIfStillPending 은 feed_list 실패(null)를 `item?.status !== "pending"`
//      으로 '이미 결재됨'과 같이 접어 조용히 끝냈다 — 사람 승인이 필요한 항목이 전환 없이 묻힌다.
// 원칙: 표시·전환 시점만 바꾼다(새 RPC 0 · 새 주기 0). 재시도 상한 1(타이머 1개) — 폭주 경로 없음.
// ★리뷰1 #1(배선 공허) 수리: 틱마다 내리는 결정(경보 실패 계수·리셋, 승인 전환의 재예약·연장·상한)을 **전부 이 모듈**로
//   올렸다. main.ts 는 의존성(invoke·setTimeout·openFeed·ceoIsActivelyGenerating)을 꽂고 결과를 그리기만 한다 —
//   그래서 bun test 가 실제 재예약 코드를 가짜 타이머로 끝까지 돌려 타이머 수·총 경과 상한을 잰다(probefail.test).
// 모듈 최상위 부수효과 0 · DOM/Tauri 무관(bun test 대상).

/** 경보 조회 연속 실패 이 횟수부터 '경보 조회 불가' 배지(CC stale 배너 ccFailStreak ≥3 과 같은 문턱). */
export const ALERTS_FAIL_LIMIT = 3;
/** 승인 전환 판정의 feed_list 조회가 실패했을 때 한 번 더 볼 때까지의 간격(유예를 통째로 다시 기다리지 않는다). */
export const FEED_LOOKUP_RETRY_MS = 5_000;
// 승인 자동 화면전환 유예(main.ts 에서 이동 — 시뮬레이션이 실제 값으로 상한을 재도록 정의처를 여기 하나로 둔다).
// master/CEO 가 이 시간 안에 자동 승인(reply)하면 전환하지 않는다. 유예 후에도 pending = 사람 수동 승인 필요 → 그때만 전환.
/** 비대상(현행) 유예. */
export const FEED_SWITCH_GRACE_MS = 30_000;
/** W3.4: auto_route 항목은 CEO 심의형 turn 이 90초 초과가 흔하므로 기본 유예를 90초로 둔다. */
export const FEED_SWITCH_GRACE_AUTO_MS = 90_000;
/** approval.stalled(5분) 경로가 사람 소환을 담당하므로 그 전까지만 동적 연장한다(그 뒤엔 escalation). */
export const FEED_SWITCH_MAX_MS = 300_000;

export type AlertSev = "warn" | "crit" | "unknown";
export type AlertRow = { sev: AlertSev; icon: string; msg: string };
export type AlertsView = { hidden: boolean; text: string; cls: AlertSev; title: string; rows: AlertRow[] };

/** control_alerts 응답에서 경보 배열을 꺼낸다. 배열이 아니면 null(미측정) — 빈 목록으로 접지 않는다. */
export function alertsListOf(a: any): any[] | null {
  const l = a?.alerts;
  return Array.isArray(l) ? l : null;
}

/** control_alerts 한 번 조회 → 경보 배열, 또는 null(호출 거부·동기 throw·미측정 응답). 예외를 밖으로 내지 않는다. */
export async function readAlerts(call: () => Promise<unknown>): Promise<any[] | null> {
  try {
    return alertsListOf(await call());
  } catch {
    return null;
  }
}

/** 경보 조회 연속 실패 수의 다음 값 — 실패(null)면 +1, 성공(배열이면 빈 목록도)이면 0. */
export function nextAlertsFailStreak(prev: number, list: any[] | null): number {
  return list === null ? prev + 1 : 0;
}

/**
 * 경보 실패 계수기 — 계수·리셋·표시 판정을 한 곳에(main.ts 는 step 결과를 그리기만 한다).
 *   step(list) : 이번 조회 결과를 반영하고 그릴 것을 돌려준다(null = 직전 표시 유지)
 *   reset()    : Control Center 를 다시 열 때 — 닫혀 있던 동안의 옛 실패가 재개 직후 일시 실패 1회와 합산돼
 *                곧바로 '조회 불가'가 뜨지 않게(리뷰1 #7)
 */
export type AlertsTracker = { step(list: any[] | null): AlertsView | null; reset(): void; streak(): number };
export function makeAlertsTracker(): AlertsTracker {
  let streak = 0;
  return {
    step(list) {
      streak = nextAlertsFailStreak(streak, list);
      return alertsView(list, streak);
    },
    reset() {
      streak = 0;
    },
    streak() {
      return streak;
    },
  };
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

/** 이번 타이머의 유예 — 조회 재시도면 짧게, auto_route 면 90초, 아니면 30초. */
export function feedSwitchGraceMs(autoRoute: boolean, lookupRetried: boolean): number {
  return lookupRetried ? FEED_LOOKUP_RETRY_MS : autoRoute ? FEED_SWITCH_GRACE_AUTO_MS : FEED_SWITCH_GRACE_MS;
}

/** CEO 활성으로 유예를 연장할 수 있는 걸음인가 — pending ∧ auto_route ∧ 상한 이내일 때만.
 *  조회 2회 실패(open)는 연장하지 않는다(리뷰1 #1 M25). 이 값이 false 면 CEO 판정(org_status 재조회)도 하지 않는다. */
export function feedSwitchMayExtend(step: FeedSwitchStep, autoRoute: boolean, elapsedMs: number, grace: number): boolean {
  return step === "pending" && autoRoute && elapsedMs + grace < FEED_SWITCH_MAX_MS;
}

export type FeedSwitchPlan =
  | { kind: "end" }
  | { kind: "open" }
  | { kind: "reschedule"; autoRoute: boolean; elapsedMs: number; lookupRetried: boolean };

/**
 * 한 틱의 결정. settled → end(전환 없음) · retry → 짧은 간격으로 **재시도 표식을 단 채** 재예약(상한 1) ·
 * 연장 가능 ∧ CEO 활성 → 재시도 표식을 푼 채 재예약(성공 조회가 있었으므로) · 그 밖 → open(사람에게 보인다).
 */
export function feedSwitchPlan(
  step: FeedSwitchStep,
  o: { autoRoute: boolean; elapsedMs: number; grace: number; ceoActive: boolean },
): FeedSwitchPlan {
  if (step === "settled") return { kind: "end" };
  if (step === "retry")
    return { kind: "reschedule", autoRoute: o.autoRoute, elapsedMs: o.elapsedMs + o.grace, lookupRetried: true };
  if (feedSwitchMayExtend(step, o.autoRoute, o.elapsedMs, o.grace) && o.ceoActive === true)
    return { kind: "reschedule", autoRoute: true, elapsedMs: o.elapsedMs + o.grace, lookupRetried: false };
  return { kind: "open" };
}

/** 승인 자동 전환 스케줄러의 바깥 세계. main.ts 가 실제 것을 꽂고, 테스트는 가짜 타이머·가짜 조회를 꽂는다. */
export type FeedSwitchDeps = {
  /** 타이머 예약(main.ts = setTimeout). 콜백은 끝날 때 resolve 되는 promise 를 돌려준다(테스트가 기다린다). */
  setTimer: (fn: () => Promise<void>, ms: number) => unknown;
  /** feed_list 조회(main.ts = invoke("feed_list", {status:null})). reject·동기 throw·비배열 모두 '조회 실패'. */
  feedList: () => Promise<unknown>;
  /** CEO 가 살아서 생성 중인가(main.ts = ceoIsActivelyGenerating). reject·throw 는 false(보여 주는 쪽으로 실패). */
  ceoActive: () => Promise<boolean>;
  /** 승인 Feed 열기(main.ts = openFeed). */
  open: () => void;
};

/**
 * 승인 자동 전환 스케줄러(main.ts scheduleFeedSwitchIfStillPending 의 본체). 요청 하나에 대해 **언제나 타이머 1개**만
 * 걸려 있다(다음 타이머는 앞 타이머의 콜백 안에서만 건다). 상한(코드에 박힘):
 *   - 조회 재시도는 연속 1회(lookupRetried) — 두 번 연속 실패하면 open
 *   - 연장은 elapsed+grace < FEED_SWITCH_MAX_MS 일 때만 · 한 번 넘어갈 때마다 elapsed 가 5초 이상 는다
 *   → auto_route 최악 8타이머 · 총 경과 ≤ MAX + GRACE_AUTO + RETRY(probefail.test 전수 시뮬레이션이 잰다).
 */
export function makeFeedSwitchScheduler(d: FeedSwitchDeps) {
  const schedule = (requestId: string, autoRoute = false, elapsedMs = 0, lookupRetried = false): void => {
    if (!requestId) return;
    const grace = feedSwitchGraceMs(autoRoute, lookupRetried);
    const tick = async (): Promise<void> => {
      let r: unknown = null;
      try {
        r = await d.feedList();
      } catch {
        r = null;
      }
      const step = feedSwitchStep(r, requestId, lookupRetried);
      let ceo = false;
      if (feedSwitchMayExtend(step, autoRoute, elapsedMs, grace)) {
        try {
          ceo = (await d.ceoActive()) === true;
        } catch {
          ceo = false;
        }
      }
      const plan = feedSwitchPlan(step, { autoRoute, elapsedMs, grace, ceoActive: ceo });
      if (plan.kind === "reschedule") schedule(requestId, plan.autoRoute, plan.elapsedMs, plan.lookupRetried);
      else if (plan.kind === "open") {
        try {
          d.open();
        } catch {
          /* 표시 실패 — 승인 대기 배지·approval.stalled 경로가 남는다 */
        }
      }
    };
    d.setTimer(tick, grace);
  };
  return schedule;
}
