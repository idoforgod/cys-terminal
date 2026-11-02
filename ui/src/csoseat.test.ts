// P8(0.14.57 · 오너 2026-11-03): CSO 기본 자리를 master 밑에서 해제한다 — 배치 동작 핀.
// 오너 확정: 왼쪽 1/3 은 master 혼자 · CSO 는 오른쪽 2/3 의 평범한 좌석 · 창 배치만 바뀐다(역할·권한 불변) ·
// 저장된 화면은 자동으로 옮기지 않는다(새 창과 `정렬` 만 새 배치).
// ★이 파일은 공개 API 만 쓴다 — 옛 규칙에서 적색이 나면 그것은 없는 심볼이 아니라 **동작**의 적색이다.
import { describe, it, expect } from "bun:test";
import {
  MASTER_FRAC,
  HEAD_COL_MASTER,
  adoptSeat,
  anchorHead,
  anchorHeadSafe,
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
