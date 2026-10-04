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
//   · ★(0.14.42 · 오너 제보 "모든 AI 계정이 나타나지 않는다") 발견된 계정은 **관측이 없어도 한 줄씩** 보인다.
//     종전엔 관측 전 계정을 "관측 없음 N개" 한 줄로 접고 이름은 툴팁에만 두어, cys 창 밖에서 쓰는 Claude
//     계정·agy 계정이 화면에서 사라졌다. 관측 전 행은 흐리게 "관측 전 · 사유", 관측 경로가 고장이면
//     "관측 실패 · 사유"(source_error) — 값(%)은 지어내지 않는다. 라벨·🔒 가림 규칙은 관측 행과 같다.
//   · ★(0.14.42 RC4-b · RC2-b) 값의 새 출처 둘: cys 창 밖 Claude 세션(source "statusline-outside" — 표시용·경보
//     제외)과 agy 상태줄 훅(source "agy-statusline"). 둘 다 라이브 관측이며 툴팁 출처 줄은 사람 말로 적는다.
//   · ★(0.14.43 · 오너 제보 "다른 계정으로 다시 로그인했는데 옛 계정이 계속 주 계정으로 뜬다") 데몬이 보내는 가산 키로
//     '지금'을 읽는다 — 별명(alias)·현재 로그인 폴더(current_profiles)로 라벨을 만들고, 지금 쓰이는 계정(in_use)에
//     '● 사용 중' 표식을 단다. 주 계정은 사용 중 → 관측 신선도 순이다. 30분 넘은 관측·스냅샷은 경고색 없이 '오래됨'으로
//     적는다(값·게이지 폭은 그대로 — 지난 값을 빨갛게 외치지 않는다). 숨긴 계정(뷰어별 목록은 main.ts 가 읽어 인자로
//     넘긴다)은 후보·줄·요약에서 뺀다. 가산 키가 없으면(구버전 데몬) 종전 규칙으로 폴백한다 — IPC 데이터라 전부 의심한다.
//   · ★(0.14.43 · UI2) Control Center Live KPI 의 '전 좌석 폴백'(계정 병합 값이 없을 때)도 옛 좌석 값을 되살리지 않는다 — aggSeatRates 가 경보 부적격(alert_eligible=false)
//     창과 리셋 지난 창을 집계에서 뺀다. 구버전 데몬(키 없음)은 리셋 지난 창만 뺀다.
//
// ★이 모듈의 불변식(usagewiring.test.ts 가 핀으로 고정):
//   · 최상위 부수효과 0 — 선언(export/const/function/type)만. 브라우저 저장소(local·session)·document·window·타이머 접근 0.
//     main.js 는 번들 하나라 여기서 평가 중 예외가 나면 앱 전체가 백지가 된다(④).
//   · 구형 WKWebView 가 파싱하지 못하는 문법 0 — 정규식 lookbehind, `.at(`, findLast, structuredClone,
//     Object.hasOwn, replaceAll. `bun build --target browser` 는 다운레벨하지 않으므로 파싱 실패가 곧 백지다.

/** usage.accounts 응답의 창 하나(`accounts.rs` RateWindow 직렬화). */
export interface AcctRateWindow {
  label: string;
  used_pct: number;
  resets_at: number | null;
  /** 0.14.43(B3 가산) 경보 입력 적격 — 표시 판정에는 쓰지 않는다(데몬이 경보에 쓸지 정한 값). 구버전은 키 없음. */
  alert_eligible?: boolean;
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
  source_error?: string | null; // 관측 경로 고장 코드(예: "agy_http_403") · null = 고장 없음(0.14.42)
  /** 0.14.43(B1 가산) 지금 이 계정으로 로그인돼 있는 설정 폴더(profiles 와 같은 표기). **배열로 있으면 그것만**이 '현재'다
   *  (빈 배열 = 어느 폴더에도 로그인돼 있지 않은 이전 계정). 키가 없으면 구버전 데몬 — profiles 로 폴백한다. */
  current_profiles?: string[];
  /** 0.14.43(B1) 오너가 적어 둔 별명(데몬이 24자로 자른다) — 표시 전용. null·키 없음 = 별명 없음. */
  alias?: string | null;
  /** 0.14.43(B3) 지금 로그인돼 쓰이는 계정인가 — true/false/null(판정 불가) · 키 없음 = 구버전. **true 일 때만** '사용 중'. */
  in_use?: boolean | null;
  /** 0.14.43(B3) 경보 입력이 된 rate 의 관측 시각(epoch 초). 사이드바의 관측 나이는 계속 updated_at 으로 센다. */
  rate_observed_at?: number | null;
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
/** 주 계정 밖의 계정(관측·관측 전 모두)은 이 수까지만 한 줄씩 — 넘치면 "외 N개"(꼬리 높이 상한).
 *  0.14.42: 4 → 8. 한 사용자의 실사용 계정(2026-09-23 실측: Claude 4 + Codex + Antigravity = 6)이 전부
 *  보여야 한다. 꼬리는 자체 스크롤(#wsbar-foot max-height)이라 줄이 늘어도 탭 목록을 밀지 않는다. */
export const USAGE_OTHERS_MAX = 8;
/** 별명 표시 상한(글자 수) — 데몬의 절단(24자)과 같은 값. 구버전·이상 값이 길게 와도 화면은 이 수까지만. */
export const USAGE_ALIAS_MAX = 24;
/** 뷰어별 숨김 목록의 최대 항목 수 — main.ts 저장소 검증(sanitizeHiddenKeys)과 숨기기 단추가 같이 쓴다. */
export const USAGE_HIDDEN_MAX = 64;
/** '관측 전 N계정 — …' 접힘 줄의 말줄임 기준(글자 수). 전체 라벨은 툴팁에 있다. */
export const USAGE_FOLD_TEXT_MAX = 60;

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
  if (p === "antigravity") return "Antigravity"; // 데몬의 실제 provider 키(accounts.rs) — 오너가 부르는 이름
  return p || "계정";
}

/** 별명 — 문자열이고 앞뒤 공백을 걷은 뒤 비어 있지 않을 때만(24자 초과분은 자른다). 없으면 빈 문자열. 타입부터 의심한다. */
export function acctAlias(a: AcctRow): string {
  const v: unknown = isObj(a) ? a.alias : undefined;
  if (typeof v !== "string") return "";
  const t = v.trim();
  if (!t) return "";
  const cs = Array.from(t); // 코드 포인트 단위 — 이모지 같은 서로게이트 쌍을 반으로 자르지 않는다
  return cs.length > USAGE_ALIAS_MAX ? cs.slice(0, USAGE_ALIAS_MAX).join("") : t;
}

/** claude 계정이 지금 어느 폴더에도 로그인돼 있지 않은가 — current_profiles 가 (쓸 수 있는 원소가 없는) **배열로 있을 때만**.
 *  키가 없는 구버전 데몬은 판정하지 않는다(false). Control Center 의 '이전 로그인' 배지가 쓴다.
 *  ★(성찰 1회차 R1F-UB · S2 m-1 ⓑ) **in_use === true 인 계정은 '이전 로그인'이 아니다** — 좌석 폴더가 열거 밖(`CYS_ACCOUNT_DIR` 임의 경로 · 부서 카탈로그의 임의 계정 폴더)이면
 *  데몬이 `in_use:true` 와 `current_profiles:[]` 를 함께 보낸다(`in_use` 는 좌석 폴더의 신원으로, `current_profiles` 는 열거된 폴더의 신원으로만 만든다 — 출처가 다르다).
 *  지금 쓰이는 계정을 '이전'이라 부르면 모순이다 — 데몬 쪽 원인 수정과 별개로 화면이 구 데몬·혼재 구성에서도 모순을 내지 않게 막는다. */
export function isPreviousLogin(a: AcctRow): boolean {
  return a.in_use !== true && a.provider === "claude" && Array.isArray(a.current_profiles) && normalizeProfiles(a.current_profiles).length === 0;
}

/** 프로필 집합에서 만든 폴더 라벨(claude-N > 좌석 > 부서 x). 만들 수 없으면 null. */
function folderLabel(profiles: unknown): string | null {
  let best: { rank: number; label: string } | null = null;
  for (const p of normalizeProfiles(profiles)) {
    const pl = profileLabel(p);
    if (!pl) continue;
    if (!best || pl.rank < best.rank || (pl.rank === best.rank && pl.label < best.label)) best = pl;
  }
  return best ? best.label : null;
}

/** 계정의 화면 라벨 — 별명 > 현재 로그인 폴더(claude-N > 좌석 > 부서 x) > 제공자. **이메일(label 필드)은 절대 쓰지 않는다.**
 *  · current_profiles 가 배열로 있으면 **그것만**으로 폴더 라벨을 만든다(profiles 는 추가 전용이라 옛 로그인 폴더가 남아 있다).
 *    폴더 라벨을 하나도 못 만들면(보통 빈 배열 = 지금은 어느 폴더에도 로그인돼 있지 않다) claude 는 'Claude (이전 로그인)',
 *    그 밖은 제공자 라벨. ★단 **in_use === true 면 '이전 로그인'이 아니다**(R1F-UB · S2 m-1 ⓑ — isPreviousLogin 과 같은 규칙) — 아래 profiles 폴백을 쓴다.
 *  · 키가 없으면(구버전 데몬) 종전처럼 profiles 로 만든다. */
export function accountShortLabel(a: AcctRow): string {
  const alias = acctAlias(a);
  if (alias) return alias;
  if (Array.isArray(a.current_profiles)) {
    const cur = folderLabel(a.current_profiles);
    if (cur) return cur;
    if (a.in_use !== true) return a.provider === "claude" ? `${providerLabel(a.provider)} (이전 로그인)` : providerLabel(a.provider);
    // in_use === true — 지금 쓰이는 계정이다(좌석 폴더가 열거 밖이라 current_profiles 가 비었을 뿐). 0.14.42 와 같은 profiles 폴백.
  }
  return folderLabel(a.profiles) ?? providerLabel(a.provider);
}

/** 이 계정의 '설정 폴더' 표시 목록(툴팁용) — current_profiles 가 배열이면 그것(추가 전용 profiles 에 남은 옛 로그인 폴더는 쓰지 않는다 · 라벨과 같은 규칙),
 *  키가 없으면(구버전 데몬) 종전 profiles. 단 in_use === true 인데 current_profiles 에서 쓸 폴더가 하나도 안 나오면(좌석 폴더가 열거 밖) 라벨처럼 profiles 로 폴백한다
 *  (R1F-UB · S2 n-9 ⓐ · m-1 ⓑ). */
function shownProfiles(a: AcctRow): unknown {
  if (Array.isArray(a.current_profiles) && !(a.in_use === true && normalizeProfiles(a.current_profiles).length === 0)) return a.current_profiles;
  return a.profiles;
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
/** 계정 키 — `provider:account_id`. 숨김 목록·정렬 동률 해소·겹침 꼬리표의 공통 키(main.ts 도 이 함수로 만든다). */
export const acctKey = (a: AcctRow): string => `${String(a.provider ?? "")}:${String(a.account_id ?? "")}`;
/** 뷰어가 숨긴 계정인가 — hidden 이 없거나 Set 모양이 아니면 숨김 없음(던지지 않는다). */
const isHiddenAcct = (a: AcctRow, hidden: ReadonlySet<string> | undefined): boolean =>
  !!hidden && typeof hidden.has === "function" && hidden.has(acctKey(a));

/** 저장소에서 읽은 숨김 목록의 검증 — 배열 · 문자열 원소 · 최대 USAGE_HIDDEN_MAX 개. 저장소 접근은 main.ts 만 하고 여기는 값만 본다. */
export function sanitizeHiddenKeys(raw: unknown): Set<string> {
  const out = new Set<string>();
  if (!Array.isArray(raw)) return out;
  for (const k of raw) {
    if (out.size >= USAGE_HIDDEN_MAX) break;
    if (typeof k === "string" && k !== "") out.add(k);
  }
  return out;
}

/** 관측 나이(초) — updated_at 이 유효할 때만(아니면 null). 시계가 뒤로 가도 음수가 되지 않는다. */
function obsAgeSecs(a: AcctRow, nowSec: number): number | null {
  const u = finiteNum(a.updated_at);
  return u !== null && u > 0 ? Math.max(0, nowSec - u) : null;
}

/** 관측 신선도 등급(주 계정 선정용) — 라이브(30분 안: fresh·recent) 2 > 오래됨(stale · 30분 초과) 1 > 스냅샷 0. 관측 전은 -1(후보 아님).
 *  fresh 와 recent 는 **한 등급**이다 — 2분 경계로 가르면 리뷰어 좌석처럼 가끔만 보고하는 계정이 fresh↔recent 를 수 분 간격으로
 *  오가며 주 계정이 깜빡이고, 한도에 가까운 계정이 2분만 조용해도 요약 줄에서 밀려난다. 라이브 안에서는 종전(0.14.42)처럼 5h 순이다.
 *  source "snapshot"(부트 예열)은 나이와 무관하게 0 — 종전 '라이브 무리 우선' 규칙이 이 등급에 흡수된다. */
function freshGrade(a: AcctRow, nowSec: number): number {
  if (!isObserved(a)) return -1;
  if (a.source === "snapshot") return 0;
  const lv = freshness(a, nowSec).level;
  return lv === "fresh" || lv === "recent" ? 2 : lv === "stale" ? 1 : -1;
}
/** 오래된 값(등급 stale·snapshot) — 경고색을 걷고 '오래됨'으로 적는 대상. */
const isStaleGrade = (g: number): boolean => g === 0 || g === 1;

/** 화면용 신선도 — 오래된 값(stale · 스냅샷이 아닌 쪽)은 '오래됨 · ' 머리말을 붙인다. 스냅샷은 종전 '지난 기록 · …' 그대로.
 *  freshness() 자체는 종전 그대로다(그 문구를 핀이 잡고 있고, 머리말은 화면 모델의 몫). */
function displayFresh(a: AcctRow, nowSec: number): Freshness {
  const f = freshness(a, nowSec);
  return f.level === "stale" && a.source !== "snapshot" ? { level: f.level, note: `오래됨 · ${f.note}` } : f;
}

/** 창 표기 묶음 — 오래된 값(등급 stale·snapshot)은 경고색(sev)을 걷는다. 값(text)·게이지 폭(pct)은 그대로다. */
function acctWindowViews(a: AcctRow, nowSec: number): WindowView[] {
  const stale = isStaleGrade(freshGrade(a, nowSec));
  return USAGE_WINDOWS.map((l) => {
    const v = windowView(a, l, nowSec);
    if (!stale || !v.sev) return v;
    const calm: WindowView = { ...v, sev: "" };
    return calm;
  });
}

/** 사용 중 순위 — true 2 · 판정 불가(null·키 없음·모르는 값) 1 · false 0. */
const inUseRank = (a: AcctRow): number => (a.in_use === true ? 2 : a.in_use === false ? 0 : 1);

/** 주 계정 — 관측된 계정(숨김 제외) 가운데:
 *  ① 사용 중인 계정이 하나라도 있으면 사용 중 순위 ↓(하나도 없으면 — 전부 구버전·판정 불가 — 이 키는 건너뛴다)
 *  ② 신선도 등급 ↓ — 라이브(fresh·recent = 30분 안) > 오래됨(stale) > 스냅샷. fresh·recent 는 한 등급이라 2분 경계가 순위를 바꾸지 않는다
 *  ③ 라이브면 5h ↓ → 7d ↓ → 최신 관측 ↓ · 오래됨·스냅샷이면 최신 관측 ↓ → 5h ↓ → 7d ↓
 *  ④ 좌석 경로 → 키 사전순(결정론). 좌석 경로 정규식은 동률 해소용일 뿐이다(반박 D1). */
export function pickPrimaryAccount(accounts: AcctRow[], nowSec: number, hidden?: ReadonlySet<string>): AcctRow | null {
  const list = (Array.isArray(accounts) ? accounts : []).filter((a) => isObj(a) && isObserved(a) && !isHiddenAcct(a, hidden));
  if (!list.length) return null;
  const anyInUse = list.some((a) => a.in_use === true);
  const sorted = [...list].sort((x, y) => {
    if (anyInUse) {
      const di = inUseRank(y) - inUseRank(x);
      if (di) return di;
    }
    const gx = freshGrade(x, nowSec);
    const gy = freshGrade(y, nowSec);
    if (gx !== gy) return gy - gx;
    const du = (finiteNum(y.updated_at) ?? 0) - (finiteNum(x.updated_at) ?? 0);
    const d5 = rankPct(y, "5h", nowSec) - rankPct(x, "5h", nowSec);
    const d7 = rankPct(y, "7d", nowSec) - rankPct(x, "7d", nowSec);
    if (isStaleGrade(gx)) {
      if (du) return du;
      if (d5) return d5;
      if (d7) return d7;
    } else {
      if (d5) return d5;
      if (d7) return d7;
      if (du) return du;
    }
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
  /** 한 번도 관측되지 않은 계정의 줄(값 없음 — "관측 전·관측 실패·어댑터 없음"). */
  unobserved: boolean;
  /** 지금 로그인돼 쓰이는 계정 — in_use === true 일 때만 true(null·false·키 없음은 false). 줄 앞에 '●' 표식. */
  inUse: boolean;
}
export interface UsagePrimary {
  label: string;
  tooltip: string;
  windows: WindowView[];
  fresh: Freshness;
  exhaust: string;
  /** 지금 로그인돼 쓰이는 계정(in_use === true) — 이름 옆에 '● 사용 중' 배지. */
  inUse: boolean;
}
export interface UsageBarModel {
  /** 접힘 상태·헤더 요약 한 줄. */
  headline: string;
  /** 요약 줄 색 — 줄에 실린 값들의 최고 심각도. 제공자별 약식이면 요약에 실린 값들(오래됨 `?`·'리셋됨' 제외), 아니면 주 계정 창들(오래된 값이면 ""). */
  headlineSev: "" | "warn" | "crit";
  /** 요약 줄 툴팁 — 제공자별 약식(`C 5h12%·7d30% │ X 7d50%`)일 때 풀이. 아니면 빈 문자열. */
  headlineTitle: string;
  primary: UsagePrimary | null;
  others: UsageLine[];
  /** 상한에 잘려 나간 **관측 줄** 수(관측 전 계정은 unobservedFold 가 따로 맡는다). */
  moreCount: number;
  /** 잘려 나간 관측 줄의 라벨 나열(툴팁). */
  moreTooltip: string;
  /** 상한에 잘려 나간 관측 전 계정 — 개수로만 사라지지 않게 라벨을 나열한 한 줄(없으면 null). */
  unobservedFold: { text: string; tooltip: string } | null;
  /** 관측 전 계정 수(그 계정들도 others 에 한 줄씩 있다 — 개수로 접지 않는다). 숨긴 계정은 뺀다. */
  unobservedCount: number;
  /** 뷰어가 숨긴 계정 수(현재 목록에 있는 것만) — 0 이면 표시 없음. */
  hiddenCount: number;
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

/** 집계 범위 고지. 0.14.42: 기본 `claude`(~/.claude — 신원 ~/.claude.json)도 모이고(RC3), 외부 터미널(cys 창 밖)
 *  Claude 세션도 그 프로필의 상태줄이 cys 로 연결돼 있으면 계정 전용 입구(`usage.report_account`)로 모인다(RC4-b).
 *  창 밖 값은 **표시용**이다 — 같은 UID 의 프로세스가 보낼 수 있어 계정 경보의 근거로 쓰지 않는다(데몬 alert_rates). */
export const USAGE_SCOPE_NOTE =
  "집계 범위: cys 창 안 세션과, 상태줄이 cys 로 연결된 외부 터미널(cys 창 밖) Claude 세션이 계정에 모입니다" +
  " — 창 밖 값은 표시용(경보 제외)이고, 명명 규칙 밖 설정 폴더·창 밖 agy 세션은 집계 대상이 아닙니다.";

/** 관측 출처(source) → 툴팁용 사람 말. 모르는 출처는 원문 그대로(정직). */
function sourceLabel(src: unknown): string {
  if (src === "statusline-outside") return "cys 창 밖 상태줄";
  if (src === "agy-statusline") return "agy 상태줄";
  return typeof src === "string" && src ? src : "?";
}

/** 관측 경로 고장 코드(accounts.rs source_error) → 짧은 사유·자세한 설명. 모르는 코드도 '고장'으로 정직하게. */
function sourceErrorText(code: string): { short: string; detail: string } {
  const http = /^agy_http_(\d+)$/.exec(code);
  if (http)
    return {
      short: `agy 조회 거부(HTTP ${http[1]})`,
      detail: `agy 언어 서버가 쿼터 조회를 거부했습니다(HTTP ${http[1]}).`,
    };
  if (code === "agy_csrf_required")
    return {
      short: "agy 상태줄 연결 필요",
      detail:
        "agy(1.2 이후) 언어 서버가 쿼터 조회에 CSRF 토큰을 요구합니다. cys 는 그 토큰을 읽지 않습니다 — " +
        "agy 설정의 상태줄(statusLine)을 cys 로 연결하면 값이 들어옵니다(사용 설명서 「사이드바 바닥: 사용량」).",
    };
  // fatal-fix W5: RPC 경로가 구조적으로 없는 플랫폼(Windows — 언어 서버 포트를 찾을 길이 없다) — 값을 얻는 길을 가리킨다.
  if (code === "agy_statusline_required")
    return {
      short: "agy 상태줄 연결 필요",
      detail:
        "이 컴퓨터(Windows)에서는 cys 가 agy 내부 서버에서 쿼터를 읽을 수 없습니다 — " +
        "agy 설정의 상태줄(statusLine)을 cys 로 연결하면 값이 들어옵니다(사용 설명서 「사이드바 바닥: 사용량」).",
    };
  if (code === "agy_no_quota") return { short: "agy 응답에 쿼터 없음", detail: "agy 가 답했지만 Gemini 쿼터 항목이 없습니다." };
  if (code === "agy_unreachable") return { short: "agy 응답 없음", detail: "agy 언어 서버 포트가 응답하지 않습니다." };
  if (code === "agy_no_port") return { short: "agy 포트 못 찾음", detail: "cys 창의 agy 가 연 포트를 찾지 못했습니다." };
  if (code === "agy_no_process") return { short: "agy 프로세스 없음", detail: "agy 좌석 아래에서 agy 프로세스를 찾지 못했습니다." };
  return { short: "관측 경로 오류", detail: "관측 경로가 오류를 보고했습니다." };
}

/** 관측 전 계정 줄의 표기 — 상태 머리말 + 짧은 사유(줄) · 자세한 설명(툴팁). 값(%)은 만들지 않는다. */
export function unobservedStatus(a: AcctRow): { text: string; detail: string } {
  const err = typeof a.source_error === "string" ? a.source_error.trim() : "";
  if (err) {
    const t = sourceErrorText(err);
    return { text: `관측 실패 · ${t.short}`, detail: `${t.detail} (코드 ${err})` };
  }
  if (a.adapter === false)
    return { text: "관측 어댑터 없음", detail: "이 계정은 관측 어댑터 없이 선언됐습니다(~/.cys/accounts.json)." };
  const p = typeof a.provider === "string" ? a.provider : "";
  if (p === "antigravity" || p === "gemini")
    return {
      text: "관측 전 · agy 상태줄 연결 후",
      detail:
        "cys 창의 agy 좌석에서 agy 상태줄(statusLine)이 cys 로 연결돼 있으면 값이 들어옵니다(사용 설명서 「사이드바 바닥: 사용량」).",
    };
  if (p === "codex")
    return { text: "관측 전 · cys 창 codex 응답 후", detail: "cys 창의 codex 좌석이 응답하면 값이 들어옵니다." };
  return {
    text: "관측 전 · Claude 응답 후 표시",
    detail:
      "이 계정으로 Claude 가 응답하면 값이 들어옵니다 — cys 창 안이든 외부 터미널(cys 창 밖)이든, 그 프로필의 상태줄이 " +
      "cys 로 연결돼 있으면 모입니다(cys 설치가 연결합니다). 명명 규칙 밖 설정 폴더(CLAUDE_CONFIG_DIR)의 세션은 모이지 않습니다.",
  };
}
const PROVIDER_ORDER = ["claude", "codex", "antigravity"];
const providerRank = (a: AcctRow): number => {
  const i = PROVIDER_ORDER.indexOf(typeof a.provider === "string" ? a.provider : "");
  return i < 0 ? PROVIDER_ORDER.length : i;
};

function tooltipFor(
  a: AcctRow,
  views: WindowView[],
  fr: Freshness,
  redactEmail: (s: string) => string,
  hidePaths: boolean,
  note = "",
): string {
  const lines: string[] = [];
  // 신원 줄 — 라벨(이메일)이 비면 account_id 가 나오므로 그것도 가림 함수를 거친다(CC 계정 섹션과 같은 규칙).
  const whoRaw = typeof a.label === "string" && a.label ? a.label : String(a.account_id ?? "");
  lines.push(`${providerLabel(a.provider)} 계정 — ${whoRaw ? redactEmail(whoRaw) : "?"}`);
  if (typeof a.plan === "string" && a.plan) lines.push(`요금제: ${a.plan}`);
  // 🔒 가림이면 경로는 끝 이름만(가린 뒤 같아진 줄은 다시 접는다) — 사이드바는 늘 화면에 떠 있어 공유 화면에 찍힌다.
  //   ★R1F-UB(S2 n-9 ⓐ): 목록은 current_profiles 가 있으면 그것(shownProfiles) — 옛 계정 툴팁에 지금 폴더가 남지 않는다.
  const shown = shownProfiles(a);
  const profs = hidePaths ? normalizeProfiles(normalizeProfiles(shown).map(profileTail)) : normalizeProfiles(shown);
  if (profs.length) lines.push(`설정 폴더: ${profs.join(", ")}`);
  const u = finiteNum(a.updated_at);
  if (u !== null && u > 0) lines.push(`관측: ${sourceLabel(a.source)} · ${hhmm(u)}${fr.note ? ` (${fr.note})` : ""}`);
  for (const v of views) lines.push(`${v.label}: ${v.text}${v.resetText ? ` · ${v.resetText}` : ""}`);
  if (note) lines.push(note);
  // 관측된 행이라도 그 경로가 지금 고장이면 적는다(값은 마지막 관측 그대로 — 오래됨 표기가 따로 붙는다).
  const err = typeof a.source_error === "string" ? a.source_error.trim() : "";
  if (err && isObserved(a)) lines.push(`관측 경로 오류: ${sourceErrorText(err).detail} (코드 ${err})`);
  lines.push(USAGE_SCOPE_NOTE);
  return lines.join("\n");
}

/** 라벨이 겹치는 계정의 구분 꼬리표 — 이메일(label)이 있으면 🔒 가림 함수를 거친 값이다(🔒 끔 = 원문 · 켬 = Control Center 계정 표와
 *  같은 `#hash6` 라 두 화면을 대조할 수 있다). 이메일이 없으면 종전 결정론 tag4. 사이드바 본문에 이메일이 나올 수 있는 **유일한** 경로. */
function overlapTag(a: AcctRow, redactEmail: (s: string) => string): string {
  const raw = typeof a.label === "string" ? a.label : "";
  if (raw.trim()) {
    const r = redactEmail(raw);
    if (typeof r === "string" && r.trim()) return r;
  }
  return tag4(acctKey(a));
}

/** 줄 상한에 잘려 나간 관측 전 계정 — 개수로만 사라지지 않게 라벨을 나열한다(전체 줄이 60자를 넘으면 말줄임 · 툴팁은 전체 나열). */
function unobservedFoldOf(cut: UsageLine[]): { text: string; tooltip: string } | null {
  if (!cut.length) return null;
  const names = cut.map((l) => l.label).join(" · ");
  const chars = Array.from(`관측 전 ${cut.length}계정 — ${names}`);
  return {
    text: chars.length > USAGE_FOLD_TEXT_MAX ? `${chars.slice(0, USAGE_FOLD_TEXT_MAX - 1).join("")}…` : chars.join(""),
    tooltip: names,
  };
}

/** 제공자 묶음 키 — 옛 표기 gemini 는 데몬의 실제 키 antigravity 와 한 무리다(둘 다 머리글자 A). */
const provGroup = (a: AcctRow): string => {
  const p = typeof a.provider === "string" ? a.provider : "";
  return p === "gemini" ? "antigravity" : p;
};
const groupRank = (key: string): number => {
  const i = PROVIDER_ORDER.indexOf(key);
  return i < 0 ? PROVIDER_ORDER.length : i;
};
/** 제공자 머리글자 — C=claude · X=codex · A=antigravity(gemini) · 그 밖은 표시명 첫 글자 대문자. */
function providerInitial(key: string): string {
  if (key === "claude") return "C";
  if (key === "codex") return "X";
  if (key === "antigravity" || key === "gemini") return "A";
  const first = Array.from(providerLabel(key))[0];
  return first ? first.toUpperCase() : "?";
}
/** 비교용 값 — 100% 초과('100%+')는 100% 위. */
const viewRank = (v: WindowView): number => (v.text === "100%+" ? 101 : v.pct ?? 0);

/** 한 제공자의 한 창 — 오래되지 않은(fresh·recent) 계정들의 최댓값. 그런 계정에 값이 없으면 오래된 계정들의 최댓값(stale=true →
 *  호출측이 `?`). 리셋 지난 창은 '리셋됨'. 값 없는 창은 null(생략).
 *  sev = 그 값의 심각도(windowView 의 임계 70/90) — 요약 줄 색(headlineSev)이 **요약에 실린 값**으로 계산되도록 값과 함께 돌려준다(R1F-UB · S2 m-3).
 *  오래된 값(stale · `?`)과 '리셋됨'은 경고색이 없다(오래된 값의 경고색을 걷는 규칙과 같다). */
function providerWindow(accts: AcctRow[], label: string, nowSec: number): { text: string; stale: boolean; sev: WindowView["sev"] } | null {
  const pick = (list: AcctRow[]): WindowView | null => {
    let top: WindowView | null = null;
    let rolled: WindowView | null = null;
    for (const a of list) {
      const v = windowView(a, label, nowSec);
      if (v.state === "ok") {
        if (!top || viewRank(v) > viewRank(top)) top = v;
      } else if (v.state === "rolled" && !rolled) rolled = v;
    }
    return top ?? rolled;
  };
  const live = pick(accts.filter((a) => !isStaleGrade(freshGrade(a, nowSec))));
  if (live) return { text: live.text, stale: false, sev: live.sev };
  const old = pick(accts.filter((a) => isStaleGrade(freshGrade(a, nowSec))));
  return old ? { text: old.text, stale: old.state === "ok", sev: "" } : null;
}

/** 제공자별 접힘 요약 — 관측 계정이 두 제공자 이상에 걸칠 때만(아니면 null → 종전 형식). 예 `C 5h12%·7d30% │ X 7d50%`.
 *  풀이(title)는 `Claude 5h 12% · 7d 30% / Codex 7d 50%`. 제공자 순서는 PROVIDER_ORDER.
 *  sev = **요약에 실린 값들**의 최고 심각도(crit > warn > "") — 오래됨 `?` 값·'리셋됨'은 뺀다(R1F-UB · S2 m-3: 색이 주 계정 창만 보면 숫자와 색이 서로 다른 계정을 봤다). */
function providerSummary(observed: AcctRow[], nowSec: number): { short: string; title: string; sev: UsageBarModel["headlineSev"] } | null {
  const groups = new Map<string, AcctRow[]>();
  for (const a of observed) {
    const k = provGroup(a);
    const g = groups.get(k);
    if (g) g.push(a);
    else groups.set(k, [a]);
  }
  if (groups.size < 2) return null;
  const keys = [...groups.keys()].sort((x, y) => groupRank(x) - groupRank(y) || (x < y ? -1 : x > y ? 1 : 0));
  const shorts: string[] = [];
  const titles: string[] = [];
  let sev: UsageBarModel["headlineSev"] = "";
  for (const k of keys) {
    const segs: string[] = [];
    const full: string[] = [];
    for (const l of USAGE_WINDOWS) {
      const w = providerWindow(groups.get(k)!, l, nowSec);
      if (!w) continue;
      segs.push(`${l}${w.text}${w.stale ? "?" : ""}`);
      full.push(`${l} ${w.text}${w.stale ? " (오래됨)" : ""}`);
      if (!w.stale && w.sev === "crit") sev = "crit";
      else if (!w.stale && w.sev === "warn" && sev === "") sev = "warn";
    }
    shorts.push(`${providerInitial(k)} ${segs.length ? segs.join("·") : "—"}`);
    titles.push(`${providerLabel(k)} ${full.length ? full.join(" · ") : "—"}`);
  }
  return { short: shorts.join(" │ "), title: titles.join(" / "), sev };
}

/** 렌더용 모델. main.ts 는 이 모델을 textContent 로만 옮긴다.
 *  redactEmail = CC 🔒 가림 함수(신원 줄·겹침 꼬리표) · hidePaths = 🔒 가림 상태(설정 폴더를 끝 이름으로 — 리뷰1 M9) ·
 *  hidden = 뷰어가 숨긴 계정 키(`provider:account_id`) 집합 — 후보·줄·제공자 요약에서 뺀다(저장소는 main.ts 만 읽는다). */
export function buildUsageBarModel(
  accounts: AcctRow[],
  nowSec: number,
  fetch: UsageFetchState,
  redactEmail: (s: string) => string,
  hidePaths = false,
  hidden?: ReadonlySet<string>,
): UsageBarModel {
  const everyone = (Array.isArray(accounts) ? accounts : []).filter(isObj) as AcctRow[];
  const list = everyone.filter((a) => !isHiddenAcct(a, hidden)); // 이하 list = 화면에 나올 계정
  const hiddenCount = everyone.length - list.length;
  const failing = fetch.failStreak >= USAGE_FAIL_STREAK_WARN;
  const footer = !failing
    ? ""
    : fetch.everOk && fetch.okAtSec
      ? `데몬 응답 없음 — ${hhmm(fetch.okAtSec)} 기준 값(자동 재시도 중)`
      : "데몬 응답 없음 — 사용량을 가져오지 못했습니다(자동 재시도 중)";
  const empty: UsageBarModel = {
    headline: "",
    headlineSev: "",
    headlineTitle: "",
    primary: null,
    others: [],
    moreCount: 0,
    moreTooltip: "",
    unobservedFold: null,
    unobservedCount: 0,
    hiddenCount,
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

  // 라벨: 겹치면 구분 꼬리표로(둘 다 "Claude" 로 보이지 않게) — 이메일(🔒 가림 거침)로 먼저, 그래도 같으면(같은 이메일·이메일 없음)
  // 종전 결정론 tag4. 숨긴 계정은 화면에 없으므로 겹침 계산에서도 뺀다.
  const base = new Map<AcctRow, string>();
  const baseCount = new Map<string, number>();
  for (const a of list) {
    const l = accountShortLabel(a);
    base.set(a, l);
    baseCount.set(l, (baseCount.get(l) ?? 0) + 1);
  }
  const labels = new Map<AcctRow, string>();
  const labelCount = new Map<string, number>();
  for (const a of list) {
    const b = base.get(a)!;
    const l = (baseCount.get(b) ?? 0) > 1 ? `${b} ·${overlapTag(a, redactEmail)}` : b;
    labels.set(a, l);
    labelCount.set(l, (labelCount.get(l) ?? 0) + 1);
  }
  for (const a of list) {
    if ((labelCount.get(labels.get(a)!) ?? 0) > 1) labels.set(a, `${base.get(a)!} ·${tag4(acctKey(a))}`);
  }

  // 관측 전 계정 — 개수로 접지 않고 한 줄씩(0.14.42). 제공자 순(claude·codex·antigravity) → 라벨 순(결정론).
  const unobserved = list
    .filter((a) => !isObserved(a))
    .sort((x, y) => providerRank(x) - providerRank(y) || (labels.get(x)! < labels.get(y)! ? -1 : labels.get(x)! > labels.get(y)! ? 1 : 0));
  const unobservedLines: UsageLine[] = unobserved.map((a) => {
    const v = USAGE_WINDOWS.map((l) => windowView(a, l, nowSec));
    const st = unobservedStatus(a);
    return {
      label: labels.get(a)!,
      text: st.text,
      tooltip: tooltipFor(a, v, freshness(a, nowSec), redactEmail, hidePaths, st.detail),
      dim: true,
      unobserved: true,
      inUse: a.in_use === true,
    };
  });
  const primaryAcct = pickPrimaryAccount(list, nowSec);
  if (!primaryAcct) {
    // 관측된 계정이 없다. 관측 전 계정이 있으면 그 수를 요약에(0.14.43 A4 — '관측 없음' 은 계정이 아예 없을 때만).
    // 화면에 나올 계정이 하나도 없는데 숨긴 계정만 있으면 '관측 없음' 이 거짓이므로 숨김 수를 적는다.
    const allHidden = list.length === 0 && hiddenCount > 0;
    return {
      ...empty,
      headline: unobserved.length ? `관측 전 ${unobserved.length}계정` : allHidden ? `숨김 ${hiddenCount}계정` : "관측 없음",
      others: unobservedLines.slice(0, USAGE_OTHERS_MAX),
      unobservedFold: unobservedFoldOf(unobservedLines.slice(USAGE_OTHERS_MAX)),
      unobservedCount: unobserved.length,
      message: allHidden ? "" : "아직 관측된 사용량 없음 — 에이전트 첫 응답 후 표시",
    };
  }

  const pv = acctWindowViews(primaryAcct, nowSec);
  const pf = displayFresh(primaryAcct, nowSec);
  const primaryStale = isStaleGrade(freshGrade(primaryAcct, nowSec));
  const ex = finiteNum(primaryAcct.exhaust_at);
  const primary: UsagePrimary = {
    label: labels.get(primaryAcct)!,
    tooltip: tooltipFor(primaryAcct, pv, pf, redactEmail, hidePaths),
    windows: pv,
    fresh: pf,
    exhaust: ex !== null && ex > nowSec && (pf.level === "fresh" || pf.level === "recent") ? `이 속도면 ${hhmm(ex)} 소진` : "",
    inUse: primaryAcct.in_use === true,
  };

  const rest = list
    .filter((a) => a !== primaryAcct && isObserved(a))
    .sort((x, y) => {
      const dl = Number(isLiveAccount(y)) - Number(isLiveAccount(x));
      if (dl) return dl;
      return (finiteNum(y.updated_at) ?? 0) - (finiteNum(x.updated_at) ?? 0);
    });
  const observedLines: UsageLine[] = rest.map((a) => {
    const v = acctWindowViews(a, nowSec);
    const f = displayFresh(a, nowSec);
    const txt = v.map((w) => `${w.label} ${w.text}`).join(" · ");
    return {
      label: labels.get(a)!,
      text: f.note && f.level !== "fresh" ? `${txt} (${f.note})` : txt,
      tooltip: tooltipFor(a, v, f, redactEmail, hidePaths),
      dim: f.level === "stale",
      unobserved: false,
      inUse: a.in_use === true,
    };
  });
  // 관측 행이 먼저 — 상한이 관측값을 밀어내지 않는다. 잘려 나간 줄은 종류별로 따로 알린다: 관측 줄은 개수(외 N개 · 툴팁에 라벨),
  // 관측 전 계정은 라벨을 나열한 접힘 줄(개수로만 사라지지 않게).
  const all = observedLines.concat(unobservedLines);
  const cut = all.slice(USAGE_OTHERS_MAX);
  const cutObserved = cut.filter((l) => !l.unobserved);

  // 접힘 요약 — 관측 계정이 두 제공자 이상이면 제공자별 약식, 아니면 종전 형식(주 계정의 두 창). 주 계정이 오래된 값이면 끝에 '(오래됨)'.
  const sum = providerSummary(list.filter(isObserved), nowSec);
  const headline = (sum ? sum.short : pv.map((w) => `${w.label} ${w.text}`).join(" · ")) + (primaryStale ? " (오래됨)" : "");
  // ★R1F-UB(S2 m-3): 색은 **줄에 실린 값**으로 계산한다 — 제공자별 약식이면 요약에 실린 값들(sum.sev · 오래됨 `?` 제외), 한 제공자뿐이면 종전대로 주 계정의 두 창
  //   (오너 결재 "주 계정 = 사용 중 우선" — 그 경우의 동작은 바꾸지 않는다). 종전엔 약식에서도 주 계정 창만 봐서 96% 계정이 숫자로는 실리고 색은 무색이었다.
  const headlineSev: UsageBarModel["headlineSev"] = sum
    ? sum.sev
    : pv.some((w) => w.sev === "crit")
      ? "crit"
      : pv.some((w) => w.sev === "warn")
        ? "warn"
        : "";

  return {
    headline,
    headlineSev,
    headlineTitle: sum ? sum.title : "",
    primary,
    others: all.slice(0, USAGE_OTHERS_MAX),
    moreCount: cutObserved.length,
    moreTooltip: cutObserved.map((l) => l.label).join(" · "),
    unobservedFold: unobservedFoldOf(cut.filter((l) => l.unobserved)),
    unobservedCount: unobserved.length,
    hiddenCount,
    message: "",
    footer,
  };
}

/** Control Center 계정 행의 '오래된 관측' — 관측 나이 > 30분 또는 스냅샷(부트 예열). 관측 전 계정은 아니다(그쪽은 '관측 전' 배지). */
export function isOldObservation(a: AcctRow, nowSec: number): boolean {
  if (!isObserved(a)) return false;
  if (a.source === "snapshot") return true;
  const age = obsAgeSecs(a, nowSec);
  return age !== null && age > USAGE_STALE_SECS;
}

/** Control Center Live KPI('세션 5h'·'주간 7d' 대표값) 후보 계정 — **숨기지 않았고 ∧ 그 창이 리셋 전이고 ∧ (사용 중 ∨ 관측 나이 ≤ 30분 ∧ 스냅샷 아님)**.
 *  옛 계정의 지난 100% 가 '최고 사용 계정'으로 KPI 를 빨갛게 만들지 않게 한다. 사용 중은 나이와 무관하게 후보(활동 중인 계정은 값이 곧
 *  갱신된다). 관측 나이를 알 수 없고 사용 중도 아니면 후보가 아니다(지난 값일 수 있다). 후보의 행을 그대로 돌려준다 — 대표값(최댓값)은
 *  호출측(main.ts ccAcctMax)이 종전 순회로 고른다. */
export function kpiCandidates(accounts: AcctRow[], label: string, nowSec: number, hidden?: ReadonlySet<string>): AcctRow[] {
  const out: AcctRow[] = [];
  for (const a of Array.isArray(accounts) ? accounts : []) {
    if (!isObj(a) || isHiddenAcct(a, hidden)) continue;
    const rate = Array.isArray(a.rate) ? a.rate : [];
    const w = rate.find((x) => isObj(x) && x.label === label);
    if (!w || finiteNum(w.used_pct) === null) continue;
    const resets = finiteNum(w.resets_at);
    if (resets !== null && resets > 0 && nowSec >= resets) continue; // 리셋 지난 창 = 옛 값
    if (a.in_use !== true) {
      const age = obsAgeSecs(a, nowSec);
      if (age === null || age > USAGE_STALE_SECS || a.source === "snapshot") continue;
    }
    out.push(a);
  }
  return out;
}

/** Control Center Live KPI 폴백('세션 5h'·'주간 7d' — 계정 병합 값이 없을 때 main.ts ccAggRate)의 전 좌석 집계 — 라벨별 사용률 최댓값·가장 이른 리셋.
 *  좌석의 `usage.rate` 는 로그인을 바꾼 뒤에도 옛 계정의 마지막 값(예 99%)을 리셋 전까지 들고 있을 수 있어, 그대로 최댓값을 잡으면 폴백이 그 값을
 *  대표값으로 되살린다(0.14.43 UI2). 그래서 **창 단위로 뺀다**:
 *   · `alert_eligible === false` — 데몬(B3)의 경보 입력 부적격 판정(리셋 전 ∧ (사용 중 ∨ 관측 30분 이내)가 아니다). 키가 없는 구버전 데몬은 이 줄이 안 걸린다.
 *   · 리셋 시각이 유효(유한한 양수)하고 `nowSec` 이 그 이후 — 리셋 지난 값(사이드바 windowView 의 '리셋됨' 규칙과 같다). 구버전 데몬은 이것만 걸린다.
 *     `alert_eligible` 이 true 여도 리셋이 지났으면 뺀다(데몬이 판정한 시각과 지금 사이에 리셋이 지날 수 있다 — 지난 창은 어떤 경우에도 대표값이 아니다).
 *   · `used_pct` 가 유한한 숫자가 아님(문자열·null·NaN·무한대) · 라벨이 비어 있지 않은 문자열이 아님.
 *  나머지(최댓값 — 0 미만은 0 으로 접힌다 · 가장 이른 리셋)는 종전 main.ts ccAggRate 와 같다: 리셋 전 · 유한한 값만 있는 fleet 은 종전과 같은 결과다(동치 속성 검체).
 *  fleet = `control.dashboard` 의 좌석 배열 — IPC 데이터라 모든 층을 의심하고(배열 아님·null·원시값) 던지지 않는다. 결과는 프로토타입 없는 객체다
 *  (`__proto__`·`constructor` 같은 이상한 라벨이 와도 무해). */
export function aggSeatRates(fleet: unknown, nowSec: number): Record<string, { used: number; reset: number | null }> {
  const agg: Record<string, { used: number; reset: number | null }> = Object.create(null);
  if (!Array.isArray(fleet)) return agg;
  for (const seat of fleet) {
    const usage: unknown = isObj(seat) ? seat.usage : undefined;
    const rate: unknown = isObj(usage) ? usage.rate : undefined;
    if (!Array.isArray(rate)) continue;
    for (const w of rate) {
      if (!isObj(w) || w.alert_eligible === false) continue;
      const label = w.label;
      const used = w.used_pct;
      if (typeof label !== "string" || label === "" || typeof used !== "number" || !Number.isFinite(used)) continue;
      const reset = typeof w.resets_at === "number" && Number.isFinite(w.resets_at) ? w.resets_at : null;
      if (reset !== null && reset > 0 && nowSec >= reset) continue; // 리셋 지난 창 = 옛 값
      const cur = agg[label] ?? { used: 0, reset: null };
      if (used > cur.used) cur.used = used;
      if (reset !== null && (cur.reset === null || reset < cur.reset)) cur.reset = reset;
      agg[label] = cur;
    }
  }
  return agg;
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
