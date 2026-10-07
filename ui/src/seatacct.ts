// ★0.14.45 부서(워크스페이스) 카드의 노드별 계정 줄 + 사용량 패널의 역매핑 — 순수 판정·표기 모듈 (DOM·Tauri·저장소 무관).
//
// 오너 요청(2026-10-07): "어느 부서·노드가 어느 Claude·AI 계정을 쓰는지 몰라 pane 마다 `/config` 를 쳐야 한다 — 그걸 없애 달라."
// 데몬은 좌석마다 설정 폴더의 **현재** 신원을 이미 안다(accounts.rs SeatIdentityView). 0.14.45 부터 `org.status` 좌석 행에
// `account: {provider, agent, account_id, profile, state}` 가산 키로 싣고, 화면은 그것을 사용량 패널의 계정 행(`usage_accounts_all`)과
// (provider, account_id) 로 맞춰 **패널과 같은 이름**(accountDisplayLabels — 별명 > 폴더 claude-N > 제공자)으로 부른다.
//
// ★정직한 표기 규칙:
//   · 이메일은 툴팁에만(오너 결재 2026-09-30) — 🔒 가림 함수를 거친다. 카드 본문에는 이름(별명·폴더·제공자)만.
//   · 모르는 계정은 '계정 미확인' — 폴더 이름으로 계정을 짐작하지 않는다(결측은 값이 아니다). 사유는 툴팁.
//   · 데몬 응답이 낡았으면(최근 성공 조회가 낡음 창보다 오래) 옛 계정을 말하지 않고 '계정 미확인'.
//   · 같은 계정을 쓰는 노드는 한 묶음(`claude-4 master·worker`) — 사이드바 폭에서 접어 보이기 위해.
//
// ★불변식: 최상위 부수효과 0 · 구형 WKWebView 가 파싱하지 못하는 문법 0(usagebar.ts 와 같다).

import { type AcctRow, accountShortLabel, acctKey, profileTail, providerLabel } from "./usagebar";

/** main.ts 가 org.status 성공 조회마다 좌석별로 적어 두는 것(키 = `${socket}#${surface_id}`). */
export interface SeatAcctSig {
  /** 응답 좌석 행에 `account` 키가 있었는가(false = 구버전 데몬). */
  present: boolean;
  /** `account` 키의 값(IPC 데이터 — 전부 의심한다). */
  raw: unknown;
  role: string | null;
  exited: boolean;
  /** 성공 조회 시각(ms). */
  at: number;
}

/** 좌석 계정의 해석 결과. known=true 일 때만 key 가 계정 키(`provider:account_id` — usagebar acctKey 와 같은 꼴)다. */
export interface SeatAcct {
  known: boolean;
  key: string;
  provider: string;
  accountId: string;
  profile: string;
  /** 모를 때의 사유 코드(no_key · none · folder_unknown · no_login · unread · unsupported · stale · bad). known 이면 "known". */
  state: string;
}

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null;
const str = (v: unknown): string => (typeof v === "string" ? v : "");

/** 사유 코드 → 사람 말(툴팁). */
export const SEAT_ACCT_REASON: Record<string, string> = {
  no_key: "이 데몬이 좌석 계정 정보를 보내지 않습니다(구버전 데몬)",
  none: "에이전트가 등록되지 않은 좌석이거나 데몬이 좌석 목록을 읽지 못했습니다",
  folder_unknown: "이 좌석의 설정 폴더를 알 수 없습니다",
  no_login: "설정 폴더에 로그인 정보가 없습니다(로그아웃 상태)",
  unread: "로그인 정보를 아직 읽지 못했습니다(잠시 뒤 다시 확인)",
  unsupported: "이 에이전트의 계정은 cys 가 확인할 수 없습니다",
  stale: "데몬 응답이 끊겨 최근 계정 정보가 없습니다",
  bad: "데몬이 보낸 계정 정보를 해석하지 못했습니다",
};
export const SEAT_ACCT_UNKNOWN_LABEL = "계정 미확인";

/** 좌석 신호 → 계정 해석. 낡은 신호(staleMs 초과)는 stale — 옛 계정을 지금 것으로 말하지 않는다. */
export function parseSeatAccount(sig: SeatAcctSig, nowMs: number, staleMs: number): SeatAcct {
  const unknown = (state: string, provider = "", profile = ""): SeatAcct => ({ known: false, key: "", provider, accountId: "", profile, state });
  if (!(Number.isFinite(sig.at) && nowMs - sig.at <= staleMs)) return unknown("stale");
  if (!sig.present) return unknown("no_key");
  const r = sig.raw;
  if (r === null || r === undefined) return unknown("none");
  if (!isObj(r)) return unknown("bad");
  const provider = str(r.provider);
  const accountId = str(r.account_id);
  const profile = str(r.profile);
  const state = str(r.state);
  if (state === "known" && provider && accountId) return { known: true, key: `${provider}:${accountId}`, provider, accountId, profile, state };
  return unknown(state && SEAT_ACCT_REASON[state] ? state : "bad", provider, profile);
}

/** 역할 → 짧은 표기(사이드바 폭). reviewer-codex → rv-codex. 역할이 없으면 `#sid`. */
export function roleShort(role: string | null | undefined, sid: number): string {
  const r = typeof role === "string" ? role.trim() : "";
  if (!r) return `#${sid}`;
  return r.startsWith("reviewer-") ? `rv-${r.slice("reviewer-".length)}` : r;
}
/** 역할 정렬 순위 — master > cso > worker > reviewer > 그 밖. */
function roleRank(r: string): number {
  if (r === "master" || r.startsWith("master-")) return 0;
  if (r === "cso" || r.startsWith("cso-")) return 1;
  if (r === "worker" || r.startsWith("worker-")) return 2;
  if (r.startsWith("rv-")) return 3;
  return 4;
}
const byRole = (x: string, y: string): number => roleRank(x) - roleRank(y) || (x < y ? -1 : x > y ? 1 : 0);
const PROVIDER_ORDER = ["claude", "codex", "antigravity"];
const provRank = (p: string): number => {
  const i = PROVIDER_ORDER.indexOf(p);
  return i < 0 ? PROVIDER_ORDER.length : i;
};

/** 부서 카드의 계정 묶음 하나 — 같은 계정을 쓰는 노드들(또는 계정 미확인 노드들). */
export interface WsAcctGroup {
  /** 계정 키 · 미확인 묶음은 "" */
  key: string;
  label: string;
  roles: string[];
  title: string;
  unknown: boolean;
}

/** 카드·패널이 같은 이름을 쓰게 하는 표 — 계정 키 → 표시 라벨(accountDisplayLabels 결과)과 계정 행. */
export interface AcctIndex {
  labels: ReadonlyMap<string, string>;
  rows: ReadonlyMap<string, AcctRow>;
}
export function buildAcctIndex(list: AcctRow[], displayLabels: ReadonlyMap<AcctRow, string>): AcctIndex {
  const labels = new Map<string, string>();
  const rows = new Map<string, AcctRow>();
  for (const a of Array.isArray(list) ? list : []) {
    if (!isObj(a)) continue;
    const k = acctKey(a);
    rows.set(k, a);
    labels.set(k, displayLabels.get(a) ?? accountShortLabel(a));
  }
  return { labels, rows };
}

/** 계정 행이 없을 때의 이름 — claude 는 좌석 설정 폴더(사용량 패널의 폴더 라벨 규칙과 같다), 그 밖은 제공자. 계정이 **확인된** 좌석에만 쓴다. */
function labelWithoutRow(sa: SeatAcct): string {
  if (sa.provider === "claude" && sa.profile) return accountShortLabel({ provider: "claude", current_profiles: [sa.profile], in_use: true, profiles: [sa.profile] });
  return providerLabel(sa.provider);
}

/** 계정 키·라벨 — 카드와 역매핑이 같은 이름을 쓴다. */
export function seatAcctLabel(sa: SeatAcct, idx: AcctIndex): string {
  return idx.labels.get(sa.key) ?? labelWithoutRow(sa);
}

/** 한 부서(워크스페이스)의 좌석들 → 계정 묶음(제공자 순 → 라벨 순 · 미확인은 맨 끝).
 *  셸 좌석(역할 없음 ∧ account null)·종료 좌석·아직 조회되지 않은 좌석은 뺀다. */
export function buildWsAccountGroups(
  seats: { sid: number; sig: SeatAcctSig | undefined }[],
  idx: AcctIndex,
  redactEmail: (s: string) => string,
  hidePaths: boolean,
  nowMs: number,
  staleMs: number,
): WsAcctGroup[] {
  const known = new Map<string, { sa: SeatAcct; roles: string[] }>();
  const unknownRoles: { role: string; why: string }[] = [];
  for (const { sid, sig } of seats) {
    if (!sig || sig.exited) continue;
    const hasRole = typeof sig.role === "string" && sig.role.trim() !== "";
    if (!hasRole && (sig.raw === null || sig.raw === undefined)) continue; // 셸 pane — 노드가 아니다
    const sa = parseSeatAccount(sig, nowMs, staleMs);
    const role = roleShort(sig.role, sid);
    if (!sa.known) {
      unknownRoles.push({ role, why: SEAT_ACCT_REASON[sa.state] ?? SEAT_ACCT_REASON.bad });
      continue;
    }
    const g = known.get(sa.key);
    if (g) g.roles.push(role);
    else known.set(sa.key, { sa, roles: [role] });
  }
  const out: WsAcctGroup[] = [];
  for (const { sa, roles } of known.values()) {
    roles.sort(byRole);
    const label = seatAcctLabel(sa, idx);
    out.push({ key: sa.key, label, roles, title: acctTitle(sa, idx, label, roles, redactEmail, hidePaths), unknown: false });
  }
  out.sort((x, y) => {
    const px = provRank(x.key.split(":")[0] ?? "");
    const py = provRank(y.key.split(":")[0] ?? "");
    return px - py || (x.label < y.label ? -1 : x.label > y.label ? 1 : 0);
  });
  if (unknownRoles.length) {
    unknownRoles.sort((x, y) => byRole(x.role, y.role));
    out.push({
      key: "",
      label: SEAT_ACCT_UNKNOWN_LABEL,
      roles: unknownRoles.map((u) => u.role),
      title: [`${SEAT_ACCT_UNKNOWN_LABEL} — 계정을 짐작해 표시하지 않습니다`, ...unknownRoles.map((u) => `${u.role}: ${u.why}`)].join("\n"),
      unknown: true,
    });
  }
  return out;
}

/** 묶음 툴팁 — 이름 · 제공자 계정(이메일 — 🔒 가림 함수를 거친다) · 설정 폴더 · 노드. codex·agy 는 기기의 로그인 하나라 이메일을 cys 가 읽지 않는다. */
function acctTitle(sa: SeatAcct, idx: AcctIndex, label: string, roles: string[], redactEmail: (s: string) => string, hidePaths: boolean): string {
  const row = idx.rows.get(sa.key);
  const lines: string[] = [];
  const email = row && typeof row.label === "string" && row.label.indexOf("@") > 0 ? row.label : "";
  if (sa.provider === "claude") lines.push(`${label} — Claude 계정 ${email ? redactEmail(email) : "(이메일 미확인)"}`);
  else lines.push(`${label} — ${providerLabel(sa.provider)} · 이 기기의 로그인 하나(이메일은 cys 가 읽지 않습니다)`);
  if (sa.profile) lines.push(`설정 폴더: ${hidePaths ? profileTail(sa.profile) : sa.profile}`);
  lines.push(`노드: ${roles.join(", ")}`);
  return lines.join("\n");
}

/** 카드 한 줄 텍스트(묶음별 `라벨 역할·역할`) — 렌더는 묶음마다 span 을 만들지만 시험·접근성 텍스트는 이 꼴이다. */
export function wsAccountLineText(groups: WsAcctGroup[]): string {
  return groups.map((g) => `${g.label} ${g.roles.join("·")}`).join(" · ");
}

/** 역매핑 — 계정 키 → `부서이름 역할·역할` 목록(부서 순서 = 입력 순서). 확인된 좌석만 센다(미확인 좌석은 어느 계정에도 붙지 않는다). */
export function accountUsers(
  depts: { name: string; seats: { sid: number; sig: SeatAcctSig | undefined }[] }[],
  nowMs: number,
  staleMs: number,
): Map<string, string[]> {
  const out = new Map<string, string[]>();
  for (const d of depts) {
    const per = new Map<string, string[]>();
    for (const { sid, sig } of d.seats) {
      if (!sig || sig.exited) continue;
      const sa = parseSeatAccount(sig, nowMs, staleMs);
      if (!sa.known) continue;
      const r = roleShort(sig.role, sid);
      const l = per.get(sa.key);
      if (l) l.push(r);
      else per.set(sa.key, [r]);
    }
    for (const [k, roles] of per) {
      roles.sort(byRole);
      const name = d.name.trim() || "이름 없는 부서";
      const l = out.get(k);
      const item = `${name} ${roles.join("·")}`;
      if (l) l.push(item);
      else out.set(k, [item]);
    }
  }
  return out;
}
