// U1 사이드바 사용량 패널(#wsbar-usage) — 순수 판정·표기 모듈 (DOM·Tauri·저장소 무관).
//
// 오너 요청(2026-09-23): "왼쪽에 사용량 표시". 계정별 5시간·7일 한도 사용률은 지금까지 Control Center
// Live 탭 안에만 있었다. 데이터는 이미 `usage_accounts_all`(본부+부서 데몬 병합)로 나오므로 새 RPC·Rust
// 변경 없이 사이드바 바닥에 붙인다. main.ts 는 조회·DOM 배선만 하고, 무엇을 어떻게 보일지는 여기서 정한다.
//
// ★정직한 표기 규칙(설계 §3 U1 · 반박 보고서 반영):
//   · 주 계정은 **관측 출처**로 고른다 — 라이브 관측(statusline·rollout·agy-rpc)이 스냅샷보다 우선이고,
//     라이브끼리는 5h 사용률이 높은 쪽(한도 임박 경보 목적). 좌석 경로 정규식(`.cys/claude…`)은 **동률
//     해소용**일 뿐이다 — 카탈로그 부서는 계정 폴더가 `.claude-2` 처럼 좌석 경로가 아니다(반박 D1).
//   · 라벨은 프로필명(claude-N / 좌석 / 부서 x / 제공자)만. 이메일은 툴팁에만 두고 🔒 가림을 따른다 —
//     사이드바는 늘 화면에 떠 있어 화면 공유·스크린샷에 그대로 찍힌다(반박 D2).
//   · 100% 초과는 "100%+", 창이 없으면 0% 가 아니라 "—", 리셋 시각이 지났으면 옛 % 를 숨긴다(리셋 뒤에도
//     빨간 78% 가 남는 오경보 차단), 오래된 값은 흐리게, 조회가 3회 연속 실패하면 "데몬 응답 없음".
//
// ★이 모듈의 불변식(usagewiring.test.ts 가 핀으로 고정):
//   · 최상위 부수효과 0 — 선언(export/const/function/type)만. localStorage·document·window·타이머 접근 0.
//     main.js 는 번들 하나라 여기서 평가 중 예외가 나면 앱 전체가 백지가 된다(④).
//   · 구형 WKWebView 가 파싱하지 못하는 문법 0 — 정규식 lookbehind, `.at(`, findLast, structuredClone,
//     Object.hasOwn, replaceAll. `bun build --target browser` 는 다운레벨하지 않으므로 파싱 실패가 곧 백지다.

/** usage.accounts 응답의 창 하나(`accounts.rs` RateWindow 직렬화). */
export interface AcctRateWindow {
  label: string;
  used_pct: number;
  resets_at: number | null;
}
/** usage_accounts_all 병합 행(`accounts.rs::local_json` 계약). IPC 데이터라 모든 필드를 의심한다. */
export interface AcctRow {
  provider?: string;
  account_id?: string;
  label?: string; // claude=이메일 · codex="OpenAI Codex" · agy="Antigravity (agy)" — 화면 라벨로 쓰지 않는다
  plan?: string | null;
  profiles?: string[];
  rate?: AcctRateWindow[];
  updated_at?: number | null; // epoch 초 · null = 관측 전(발견만)
  stale_secs?: number | null;
  source?: string; // "statusline" | "rollout" | "agy-rpc" | "adapter:<p>" | "snapshot"(부트 예열)
  adapter?: boolean;
  exhaust_at?: number | null; // 신선한 5h 창의 선형 소진 예측(epoch 초)
}

export const USAGE_WARN_PCT = 70; // pane 헤더 배지·CC 계정 섹션과 같은 선(main.ts sevClass(…, 70, 90))
export const USAGE_CRIT_PCT = 90;
/** 이보다 오래된 관측은 "N분 전 관측"을 붙인다 — CC 계정 섹션의 120초 배지와 같은 선. */
export const USAGE_RECENT_SECS = 120;
/** 이보다 오래된 관측은 흐리게(오래된 값) — 5h 창 안에서도 30분이면 실제와 크게 어긋날 수 있다. */
export const USAGE_STALE_SECS = 30 * 60;
/** 패널이 보여 주는 창(순서 고정). codex 는 5h 가 없고 7d 만 있다 → "5h —". */
export const USAGE_WINDOWS = ["5h", "7d"];
/** 연속 실패가 이 횟수에 닿으면 "데몬 응답 없음" — CC 의 ccFailStreak(3틱)과 같은 선. */
export const USAGE_FAIL_STREAK_WARN = 3;
/** 주 계정 밖의 관측 계정은 이 수까지만 한 줄씩 — 넘치면 "외 N개"(꼬리 높이 상한). */
export const USAGE_OTHERS_MAX = 4;

const p2 = (x: number): string => String(x).padStart(2, "0");
const isObj = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null;
const finiteNum = (v: unknown): number | null => {
  if (v === null || v === undefined || v === "") return null;
  const n = Number(v);
  return Number.isFinite(n) ? n : null;
};

/** epoch 초 → 로컬 "HH:MM" (값이 이상하면 빈 문자열). */
export function hhmm(epochSec: number): string {
  if (!Number.isFinite(epochSec) || epochSec <= 0) return "";
  const d = new Date(epochSec * 1000);
  return `${p2(d.getHours())}:${p2(d.getMinutes())}`;
}

/** 프로필 표기 정규화: 역슬래시 → 슬래시, 끝 슬래시 제거. 윈도우는 seed 경로(`\`)와 statusline 경로(`/`)가
 *  섞여 같은 프로필이 두 줄로 오므로(usage_accounts_all 의 dedup 은 문자열 완전일치) 여기서 접는다. */
export function normalizeProfile(p: string): string {
  return p.replace(/\\/g, "/").replace(/\/+$/, "");
}
/** 🔒 가림용 짧은 표기 — 좌석·부서 폴더는 `.cys/<이름>`, 그 밖은 끝 이름(`.claude-2`). 윈도우에서 프로필이
 *  절대경로(`C:\Users\<OS 사용자명>\…`)로 오면 원문 그대로는 사용자명이 툴팁에 찍힌다(리뷰1 M9). */
export function profileTail(p: string): string {
  const n = normalizeProfile(p);
  const cys = /(?:^|\/)(\.cys\/[^/]+)$/.exec(n);
  if (cys) return cys[1];
  const i = n.lastIndexOf("/");
  return i >= 0 ? n.slice(i + 1) : n;
}
export function normalizeProfiles(ps: unknown): string[] {
  if (!Array.isArray(ps)) return [];
  const set = new Set<string>();
  for (const p of ps) if (typeof p === "string" && p.trim()) set.add(normalizeProfile(p.trim()));
  return [...set].sort();
}

/** 프로필 하나의 화면 라벨과 우선순위(작을수록 먼저). 해당 없으면 null. 비앵커 — 윈도우 절대경로도 받는다. */
function profileLabel(p: string): { rank: number; label: string } | null {
  const n = normalizeProfile(p);
  const home = /(?:^|\/)\.(claude(?:-[^/]+)?)$/.exec(n);
  if (home) return { rank: 0, label: home[1] };
  if (/(?:^|\/)\.cys\/claude$/.test(n)) return { rank: 1, label: "좌석" };
  const dept = /(?:^|\/)\.cys\/claude-([^/]+)$/.exec(n);
  if (dept) return { rank: 2, label: "부서 " + dept[1].replace(/^default-/, "") };
  return null;
}

/** 제공자 표시명 — 프로필이 없을 때의 라벨. */
export function providerLabel(provider: unknown): string {
  const p = typeof provider === "string" ? provider : "";
  if (p === "claude") return "Claude";
  if (p === "codex") return "Codex";
  if (p === "gemini") return "agy";
  return p || "계정";
}

/** 계정의 화면 라벨 — 프로필명(claude-N > 좌석 > 부서 x) > 제공자. **이메일(label 필드)은 절대 쓰지 않는다.** */
export function accountShortLabel(a: AcctRow): string {
  let best: { rank: number; label: string } | null = null;
  for (const p of normalizeProfiles(a.profiles)) {
    const pl = profileLabel(p);
    if (!pl) continue;
    if (!best || pl.rank < best.rank || (pl.rank === best.rank && pl.label < best.label)) best = pl;
  }
  return best ? best.label : providerLabel(a.provider);
}

/** 좌석(또는 부서 포크) 폴더를 쓰는 계정인가 — 동률 해소용. 비앵커·두 구분자. */
export function hasSeatProfile(a: AcctRow): boolean {
  const ps = Array.isArray(a.profiles) ? a.profiles : [];
  return ps.some((p) => typeof p === "string" && /(?:^|[\\/])\.cys[\\/]claude(?:-[^\\/]+)?[\\/]?$/.test(p));
}

/** 관측된 적이 있는가(updated_at 이 유효한 epoch). */
export function isObserved(a: AcctRow): boolean {
  const u = finiteNum(a.updated_at);
  return u !== null && u > 0;
}
/** 라이브 관측인가 — 부트 스냅샷 예열(source "snapshot")·출처 빈값은 아니다. */
export function isLiveAccount(a: AcctRow): boolean {
  return isObserved(a) && typeof a.source === "string" && a.source !== "" && a.source !== "snapshot";
}

export interface WindowView {
  label: string;
  pct: number | null; // 게이지 폭(0~100). null = 표시할 값 없음(누락·리셋 지남)
  text: string; // "78%" | "100%+" | "—" | "리셋됨"
  sev: "" | "warn" | "crit";
  resetText: string;
  state: "ok" | "missing" | "rolled";
}

function resetLabel(label: string, epoch: number): string {
  const d = new Date(epoch * 1000);
  if (label === "5h") return `리셋 ${p2(d.getHours())}:${p2(d.getMinutes())}`;
  if (label === "7d") return `리셋 ${p2(d.getMonth() + 1)}/${p2(d.getDate())}`;
  return `리셋 ${p2(d.getMonth() + 1)}/${p2(d.getDate())} ${p2(d.getHours())}:${p2(d.getMinutes())}`;
}

/** 창 하나의 표기. 누락="—" · 리셋 지남=값 숨김 · 0~100 클램프(초과는 "100%+"). */
export function windowView(a: AcctRow, label: string, nowSec: number): WindowView {
  const rate = Array.isArray(a.rate) ? a.rate : [];
  const w = rate.find((x) => isObj(x) && x.label === label);
  const missing: WindowView = { label, pct: null, text: "—", sev: "", resetText: "", state: "missing" };
  if (!w) return missing;
  const used = finiteNum(w.used_pct);
  if (used === null) return missing;
  const resets = finiteNum(w.resets_at);
  if (resets !== null && resets > 0 && nowSec >= resets)
    return { label, pct: null, text: "리셋됨", sev: "", resetText: "재관측 대기", state: "rolled" };
  const pct = Math.max(0, Math.min(100, Math.round(used)));
  return {
    label,
    pct,
    text: used > 100 ? "100%+" : `${pct}%`,
    sev: used >= USAGE_CRIT_PCT ? "crit" : used >= USAGE_WARN_PCT ? "warn" : "",
    resetText: resets !== null && resets > 0 ? resetLabel(label, resets) : "",
    state: "ok",
  };
}

export interface Freshness {
  level: "fresh" | "recent" | "stale" | "never";
  note: string;
}
function ageText(secs: number): string {
  if (secs < 3600) return `${Math.max(1, Math.floor(secs / 60))}분 전`;
  if (secs < 86400) return `${Math.floor(secs / 3600)}시간 전`;
  return `${Math.floor(secs / 86400)}일 전`;
}
/** 관측 신선도 — 응답의 updated_at(epoch 초)과 로컬 시계(같은 기계)로 계산해 재조회 없이 전진한다. */
export function freshness(a: AcctRow, nowSec: number): Freshness {
  const u = finiteNum(a.updated_at);
  if (u === null || u <= 0) return { level: "never", note: "관측 없음" };
  const age = Math.max(0, nowSec - u);
  if (a.source === "snapshot") return { level: "stale", note: `지난 기록 · ${ageText(age)}` };
  if (age >= USAGE_STALE_SECS) return { level: "stale", note: `${ageText(age)} 관측` };
  if (age >= USAGE_RECENT_SECS) return { level: "recent", note: `${ageText(age)} 관측` };
  return { level: "fresh", note: "" };
}

/** 순위용 사용률 — 표시 가능한 값(0~100 클램프)만, 누락·리셋 지남은 -1(값 있는 쪽 아래로). */
const rankPct = (a: AcctRow, label: string, nowSec: number): number => windowView(a, label, nowSec).pct ?? -1;
const acctKey = (a: AcctRow): string => `${String(a.provider ?? "")}:${String(a.account_id ?? "")}`;

/** 주 계정: 라이브 관측 > 스냅샷. 같은 무리 안에서 5h ↓ → 7d ↓ → 최신 관측 ↓ → 좌석 경로 → 키(결정론). */
export function pickPrimaryAccount(accounts: AcctRow[], nowSec: number): AcctRow | null {
  const list = (Array.isArray(accounts) ? accounts : []).filter((a) => isObj(a) && isObserved(a));
  const live = list.filter(isLiveAccount);
  const pool = live.length ? live : list;
  if (!pool.length) return null;
  const sorted = [...pool].sort((x, y) => {
    const d5 = rankPct(y, "5h", nowSec) - rankPct(x, "5h", nowSec);
    if (d5) return d5;
    const d7 = rankPct(y, "7d", nowSec) - rankPct(x, "7d", nowSec);
    if (d7) return d7;
    const du = (finiteNum(y.updated_at) ?? 0) - (finiteNum(x.updated_at) ?? 0);
    if (du) return du;
    const ds = Number(hasSeatProfile(y)) - Number(hasSeatProfile(x));
    if (ds) return ds;
    return acctKey(x) < acctKey(y) ? -1 : acctKey(x) > acctKey(y) ? 1 : 0;
  });
  return sorted[0];
}

/** 결정론 짧은 꼬리표(djb2 + 마무리 섞기) — 라벨이 겹치는 두 계정을 구분할 때만 쓴다.
 *  djb2 는 마지막 글자 차이가 하위 비트에만 남아 상위 4자리가 같아지므로(예: id x1·x2) 섞은 뒤 자른다. */
function tag4(s: string): string {
  let h = 5381;
  for (let i = 0; i < s.length; i++) h = ((h << 5) + h + s.charCodeAt(i)) >>> 0;
  h = Math.imul(h ^ (h >>> 16), 0x45d9f3b) >>> 0;
  h = (h ^ (h >>> 16)) >>> 0;
  return h.toString(16).padStart(8, "0").slice(-4);
}

export interface UsageLine {
  label: string;
  text: string;
  tooltip: string;
  dim: boolean;
}
export interface UsagePrimary {
  label: string;
  tooltip: string;
  windows: WindowView[];
  fresh: Freshness;
  exhaust: string;
}
export interface UsageBarModel {
  /** 접힘 상태·헤더 요약 한 줄. */
  headline: string;
  primary: UsagePrimary | null;
  others: UsageLine[];
  moreCount: number;
  unobservedCount: number;
  unobservedTooltip: string;
  /** 주 계정이 없을 때 본문 대신 보일 한 줄(빈 문자열이면 없음). */
  message: string;
  /** 조회 연속 실패 경고(빈 문자열이면 없음) — 값은 지우지 않고 기준 시각을 밝힌다. */
  footer: string;
}
export interface UsageFetchState {
  everOk: boolean; // 한 번이라도 조회에 성공했는가
  failStreak: number;
  okAtSec: number | null; // 마지막 성공 시각(epoch 초)
}

/** 집계 범위 고지 — 반박 §5-8: 기본 `claude`(~/.claude)와 외부 터미널 세션은 계정에 집계되지 않는다. */
export const USAGE_SCOPE_NOTE =
  "집계 범위: cys 창 안에서 계정 전용 설정 폴더(CLAUDE_CONFIG_DIR)로 띄운 세션만 계정에 모입니다 — 기본 claude·외부 터미널 세션은 빠집니다.";

function tooltipFor(
  a: AcctRow,
  views: WindowView[],
  fr: Freshness,
  redactEmail: (s: string) => string,
  hidePaths: boolean,
): string {
  const lines: string[] = [];
  // 신원 줄 — 라벨(이메일)이 비면 account_id 가 나오므로 그것도 가림 함수를 거친다(CC 계정 섹션과 같은 규칙).
  const whoRaw = typeof a.label === "string" && a.label ? a.label : String(a.account_id ?? "");
  lines.push(`${providerLabel(a.provider)} 계정 — ${whoRaw ? redactEmail(whoRaw) : "?"}`);
  if (typeof a.plan === "string" && a.plan) lines.push(`요금제: ${a.plan}`);
  // 🔒 가림이면 경로는 끝 이름만(가린 뒤 같아진 줄은 다시 접는다) — 사이드바는 늘 화면에 떠 있어 공유 화면에 찍힌다.
  const profs = hidePaths ? normalizeProfiles(normalizeProfiles(a.profiles).map(profileTail)) : normalizeProfiles(a.profiles);
  if (profs.length) lines.push(`설정 폴더: ${profs.join(", ")}`);
  const u = finiteNum(a.updated_at);
  if (u !== null && u > 0) lines.push(`관측: ${String(a.source || "?")} · ${hhmm(u)}${fr.note ? ` (${fr.note})` : ""}`);
  for (const v of views) lines.push(`${v.label}: ${v.text}${v.resetText ? ` · ${v.resetText}` : ""}`);
  lines.push(USAGE_SCOPE_NOTE);
  return lines.join("\n");
}

/** 렌더용 모델. main.ts 는 이 모델을 textContent 로만 옮긴다.
 *  redactEmail = CC 🔒 가림 함수(신원 줄) · hidePaths = 🔒 가림 상태(설정 폴더를 끝 이름으로 — 리뷰1 M9). */
export function buildUsageBarModel(
  accounts: AcctRow[],
  nowSec: number,
  fetch: UsageFetchState,
  redactEmail: (s: string) => string,
  hidePaths = false,
): UsageBarModel {
  const list = (Array.isArray(accounts) ? accounts : []).filter(isObj) as AcctRow[];
  const failing = fetch.failStreak >= USAGE_FAIL_STREAK_WARN;
  const footer = !failing
    ? ""
    : fetch.everOk && fetch.okAtSec
      ? `데몬 응답 없음 — ${hhmm(fetch.okAtSec)} 기준 값(자동 재시도 중)`
      : "데몬 응답 없음 — 사용량을 가져오지 못했습니다(자동 재시도 중)";
  const empty: UsageBarModel = {
    headline: "",
    primary: null,
    others: [],
    moreCount: 0,
    unobservedCount: 0,
    unobservedTooltip: "",
    message: "",
    footer,
  };
  if (!fetch.everOk) {
    return failing
      ? { ...empty, headline: "응답 없음" }
      : // 기한 없이 남아도 참인 말만 쓴다 — start() 실패 경로엔 10초 틱이 없어 '복원 뒤 표시' 는 거짓 약속이 된다
        // (리뷰1 M8 · 반박 D5). Control Center Live 는 force 조회라 그 경로에서도 값을 가져와 이 칸까지 채운다.
        { ...empty, headline: "대기 중", message: "사용량 확인 대기 중 — 바로 보려면 Control Center > Live" };
  }

  // 라벨: 겹치면 결정론 꼬리표로 구분(둘 다 "Claude" 로 보이지 않게).
  const labels = new Map<AcctRow, string>();
  const count = new Map<string, number>();
  for (const a of list) {
    const l = accountShortLabel(a);
    labels.set(a, l);
    count.set(l, (count.get(l) ?? 0) + 1);
  }
  for (const a of list) {
    const l = labels.get(a)!;
    if ((count.get(l) ?? 0) > 1) labels.set(a, `${l} ·${tag4(acctKey(a))}`);
  }

  const unobserved = list.filter((a) => !isObserved(a));
  const unobservedTooltip = unobserved.length
    ? `아직 관측되지 않은 계정: ${unobserved.map((a) => labels.get(a)).join(", ")}\n${USAGE_SCOPE_NOTE}`
    : "";
  const primaryAcct = pickPrimaryAccount(list, nowSec);
  if (!primaryAcct) {
    return {
      ...empty,
      headline: "관측 없음",
      unobservedCount: unobserved.length,
      unobservedTooltip,
      message: "아직 관측된 사용량 없음 — 에이전트 첫 응답 후 표시",
    };
  }

  const pv = USAGE_WINDOWS.map((l) => windowView(primaryAcct, l, nowSec));
  const pf = freshness(primaryAcct, nowSec);
  const ex = finiteNum(primaryAcct.exhaust_at);
  const primary: UsagePrimary = {
    label: labels.get(primaryAcct)!,
    tooltip: tooltipFor(primaryAcct, pv, pf, redactEmail, hidePaths),
    windows: pv,
    fresh: pf,
    exhaust: ex !== null && ex > nowSec && (pf.level === "fresh" || pf.level === "recent") ? `이 속도면 ${hhmm(ex)} 소진` : "",
  };

  const rest = list
    .filter((a) => a !== primaryAcct && isObserved(a))
    .sort((x, y) => {
      const dl = Number(isLiveAccount(y)) - Number(isLiveAccount(x));
      if (dl) return dl;
      return (finiteNum(y.updated_at) ?? 0) - (finiteNum(x.updated_at) ?? 0);
    });
  const others: UsageLine[] = rest.slice(0, USAGE_OTHERS_MAX).map((a) => {
    const v = USAGE_WINDOWS.map((l) => windowView(a, l, nowSec));
    const f = freshness(a, nowSec);
    const txt = v.map((w) => `${w.label} ${w.text}`).join(" · ");
    return {
      label: labels.get(a)!,
      text: f.note && f.level !== "fresh" ? `${txt} (${f.note})` : txt,
      tooltip: tooltipFor(a, v, f, redactEmail, hidePaths),
      dim: f.level === "stale",
    };
  });

  return {
    headline: pv.map((w) => `${w.label} ${w.text}`).join(" · "),
    primary,
    others,
    moreCount: Math.max(0, rest.length - USAGE_OTHERS_MAX),
    unobservedCount: unobserved.length,
    unobservedTooltip,
    message: "",
    footer,
  };
}

export interface FetchGate {
  started: boolean; // start() 복원이 끝났는가
  startedAtMs: number | null; // 복원 완료를 처음 본 시각
  lastAttemptAtMs: number | null; // 마지막 조회 시도(사이드바·CC 공통)
  force: boolean; // Control Center Live — 유예·간격 무시(in-flight 가드는 호출측이 지킨다)
  graceMs: number;
  minIntervalMs: number;
}
/** 사이드바 경로의 조회 여부. 새 타이머 없이 기존 10초 틱에 얹으므로 대부분의 호출은 여기서 false 로 끝난다.
 *  시계가 뒤로 가면(음수 경과) 막지 않는다 — 막으면 시계가 따라잡을 때까지 조회가 영구히 멈춘다. */
export function shouldFetchAccounts(nowMs: number, st: FetchGate): boolean {
  if (st.force) return true;
  if (!st.started || st.startedAtMs === null) return false;
  const up = nowMs - st.startedAtMs;
  if (up >= 0 && up < st.graceMs) return false;
  if (st.lastAttemptAtMs !== null) {
    const since = nowMs - st.lastAttemptAtMs;
    if (since >= 0 && since < st.minIntervalMs) return false;
  }
  return true;
}
