// ★S1 수정1(오너 2026-11-22 「CSO 는 리뷰어 컬럼 맨 아래 · 대표 칸은 master 혼자」) — main.ts 배선.
//
// 모양의 규칙은 seatlayout.ts 가 정하고 seatlayout.test.ts·seatbind.test.ts 가 잰다. 여기는 그 규칙이 **앱 시작**에
// 실제로 걸려 있는가를 잰다 — 소스 문자열 핀이 아니라 main.ts 의 **실제 기동 S5 루프**와 **실제 saveLayout** 을 그대로
// 떼어 실행한다:
//   · 자동(비수동) 탭: 구버전 모양으로 저장된 탭이 기동에서 새 표준 모양으로 선다(매 기동).
//   · 수동 탭: 다시 짜지 않는다. 다만 이 판 이전의 저장본이면 CSO 한 칸만 **한 번** 리뷰어 열 맨 아래로 옮긴다.
//     한 번 옮긴 저장본(csoSide: 1)은 다음 기동이 다시 옮기지 않는다 — 주인이 그 뒤에 끌어 둔 배치를 지킨다.
//   · 그 「한 번 했다」는 저장본에 남는다(saveLayout) · 못 했으면 남지 않는다(다음 기동이 한다).
import { describe, it, expect } from "bun:test";
import { readFileSync } from "node:fs";

// tsconfig.check.json 은 @types 를 쓰지 않는다 — Bun 전역은 쓰는 만큼만 여기서 선언한다(deptprogresswiring.test.ts 와 같은 관례).
declare const Bun: { Transpiler: new (o: { loader: "ts" }) => { transformSync(code: string): string } };
import * as SL from "./seatlayout";
import { MASTER_FRAC, anchorHeadSafe, csoMovePending, type LNode } from "./seatlayout";

const MAIN = readFileSync(new URL("./main.ts", import.meta.url), "utf8");
const HTML = readFileSync(new URL("../index.html", import.meta.url), "utf8");
const TS = new Bun.Transpiler({ loader: "ts" });
// 이 판에서 생기는 함수 — 없으면(구판) undefined 다. 구판에서 import 오류로 파일 전체가 죽지 않게 이름으로 꺼낸다.
const moveCsoToSide = (SL as unknown as Record<string, unknown>).moveCsoToSide;

function realFunction(name: string): string {
  const start = MAIN.indexOf(`\nfunction ${name}(`);
  if (start < 0) throw new Error(`main.ts 에 function ${name} 이 없다`);
  const end = MAIN.indexOf("\n}\n", start);
  if (end < 0) throw new Error(`function ${name} 의 끝을 찾지 못했다`);
  return TS.transformSync(MAIN.slice(start + 1, end + 2));
}

// ---------- 화면 사각형 오라클(렌더러와 같은 규칙) ----------
interface Rect { x: number; y: number; w: number; h: number }
function rect(t: LNode, sid: number, r: Rect = { x: 0, y: 0, w: 1, h: 1 }): Rect | null {
  if (t.type === "pane") return t.sid === sid ? r : null;
  const q = t.ratio ?? 0.5;
  const ra: Rect = t.dir === "row" ? { ...r, w: r.w * q } : { ...r, h: r.h * q };
  const rb: Rect = t.dir === "row" ? { x: r.x + r.w * q, y: r.y, w: r.w * (1 - q), h: r.h } : { x: r.x, y: r.y + r.h * q, w: r.w, h: r.h * (1 - q) };
  return rect(t.a, sid, ra) ?? rect(t.b, sid, rb);
}
const r3 = (n: number) => Math.round(n * 1000) / 1000;
const box = (t: LNode, sid: number) => {
  const r = rect(t, sid);
  return r ? { x: r3(r.x), y: r3(r.y), w: r3(r.w), h: r3(r.h) } : null;
};
const P = (sid: number, role?: string): LNode => (role ? { type: "pane", sid, role } : { type: "pane", sid });

// 좌석: 1 master · 2 cso · 3 worker · 4 reviewer-gemini · 5 reviewer-codex
const ROLES = new Map<number, string | null>([[1, "master"], [2, "cso"], [3, "worker"], [4, "reviewer-gemini"], [5, "reviewer-codex"]]);
const SOCK = "/tmp/s1-fix1.sock";
/** v0.15.1 이 저장하던 모양: 왼쪽 1/3 = master 위(3/4)·CSO 아래 / 가운데 worker / 오른쪽 agy 위·codex 아래. */
const oldAuto = (): LNode => ({
  type: "split", dir: "row", ratio: 1 / 3,
  a: { type: "split", dir: "col", ratio: 3 / 4, a: P(1, "master"), b: P(2, "cso") },
  b: { type: "split", dir: "row", ratio: 1 / 2, a: P(3, "worker"), b: { type: "split", dir: "col", ratio: 1 / 2, a: P(4, "reviewer-gemini"), b: P(5, "reviewer-codex") } },
});
/** 같은 자리 배치인데 주인이 분할선을 끌어 둔 수동 탭(대표 칸 0.41 · 가운데/오른쪽 0.37). */
const oldManual = (): LNode => ({
  type: "split", dir: "row", ratio: 0.41,
  a: { type: "split", dir: "col", ratio: 3 / 4, a: P(1, "master"), b: P(2, "cso") },
  b: { type: "split", dir: "row", ratio: 0.37, a: P(3, "worker"), b: { type: "split", dir: "col", ratio: 1 / 2, a: P(4, "reviewer-gemini"), b: P(5, "reviewer-codex") } },
});

/**
 * main.ts 의 실제 기동 S5 블록(주석 `★U3 S5(` ~ `★W-2-d: newSurface 직전`)을 그 워크스페이스들에 돌린다. 돌린 뒤의 csoSideDone 을 돌려준다.
 * o.move = 이관 함수 대역(원 트리로 떨어진 이관을 흉내 낼 때만) · o.liveBySock = 소켓별 데몬 응답(기본: SOCK 하나 · rolesKnown).
 */
function runS5(
  workspaces: Record<string, unknown>[],
  csoSideDone: boolean,
  rolesKnown = true,
  o: { move?: unknown; liveBySock?: Map<string, unknown> } = {},
): boolean {
  const a = MAIN.indexOf("  // ★U3 S5(");
  const b = MAIN.indexOf("  // ★W-2-d: newSurface 직전", a);
  expect(a).toBeGreaterThan(0);
  expect(b).toBeGreaterThan(a);
  const body = TS.transformSync(MAIN.slice(a, b));
  const liveBySock = o.liveBySock ?? new Map([[SOCK, { ok: rolesKnown, roles: ROLES }]]);
  return new Function(
    "workspaces", "liveBySock", "anchorHeadSafe", "moveCsoToSide", "csoMovePending", "csoSideDone0",
    `let csoSideDone = csoSideDone0;\n${body}\nreturn csoSideDone;`,
  )(workspaces, liveBySock, anchorHeadSafe, o.move ?? moveCsoToSide, csoMovePending, csoSideDone) as boolean;
}
/** main.ts 의 실제 saveLayout 이 쓴 저장본. */
function persist(workspaces: Record<string, unknown>[], csoSideDone: boolean): Record<string, unknown> {
  let saved = "";
  const make = new Function(
    "layoutLoaded", "normalizeWorkspaces", "normalizeGroups", "workspaces", "groups", "activeWs", "wsCounter", "groupCounter",
    "localStorage", "LAYOUT_KEY", "csoSideDone",
    `${realFunction("saveLayout")}\nreturn saveLayout;`,
  );
  (make(true, (w: unknown) => w, (_n: unknown, g: unknown) => g, workspaces, [], 0, 9, 1,
    { setItem: (_k: string, v: string) => void (saved = v) }, "cys-layout-v2", csoSideDone) as () => void)();
  return JSON.parse(saved) as Record<string, unknown>;
}

describe("★S1 수정1 배선 — 앱 시작이 CSO 를 리뷰어 열 맨 아래에 세운다(실제 기동 S5 루프)", () => {
  it("자동 탭: 구버전 모양(master 위·CSO 아래)으로 저장된 탭이 기동에서 새 표준 모양이 된다 — master 혼자 1/3 · 리뷰어 둘 + CSO 각 1/3 높이", () => {
    const ws = [{ id: 1, name: "본부", socket: SOCK, layoutManual: false, tree: oldAuto() }];
    runS5(ws, false);
    const t = ws[0].tree as LNode;
    expect(box(t, 1)).toEqual({ x: 0, y: 0, w: r3(MASTER_FRAC), h: 1 }); // master: 왼쪽 1/3 을 위아래 전부 혼자
    expect(box(t, 3)).toEqual({ x: 0.333, y: 0, w: 0.333, h: 1 }); // worker: 가운데 한 컬럼
    expect(box(t, 4)).toEqual({ x: 0.667, y: 0, w: 0.333, h: 0.333 }); // agy
    expect(box(t, 5)).toEqual({ x: 0.667, y: 0.333, w: 0.333, h: 0.333 }); // codex
    expect(box(t, 2)).toEqual({ x: 0.667, y: 0.667, w: 0.333, h: 0.333 }); // CSO: 리뷰어 열 맨 아래
  });
  it("자동 탭은 표식(csoSideDone)과 무관하게 매 기동 표준 모양이다", () => {
    const ws = [{ id: 1, name: "본부", socket: SOCK, layoutManual: false, tree: oldAuto() }];
    runS5(ws, true);
    expect(box(ws[0].tree as LNode, 2)).toEqual({ x: 0.667, y: 0.667, w: 0.333, h: 0.333 });
    expect(box(ws[0].tree as LNode, 1)).toEqual({ x: 0, y: 0, w: 0.333, h: 1 });
  });
  it("수동 탭 · 표식 없는 저장본: CSO 한 칸만 리뷰어 열 맨 아래로 옮긴다 — master 는 대표 칸을 혼자 쓰고, 주인이 끈 폭(0.41 · 0.37)은 그대로다", () => {
    const ws = [{ id: 2, name: "dept", socket: SOCK, layoutManual: true, tree: oldManual() }];
    const done = runS5(ws, false);
    const t = ws[0].tree as LNode;
    const right = r3(0.41 + 0.59 * 0.37); // 오른쪽 열의 시작 — 주인이 끈 그대로
    expect(box(t, 1)).toEqual({ x: 0, y: 0, w: 0.41, h: 1 }); // 대표 칸: 폭은 주인의 0.41 · 이제 master 혼자
    expect(box(t, 3)).toEqual({ x: 0.41, y: 0, w: r3(0.59 * 0.37), h: 1 });
    expect(box(t, 4)).toEqual({ x: right, y: 0, w: r3(1 - right), h: 0.333 });
    expect(box(t, 5)).toEqual({ x: right, y: 0.333, w: r3(1 - right), h: 0.333 });
    expect(box(t, 2)).toEqual({ x: right, y: 0.667, w: r3(1 - right), h: 0.333 }); // CSO: 리뷰어 열 맨 아래
    expect(done).toBe(true); // 한 번 했다 — saveLayout 이 저장본에 남긴다
    expect(ws[0].layoutManual).toBe(true); // 수동 표식은 그대로(표준 재배치가 아니다)
  });
  it("수동 탭 · 이미 옮긴 저장본(csoSide: 1): 다시 옮기지 않는다 — 주인이 그 뒤에 CSO 를 대표 칸으로 끌어다 둔 배치를 지킨다", () => {
    const tree = oldManual();
    const ws = [{ id: 2, name: "dept", socket: SOCK, layoutManual: true, tree }];
    runS5(ws, true);
    expect(ws[0].tree).toEqual(oldManual()); // 한 글자도 바뀌지 않는다
  });
  it("수동 탭 · 데몬이 아직 답하지 않아 역할을 모를 때는 칸의 기억(role)으로 옮긴다 · 기억도 없으면 그대로 둔다(던지지 않는다)", () => {
    const remembered = [{ id: 2, name: "dept", socket: SOCK, layoutManual: true, tree: oldManual() }];
    runS5(remembered, false, false);
    expect(box(remembered[0].tree as LNode, 2)?.y).toBe(0.667);
    const bare: LNode = { type: "split", dir: "row", ratio: 0.41, a: { type: "split", dir: "col", ratio: 3 / 4, a: P(1), b: P(2) }, b: P(3) };
    const unknown = [{ id: 3, name: "dept", socket: "/tmp/other.sock", layoutManual: true, tree: JSON.parse(JSON.stringify(bare)) }];
    expect(() => runS5(unknown, false, false)).not.toThrow();
    expect(unknown[0].tree).toEqual(bare);
  });
  it("트리가 없는 탭(빈 탭)은 건너뛴다 — 수동이든 자동이든", () => {
    const ws = [{ id: 4, name: "a", socket: SOCK, layoutManual: true, tree: null }, { id: 5, name: "b", socket: SOCK, layoutManual: false, tree: null }];
    expect(runS5(ws, false)).toBe(true);
    expect(ws.map((w) => w.tree)).toEqual([null, null]);
  });
});

describe("★S1 수정1 배선 — 「한 번 옮겼다」는 저장본에 남고, 다음 기동이 그것을 읽는다", () => {
  it("실제 saveLayout: 이관을 마쳤으면 저장본 루트에 csoSide: 1 · 아직이면 키가 없다(다음 기동이 한다)", () => {
    const ws = [{ id: 1, name: "본부", socket: SOCK, tree: P(1, "master") }];
    expect(persist(ws, true).csoSide).toBe(1);
    expect("csoSide" in persist(ws, false)).toBe(false);
  });
  /** main.ts 의 실제 저장본 적재 블록(`try { const saved = JSON.parse(…` ~ `layoutLoaded = true;`)을 그 저장본으로 돌린 뒤의 csoSideDone. */
  function loadMarker(savedJson: string | null): unknown {
    const a = MAIN.indexOf('  try {\n    const saved = JSON.parse(localStorage.getItem(LAYOUT_KEY) ?? "null");');
    const b = MAIN.indexOf("  layoutLoaded = true;", a);
    expect(a).toBeGreaterThan(0);
    expect(b).toBeGreaterThan(a);
    return new Function(
      "localStorage", "LAYOUT_KEY",
      `let workspaces, groups, activeWs, wsCounter, groupCounter, csoSideDone = false;\n${TS.transformSync(MAIN.slice(a, b))}\nreturn csoSideDone;`,
    )({ getItem: () => savedJson }, "cys-layout-v2");
  }
  it("기동의 저장본 적재가 그 표식을 읽는다(실제 적재 블록 실행) — 정확히 1 일 때만 '했다' · 없음·다른 값·손상 저장본 = 아직", () => {
    const ws = [{ id: 1, name: "본부", tree: null }];
    expect(loadMarker(JSON.stringify({ workspaces: ws, csoSide: 1 }))).toBe(true);
    expect(loadMarker(JSON.stringify({ workspaces: ws }))).toBe(false); // 이 판 이전의 저장본
    expect(loadMarker(JSON.stringify({ workspaces: ws, csoSide: true }))).toBe(false);
    expect(loadMarker(JSON.stringify({ workspaces: ws, csoSide: "1" }))).toBe(false);
    expect(loadMarker("{not json")).toBe(false);
    expect(loadMarker(null)).toBe(false); // 저장본이 아직 없는 새 설치
  });
  it("왕복: 구버전 수동 탭 → 기동(이관) → 저장 → 주인이 CSO 를 대표 칸으로 다시 끌어 옮김 → 저장 → 다음 기동은 손대지 않는다", () => {
    const ws = [{ id: 2, name: "dept", socket: SOCK, layoutManual: true, tree: oldManual() as LNode }];
    const done = runS5(ws, false);
    const saved1 = persist(ws, done);
    expect(saved1.csoSide).toBe(1);
    const dragged = oldManual(); // 주인이 다시 끌어다 둔 배치(= CSO 가 대표 칸 아래)
    const ws2 = [{ id: 2, name: "dept", socket: SOCK, layoutManual: true, tree: dragged }];
    runS5(ws2, saved1.csoSide === 1);
    expect(ws2[0].tree).toEqual(oldManual());
  });
});

// ★S1 M3(R6 · 리뷰 지적) — 표식 csoSideDone 은 「수동 탭의 1회 이관이 정말 끝났다 / 정말 필요 없다」일 때만 선다.
//   종전에는 기동 S5 가 무조건 세웠다: 이관이 원 트리로 떨어졌어도(예외 · 멤버 대조 실패 · 손상 트리), 그 기동에 역할을 몰라
//   (데몬 무응답 + 칸의 기억 없음) CSO 를 알아보지 못했어도 섰고, 그 탭의 CSO 는 대표 칸에 영영 남았다.
//   규칙: 「수동 탭 하나라도 대표 칸 안에 CSO 가 아직 있거나 있을 수 있는 기동은 표식을 쓰지 않는다 — 다음 기동이 다시 한다」.
describe("★S1 M3(R6) — 표식은 수동 탭의 이관이 정말 끝났거나 정말 필요 없을 때만 선다(실제 기동 S5 루프)", () => {
  const OTHER = "/tmp/s1-m3-other.sock";
  const manualWs = (id: number, tree: LNode | null, socket = SOCK) => ({ id, name: `dept-${id}`, socket, layoutManual: true, tree });
  /** 역할 기억이 없는 구버전 저장본(다른 소켓 · sid 11 master / 12 cso / 13 worker 였던 자리). */
  const noMemory = (): LNode => ({ type: "split", dir: "row", ratio: 0.41, a: { type: "split", dir: "col", ratio: 3 / 4, a: P(11), b: P(12) }, b: P(13) });
  const OTHER_ROLES = new Map<number, string | null>([[11, "master"], [12, "cso"], [13, "worker"]]);
  /** CSO(sid) 가 아직 왼쪽 끝 컬럼(대표 칸)에 있는가 — 화면 사각형으로 본다(제품의 술어를 빌리지 않는다). */
  const inHeadColumn = (t: LNode, sid: number) => box(t, sid)?.x === 0;

  it("옮겼다 → 선다", () => {
    const ws = [manualWs(2, oldManual())];
    expect(runS5(ws, false)).toBe(true);
    expect(inHeadColumn(ws[0].tree as LNode, 2)).toBe(false);
  });
  it("옮길 것이 없다 → 선다: CSO 가 이미 리뷰어 열에 있는 수동 탭 · CSO 가 없는 수동 탭 · 수동 탭이 하나도 없는 저장본", () => {
    const realMove = moveCsoToSide as (t: LNode | null, r: (s: number) => string | null | undefined) => LNode | null;
    const already = realMove(oldManual(), (s) => ROLES.get(s)) as LNode;
    const noCso: LNode = { type: "split", dir: "row", ratio: 0.41, a: P(1, "master"), b: P(3, "worker") };
    const ws = [manualWs(2, JSON.parse(JSON.stringify(already))), manualWs(3, JSON.parse(JSON.stringify(noCso)))];
    expect(runS5(ws, false)).toBe(true);
    expect(ws.map((w) => w.tree)).toEqual([already, noCso]); // 한 글자도 바뀌지 않는다
    expect(runS5([{ id: 1, name: "본부", socket: SOCK, layoutManual: false, tree: oldAuto() }], false)).toBe(true);
    expect(runS5([], false)).toBe(true);
  });
  it("이관이 원 트리로 떨어졌다(멤버 대조 실패·예외 때 moveCsoToSide 가 돌려주는 것 = 받은 트리) → 서지 않는다 · 트리는 그대로다", () => {
    const ws = [manualWs(2, oldManual())];
    const done = runS5(ws, false, true, { move: (t: LNode | null) => t });
    expect(ws[0].tree).toEqual(oldManual());
    expect(inHeadColumn(ws[0].tree as LNode, 2)).toBe(true); // CSO 가 대표 칸에 남았다
    expect(done).toBe(false); // 다음 기동이 다시 한다
  });
  it("역할 조회가 던진다 → 실제 moveCsoToSide 가 원 트리로 떨어진다 → 서지 않는다 · 기동은 던지지 않는다", () => {
    const boom = { has: () => { throw new Error("boom"); }, get: () => undefined };
    const ws = [manualWs(2, oldManual())];
    let done: boolean | null = null;
    expect(() => { done = runS5(ws, false, true, { liveBySock: new Map([[SOCK, { ok: true, roles: boom }]]) }); }).not.toThrow();
    expect(ws[0].tree).toEqual(oldManual());
    expect(done).toBe(false);
  });
  it("손상 트리(같은 sid 가 두 번)인 수동 탭 → 이관은 손대지 않는다 → 서지 않는다", () => {
    const dup = (): LNode => ({ type: "split", dir: "row", ratio: 0.41, a: { type: "split", dir: "col", ratio: 3 / 4, a: P(1, "master"), b: P(2, "cso") }, b: P(1, "master") });
    const ws = [manualWs(2, dup())];
    const done = runS5(ws, false);
    expect(ws[0].tree).toEqual(dup());
    expect(done).toBe(false);
  });
  it("역할을 모른다(그 탭의 데몬이 답하지 않았고 칸에 기억한 역할도 없다) → CSO 를 알아볼 수 없다 → 서지 않는다 · 트리는 그대로다", () => {
    const ws = [manualWs(3, noMemory(), OTHER)];
    const done = runS5(ws, false); // liveBySock 에 OTHER 가 없다 = 그 데몬의 응답이 없다
    expect(ws[0].tree).toEqual(noMemory());
    expect(done).toBe(false);
    const silent = [manualWs(3, noMemory(), OTHER)];
    expect(runS5(silent, false, true, { liveBySock: new Map([[OTHER, { ok: false, roles: new Map() }]]) })).toBe(false); // 물었으나 답이 없었다
  });
  it("대조 — 데몬이 답하지 않아도 칸의 기억(role)으로 CSO 를 알아보고 옮겼으면 선다", () => {
    const ws = [manualWs(2, oldManual())];
    expect(runS5(ws, false, false)).toBe(true);
    expect(inHeadColumn(ws[0].tree as LNode, 2)).toBe(false);
  });
  it("대조 — 모르는 칸이 대표 칸 **밖**에만 있으면(데몬 무응답 · master 는 기억으로 왼쪽 칸에 혼자) 표식을 막지 않는다", () => {
    const shellRight = (): LNode => ({ type: "split", dir: "row", ratio: 0.41, a: P(1, "master"), b: { type: "split", dir: "row", ratio: 0.5, a: P(3, "worker"), b: P(9) } });
    const ws = [manualWs(2, shellRight())];
    expect(runS5(ws, false, false)).toBe(true);
    expect(ws[0].tree).toEqual(shellRight());
  });
  it("수동 탭 둘 — 하나는 옮겼고 하나는 역할을 모른다 → 서지 않는다 · 옮긴 탭은 옮겨진 채다 → 다음 기동(역할을 안다)이 나머지를 옮기고 선다", () => {
    const ws = [manualWs(2, oldManual()), manualWs(3, noMemory(), OTHER)];
    expect(runS5(ws, false)).toBe(false);
    const first = JSON.parse(JSON.stringify(ws[0].tree)) as LNode;
    expect(inHeadColumn(first, 2)).toBe(false); // 알아본 탭의 이관은 이 기동에 이미 됐다(잃지 않는다)
    expect(ws[1].tree).toEqual(noMemory());
    const both = new Map<string, unknown>([[SOCK, { ok: true, roles: ROLES }], [OTHER, { ok: true, roles: OTHER_ROLES }]]);
    expect(runS5(ws, false, true, { liveBySock: both })).toBe(true);
    expect(ws[0].tree).toEqual(first); // 먼저 옮긴 탭은 한 글자도 바뀌지 않는다
    expect(inHeadColumn(ws[1].tree as LNode, 12)).toBe(false);
    expect(box(ws[1].tree as LNode, 11)).toEqual({ x: 0, y: 0, w: 0.41, h: 1 }); // 대표 칸: 폭 그대로 · master 혼자
    expect([11, 12, 13].every((s) => box(ws[1].tree as LNode, s) !== null)).toBe(true); // 칸은 하나도 잃지 않는다
  });
  it("대조 — 이미 선 표식은 내려가지 않는다(주인이 CSO 를 대표 칸으로 다시 끌어다 둔 수동 탭이 있어도)", () => {
    const ws = [manualWs(2, oldManual())];
    expect(runS5(ws, true)).toBe(true);
    expect(ws[0].tree).toEqual(oldManual());
  });
  it("왕복: 못 끝낸 기동은 저장본에 csoSide 를 남기지 않는다 → 다음 기동이 다시 해서 끝내고 → 그제야 csoSide: 1", () => {
    const ws = [manualWs(3, noMemory(), OTHER)];
    const done1 = runS5(ws, false);
    expect("csoSide" in persist(ws, done1)).toBe(false);
    const done2 = runS5(ws, done1, true, { liveBySock: new Map([[OTHER, { ok: true, roles: OTHER_ROLES }]]) });
    expect(inHeadColumn(ws[0].tree as LNode, 12)).toBe(false);
    expect(persist(ws, done2).csoSide).toBe(1);
  });
});

describe("★S1 수정1 — 화면의 말이 새 배치와 같다", () => {
  it("`정렬` 버튼 설명: master 는 좌 1/3 단독 · CSO 는 리뷰어와 같은 열 맨 아래(옛 문구 'master·CSO(좌 1/3)' 는 없다)", () => {
    const tip = /id="btn-equalize" title="([^"]*)"/.exec(HTML)?.[1] ?? "";
    expect(tip).toContain("master(좌 1/3 단독)");
    expect(tip).toContain("리뷰어·CSO(우 한 열, 같은 높이 · CSO 맨 아래)");
    expect(tip.includes("master·CSO(좌 1/3)")).toBe(false);
  });
});

describe("★S1 수정1 — isAnchored: 새 표준 모양만 '이미 섰다'고 답한다", () => {
  const isAnchored = (SL as unknown as Record<string, (t: unknown, r: (s: number) => string | null | undefined) => boolean>).isAnchored;
  const roleOf = (s: number) => (ROLES.has(s) ? ROLES.get(s) : undefined);
  it("구버전 모양(CSO 가 대표 칸 아래)은 선 것이 아니다 · 기동이 옮긴 뒤에는 섰다", () => {
    expect(isAnchored(oldAuto(), roleOf)).toBe(false);
    expect(isAnchored(anchorHeadSafe(oldAuto(), roleOf), roleOf)).toBe(true);
  });
  it("대표 칸은 master 혼자인데 CSO 가 리뷰어 열 밖(가운데 컬럼)에 있으면 선 것이 아니다", () => {
    const stray: LNode = {
      type: "split", dir: "row", ratio: 1 / 3, a: P(1, "master"),
      b: { type: "split", dir: "row", ratio: 1 / 3, a: P(2, "cso"), b: { type: "split", dir: "row", ratio: 1 / 2, a: P(3, "worker"), b: { type: "split", dir: "col", ratio: 1 / 2, a: P(4, "reviewer-gemini"), b: P(5, "reviewer-codex") } } },
    };
    expect(isAnchored(stray, roleOf)).toBe(false);
    const fixed = anchorHeadSafe(stray, roleOf) as LNode;
    expect(isAnchored(fixed, roleOf)).toBe(true);
    expect(box(fixed, 2)).toEqual({ x: 0.667, y: 0.667, w: 0.333, h: 0.333 });
  });
});

describe("★S1 수정1 — 수동 탭 1회 이관은 대표 칸 안의 CSO 만 옮긴다(moveCsoToSide)", () => {
  const move = moveCsoToSide as (t: LNode | null, r: (s: number) => string | null | undefined) => LNode | null;
  const roleOf = (s: number) => (ROLES.has(s) ? ROLES.get(s) : undefined);
  it("탭 전체가 한 열로 쌓인 두 좌석 탭(master 위 · CSO 아래): master 왼쪽 1/3 · CSO 오른쪽", () => {
    const stacked: LNode = { type: "split", dir: "col", ratio: 3 / 4, a: P(1, "master"), b: P(2, "cso") };
    const t = move(stacked, roleOf) as LNode;
    expect(box(t, 1)).toEqual({ x: 0, y: 0, w: 0.333, h: 1 });
    expect(box(t, 2)).toEqual({ x: 0.333, y: 0, w: 0.667, h: 1 });
  });
  it("주인이 CSO 를 master **옆에** 따로 세워 둔 탭(좌 30% CSO · 우 70% master)은 한 글자도 바꾸지 않는다 — CSO 는 이미 대표 칸 밖이다", () => {
    const side = (): LNode => ({ type: "split", dir: "row", ratio: 0.3, a: P(2, "cso"), b: P(1, "master") });
    expect(move(side(), roleOf)).toEqual(side());
    const side2 = (): LNode => ({ type: "split", dir: "row", ratio: 0.6, a: P(1, "master"), b: P(2, "cso") });
    expect(move(side2(), roleOf)).toEqual(side2());
  });
  it("한 열에 master · worker · CSO 가 함께 쌓인 탭은 대표 칸이 아니다(다른 좌석이 섞였다) — 손대지 않는다", () => {
    const mixed = (): LNode => ({ type: "split", dir: "col", ratio: 1 / 2, a: P(1, "master"), b: { type: "split", dir: "col", ratio: 1 / 2, a: P(3, "worker"), b: P(2, "cso") } });
    expect(move(mixed(), roleOf)).toEqual(mixed());
  });
  it("옮길 CSO 가 없으면 그대로 · null 은 null", () => {
    const noCso = (): LNode => ({ type: "split", dir: "row", ratio: 0.41, a: P(1, "master"), b: P(3, "worker") });
    expect(move(noCso(), roleOf)).toEqual(noCso());
    expect(move(null, roleOf)).toBeNull();
  });
});
