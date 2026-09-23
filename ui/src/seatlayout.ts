// 좌석 배치(U3)와 역할 기억 결속(U2)의 **순수부** — main.ts 는 여기에 배선만 한다
// (wsreconcile.ts·deptlabel.ts 와 같은 관례: 순수 계산은 사이드 모듈 · DOM/부작용은 main.ts).
//
// ────────────────────────────────────────────────────────────────────────────
// ★무엇이 깨져 있었나 (0.14.41 오너 항목 U2·U3)
//
// U3 — 좌석이 생길 때마다 UI 가 새 pane 을 **트리 전체의 오른쪽에 50:50** 으로 붙였다(비율 미지정 =
//   0.5). 좌석 하나마다 대표(master) 칸이 절반이 된다: 1/2 → 1/4 → 1/8 → 1/16 …. `정렬` 도 모든
//   컬럼을 같은 폭으로 나눠 컬럼이 N 개면 대표가 1/N 이 됐다.
// U2 — 레이아웃 트리는 pane 을 surface id(sid) 로만 기억했다. sid 는 데몬 세대마다 새로 발급되므로
//   재부팅 뒤 저장 트리의 sid 는 전부 죽고, 복원이 새 창들을 **생성 순서(무작위)** 대로 오른쪽에
//   덧붙였다 — 대표가 엉뚱한 칸(실측: 6개 중 5번째)으로 갔다.
//
// ★수리의 형태
//   · 칸(pane 노드)에 **역할(role)** 도 적어 둔다. 역할의 진실원은 데몬(`surface.list` 의 role)이고
//     UI 는 "데몬이 X 라고 말한 창을 어디에 그릴지"만 정한다 — 역할을 주거나 뺏지 않는다.
//   · 주인이 없는 기억 칸 = **구멍(hole)** — sid 가 **음수**인 pane 노드다(role 필수).
//       - 구버전 앱(0.14.40)은 음수 sid 를 '죽은 sid' 로 보고 복원 때 버린다(하위 호환 · 다운그레이드 안전).
//       - main.ts collectSids 는 음수 sid 를 건너뛴다 → 닫기·포커스·RPC 경로가 구멍을 절대 보지 않는다.
//   · 새 좌석은 ① 같은 역할 구멍에 **결속**(칸·비율 그대로) → ② 없으면 placeSeat(대표 1/3 규칙).
//   · 대표(master) 칸 폭은 **단일 상수 MASTER_FRAC = 1/3** — U2 의 옛 0.25 폴백과 U3 의 1/3 이 충돌하던
//     것을 이 상수 하나로 없앴다(반박 U2 major ⓐ).
//
// ★불변식(이 모듈이 틀리면 무엇이 깨지는가 — 오너 4대 위험 ④ pane 전멸 · ③ 자가치유 전멸)
//   1. **전역(total)**: 어떤 JSON 을 넣어도 던지지 않는다. 손상 저장본 하나가 3초 입양 틱을 매번
//      죽이면 그 소켓의 새 좌석이 영영 화면에 붙지 않는다(③·④). 그래서 모든 공개 연산은 입력을
//      isValidTree 로 먼저 보고, *Safe 래퍼는 결과의 sid 집합까지 대조해 어긋나면 **종전 오른쪽 부착**
//      (legacyAppend) 또는 원 트리로 떨어진다(fail-open to legacy).
//   2. **보존**: 배치는 칸을 옮기기만 한다 — sid 를 잃거나(빈 칸) 겹치게(DOM 한 칸이 비는 중복) 하지 않는다.
//   3. **무부작용**: invoke·DOM·타이머·저장소를 import 하지도 부르지도 않는다(소스 핀 wswiring.test.ts).
//      최상위 부수효과 0. 구형 WKWebView 비호환 문법 0(정규식 lookbehind·배열 at·findLast 등).
// ────────────────────────────────────────────────────────────────────────────

/** 레이아웃 트리 노드. pane.role = 이 칸의 마지막 관측 역할(자리 기억 키). sid<0 = 구멍(주인 없는 기억 칸). */
export type LNode =
  | { type: "split"; dir: "row" | "col"; ratio?: number; a: LNode; b: LNode }
  | { type: "pane"; sid: number; role?: string };
export type LPane = Extract<LNode, { type: "pane" }>;
export type LSplit = Extract<LNode, { type: "split" }>;
/** sid → 역할. string=그 역할 · null=역할 없음(plain 셸·종료 좌석) · undefined=모름(칸의 기억으로 폴백). */
export type RoleOf = (sid: number) => string | null | undefined;

/** 대표(master) 컬럼 폭 — **유일한 정의처**(U2·U3 공용). pane 영역(#root) 기준 1/3. */
export const MASTER_FRAC = 1 / 3;
/** 대표 컬럼 안 master : cso 높이 = 3 : 1 (종전 `정렬` 과 동일). */
export const HEAD_COL_MASTER = 3 / 4;
/** 빈 기억 칸(구멍)을 '복원을 기다리는 중' 으로 보여 주는 기한. main.ts 가 winScaled(Windows ×2 = 480s). */
export const ROLE_SLOT_GRACE_MS = 240_000;
/** 트리 하나에 남길 구멍 상한 — 기억이 무한히 쌓이지 않게(역할당 1개 규칙과 별도의 바닥). */
export const MAX_HOLES_PER_TREE = 16;

const MAX_DEPTH = 256;
const ROLE_MAX_LEN = 64;
const CSO_N = /^cso-\d+$/;
const CTRL_CHARS = /[\u0000-\u001f\u007f]/;

// ---------- 역할 술어 (배치 전용 — 데몬의 경보·특권 술어와 목적이 다르다) ----------
// 데몬은 `cso*` 접두를 경보 라우팅·특권 판정에 쓰지만(alert_route.rs CSO_ROLE_PREFIX), 화면 배치에서
// 일회용 `cso-fresh-<epoch>`(스케줄 좌석 · TTL 600s)을 대표 칸에 넣으면 붙어 있는 동안 master 높이가
// 3/4 → 1/2 로 준다(반박 U3 D5). 그래서 대표 칸 소속은 `cso` 또는 `cso-<숫자>` 뿐이다.

/** 일회용 좌석(`<역할>-fresh-<epoch>`) — 기억하지 않고 대표 칸에도 넣지 않는다. */
export function isFreshRole(r: unknown): boolean {
  return typeof r === "string" && r.indexOf("-fresh-") >= 0;
}
export function isMasterRole(r: unknown): boolean {
  return r === "master";
}
export function isCsoRole(r: unknown): boolean {
  return typeof r === "string" && (r === "cso" || CSO_N.test(r));
}
/** 대표 칸 소속 — `정렬` 과 자동 배치가 **같은 술어**를 쓴다(반박 U3 D6). */
export function isHeadRole(r: unknown): boolean {
  return isMasterRole(r) || isCsoRole(r);
}

/**
 * 칸에 기억해도 되는 역할이면 그 문자열, 아니면 undefined.
 * 데몬은 예약어(owner·creator) 외 임의 문자열을 역할로 받는다 — 여기서 과하게 좁히면 그 역할은 조용히
 * 기억되지 않는다(반박 U2 §6). 그래서 막는 것은 저장·비교를 망가뜨릴 모양(비문자·빈값·과길이·제어문자·
 * 앞뒤 공백)과 일회용 좌석뿐이다.
 */
export function sanitizeRole(v: unknown): string | undefined {
  if (typeof v !== "string") return undefined;
  if (v.length === 0 || v.length > ROLE_MAX_LEN) return undefined;
  if (CTRL_CHARS.test(v) || v.trim() !== v) return undefined;
  if (isFreshRole(v)) return undefined;
  return v;
}

/** 입양 순서(master > cso > worker > reviewer > 그 밖) — 3초 틱과 기동 병합이 공용한다. */
export function seatPriority(role: unknown): number {
  if (isMasterRole(role)) return 0;
  if (isCsoRole(role)) return 1;
  if (typeof role === "string" && role.indexOf("worker") === 0) return 2;
  if (typeof role === "string" && role.indexOf("reviewer") === 0) return 3;
  return 4;
}

// ---------- 전역(total) 판독 ----------

function asObj(v: unknown): Record<string, unknown> | null {
  return v !== null && typeof v === "object" && !Array.isArray(v) ? (v as Record<string, unknown>) : null;
}

/**
 * 이 값이 이 모듈이 다룰 수 있는 트리인가. sid 는 0 이 아닌 안전 정수·중복 없음, ratio 는 (0,1) 또는
 * 미지정(null 포함 — JSON 왕복이 undefined 를 지우지 않는 경우), split 방향은 row/col.
 * 거짓이면 모든 공개 연산은 **아무것도 바꾸지 않거나 종전 동작**으로 떨어진다.
 */
export function isValidTree(t: unknown): t is LNode {
  const seen = new Set<number>();
  const walk = (v: unknown, depth: number): boolean => {
    const o = asObj(v);
    if (!o || depth > MAX_DEPTH) return false;
    if (o.type === "pane") {
      const sid = o.sid;
      if (typeof sid !== "number" || !Number.isSafeInteger(sid) || sid === 0) return false;
      if (o.role !== undefined && typeof o.role !== "string") return false;
      if (seen.has(sid)) return false;
      seen.add(sid);
      return true;
    }
    if (o.type === "split") {
      if (o.dir !== "row" && o.dir !== "col") return false;
      const r = o.ratio;
      if (r !== undefined && r !== null && !(typeof r === "number" && r > 0 && r < 1)) return false;
      return walk(o.a, depth + 1) && walk(o.b, depth + 1);
    }
    return false;
  };
  return walk(t, 0);
}

/** 트리의 pane sid 를 걷는다 — 손상 노드는 건너뛴다(던지지 않는다). */
function walkSids(t: unknown, out: number[], depth: number): number[] {
  const o = asObj(t);
  if (!o || depth > MAX_DEPTH) return out;
  if (o.type === "pane") {
    if (typeof o.sid === "number" && Number.isSafeInteger(o.sid)) out.push(o.sid);
  } else if (o.type === "split") {
    walkSids(o.a, out, depth + 1);
    walkSids(o.b, out, depth + 1);
  }
  return out;
}
/** 살아 있을 수 있는(양수) sid — 구멍 제외. 전역. */
export function liveSidsOf(t: unknown): number[] {
  return walkSids(t, [], 0).filter((s) => s > 0);
}
/** 구멍(음수) sid. 전역. */
export function holeSidsOf(t: unknown): number[] {
  return walkSids(t, [], 0).filter((s) => s < 0);
}
export function isHoleSid(sid: unknown): boolean {
  return typeof sid === "number" && Number.isSafeInteger(sid) && sid < 0;
}

/** 새 구멍 sid — 주어진 모든 트리의 구멍보다 작은 음수(탭 간에도 유일). */
export function nextHoleSid(trees: readonly unknown[]): number {
  let min = 0;
  for (const t of trees) for (const s of holeSidsOf(t)) if (s < min) min = s;
  return min - 1;
}

/**
 * 이 노드가 화면에 **보이는가**. 양수 sid 칸은 항상 보인다(런타임이 없으면 종전처럼 `(없음)` 자리표시).
 * 구멍은 holeShown(sid) 가 참일 때(보류 기한 안)만 보이고, 기한이 지나면 **화면에서만 접힌다** —
 * 기억(트리의 구멍)은 저장본에 남는다(반박 U2 minor: 늦게 오는 역할의 자리 기억 보존).
 * split 은 한쪽이라도 보이면 보인다(안 보이는 쪽은 renderNode 가 접는다). 전역.
 */
export function nodeShown(t: unknown, holeShown: (sid: number) => boolean): boolean {
  const walk = (v: unknown, depth: number): boolean => {
    const o = asObj(v);
    if (!o || depth > MAX_DEPTH) return false;
    if (o.type === "pane") {
      // 숨길 수 있는 것은 **음수 숫자 sid(구멍)** 뿐이다 — 그 밖의 모양은 종전 렌더러처럼 보이게 둔다
      // (틀리게 숨기면 산 칸이 idle 패널 뒤로 사라진다 · ④ 쪽으로 무너지지 않게 보수적으로).
      if (typeof o.sid !== "number" || !(o.sid < 0)) return true;
      try {
        return holeShown(o.sid) === true;
      } catch {
        return false;
      }
    }
    if (o.type === "split") return walk(o.a, depth + 1) || walk(o.b, depth + 1);
    return false;
  };
  return walk(t, 0);
}

// ---------- 내부 도우미 (유효 트리 전제) ----------

function panesOf(t: LNode | null, out: LPane[] = []): LPane[] {
  if (!t) return out;
  if (t.type === "pane") out.push(t);
  else {
    panesOf(t.a, out);
    panesOf(t.b, out);
  }
  return out;
}

/** 칸의 실효 역할: 구멍은 기억한 역할, 산 칸은 데몬 역할 — 데몬이 모르면(undefined) 칸의 기억. */
function effRole(p: LPane, roleOf: RoleOf): string | null | undefined {
  if (p.sid < 0) return p.role;
  const r = roleOf(p.sid);
  return r === undefined ? p.role : r;
}
const isBare = (r: string | null | undefined): boolean => r === null || r === undefined;

function mkPane(sid: number, role?: unknown): LPane {
  const r = sanitizeRole(role);
  return r === undefined ? { type: "pane", sid } : { type: "pane", sid, role: r };
}

function safeRole(roleOf: RoleOf, sid: number): string | null | undefined {
  try {
    return roleOf(sid);
  } catch {
    return undefined;
  }
}

/** sid 칸을 떼고 형제로 접는다(main.ts replaceNode 의 제거 특수형과 같은 의미). */
function removeSid(t: LNode, sid: number): LNode | null {
  if (t.type === "pane") return t.sid === sid ? null : t;
  const a = removeSid(t.a, sid);
  const b = removeSid(t.b, sid);
  if (a && b) return { ...t, a, b };
  return a ?? b;
}

/** sid 칸을 다른 노드로 바꾼다(모양·비율 보존). */
function replacePane(t: LNode, sid: number, next: LNode): LNode {
  if (t.type === "pane") return t.sid === sid ? next : t;
  const a = replacePane(t.a, sid, next);
  const b = replacePane(t.b, sid, next);
  return a === t.a && b === t.b ? t : { ...t, a, b };
}

function hasLive(t: LNode): boolean {
  return panesOf(t).some((p) => p.sid > 0);
}
/** row 척추의 **보이는 컬럼** 수 — 구멍만으로 된 컬럼은 0(접힐 수 있으므로 균등 분배에서 뺀다). */
function colsVis(t: LNode): number {
  if (t.type === "split" && t.dir === "row") return colsVis(t.a) + colsVis(t.b);
  return hasLive(t) ? 1 : 0;
}
/** col 척추의 칸 수(구멍 포함 — 대표 칸의 기억 자리는 높이를 지킨다). */
function cellsAll(t: LNode): number {
  return t.type === "split" && t.dir === "col" ? cellsAll(t.a) + cellsAll(t.b) : 1;
}

/** 같은 폭 컬럼(또는 같은 높이 칸)으로 나란히 — `정렬` 의 기본 빗질. nodes 는 1개 이상. */
export function evenComb(nodes: LNode[], dir: "row" | "col"): LNode {
  let acc = nodes[nodes.length - 1];
  for (let i = nodes.length - 2; i >= 0; i--) acc = { type: "split", dir, ratio: 1 / (nodes.length - i), a: nodes[i], b: acc };
  return acc;
}

/** 새 컬럼을 균등 몫으로 덧붙인다(기존 컬럼끼리의 상대 비율 유지 · 새 컬럼 = 1/(n+1)). */
function appendColumn(t: LNode, p: LNode): LNode {
  const n = colsVis(t);
  return { type: "split", dir: "row", ratio: n > 0 ? n / (n + 1) : 0.5, a: t, b: p };
}
/** 대표 컬럼 아래에 칸을 쌓는다(master:cso = 3:1, 그 뒤는 균등 몫). */
function appendCell(t: LNode, p: LNode): LNode {
  const m = cellsAll(t);
  return { type: "split", dir: "col", ratio: m === 1 ? HEAD_COL_MASTER : m / (m + 1), a: t, b: p };
}

function hasMasterIn(t: LNode, roleOf: RoleOf): boolean {
  return panesOf(t).some((p) => isMasterRole(effRole(p, roleOf)));
}
function allHead(t: LNode, roleOf: RoleOf): boolean {
  const ps = panesOf(t);
  return ps.length > 0 && ps.every((p) => isHeadRole(effRole(p, roleOf)));
}
/** 대표 컬럼이 될 수 있는 부분 트리: 대표 역할이 1개 이상이고 나머지는 대표 역할이거나 역할 없는 셸. */
function headColumnLike(t: LNode, roleOf: RoleOf): boolean {
  const ps = panesOf(t);
  let head = false;
  for (const p of ps) {
    const r = effRole(p, roleOf);
    if (isHeadRole(r)) head = true;
    else if (!isBare(r)) return false;
  }
  return head;
}
/** 루트가 row 분할이고 왼쪽(a)에 master(산 칸 또는 master 구멍)가 있다 — 새 좌석은 오른쪽(b)으로 간다. */
function headSide(t: LNode | null, roleOf: RoleOf): t is LSplit {
  return !!t && t.type === "split" && t.dir === "row" && hasMasterIn(t.a, roleOf);
}
/**
 * 앵커(대표 칸이 제자리): 루트 row 분할의 왼쪽에 master 가 있고, 대표 역할 칸은 **전부** 왼쪽에 있으며,
 * 왼쪽에는 대표 역할과 역할 없는 셸만 있다.
 * ★'전부' 조건이 없으면 master 만 왼쪽에 있어도 참이 되어 흩어진 cso 를 모으지 않는다 —
 *   표준 6석 도착 순서 336/720 에서 cso 가 대표 칸 밖에 남던 결함(반박 U3 D1)이 여기서 닫힌다.
 * ★역할 없는 셸을 허용하는 이유: master 옆에서 ⌘D 를 한 탭(대표 칸에 셸이 섞임)을 '앵커 아님' 으로
 *   보면 새 좌석이 루트에 붙어 대표가 1/n 씩 끝없이 준다(반박 U3 D2).
 */
function anchoredCore(t: LNode, roleOf: RoleOf): t is LSplit {
  if (!headSide(t, roleOf)) return false;
  return headColumnLike(t.a, roleOf) && !panesOf(t.b).some((p) => isHeadRole(effRole(p, roleOf)));
}

/** 대표 컬럼: 첫 master 가 위(3/4), 나머지 대표(추가 master·cso)가 아래를 균등히. masters 는 1개 이상. */
function headColumn(masters: LNode[], csos: LNode[]): LNode {
  const top = masters[0];
  const rest = [...masters.slice(1), ...csos];
  if (!top) return evenComb(rest, "col");
  if (!rest.length) return top;
  return { type: "split", dir: "col", ratio: HEAD_COL_MASTER, a: top, b: evenComb(rest, "col") };
}

/**
 * row 척추의 비율을 **보이는 컬럼 수** 기준 균등으로 다시 맞춘다(컬럼 하위 트리는 보존).
 * 맨 끝이 아닌 컬럼이 빠지거나 유령 sid 가 섞인 틱에 입양되면 한쪽으로 쏠리던 폭(반박 U3 D7)을 편다.
 */
function rebalanceRow(t: LNode): LNode {
  if (t.type !== "split" || t.dir !== "row") return t;
  const a = rebalanceRow(t.a);
  const b = rebalanceRow(t.b);
  const ca = colsVis(a);
  const cb = colsVis(b);
  if (ca > 0 && cb > 0) return { ...t, ratio: ca / (ca + cb), a, b };
  return a === t.a && b === t.b ? t : { ...t, a, b };
}

// ---------- U3 배치 ----------

/** 종전 규칙 그대로: 트리 전체 오른쪽에 비율 미지정(=0.5)으로. 이미 있으면 그대로(중복 = 빈 칸). 전역. */
export function legacyAppend(tree: LNode | null, sid: number, role?: unknown): LNode {
  if (tree && walkSids(tree, [], 0).indexOf(sid) >= 0) return tree;
  const p = mkPane(sid, role);
  return tree ? { type: "split", dir: "row", a: tree, b: p } : p;
}

/** 보수적 삽입 — 대표 컬럼이 제자리면 폭을 건드리지 않는다. */
function insertSeat(tree: LNode | null, sid: number, roleOf: RoleOf): LNode {
  const role = roleOf(sid);
  const p = mkPane(sid, role);
  if (!tree) return p;
  if (isMasterRole(role)) {
    // 대표 컬럼(cso·master 구멍·셸)이 이미 왼쪽에 있으면 그 **위**로 — master 재기동이 제자리로 돌아온다.
    if (tree.type === "split" && tree.dir === "row" && headColumnLike(tree.a, roleOf)) {
      return { ...tree, a: { type: "split", dir: "col", ratio: HEAD_COL_MASTER, a: p, b: tree.a } };
    }
    if (headColumnLike(tree, roleOf)) return { type: "split", dir: "col", ratio: HEAD_COL_MASTER, a: p, b: tree };
    return { type: "split", dir: "row", ratio: MASTER_FRAC, a: p, b: tree };
  }
  if (isCsoRole(role)) {
    if (headSide(tree, roleOf)) return { ...tree, a: appendCell(tree.a, p) };
    if (allHead(tree, roleOf) && hasMasterIn(tree, roleOf)) return appendCell(tree, p);
    // master 가 없으면 대표로 세우지 않는다(마스터 재기동 공백에 cso 가 대표 자리를 가로채지 않게) — 아래 일반 규칙.
  }
  if (headSide(tree, roleOf)) return { ...tree, b: appendColumn(tree.b, p) };
  if (allHead(tree, roleOf) && hasMasterIn(tree, roleOf)) return { type: "split", dir: "row", ratio: MASTER_FRAC, a: tree, b: p };
  // 대표가 없는 탭은 **종전 규칙 그대로**(반박 U3 D12 — 오너 지시 범위는 대표 자리 · 놀람 최소화).
  if (!hasMasterIn(tree, roleOf)) return { type: "split", dir: "row", a: tree, b: p };
  // master 가 왼쪽이 아닌 곳에 있다(사용자가 옮긴 수동 탭) — 절반이 아니라 균등 몫으로.
  return appendColumn(tree, p);
}

function anchorHeadCore(t: LNode, roleOf: RoleOf): LNode {
  const ps = panesOf(t);
  const masters = ps.filter((p) => isMasterRole(effRole(p, roleOf)));
  if (!masters.length) return t; // 대표 없는 탭 — 1/3 규칙 미적용
  if (ps.every((p) => isHeadRole(effRole(p, roleOf)))) return t; // 대표만 있는 탭 — 전체 폭
  if (anchoredCore(t, roleOf)) return { ...t, ratio: MASTER_FRAC, b: rebalanceRow(t.b) };
  // 대표 역할이 흩어져 있다(구버전 저장본·도착 순서) — master·cso 만 빼서 왼쪽 1/3 컬럼으로, 나머지 구조는 그대로.
  const ordered = [...masters.filter((p) => p.sid > 0), ...masters.filter((p) => p.sid < 0)]; // 산 master 가 위
  const csos = ps.filter((p) => isCsoRole(effRole(p, roleOf)));
  let rest: LNode | null = t;
  for (const p of [...ordered, ...csos]) rest = rest ? removeSid(rest, p.sid) : null;
  const head = headColumn(ordered, csos);
  return rest ? { type: "split", dir: "row", ratio: MASTER_FRAC, a: head, b: rebalanceRow(rest) } : head;
}

/**
 * 모든 자동 경로(3초 입양·기동 병합·+New·전출 런처 셸)의 단일 진입점.
 * manual=true(사용자가 분할선·pane 을 끈 탭)는 **재배치·비율 리셋 금지**만 뜻한다 — 그래도 새 좌석은
 * 대표 컬럼 밖(root.b)으로 가서 대표 폭이 줄지 않는다.
 */
export function placeSeat(tree: LNode | null, sid: number, roleOf: RoleOf, manual: boolean): LNode {
  if (!Number.isSafeInteger(sid) || sid <= 0 || (tree !== null && !isValidTree(tree))) {
    return legacyAppend(tree, sid, safeRole(roleOf, sid));
  }
  if (tree && panesOf(tree).some((p) => p.sid === sid)) return tree; // 멱등 — 중복 sid 는 DOM 한 칸을 비운다(④)
  const t1 = insertSeat(tree, sid, roleOf);
  return manual ? t1 : anchorHeadCore(t1, roleOf);
}

function sameMembers(before: LNode | null, after: unknown, added?: number): boolean {
  if (!isValidTree(after)) return false;
  const want = new Set(before ? panesOf(before).map((p) => p.sid) : []);
  if (added !== undefined) want.add(added);
  const got = panesOf(after).map((p) => p.sid);
  return got.length === want.size && got.every((s) => want.has(s));
}

/**
 * placeSeat 의 안전 래퍼 — 예외·불변식 위반(sid 손실·중복·손상 트리) 이면 **종전 오른쪽 부착**으로.
 * 새 규칙이 틀려도 결과는 "예전처럼 절반" 으로만 떨어진다(③ 자가치유·④ 빈 칸 방어).
 */
export function placeSeatSafe(tree: LNode | null, sid: number, roleOf: RoleOf, manual: boolean): LNode {
  try {
    if (tree === null || isValidTree(tree)) {
      const out = placeSeat(tree, sid, roleOf, manual);
      if (sameMembers(tree, out, sid)) return out;
    }
  } catch {
    /* 아래 종전 규칙 */
  }
  return legacyAppend(tree, sid, safeRole(roleOf, sid));
}

/** 대표 칸을 왼쪽 1/3 로 세운다(수동 탭에는 부르지 않는다). 대표가 없거나 대표만 있으면 그대로. 전역. */
export function anchorHead(tree: LNode | null, roleOf: RoleOf): LNode | null {
  if (!tree || !isValidTree(tree)) return tree;
  return anchorHeadCore(tree, roleOf);
}
/** anchorHead 안전 래퍼 — 실패·불변식 위반이면 **원 트리** 그대로(기동 S5·틱 재정렬용). */
export function anchorHeadSafe(tree: LNode | null, roleOf: RoleOf): LNode | null {
  if (!tree) return tree;
  try {
    if (!isValidTree(tree)) return tree;
    const out = anchorHeadCore(tree, roleOf);
    return sameMembers(tree, out) ? out : tree;
  } catch {
    return tree;
  }
}
/** 대표 칸이 제자리인가(테스트·배선 판독용). 전역. */
export function isAnchored(tree: unknown, roleOf: RoleOf): boolean {
  if (!isValidTree(tree)) return false;
  try {
    return anchoredCore(tree, roleOf);
  } catch {
    return false;
  }
}

/**
 * `정렬`: 대표 컬럼(master 위 / cso 아래 3:1) 1/3 · 가운데 worker·미분류 균등 · 오른쪽 끝 리뷰어 묶음
 * (agy 위 / codex 아래). 대표 역할이 없으면 종전처럼 모든 컬럼 균등. sids 가 비면 null.
 */
export function roleLayout(sids: readonly number[], roleOf: RoleOf): LNode | null {
  const uniq = sids.filter((s, i) => Number.isSafeInteger(s) && s > 0 && sids.indexOf(s) === i);
  if (!uniq.length) return null;
  const r = (s: number) => safeRole(roleOf, s);
  const pane = (s: number): LNode => mkPane(s, r(s));
  const masters = uniq.filter((s) => isMasterRole(r(s)));
  const csos = uniq.filter((s) => isCsoRole(r(s)));
  const agy = uniq.filter((s) => r(s) === "reviewer-gemini")[0];
  const codex = uniq.filter((s) => r(s) === "reviewer-codex")[0];
  const placed = new Set<number>([...masters, ...csos]);
  if (agy !== undefined) placed.add(agy);
  if (codex !== undefined) placed.add(codex);
  const head = masters.length || csos.length ? headColumn(masters.map(pane), csos.map(pane)) : null;
  const rest: LNode[] = uniq.filter((s) => !placed.has(s)).map(pane);
  if (agy !== undefined && codex !== undefined) rest.push({ type: "split", dir: "col", ratio: 1 / 2, a: pane(agy), b: pane(codex) });
  else if (agy !== undefined) rest.push(pane(agy));
  else if (codex !== undefined) rest.push(pane(codex));
  if (!head) return evenComb(rest, "row");
  if (!rest.length) return head;
  return { type: "split", dir: "row", ratio: MASTER_FRAC, a: head, b: evenComb(rest, "row") };
}

// ---------- 수동 탭 추정(구버전 저장본 1회 이관) ----------

/** 자동 배치가 만드는 비율인가: 미지정 또는 분모 16 이하의 단순 분수(1/k·3/4·n/(n+1)·균등 재분배). */
export function isAutoRatio(r: unknown): boolean {
  if (r === undefined || r === null) return true;
  if (typeof r !== "number" || !Number.isFinite(r)) return false;
  for (let q = 2; q <= 16; q++) {
    const p = Math.round(r * q);
    if (p >= 1 && p < q && Math.abs(r - p / q) < 1e-9) return true;
  }
  return false;
}
/**
 * 구버전 저장본(수동 표식 필드가 없던 시절)의 탭을 사용자가 손댔는가 — 자동값이 아닌 비율이 하나라도
 * 있으면 참. 분할선을 끌면 pos/size 라는 임의 실수가 되고, 드래그 클램프 0.15·0.85 도 분모 20 이라 여기서
 * 걸린다. 오너의 CEO 탭(0.25·0.75·1/3·0.5·0.5)은 전부 자동값이라 1/3 로 고쳐진다(반박 U3 D3). 전역.
 */
export function looksManual(tree: unknown): boolean {
  const walk = (v: unknown, depth: number): boolean => {
    const o = asObj(v);
    if (!o || depth > MAX_DEPTH || o.type !== "split") return false;
    return !isAutoRatio(o.ratio) || walk(o.a, depth + 1) || walk(o.b, depth + 1);
  };
  return walk(tree, 0);
}

// ---------- U2 역할 기억 · 결속 ----------

/**
 * 산 칸에 데몬 역할을 적는다(제자리 변경). 역할이 바뀌면 갱신, 역할이 없어지면(plain 셸·일회용) 지운다.
 * liveRole 이 undefined 인 sid(모름·종료 좌석)와 구멍은 **건드리지 않는다** — 종료 직전 기억을 지키려고.
 * 바뀐 것이 있으면 참(호출측은 그때만 저장한다 · 매 틱 쓰기 0). 전역.
 */
export function annotateRoles(tree: LNode | null, liveRole: RoleOf): boolean {
  if (!tree || !isValidTree(tree)) return false;
  let changed = false;
  for (const p of panesOf(tree)) {
    if (p.sid < 0) continue;
    let r: string | null | undefined;
    try {
      r = liveRole(p.sid);
    } catch {
      continue;
    }
    if (r === undefined) continue;
    const s = sanitizeRole(r);
    if (s === undefined) {
      if (p.role !== undefined) {
        delete p.role;
        changed = true;
      }
    } else if (p.role !== s) {
      p.role = s;
      changed = true;
    }
  }
  return changed;
}

/**
 * 역할을 기억한 칸 sid 를 구멍으로 바꾼다(창은 사라져도 자리는 남긴다 — removeDeadPane·유령 수렴).
 * 역할 기억이 없거나(셸) 일회용이면 null → 호출측은 종전대로 칸을 뗀다. 전역.
 */
export function holdPane(tree: LNode | null, sid: number, holeSid: number): LNode | null {
  if (!tree || !isValidTree(tree) || !isHoleSid(holeSid) || !(sid > 0)) return null;
  const ps = panesOf(tree);
  if (ps.some((p) => p.sid === holeSid)) return null;
  const p = ps.filter((x) => x.sid === sid)[0];
  const role = p ? sanitizeRole(p.role) : undefined;
  if (!p || role === undefined) return null;
  return replacePane(tree, sid, { type: "pane", sid: holeSid, role });
}

/** 구멍 하나를 다른 칸으로(결속 · [새 셸 열기]). 없으면 null. */
export function fillHole(tree: LNode | null, holeSid: number, sid: number, role?: unknown): LNode | null {
  if (!tree || !isValidTree(tree) || !isHoleSid(holeSid) || !(sid > 0)) return null;
  const ps = panesOf(tree);
  if (!ps.some((p) => p.sid === holeSid) || ps.some((p) => p.sid === sid)) return null;
  return replacePane(tree, holeSid, mkPane(sid, role));
}

/** 구멍 하나를 지운다([칸 비우기]). 없으면 원 트리. */
export function dropHole(tree: LNode | null, holeSid: number): LNode | null {
  if (!tree || !isValidTree(tree) || !isHoleSid(holeSid)) return tree;
  return removeSid(tree, holeSid);
}

export interface RestoreOpts {
  /** 데몬 세대가 바뀌었다 — 옛 sid 는 살아 있어 보여도 **전부 죽은 것**(sid 재사용 경로 · 반박 U2 §3-1). */
  genChanged: boolean;
  /** 이 sid 가 지금 살아 있나(종료 제외). */
  isLive: (sid: number) => boolean;
  /** 산 sid 의 현재 역할(undefined = 모름). */
  liveRole: RoleOf;
  /** 세대를 모를 때: 기억한 역할과 현재 역할이 다르면 다른 surface 로 본다(재사용 방어). */
  roleConflictIsDead: boolean;
  /** 새 구멍 sid 발급기(호출측이 전 탭 유일성을 보장). */
  allocHole: () => number;
}

/**
 * 기동 복원: 저장 트리의 죽은 칸을 **역할이 있으면 구멍으로**, 없으면 종전처럼 떼어 낸다. 산 칸은 그대로.
 * 결과 트리의 양수 sid 는 전부 산 것이다. 손상 트리는 그대로 돌려준다(호출측 종전 경로가 처리). 전역.
 */
export function restoreTree(tree: LNode | null, o: RestoreOpts): LNode | null {
  if (!tree || !isValidTree(tree)) return tree;
  const walk = (n: LNode): LNode | null => {
    if (n.type === "pane") {
      if (n.sid < 0) return sanitizeRole(n.role) !== undefined ? n : null; // 역할 없는 구멍은 쓸모가 없다
      const remembered = sanitizeRole(n.role);
      let dead = o.genChanged || !o.isLive(n.sid);
      if (!dead && o.roleConflictIsDead && remembered !== undefined) {
        const cur = o.liveRole(n.sid);
        if (cur !== undefined && sanitizeRole(cur) !== remembered) dead = true;
      }
      if (!dead) return n;
      return remembered !== undefined ? { type: "pane", sid: o.allocHole(), role: remembered } : null;
    }
    const a = walk(n.a);
    const b = walk(n.b);
    if (a && b) return a === n.a && b === n.b ? n : { ...n, a, b };
    return a ?? b;
  };
  return walk(tree);
}

/** 결속·입양에 쓰는 워크스페이스의 최소 필드. */
export interface WsView {
  socket?: string;
  tree: LNode | null;
  pending?: boolean;
  deleting?: boolean;
  layoutManual?: boolean;
}

/** 입양과 **같은** 소켓 판정(정확 일치) — sameSocket 정규화를 쓰면 결속과 입양이 갈린다(반박 U2 §2-4). */
function sameSock(a: string | undefined, b: string | undefined): boolean {
  return (a ?? undefined) === (b ?? undefined);
}

/**
 * 고아 좌석을 붙일 탭: 그 소켓의 입양 가능 탭 중 **master 가 있는 탭**(산 칸 또는 master 구멍) 우선,
 * 없으면 첫 탭(반박 U3 M1 · U2 §3-3 — '소켓의 첫 탭' 이 CEO 탭이 아닐 수 있다). 없으면 -1. 전역.
 */
export function pickAdoptIndex(wss: readonly WsView[], socket: string | undefined, roleOf: RoleOf): number {
  let first = -1;
  for (let i = 0; i < wss.length; i++) {
    const w = wss[i];
    if (!w || w.pending || w.deleting || !sameSock(w.socket, socket)) continue;
    if (first < 0) first = i;
    try {
      if (w.tree && isValidTree(w.tree) && hasMasterIn(w.tree, roleOf)) return i;
    } catch {
      /* 다음 탭 */
    }
  }
  return first;
}

export interface AdoptPlan {
  /** wss 안의 대상 탭 인덱스 */
  idx: number;
  /** 그 탭의 새 트리 */
  tree: LNode;
  /** 같은 역할 구멍에 결속했으면 그 구멍 sid(호출측이 보류 기한을 지운다), 아니면 null */
  boundHole: number | null;
}

/**
 * 좌석 하나의 입양 계획: ① 같은 역할 구멍(대상 탭 먼저, 이어 탭 순서·DFS 순)에 **결속**(칸·비율 그대로 ·
 * 비수동 탭이면 결속 뒤 대표 1/3 재정렬) → ② 없으면 대상 탭에 placeSeatSafe. 입양 가능 탭이 없으면 null.
 * 순수 — wss 를 바꾸지 않는다(호출측이 결과 트리를 대입). 어떤 실패도 **종전 오른쪽 부착**으로 떨어진다.
 */
export function adoptSeat(
  wss: readonly WsView[],
  socket: string | undefined,
  sid: number,
  role: unknown,
  roleOf: RoleOf,
): AdoptPlan | null {
  let target = -1;
  try {
    target = pickAdoptIndex(wss, socket, roleOf);
  } catch {
    target = -1;
  }
  if (target < 0) {
    for (let i = 0; i < wss.length; i++) {
      const w = wss[i];
      if (w && !w.pending && !w.deleting && sameSock(w.socket, socket)) {
        target = i;
        break;
      }
    }
  }
  if (target < 0) return null;
  const tw = wss[target];
  try {
    const r = sanitizeRole(role);
    if (r !== undefined) {
      const order = [target];
      for (let i = 0; i < wss.length; i++) if (i !== target) order.push(i);
      for (const i of order) {
        const w = wss[i];
        if (!w || w.pending || w.deleting || !sameSock(w.socket, socket) || !w.tree || !isValidTree(w.tree)) continue;
        const hole = panesOf(w.tree).filter((p) => p.sid < 0 && p.role === r)[0];
        if (!hole) continue;
        const filled = fillHole(w.tree, hole.sid, sid, r);
        if (!filled) continue;
        const tree = w.layoutManual ? filled : (anchorHeadSafe(filled, roleOf) ?? filled);
        return { idx: i, tree, boundHole: hole.sid };
      }
    }
    return { idx: target, tree: placeSeatSafe(tw.tree, sid, roleOf, !!tw.layoutManual), boundHole: null };
  } catch {
    return { idx: target, tree: legacyAppend(tw.tree, sid, role), boundHole: null };
  }
}

/**
 * 구멍 위생(소켓 하나의 트리들, 탭 순서): ① 역할 기억이 없는 구멍 ② 그 역할이 이미 산 칸으로 있는 구멍
 * (1:1 — 역할 하나에 칸 하나) ③ 같은 역할의 두 번째 이후 구멍 ④ 트리당 상한 초과분을 지운다.
 * 바뀌지 않은 트리는 같은 객체를 돌려준다. 손상 트리는 그대로. 전역.
 */
export function tidyHoles(trees: readonly (LNode | null)[], roleOf: RoleOf): (LNode | null)[] {
  const present = new Set<string>();
  for (const t of trees) {
    if (!t || !isValidTree(t)) continue;
    for (const p of panesOf(t)) {
      if (p.sid < 0) continue;
      const r = sanitizeRole(safeRole(roleOf, p.sid) === undefined ? p.role : safeRole(roleOf, p.sid));
      if (r !== undefined) present.add(r);
    }
  }
  const seen = new Set<string>();
  return trees.map((t) => {
    if (!t || !isValidTree(t)) return t;
    let kept = 0;
    const drop: number[] = [];
    for (const p of panesOf(t)) {
      if (p.sid > 0) continue;
      const r = sanitizeRole(p.role);
      if (r === undefined || present.has(r) || seen.has(r) || kept >= MAX_HOLES_PER_TREE) {
        drop.push(p.sid);
        continue;
      }
      seen.add(r);
      kept++;
    }
    let out: LNode | null = t;
    for (const s of drop) out = out ? removeSid(out, s) : null;
    return out;
  });
}

// ---------- 데몬 세대 · 보류 기한 ----------

export interface DaemonIdent {
  /** `${started_at}-${daemon_pid}` — 둘 다 있어야 값이 있다 */
  epoch: string | null;
  /** 데몬 기동 시각(ms) */
  startedAtMs: number | null;
}
/** system.identify(= daemon_status) 응답에서 세대·기동 시각을 읽는다. 전역. */
export function daemonIdentOf(status: unknown): DaemonIdent {
  const o = asObj(status);
  const st = o ? o.started_at : undefined;
  const pid = o ? o.daemon_pid : undefined;
  const okSt = typeof st === "number" && Number.isFinite(st) && st > 0;
  const okPid = typeof pid === "number" && Number.isSafeInteger(pid) && pid > 0;
  return { epoch: okSt && okPid ? `${st}-${pid}` : null, startedAtMs: okSt ? (st as number) * 1000 : null };
}
/** 저장한 세대와 지금 세대가 다른가. 어느 한쪽이라도 모르면 null(=모름 · 생존 판정 + 역할 충돌 검사로). */
export function generationChanged(stored: unknown, current: string | null): boolean | null {
  if (typeof stored !== "string" || !stored || !current) return null;
  return stored !== current;
}
/**
 * 기동 복원의 구멍 보류 기한(ms). 데몬이 **젊을 때만**(기동 후 grace 미만) 건다 — 앱을 늦게 켜서 복원이
 * 이미 끝난 경로에서는 보류 없이 곧바로 접는다(반박 U2 major ⓒ). 모르면 null(보류 안 함).
 */
export function reserveDeadline(daemonStartedAtMs: number | null, nowMs: number, graceMs: number): number | null {
  if (daemonStartedAtMs === null || !Number.isFinite(daemonStartedAtMs) || !Number.isFinite(nowMs)) return null;
  const until = Math.min(daemonStartedAtMs + graceMs, nowMs + graceMs);
  return until > nowMs ? until : null;
}

/** 보류 칸 안내 문구(textContent 전용). */
export function roleSlotText(role: unknown): string {
  const r = sanitizeRole(role);
  if (r === "master") return "대표(master) 자리 — 복원을 기다리는 중입니다";
  return `${r ?? "역할"} 자리 — 복원을 기다리는 중입니다`;
}
