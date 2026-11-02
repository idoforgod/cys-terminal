// P8(0.14.57 · 오너 2026-11-03): CSO 기본 자리를 master 밑에서 해제한다 — 배치 동작 핀.
// 오너 확정: 왼쪽 1/3 은 master 혼자 · CSO 는 오른쪽 2/3 의 평범한 좌석 · 창 배치만 바뀐다(역할·권한 불변) ·
// 저장된 화면은 자동으로 옮기지 않는다(새 창과 `정렬` 만 새 배치).
// ★이 파일은 공개 API 만 쓴다 — 옛 규칙에서 적색이 나면 그것은 없는 심볼이 아니라 **동작**의 적색이다.
import { describe, it, expect } from "bun:test";
import {
  MASTER_FRAC,
  HEAD_COL_MASTER,
  adoptSeat,
  annotateRoles,
  anchorHead,
  anchorHeadSafe,
  holdPane,
  holeSidsOf,
  isAnchored,
  liveSidsOf,
  placeSeat,
  placeSeatSafe,
  restoreTree,
  roleLayout,
  seatPriority,
  tidyHoles,
  type LNode,
  type RoleOf,
  type WsView,
} from "./seatlayout";

// ---------- 오라클 ----------
function share(t: LNode, sid: number, axis: "row" | "col", acc = 1): number | null {
  if (t.type === "pane") return t.sid === sid ? acc : null;
  const r = t.ratio ?? 0.5;
  const [ra, rb] = t.dir === axis ? [r, 1 - r] : [1, 1];
  return share(t.a, sid, axis, acc * ra) ?? share(t.b, sid, axis, acc * rb);
}
const W = (t: LNode, sid: number) => share(t, sid, "row");
const H = (t: LNode, sid: number) => share(t, sid, "col");
const close = (a: number | null, b: number, eps = 1e-9) => {
  expect(a).not.toBeNull();
  expect(Math.abs((a as number) - b)).toBeLessThan(eps);
};
const sids = (t: LNode | null): number[] => {
  if (!t) return [];
  return t.type === "pane" ? [t.sid] : [...sids(t.a), ...sids(t.b)];
};
const inLeft = (t: LNode, sid: number): boolean => t.type === "split" && t.dir === "row" && sids(t.a).includes(sid);
const mapRole = (m: Map<number, string | null>): RoleOf => (s) => (m.has(s) ? m.get(s) : undefined);
function perms<T>(xs: T[]): T[][] {
  if (xs.length <= 1) return [xs];
  return xs.flatMap((x, i) => perms([...xs.slice(0, i), ...xs.slice(i + 1)]).map((p) => [x, ...p]));
}
const rm = (n: LNode, sid: number): LNode | null =>
  n.type === "pane" ? (n.sid === sid ? null : n) : ((a, b) => (a && b ? { ...n, a, b } : a ?? b))(rm(n.a, sid), rm(n.b, sid));
function shape(t: LNode | null): unknown {
  if (!t) return null;
  if (t.type === "pane") return t.role ?? "-";
  return { dir: t.dir, ratio: t.ratio ?? 0.5, a: shape(t.a), b: shape(t.b) };
}
const ROLES6 = ["master", "cso", "worker", "worker-2", "reviewer-codex", "reviewer-gemini"];

/** 0.14.56 이 만든 저장 화면: 왼쪽 1/3 = master 위 / cso 아래. 칸에 역할이 적혀 있다. */
const SAVED = (): LNode => ({
  type: "split", dir: "row", ratio: MASTER_FRAC,
  a: { type: "split", dir: "col", ratio: HEAD_COL_MASTER, a: { type: "pane", sid: 18, role: "master" }, b: { type: "pane", sid: 17, role: "cso" } },
  b: { type: "split", dir: "row", ratio: 1 / 3, a: { type: "pane", sid: 14, role: "worker" },
    b: { type: "split", dir: "row", ratio: 0.5, a: { type: "pane", sid: 16, role: "worker-2" },
      b: { type: "split", dir: "col", ratio: 0.5, a: { type: "pane", sid: 19, role: "reviewer-gemini" }, b: { type: "pane", sid: 15, role: "reviewer-codex" } } } },
});
const SAVED_ROLE = (): Map<number, string | null> =>
  new Map<number, string | null>([[18, "master"], [17, "cso"], [14, "worker"], [16, "worker-2"], [19, "reviewer-gemini"], [15, "reviewer-codex"]]);

// ────────────────────────────────────────────────────────────────────────────
describe("P8 시나리오 1 — 새 부팅: master+cso+worker+리뷰어", () => {
  const all = perms(ROLES6);
  it(`도착 순서 ${all.length}가지 전수(비수동): 왼쪽은 master 혼자 · 나머지 5컬럼이 2/3 를 균등히`, () => {
    let bad = 0;
    for (const order of all) {
      const role = new Map<number, string | null>();
      let t: LNode | null = null;
      order.forEach((r, i) => {
        role.set(i + 1, r);
        t = placeSeat(t, i + 1, mapRole(role), false);
      });
      const m = order.indexOf("master") + 1;
      const f = t as unknown as LNode;
      const ok =
        f.type === "split" &&
        f.dir === "row" &&
        f.a.type === "pane" &&
        f.a.sid === m &&
        Math.abs((W(f, m) ?? 0) - MASTER_FRAC) < 1e-9 &&
        Math.abs((H(f, m) ?? 0) - 1) < 1e-9 &&
        order.every((r, i) => r === "master" || Math.abs((W(f, i + 1) ?? 0) - 2 / 15) < 1e-9);
      if (!ok) bad++;
    }
    expect(bad).toBe(0);
  });
  it("도착 순서 전수 × 매 단계: master 와 다른 좌석이 함께 있으면 master 폭은 언제나 1/3 이고 높이는 전체", () => {
    let checked = 0;
    for (const order of all) {
      const role = new Map<number, string | null>();
      let t: LNode | null = null;
      order.forEach((r, i) => {
        role.set(i + 1, r);
        t = placeSeat(t, i + 1, mapRole(role), false);
        if (order.indexOf("master") <= i && i >= 1) {
          const m = order.indexOf("master") + 1;
          close(W(t as unknown as LNode, m), MASTER_FRAC);
          close(H(t as unknown as LNode, m), 1);
          checked++;
        }
      });
    }
    expect(checked).toBeGreaterThan(2000);
  });
  it("cso 가 먼저 와도 master 는 그 위에 얹히지 않는다 — master 왼쪽 1/3 · cso 오른쪽 2/3", () => {
    for (const r of ["cso", "cso-2"]) {
      const role = new Map<number, string | null>([[1, r], [2, "master"]]);
      const t = placeSeat(placeSeat(null, 1, mapRole(role), false), 2, mapRole(role), false);
      close(W(t, 2), MASTER_FRAC);
      close(W(t, 1), 2 / 3);
      close(H(t, 2), 1);
    }
  });
  it("master → cso 순서: cso 는 오른쪽 2/3 전체", () => {
    const role = new Map<number, string | null>([[1, "master"], [2, "cso"]]);
    const t = placeSeat(placeSeat(null, 1, mapRole(role), false), 2, mapRole(role), false);
    close(W(t, 1), MASTER_FRAC);
    close(W(t, 2), 2 / 3);
    expect(inLeft(t, 2)).toBe(false);
  });
  it("cso 여럿 — 평범한 좌석처럼 오른쪽에 균등히(master 높이 전체)", () => {
    const role = new Map<number, string | null>([[1, "master"], [2, "cso"], [3, "cso-2"], [4, "worker"]]);
    let t: LNode | null = null;
    for (const s of [1, 2, 3, 4]) t = placeSeat(t, s, mapRole(role), false);
    const f = t as unknown as LNode;
    close(H(f, 1), 1);
    for (const s of [2, 3, 4]) close(W(f, s), 2 / 9);
  });
  it("입양 순서(seatPriority)는 그대로 — cso 는 오른쪽 영역의 맨 왼쪽에 먼저 앉는다", () => {
    const order = [...ROLES6].sort((a, b) => seatPriority(a) - seatPriority(b));
    const role = new Map<number, string | null>();
    let t: LNode | null = null;
    order.forEach((r, i) => {
      role.set(i + 1, r);
      t = placeSeat(t, i + 1, mapRole(role), false);
    });
    const f = t as unknown as LNode;
    expect(f.type === "split" && sids(f.b)[0]).toBe(order.indexOf("cso") + 1);
  });
  it("빈 탭에 adoptSeat 로 입양해도 같다(구멍이 없으면 새 배치)", () => {
    const role = new Map<number, string | null>([[1, "master"], [2, "cso"], [3, "worker"]]);
    let ws: WsView[] = [{ tree: null }];
    for (const [s, r] of [[1, "master"], [2, "cso"], [3, "worker"]] as [number, string][]) {
      const plan = adoptSeat(ws, undefined, s, r, mapRole(role));
      ws = [{ tree: (plan as NonNullable<typeof plan>).tree }];
    }
    const t = ws[0].tree as LNode;
    close(W(t, 1), MASTER_FRAC);
    close(H(t, 1), 1);
    expect(inLeft(t, 2)).toBe(false);
    close(W(t, 2), 1 / 3);
  });
});

// ────────────────────────────────────────────────────────────────────────────
describe("P8 시나리오 2 — `정렬`(roleLayout)", () => {
  it("master 왼쪽 1/3 혼자 · cso·worker 가운데 균등 · 리뷰어 오른쪽 끝 한 컬럼", () => {
    const r = new Map<number, string | null>([[1, "master"], [2, "cso"], [3, "worker"], [4, "worker-2"], [90, "reviewer-gemini"], [91, "reviewer-codex"]]);
    const t = roleLayout([...r.keys()], mapRole(r)) as LNode;
    close(W(t, 1), MASTER_FRAC);
    close(H(t, 1), 1);
    expect(inLeft(t, 2)).toBe(false);
    for (const s of [2, 3, 4, 90, 91]) close(W(t, s), 2 / 3 / 4);
    close(H(t, 90), 1 / 2);
  });
  it("cso-N 도 같다 · 입력 순서 보존", () => {
    const r = new Map<number, string | null>([[1, "master"], [5, "worker"], [6, "cso-2"], [7, "cso"]]);
    const t = roleLayout([1, 5, 6, 7], mapRole(r)) as LNode;
    expect(t.type === "split" && sids(t.b)).toEqual([5, 6, 7]);
    for (const s of [5, 6, 7]) close(W(t, s), 2 / 9);
  });
  it("master+cso 만 → master 1/3 · cso 2/3", () => {
    const r = new Map<number, string | null>([[1, "master"], [2, "cso"]]);
    const t = roleLayout([1, 2], mapRole(r)) as LNode;
    close(W(t, 1), MASTER_FRAC);
    close(W(t, 2), 2 / 3);
  });
  it("master 가 없는 탭: cso 는 왼쪽 컬럼을 세우지 않는다 — 전부 균등", () => {
    const r = new Map<number, string | null>([[1, "cso"], [2, "worker"], [3, "worker-2"]]);
    const t = roleLayout([1, 2, 3], mapRole(r)) as LNode;
    for (const s of [1, 2, 3]) close(W(t, s), 1 / 3);
  });
  it("추가 master 는 오늘 규칙 그대로 왼쪽 컬럼 아래에 쌓인다", () => {
    const r = new Map<number, string | null>([[1, "master"], [2, "master"], [3, "cso"], [4, "worker"]]);
    const t = roleLayout([1, 2, 3, 4], mapRole(r)) as LNode;
    close(W(t, 1), MASTER_FRAC);
    close(H(t, 1), HEAD_COL_MASTER);
    close(H(t, 2), 1 - HEAD_COL_MASTER);
    expect(inLeft(t, 3)).toBe(false);
  });
  it("저장 화면에서 `정렬` 을 누르면 그때 비로소 cso 가 왼쪽 컬럼을 떠난다 · 정렬 뒤 고정", () => {
    const role = SAVED_ROLE();
    const t = roleLayout(sids(SAVED()), mapRole(role)) as LNode;
    close(W(t, 18), MASTER_FRAC);
    close(H(t, 18), 1);
    expect(inLeft(t, 17)).toBe(false);
    expect(shape(anchorHead(t, mapRole(role)))).toEqual(shape(t));
  });
});

// ────────────────────────────────────────────────────────────────────────────
describe("P8 시나리오 3 — 저장 화면(cso 가 master 밑)은 재시작에도 그대로", () => {
  const live: [number, string][] = [[26, "worker"], [27, "reviewer-codex"], [28, "worker-2"], [29, "cso"], [30, "master"], [31, "reviewer-gemini"]];
  function restart(saved: LNode, manual: boolean, seats: [number, string][]): LNode {
    const role = new Map<number, string | null>(seats);
    let h = -1;
    const t = restoreTree(saved, { genChanged: true, isLive: () => false, liveRole: mapRole(role), roleConflictIsDead: false, allocHole: () => h-- });
    let ws: WsView[] = [{ tree: t, layoutManual: manual }];
    for (const [s, r] of seats) {
      const plan = adoptSeat(ws, undefined, s, r, mapRole(role));
      ws = [{ tree: (plan as NonNullable<typeof plan>).tree, layoutManual: manual }];
    }
    return tidyHoles([ws[0].tree], mapRole(role))[0] as LNode;
  }
  for (const manual of [false, true]) {
    it(`도착 순서 전수 720 · ${manual ? "수동" : "비수동"}: 칸·비율 그대로 · cso 는 master 밑 · 구멍 0`, () => {
      let bad = 0;
      for (const order of perms(live)) {
        const t = restart(SAVED(), manual, order);
        const ok =
          holeSidsOf(t).length === 0 &&
          JSON.stringify(shape(t)) === JSON.stringify(shape(SAVED())) &&
          t.type === "split" &&
          t.a.type === "split" &&
          t.a.dir === "col" &&
          sids(t.a).join() === "30,29";
        if (!ok) bad++;
      }
      expect(bad).toBe(0);
    });
  }
  it("앱만 재시작(S5): anchorHeadSafe 는 저장 화면을 한 칸도 옮기지 않는다", () => {
    const t = anchorHeadSafe(SAVED(), mapRole(SAVED_ROLE())) as LNode;
    expect(shape(t)).toEqual(shape(SAVED()));
    expect(isAnchored(SAVED(), mapRole(SAVED_ROLE()))).toBe(true);
  });
  it("저장 화면에 새 좌석: master 폭·높이 불변 · cso 제자리 · 새 좌석은 오른쪽(worker·새 cso 모두)", () => {
    for (const r of ["worker-3", "cso-2"]) {
      const role = SAVED_ROLE();
      role.set(50, r);
      const t = placeSeat(SAVED(), 50, mapRole(role), false);
      close(W(t, 18), MASTER_FRAC);
      close(H(t, 18), HEAD_COL_MASTER);
      expect(inLeft(t, 17)).toBe(true);
      expect(inLeft(t, 50)).toBe(false);
      expect(isAnchored(t, mapRole(role))).toBe(true);
    }
  });
  it("master 위 / cso 아래만 있는 저장 탭: anchorHead 불변 · 새 좌석은 오른쪽 2/3", () => {
    const col: LNode = { type: "split", dir: "col", ratio: HEAD_COL_MASTER, a: { type: "pane", sid: 1, role: "master" }, b: { type: "pane", sid: 2, role: "cso" } };
    const role = new Map<number, string | null>([[1, "master"], [2, "cso"], [3, "worker"]]);
    expect(anchorHead(col, mapRole(role))).toBe(col);
    const t = placeSeat(col, 3, mapRole(role), false);
    close(W(t, 1), MASTER_FRAC);
    close(H(t, 1), HEAD_COL_MASTER);
    expect(inLeft(t, 2)).toBe(true);
    close(W(t, 3), 2 / 3);
  });
  it("master 재기동 경합(옛 칸이 아직 트리에 있고 종료 표시): 새 master 는 옛 컬럼 위로 · cso 는 밀려나지 않는다", () => {
    const role = SAVED_ROLE();
    role.set(18, null); // 입양 틱은 종료 좌석을 null 로 본다(main.ts roleOf)
    role.set(60, "master");
    const t = placeSeat(SAVED(), 60, mapRole(role), false);
    close(W(t, 60), MASTER_FRAC);
    expect(inLeft(t, 17)).toBe(true);
    const after = anchorHead(rm(t, 18), mapRole(role)) as LNode;
    close(W(after, 60), MASTER_FRAC);
    close(H(after, 60), HEAD_COL_MASTER);
    expect(inLeft(after, 17)).toBe(true);
  });
  it("한계(DESIGN D6): master 칸이 기억 없이 사라진 저장 화면 — 새 master 는 자기 컬럼 · cso 컬럼은 그 오른쪽에 남는다", () => {
    const role = SAVED_ROLE();
    role.delete(18);
    role.set(60, "master");
    const t = placeSeat(rm(SAVED(), 18), 60, mapRole(role), false);
    close(W(t, 60), MASTER_FRAC);
    close(H(t, 60), 1);
    expect(t.type === "split" && sids(t.b)[0]).toBe(17);
  });
  it("대조: 새 배치에서 master 가 같은 경합을 겪으면 새 master 는 왼쪽에 혼자", () => {
    const role = new Map<number, string | null>([[1, "master"], [2, "cso"], [3, "worker"]]);
    let t: LNode | null = null;
    for (const s of [1, 2, 3]) t = placeSeat(t, s, mapRole(role), false);
    role.set(1, null);
    role.set(9, "master");
    const u = anchorHead(rm(placeSeat(t, 9, mapRole(role), false), 1), mapRole(role)) as LNode;
    close(W(u, 9), MASTER_FRAC);
    close(H(u, 9), 1);
    expect(inLeft(u, 2)).toBe(false);
  });
});

// ────────────────────────────────────────────────────────────────────────────
describe("P8 시나리오 4 — cso 재기동: 기억이 있을 때와 없을 때", () => {
  const holeAt = (t: LNode, sid: number, h: number, role: string): LNode =>
    t.type === "pane" ? (t.sid === sid ? { type: "pane", sid: h, role } : t) : { ...t, a: holeAt(t.a, sid, h, role), b: holeAt(t.b, sid, h, role) };
  it("저장 화면 · 기억 있음(master 밑 구멍): 같은 칸에 결속 · 비율 그대로(비수동·수동)", () => {
    for (const manual of [false, true]) {
      const role = SAVED_ROLE();
      role.delete(17);
      role.set(70, "cso");
      const tree = holeAt(SAVED(), 17, -3, "cso");
      const plan = adoptSeat([{ tree, layoutManual: manual }], undefined, 70, "cso", mapRole(role)) as NonNullable<ReturnType<typeof adoptSeat>>;
      expect(plan.boundHole).toBe(-3);
      expect(inLeft(plan.tree, 70)).toBe(true);
      close(H(plan.tree, 18), HEAD_COL_MASTER);
      expect(sids(plan.tree)).toEqual([18, 70, 14, 16, 19, 15]);
    }
  });
  it("새 배치 · 기억 있음(오른쪽 구멍): 그 칸에 결속 — 왼쪽으로 가지 않는다", () => {
    const role = new Map<number, string | null>([[1, "master"], [2, "cso"], [3, "worker"]]);
    let t: LNode | null = null;
    for (const s of [1, 2, 3]) t = placeSeat(t, s, mapRole(role), false);
    role.delete(2);
    role.set(8, "cso");
    const tree = holeAt(t as unknown as LNode, 2, -1, "cso");
    const plan = adoptSeat([{ tree }], undefined, 8, "cso", mapRole(role)) as NonNullable<ReturnType<typeof adoptSeat>>;
    expect(plan.boundHole).toBe(-1);
    expect(inLeft(plan.tree, 8)).toBe(false);
    close(W(plan.tree, 1), MASTER_FRAC);
    close(H(plan.tree, 1), 1);
  });
  it("저장 화면 · 기억 없음: 새 cso 는 오른쪽 · master 높이는 저장값(3/4) 유지 · 남은 칸은 건드리지 않는다", () => {
    for (const manual of [false, true]) {
      const role = SAVED_ROLE();
      const tree = rm(SAVED(), 17) as LNode; // 저장 화면에서 cso 칸이 기억 없이 사라졌다 → 왼쪽 = master 혼자
      role.delete(17);
      role.set(70, "cso");
      const plan = adoptSeat([{ tree, layoutManual: manual }], undefined, 70, "cso", mapRole(role)) as NonNullable<ReturnType<typeof adoptSeat>>;
      expect(plan.boundHole).toBeNull();
      expect(inLeft(plan.tree, 70)).toBe(false);
      close(W(plan.tree, 18), MASTER_FRAC);
      close(H(plan.tree, 18), 1);
    }
  });
  it("저장 화면 · 다른 cso 가 왼쪽에 있어도 cso-2 의 기억 없는 새 좌석은 오른쪽", () => {
    const role = SAVED_ROLE();
    role.set(71, "cso-2");
    const plan = adoptSeat([{ tree: SAVED() }], undefined, 71, "cso-2", mapRole(role)) as NonNullable<ReturnType<typeof adoptSeat>>;
    expect(inLeft(plan.tree, 71)).toBe(false);
    expect(inLeft(plan.tree, 17)).toBe(true);
  });
});

// ────────────────────────────────────────────────────────────────────────────
describe("P8 — anchorHead 는 흩어진 cso 를 왼쪽으로 끌어오지 않는다", () => {
  it("옛 반감 사슬: master 만 왼쪽 1/3 로 · cso 는 제 순서 그대로 오른쪽", () => {
    const role = new Map<number, string | null>([[1, "master"], [2, "cso"], [3, "worker"], [4, "worker-2"]]);
    let t: LNode = { type: "pane", sid: 1 };
    for (const s of [2, 3, 4]) t = { type: "split", dir: "row", a: t, b: { type: "pane", sid: s } };
    const u = anchorHead(t, mapRole(role)) as LNode;
    close(W(u, 1), MASTER_FRAC);
    expect(inLeft(u, 2)).toBe(false);
    expect(u.type === "split" && sids(u.b)).toEqual([2, 3, 4]);
    for (const s of [2, 3, 4]) close(W(u, s), 2 / 9);
  });
  it("왼쪽이 master 혼자이고 cso 가 오른쪽이면 앵커다 — 재배치 없음(새 배치의 고정점)", () => {
    const role = new Map<number, string | null>([[1, "master"], [2, "cso"], [3, "worker"]]);
    const t = roleLayout([1, 2, 3], mapRole(role)) as LNode;
    expect(isAnchored(t, mapRole(role))).toBe(true);
    expect(shape(anchorHead(t, mapRole(role)))).toEqual(shape(t));
  });
  it("cso-2 가 저장 왼쪽 컬럼 밖에 있어도 앵커 유지 — 끌어오지 않는다", () => {
    const role = SAVED_ROLE();
    role.set(50, "cso-2");
    const t: LNode = { ...(SAVED() as Extract<LNode, { type: "split" }>), b: { type: "split", dir: "row", ratio: 0.75, a: (SAVED() as Extract<LNode, { type: "split" }>).b, b: { type: "pane", sid: 50, role: "cso-2" } } };
    expect(isAnchored(t, mapRole(role))).toBe(true);
    expect(inLeft(anchorHead(t, mapRole(role)) as LNode, 50)).toBe(false);
  });
  it("전출로 row(master, cso) 0.5 가 된 탭 — 다음 자동 배치가 master 1/3 로 되돌린다(cso 도 평범한 좌석)", () => {
    const role = new Map<number, string | null>([[1, "master"], [2, "cso"]]);
    const t: LNode = { type: "split", dir: "row", a: { type: "pane", sid: 1 }, b: { type: "pane", sid: 2 } };
    close(W(anchorHead(t, mapRole(role)) as LNode, 1), MASTER_FRAC);
  });
});

// ────────────────────────────────────────────────────────────────────────────
describe("P8 — 수동 탭은 그대로", () => {
  const MANUAL = (): LNode => ({
    type: "split", dir: "row", ratio: 0.5,
    a: { type: "pane", sid: 2, role: "cso" },
    b: { type: "split", dir: "row", ratio: 0.5, a: { type: "pane", sid: 3, role: "worker" }, b: { type: "pane", sid: 4, role: "worker-2" } },
  });
  it("master 재기동: cso 컬럼 위로(0.5 유지 · 폭을 새로 떼지 않는다)", () => {
    const role = new Map<number, string | null>([[2, "cso"], [3, "worker"], [4, "worker-2"], [9, "master"]]);
    const t = placeSeat(MANUAL(), 9, mapRole(role), true);
    close(W(t, 9), 0.5);
    close(W(t, 2), 0.5);
    close(H(t, 9), HEAD_COL_MASTER);
  });
  it("master 위 / cso 아래만 있는 수동 저장 탭: 새 좌석이 와도 그 컬럼은 1/3 · cso 제자리(오늘 규칙 · 자동 재정렬이 없어 절반이 되면 안 된다)", () => {
    for (const r of ["worker", "cso-2"]) {
      const col: LNode = { type: "split", dir: "col", ratio: HEAD_COL_MASTER, a: { type: "pane", sid: 1, role: "master" }, b: { type: "pane", sid: 2, role: "cso" } };
      const role = new Map<number, string | null>([[1, "master"], [2, "cso"], [3, r]]);
      const t = placeSeat(col, 3, mapRole(role), true);
      close(W(t, 1), MASTER_FRAC);
      close(H(t, 1), HEAD_COL_MASTER);
      expect(inLeft(t, 2)).toBe(true);
      expect(inLeft(t, 3)).toBe(false);
    }
  });
  it("수동 탭의 cso 는 어디에 있어도 새 좌석·재배치로 움직이지 않는다", () => {
    const role = new Map<number, string | null>([[1, "master"], [2, "cso"], [3, "worker"], [5, "worker-2"]]);
    const t: LNode = { type: "split", dir: "row", ratio: 0.4, a: { type: "pane", sid: 1 }, b: { type: "split", dir: "col", ratio: 0.3, a: { type: "pane", sid: 2 }, b: { type: "pane", sid: 3 } } };
    const u = placeSeat(t, 5, mapRole(role), true);
    close(W(u, 1), 0.4);
    close(H(u, 2), 0.3);
    expect(sids(u).slice(0, 3)).toEqual([1, 2, 3]);
  });
});

// ────────────────────────────────────────────────────────────────────────────
describe("P8 — 불변식(전역·보존) 유지", () => {
  it("추가 master 는 오늘 규칙 그대로 왼쪽 컬럼 위로", () => {
    const role = new Map<number, string | null>([[1, "master"], [2, "worker"], [3, "master"]]);
    let t: LNode | null = null;
    for (const s of [1, 2]) t = placeSeat(t, s, mapRole(role), false);
    const u = placeSeat(t, 3, mapRole(role), false);
    close(W(u, 3), MASTER_FRAC);
    close(H(u, 3), HEAD_COL_MASTER);
    expect(inLeft(u, 1)).toBe(true);
  });
  it("모든 도착 순서 · cso-N 포함 7석: sid 손실·중복 0 · 안전 래퍼가 폴백하지 않는다", () => {
    const roles = ["master", "cso", "cso-2", "worker", "reviewer-gemini", "reviewer-codex", "cso-fresh-1790000000"];
    for (const order of perms(roles).slice(0, 1500)) {
      const role = new Map<number, string | null>();
      let t: LNode | null = null;
      order.forEach((r, i) => {
        role.set(i + 1, r);
        t = placeSeatSafe(t, i + 1, mapRole(role), false);
        t = anchorHeadSafe(t, mapRole(role));
      });
      expect(liveSidsOf(t).sort((a, b) => a - b)).toEqual([1, 2, 3, 4, 5, 6, 7]);
    }
  });
});

// ────────────────────────────────────────────────────────────────────────────
// r1 리뷰(codex · P8_release_CSO_seat_under_master-r1-codex.json sha16 3773e74a9a8c1aab)의 반례를 **글자 그대로** 박제한다.
describe("P8 r2 — 리뷰 반례 1: 자동 저장 col(master,cso) · 옛 master 종료 표시 · 새 master", () => {
  // codex-r1-review/counterexample.json 의 saved · after · adopted.tree
  const saved = (): LNode => ({ type: "split", dir: "col", ratio: 0.75, a: { type: "pane", sid: 1, role: "master" }, b: { type: "pane", sid: 2, role: "cso" } });
  const BAD: LNode = {
    type: "split", dir: "row", ratio: 0.3333333333333333,
    a: { type: "pane", sid: 3, role: "master" },
    b: { type: "split", dir: "col", ratio: 0.75, a: { type: "pane", sid: 1, role: "master" }, b: { type: "pane", sid: 2, role: "cso" } },
  };
  const role = new Map<number, string | null>([[1, null], [2, "cso"], [3, "master"]]);
  it("placeSeatSafe: cso 는 새 master 밑에 남는다 · 리뷰가 잡은 트리가 아니다", () => {
    const t = placeSeatSafe(saved(), 3, mapRole(role), false);
    expect(t).not.toEqual(BAD);
    expect(inLeft(t, 3) && inLeft(t, 2)).toBe(true);
    close(H(t, 3), HEAD_COL_MASTER);
    // additional-counterexamples.jsonl 1행의 old — 31f87878 의 출력과 글자 그대로 같다
    expect(t).toEqual({
      type: "split", dir: "row", ratio: 0.3333333333333333,
      a: { type: "split", dir: "col", ratio: 0.75, a: { type: "pane", sid: 3, role: "master" }, b: { type: "pane", sid: 2, role: "cso" } },
      b: { type: "pane", sid: 1, role: "master" },
    });
  });
  it("adoptSeat: 같은 결과 · 구멍 없음", () => {
    const plan = adoptSeat([{ tree: saved() }], undefined, 3, "master", mapRole(role)) as NonNullable<ReturnType<typeof adoptSeat>>;
    expect(plan.tree).not.toEqual(BAD);
    expect(plan.boundHole).toBeNull();
    expect(inLeft(plan.tree, 3) && inLeft(plan.tree, 2)).toBe(true);
  });
  it("옛 칸이 빠지면 저장 모양 그대로 — col(master, cso) 3/4 · 전체 폭(rm · 이어지는 anchorHead)", () => {
    const t = placeSeatSafe(saved(), 3, mapRole(role), false);
    const u = anchorHead(rm(t, 1), mapRole(role)) as LNode;
    expect(u.type === "split" && u.dir).toBe("col");
    close(H(u, 3), HEAD_COL_MASTER);
    close(W(u, 3), 1);
    close(W(u, 2), 1);
  });
  it("옛 칸이 종료 이벤트로 구멍이 된 뒤 위생까지 — 같은 모양(이벤트 경로)", () => {
    const t = placeSeatSafe(saved(), 3, mapRole(role), false);
    const held = holdPane(t, 1, -1) as LNode;
    const [u] = tidyHoles([anchorHeadSafe(held, mapRole(role))], mapRole(role));
    expect(holeSidsOf(u)).toEqual([]);
    expect(shape(u)).toEqual(shape(saved()));
  });
  it("수동 탭: 재배치 없이 컬럼 위로 · 옛 칸이 빠지면 같은 모양", () => {
    const t = placeSeat(saved(), 3, mapRole(role), true);
    expect(rm(t, 1)).toEqual({ type: "split", dir: "col", ratio: 0.75, a: { type: "pane", sid: 3, role: "master" }, b: { type: "pane", sid: 2, role: "cso" } });
  });
});

describe("P8 r2 — 리뷰 반례 2: 낡은 pane.role=master 가 데몬이 cso 라 하는 칸 위로 새 master 를 얹지 않는다", () => {
  // codex-r1-review/additional-counterexamples.jsonl 2행의 restored · adopted · afterMemoRefresh
  const restored = (): LNode => ({ type: "split", dir: "row", ratio: 0.3333333333333333, a: { type: "pane", sid: 1, role: "master" }, b: { type: "pane", sid: 2, role: "worker" } });
  const STACK: LNode = {
    type: "split", dir: "row", ratio: 0.3333333333333333,
    a: { type: "split", dir: "col", ratio: 0.75, a: { type: "pane", sid: 3, role: "master" }, b: { type: "pane", sid: 1, role: "cso" } },
    b: { type: "pane", sid: 2, role: "worker" },
  };
  const role = new Map<number, string | null>([[1, "cso"], [2, "worker"], [3, "master"]]);
  it("adoptSeat: master 는 왼쪽 혼자 · cso 는 오른쪽 · 기억 갱신 뒤에도 쌓임이 남지 않는다", () => {
    const plan = adoptSeat([{ tree: restored() }], undefined, 3, "master", mapRole(role)) as NonNullable<ReturnType<typeof adoptSeat>>;
    const t = plan.tree;
    close(W(t, 3), MASTER_FRAC);
    close(H(t, 3), 1);
    expect(inLeft(t, 1)).toBe(false);
    annotateRoles(t, mapRole(role));
    expect(t).not.toEqual(STACK);
    const u = anchorHeadSafe(t, mapRole(role)) as LNode;
    close(H(u, 3), 1);
    expect(inLeft(u, 1)).toBe(false);
  });
  it("★데몬이 기억을 이긴다: 산 cso 둘의 컬럼에서 한 칸의 기억이 master 여도 master 자리가 아니다(반례 2 의 한 칸은 다른 조건도 막아 주므로 이 모양이 규칙을 판별한다)", () => {
    const tree: LNode = {
      type: "split", dir: "row", ratio: 0.5,
      a: { type: "split", dir: "col", ratio: 0.5, a: { type: "pane", sid: 1, role: "master" }, b: { type: "pane", sid: 5, role: "cso-2" } },
      b: { type: "pane", sid: 2, role: "worker" },
    };
    const r = new Map<number, string | null>([[1, "cso"], [5, "cso-2"], [2, "worker"], [3, "master"]]);
    const t = placeSeat(tree, 3, mapRole(r), false);
    close(W(t, 3), MASTER_FRAC);
    close(H(t, 3), 1);
    expect(inLeft(t, 1) || inLeft(t, 5)).toBe(false);
  });
  it("대조(유지해야 할 경우): 같은 칸의 데몬 역할이 null(종료)이면 기억을 믿는다 · cso 가 함께 있으면 그 위로", () => {
    const t = placeSeat(saved2(), 3, mapRole(new Map<number, string | null>([[1, null], [2, "cso"], [3, "master"], [4, "worker"]])), false);
    close(H(t, 3), HEAD_COL_MASTER);
    expect(inLeft(t, 2)).toBe(true);
  });
  const saved2 = (): LNode => ({ type: "split", dir: "row", ratio: 1 / 3, a: { type: "split", dir: "col", ratio: 0.75, a: { type: "pane", sid: 1, role: "master" }, b: { type: "pane", sid: 2, role: "cso" } }, b: { type: "pane", sid: 4, role: "worker" } });
});

describe("P8 r2 — 같은 부류: 기억으로 결정하는 다른 입구", () => {
  it("cso 종료 칸이 아직 트리에 있고 새 cso 가 온다(이벤트 지연): 그 칸이 기억한 자리다 · 옛 칸이 빠지면 저장 모양", () => {
    for (const manual of [false, true]) {
      const role = SAVED_ROLE();
      role.set(17, null);
      role.set(70, "cso");
      const t = placeSeat(SAVED(), 70, mapRole(role), manual);
      expect(inLeft(t, 70)).toBe(true);
      close(W(t, 18), MASTER_FRAC);
      close(H(t, 18), HEAD_COL_MASTER);
      expect(shape(rm(t, 17))).toEqual(shape(SAVED()));
    }
  });
  it("그 자리는 같은 역할의 것이다: cso-2 는 종료된 cso 칸에 앉지 않고 · 산 cso 칸도 나누지 않는다", () => {
    const exited = SAVED_ROLE();
    exited.set(17, null);
    exited.set(70, "cso-2");
    const a = placeSeat(SAVED(), 70, mapRole(exited), false);
    expect(inLeft(a, 70)).toBe(false);
    const alive = SAVED_ROLE();
    alive.set(71, "cso"); // 같은 역할이 둘 — 산 칸은 기억한 자리가 아니다
    const b = placeSeat(SAVED(), 71, mapRole(alive), false);
    expect(inLeft(b, 71)).toBe(false);
  });
  it("같은 경합 · 전체 폭 저장 탭 col(master, cso)", () => {
    const col = (): LNode => ({ type: "split", dir: "col", ratio: 0.75, a: { type: "pane", sid: 1, role: "master" }, b: { type: "pane", sid: 2, role: "cso" } });
    const role = new Map<number, string | null>([[1, "master"], [2, null], [3, "cso"]]);
    const t = placeSeat(col(), 3, mapRole(role), false);
    expect(shape(rm(t, 2))).toEqual(shape(col()));
  });
  it("master·cso 가 함께 종료 표시인 채로 둘 다 돌아온다: 저장 모양 그대로(왼쪽 컬럼 · 전체 폭 두 모양)", () => {
    const col = (): LNode => ({ type: "split", dir: "col", ratio: 0.75, a: { type: "pane", sid: 1, role: "master" }, b: { type: "pane", sid: 2, role: "cso" } });
    for (const start of [SAVED(), col()]) {
      const [om, oc] = sids(start).slice(0, 2);
      const role = SAVED_ROLE();
      role.set(om, null);
      role.set(oc, null);
      role.set(80, "master");
      role.set(81, "cso");
      let t = placeSeat(start, 80, mapRole(role), false);
      t = placeSeat(t, 81, mapRole(role), false);
      t = anchorHead(rm(rm(t, om) as LNode, oc), mapRole(role)) as LNode;
      expect(shape(t)).toEqual(shape(start));
    }
  });
  it("cso 만 있는 컬럼은 master 기억이 없으면 평범하다 — 데몬이 cso 라 하고 기억도 cso", () => {
    const col: LNode = { type: "split", dir: "row", ratio: 0.5, a: { type: "split", dir: "col", ratio: 0.5, a: { type: "pane", sid: 1, role: "cso" }, b: { type: "pane", sid: 2, role: "cso-2" } }, b: { type: "pane", sid: 4, role: "worker" } };
    const role = new Map<number, string | null>([[1, "cso"], [2, "cso-2"], [4, "worker"], [9, "master"]]);
    const t = placeSeat(col, 9, mapRole(role), false);
    close(W(t, 9), MASTER_FRAC);
    close(H(t, 9), 1);
    expect(inLeft(t, 1) || inLeft(t, 2)).toBe(false);
  });
  it("cso 만 있는 컬럼은 master 가 다른 곳에 있어도 끌려가지 않는다 — master 와 같은 컬럼의 cso 만 함께 간다", () => {
    const t: LNode = {
      type: "split", dir: "row", ratio: 0.5,
      a: { type: "split", dir: "col", ratio: 0.5, a: { type: "pane", sid: 1, role: "cso" }, b: { type: "pane", sid: 2, role: "worker" } },
      b: { type: "pane", sid: 3, role: "master" },
    };
    const role = new Map<number, string | null>([[1, "cso"], [2, "worker"], [3, "master"]]);
    const u = anchorHead(t, mapRole(role)) as LNode;
    close(W(u, 3), MASTER_FRAC);
    expect(inLeft(u, 1)).toBe(false);
  });
  it("저장 쌍이 왼쪽이 아닌 곳에 있으면 쌍째 왼쪽으로(떨어뜨리지 않는다 · 오늘과 같은 결과)", () => {
    const t: LNode = {
      type: "split", dir: "row", ratio: 0.5,
      a: { type: "pane", sid: 4, role: "worker" },
      b: { type: "split", dir: "col", ratio: 0.75, a: { type: "pane", sid: 1, role: "master" }, b: { type: "pane", sid: 2, role: "cso" } },
    };
    const role = new Map<number, string | null>([[1, "master"], [2, "cso"], [4, "worker"]]);
    const u = anchorHead(t, mapRole(role)) as LNode;
    close(W(u, 1), MASTER_FRAC);
    close(H(u, 1), HEAD_COL_MASTER);
    expect(inLeft(u, 2)).toBe(true);
    close(W(u, 4), 2 / 3);
  });
  it("master 와 셸만 있는 col 탭은 오늘 규칙 그대로 — master 1/3 · 셸 2/3(cso 예외가 셸로 번지지 않는다)", () => {
    const t: LNode = { type: "split", dir: "col", ratio: 0.5, a: { type: "pane", sid: 1, role: "master" }, b: { type: "pane", sid: 2 } };
    const role = new Map<number, string | null>([[1, "master"], [2, null]]);
    const u = anchorHead(t, mapRole(role)) as LNode;
    close(W(u, 1), MASTER_FRAC);
    close(W(u, 2), 2 / 3);
  });
  it("기억만 남은 master 칸 하나(cso 없음): 새 master 는 오늘 규칙대로 자기 컬럼", () => {
    const t: LNode = { type: "split", dir: "row", ratio: 1 / 3, a: { type: "pane", sid: 1, role: "master" }, b: { type: "pane", sid: 2, role: "worker" } };
    const role = new Map<number, string | null>([[1, null], [2, "worker"], [3, "master"]]);
    const u = placeSeat(t, 3, mapRole(role), false);
    close(W(u, 3), MASTER_FRAC);
    close(H(u, 3), 1);
  });
});

describe("P8 r2 — 기동·재기동 합성(main.ts start() 와 3초 틱의 순수 골격)", () => {
  type Seat = [number, string];
  /** start(): restoreTree → 기억 갱신 → 고아 입양(seatPriority 순) → 구멍 위생 → S5 anchorHeadSafe(비수동만). */
  function start(tree: LNode, manual: boolean, gen: boolean, seats: Seat[]): LNode {
    const role = new Map<number, string | null>(seats);
    let h = -100;
    const t0 = restoreTree(tree, { genChanged: gen, isLive: (s) => role.has(s), liveRole: mapRole(role), roleConflictIsDead: false, allocHole: () => h-- });
    annotateRoles(t0, mapRole(role)); // main.ts 와 같은 자리 · 소스 핀 wswiring.test.ts
    let ws: WsView[] = [{ tree: t0, layoutManual: manual }];
    for (const [s, r] of [...seats].sort((a, b) => seatPriority(a[1]) - seatPriority(b[1]) || a[0] - b[0])) {
      if (sids(ws[0].tree).includes(s)) continue;
      const plan = adoptSeat(ws, undefined, s, r, mapRole(role));
      ws = [{ tree: (plan as NonNullable<typeof plan>).tree, layoutManual: manual }];
    }
    const t1 = tidyHoles([ws[0].tree], mapRole(role))[0] as LNode;
    return manual ? t1 : (anchorHeadSafe(t1, mapRole(role)) as LNode);
  }
  /** 3초 틱: 기억 갱신 → 위생 → 비수동 재정렬. 몇 번을 돌아도 모양이 같아야 한다. */
  function tick(t: LNode, manual: boolean, seats: Seat[]): LNode {
    const role = new Map<number, string | null>(seats);
    annotateRoles(t, mapRole(role));
    const u = tidyHoles([t], mapRole(role))[0] as LNode;
    return manual ? u : (anchorHeadSafe(u, mapRole(role)) as LNode);
  }
  const col = (): LNode => ({ type: "split", dir: "col", ratio: 0.75, a: { type: "pane", sid: 1, role: "master" }, b: { type: "pane", sid: 2, role: "cso" } });
  it("저장 화면 · 데몬 세대 변경 · 전 좌석 새 sid: 모양 불변 · cso 는 master 밑 · 틱을 돌아도 불변", () => {
    for (const manual of [false, true]) {
      const seats: Seat[] = [[26, "worker"], [27, "reviewer-codex"], [28, "worker-2"], [29, "cso"], [30, "master"], [31, "reviewer-gemini"]];
      let t = start(SAVED(), manual, true, seats);
      expect(shape(t)).toEqual(shape(SAVED()));
      for (let i = 0; i < 3; i++) t = tick(t, manual, seats);
      expect(shape(t)).toEqual(shape(SAVED()));
    }
  });
  it("같은 데몬 · 앱만 재시작 · 저장 칸의 역할이 그 사이 바뀌었다(반례 2 합성): 틱 뒤에도 master 밑에 cso 가 쌓이지 않는다", () => {
    const seats: Seat[] = [[1, "cso"], [2, "worker"], [3, "master"]];
    const tree: LNode = { type: "split", dir: "row", ratio: 1 / 3, a: { type: "pane", sid: 1, role: "master" }, b: { type: "pane", sid: 2, role: "worker" } };
    let t = start(tree, false, false, seats);
    for (let i = 0; i < 3; i++) t = tick(t, false, seats);
    close(W(t, 3), MASTER_FRAC);
    close(H(t, 3), 1);
    expect(inLeft(t, 1)).toBe(false);
    expect(JSON.stringify(t)).toContain('"sid":1,"role":"cso"');
  });
  it("첫 틱 전의 기억 전용 경로(reanchorAuto 기본값 · +New bareSeat)도 쌓지 않는다 — 기동이 기억을 먼저 고쳤다", () => {
    const seats: Seat[] = [[1, "cso"], [2, "worker"], [3, "master"]];
    const tree = (): LNode => ({ type: "split", dir: "row", ratio: 1 / 3, a: { type: "pane", sid: 1, role: "master" }, b: { type: "pane", sid: 2, role: "worker" } });
    const u = anchorHeadSafe(start(tree(), false, false, seats), () => undefined) as LNode;
    close(H(u, 3), 1);
    expect(inLeft(u, 1)).toBe(false);
    const v = placeSeatSafe(start(tree(), false, false, seats), 9, (x) => (x === 9 ? null : undefined), false);
    close(H(v, 3), 1);
    expect(inLeft(v, 1)).toBe(false);
  });
  it("전체 폭 저장 탭 · 재시작 · 틱: col(master, cso) 불변(반례 1 의 앞뒤)", () => {
    const seats: Seat[] = [[41, "master"], [42, "cso"]];
    let t = start(col(), false, true, seats);
    expect(shape(t)).toEqual(shape(col()));
    for (let i = 0; i < 3; i++) t = tick(t, false, seats);
    expect(shape(t)).toEqual(shape(col()));
  });
  it("새 부팅은 넓어지지 않는다: 저장 트리 없이 start() → 왼쪽 master 혼자 · 틱 고정점", () => {
    const seats: Seat[] = [[1, "cso"], [2, "worker"], [3, "master"], [4, "reviewer-gemini"]];
    const role = new Map<number, string | null>(seats);
    let t: LNode | null = null;
    for (const [s] of [...seats].sort((a, b) => seatPriority(a[1]) - seatPriority(b[1]))) t = placeSeat(t, s, mapRole(role), false);
    let u = t as unknown as LNode;
    for (let i = 0; i < 3; i++) u = tick(u, false, seats);
    close(W(u, 3), MASTER_FRAC);
    close(H(u, 3), 1);
    expect(inLeft(u, 1)).toBe(false);
  });
});
