// ui/src/seatsig.ts — ★RED 단계 전사본(0.14.41 · WP-A5).
// 이 파일은 현행 main.ts 인라인 판정(buildTab 집계 · refreshSidebarStatus state 산식 · ceoIsActivelyGenerating)을
// **문자 그대로** 옮긴 것이다. 목적은 하나 — seatsig.test.ts 가 '수정 전 동작'에서 실제로 어디가 틀리는지
// (신호 없음=초록 · 빈 자리=초록 · 낡은 자기보고=영구 working)를 단언 단위로 보여 주는 것.
// GREEN 커밋에서 정직화 구현으로 교체된다. 본체(main.ts)는 아직 이 모듈을 쓰지 않는다.

export const SIG_POLL_MS = 10_000;
export const SIG_STALE_MS = 3 * SIG_POLL_MS;
export const STATUS_STATE_FRESH_SECS = 600;
export const HOLLOW_QUIET_SECS = 20;

export type SeatSig = {
  role: string | null;
  state: string;
  ctx_pct: number | null;
  idle_secs: number;
  agent_alive: boolean | null;
  hollow: boolean;
  at: number;
};

export type WsDot = "working" | "idle" | "error" | "hollow" | "unknown";
export type WsSigSummary = {
  dot: WsDot;
  total: number;
  fresh: number;
  unknownN: number;
  hollowN: number;
  hollowRoles: string[];
  dead: number;
  idleN: number;
  worst: number;
  lastOkAt: number | null;
  title: string;
};

// 현행: `n.status?.state ?? (n.idle_secs > 60 ? "idle" : "working")`
export function seatState(n: any): string {
  return n.status?.state ?? (n.idle_secs > 60 ? "idle" : "working");
}

// 현행: 빈 자리 개념 없음.
export function isHollowSeat(_n: any): boolean {
  return false;
}

// 현행 buildTab: filter(Boolean) · dead ? error : idleN ? idle : working (수신 시각 무시)
export function summarizeWsSigs(sigsIn: ReadonlyArray<SeatSig | null | undefined>, _now: number, _staleMs = SIG_STALE_MS): WsSigSummary {
  const sigs = sigsIn.filter(Boolean) as SeatSig[];
  const worst = sigs.reduce((acc, s) => Math.max(acc, s.ctx_pct ?? 0), 0);
  const idleN = sigs.filter((s) => s.state === "idle" || s.idle_secs > 60).length;
  const dead = sigs.filter((s) => s.agent_alive === false).length;
  return {
    dot: dead ? "error" : idleN ? "idle" : "working",
    total: sigsIn.length,
    fresh: sigs.length,
    unknownN: 0,
    hollowN: 0,
    hollowRoles: [],
    dead,
    idleN,
    worst,
    lastOkAt: null,
    title: "",
  };
}

export function summarizeWsSigsSafe(sigs: ReadonlyArray<SeatSig | null | undefined>, now: number, staleMs = SIG_STALE_MS): WsSigSummary {
  return summarizeWsSigs(sigs, now, staleMs);
}

export function wsSubBits(s: WsSigSummary, firstTitle: string): string[] {
  const bits = [`${s.total} pane`];
  if (firstTitle) bits.push(firstTitle);
  if (s.worst >= 60) bits.push(`CTX ${s.worst}%`);
  if (s.idleN) bits.push(`💤${s.idleN}`);
  if (s.dead) bits.push(`❌${s.dead}`);
  return bits;
}

// 현행 ceoIsActivelyGenerating: alive = agent_alive !== false(null 은 산 것) · 낡음 무시
export function ceoSigActivity(sig: SeatSig, _now: number, _staleMs = SIG_STALE_MS): boolean | null {
  const alive = sig.agent_alive !== false;
  const working = sig.state === "working" || sig.idle_secs < 30;
  return alive && working;
}
