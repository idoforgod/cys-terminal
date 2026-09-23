// ★RED 검체용 스텁 — v0.14.40 main.ts 의 **현행 규칙**을 그대로 옮긴 것(수정 전 동작의 재현).
//   · 좌석 추가 = 트리 전체 오른쪽에 비율 미지정(0.5) 부착(main.ts:2154-2156 · 8147-8149)
//   · `정렬` = 모든 컬럼 같은 폭(evenComb · main.ts:3697-3732) · cso 정확 일치
//   · 칸에 역할 기억 없음 · 복원 = 죽은 sid 제거(deadLiveSids) · 입양 대상 = 그 소켓의 첫 탭
// 다음 커밋(GREEN)이 이 파일을 실제 구현으로 교체한다. 이 스텁은 테스트가 실제 수치로 red 가 되는지 보이려는 것이다.
export type LNode =
  | { type: "split"; dir: "row" | "col"; ratio?: number; a: LNode; b: LNode }
  | { type: "pane"; sid: number; role?: string };
export type LPane = Extract<LNode, { type: "pane" }>;
export type LSplit = Extract<LNode, { type: "split" }>;
export type RoleOf = (sid: number) => string | null | undefined;

export const MASTER_FRAC = 1 / 3;
export const HEAD_COL_MASTER = 3 / 4;
export const ROLE_SLOT_GRACE_MS = 240_000;
export const MAX_HOLES_PER_TREE = 16;

export const isFreshRole = (_r: unknown): boolean => false;
export const isMasterRole = (r: unknown): boolean => r === "master";
export const isCsoRole = (r: unknown): boolean => r === "cso";
export const isHeadRole = (r: unknown): boolean => isMasterRole(r) || isCsoRole(r);
export const sanitizeRole = (_v: unknown): string | undefined => undefined;
export const seatPriority = (role: unknown): number =>
  role === "master" ? 0 : role === "cso" ? 1 : typeof role === "string" && role.startsWith("worker") ? 2 : typeof role === "string" && role.startsWith("reviewer") ? 3 : 4;

function walk(t: unknown, out: number[]): number[] {
  const o = t as { type?: string; sid?: number; a?: unknown; b?: unknown } | null;
  if (!o || typeof o !== "object") return out;
  if (o.type === "pane" && typeof o.sid === "number") out.push(o.sid);
  else if (o.type === "split") {
    walk(o.a, out);
    walk(o.b, out);
  }
  return out;
}
export const isValidTree = (t: unknown): t is LNode => !!t && typeof t === "object";
export const liveSidsOf = (t: unknown): number[] => walk(t, []).filter((s) => s > 0);
export const holeSidsOf = (t: unknown): number[] => walk(t, []).filter((s) => s < 0);
export const isHoleSid = (s: unknown): boolean => typeof s === "number" && s < 0;
export const nextHoleSid = (_trees: readonly unknown[]): number => -1;
export const nodeShown = (t: unknown, _h: (sid: number) => boolean): boolean => !!t;

export function evenComb(nodes: LNode[], dir: "row" | "col"): LNode {
  let acc = nodes[nodes.length - 1];
  for (let i = nodes.length - 2; i >= 0; i--) acc = { type: "split", dir, ratio: 1 / (nodes.length - i), a: nodes[i], b: acc };
  return acc;
}
export function legacyAppend(tree: LNode | null, sid: number, _role?: unknown): LNode {
  if (tree && walk(tree, []).indexOf(sid) >= 0) return tree;
  const p: LNode = { type: "pane", sid };
  return tree ? { type: "split", dir: "row", a: tree, b: p } : p;
}
export const placeSeat = (tree: LNode | null, sid: number, _r: RoleOf, _m: boolean): LNode => legacyAppend(tree, sid);
export const placeSeatSafe = placeSeat;
export const anchorHead = (tree: LNode | null, _r: RoleOf): LNode | null => tree;
export const anchorHeadSafe = anchorHead;
export const isAnchored = (_t: unknown, _r: RoleOf): boolean => false;
export function roleLayout(sids: readonly number[], roleOf: RoleOf): LNode | null {
  if (!sids.length) return null;
  const first = (role: string) => sids.filter((s) => roleOf(s) === role)[0];
  const master = first("master");
  const cso = first("cso");
  const agy = first("reviewer-gemini");
  const codex = first("reviewer-codex");
  const corners = new Set([master, cso, agy, codex].filter((x): x is number => x != null));
  const pane = (s: number): LNode => ({ type: "pane", sid: s });
  const columns: LNode[] = [];
  if (master != null && cso != null) columns.push({ type: "split", dir: "col", ratio: 3 / 4, a: pane(master), b: pane(cso) });
  else if (master != null) columns.push(pane(master));
  else if (cso != null) columns.push(pane(cso));
  for (const s of sids) if (!corners.has(s)) columns.push(pane(s));
  if (agy != null && codex != null) columns.push({ type: "split", dir: "col", ratio: 1 / 2, a: pane(agy), b: pane(codex) });
  else if (agy != null) columns.push(pane(agy));
  else if (codex != null) columns.push(pane(codex));
  return evenComb(columns, "row");
}
export const isAutoRatio = (_r: unknown): boolean => true;
export const looksManual = (_t: unknown): boolean => false;
export const annotateRoles = (_t: LNode | null, _r: RoleOf): boolean => false;
export const holdPane = (_t: LNode | null, _s: number, _h: number): LNode | null => null;
export const fillHole = (_t: LNode | null, _h: number, _s: number, _r?: unknown): LNode | null => null;
export const dropHole = (t: LNode | null, _h: number): LNode | null => t;
export interface RestoreOpts {
  genChanged: boolean;
  isLive: (sid: number) => boolean;
  liveRole: RoleOf;
  roleConflictIsDead: boolean;
  allocHole: () => number;
}
export function restoreTree(tree: LNode | null, o: RestoreOpts): LNode | null {
  let t = tree;
  for (const s of walk(tree, [])) {
    if (o.isLive(s)) continue;
    const rm = (n: LNode): LNode | null =>
      n.type === "pane" ? (n.sid === s ? null : n) : ((a, b) => (a && b ? { ...n, a, b } : a ?? b))(rm(n.a), rm(n.b));
    t = t ? rm(t) : null;
  }
  return t;
}
export interface WsView {
  socket?: string;
  tree: LNode | null;
  pending?: boolean;
  deleting?: boolean;
  layoutManual?: boolean;
}
export function pickAdoptIndex(wss: readonly WsView[], socket: string | undefined, _r: RoleOf): number {
  return wss.findIndex((w) => !w.pending && !w.deleting && (w.socket ?? undefined) === (socket ?? undefined));
}
export interface AdoptPlan {
  idx: number;
  tree: LNode;
  boundHole: number | null;
}
export function adoptSeat(wss: readonly WsView[], socket: string | undefined, sid: number, _role: unknown, r: RoleOf): AdoptPlan | null {
  const idx = pickAdoptIndex(wss, socket, r);
  return idx < 0 ? null : { idx, tree: legacyAppend(wss[idx].tree, sid), boundHole: null };
}
export const tidyHoles = (trees: readonly (LNode | null)[], _r: RoleOf): (LNode | null)[] => [...trees];
export interface DaemonIdent {
  epoch: string | null;
  startedAtMs: number | null;
}
export const daemonIdentOf = (_s: unknown): DaemonIdent => ({ epoch: null, startedAtMs: null });
export const generationChanged = (_s: unknown, _c: string | null): boolean | null => null;
export const reserveDeadline = (_a: number | null, _n: number, _g: number): number | null => null;
export const roleSlotText = (_r: unknown): string => "";
