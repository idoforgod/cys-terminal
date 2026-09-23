// U3(대표 자리 1/3 유지) — 좌석 배치 순수부 seatlayout.ts 의 동작 핀.
// (U2 역할 결속은 seatbind.test.ts · main.ts 배선은 wswiring.test.ts)
//
// ★무엇을 고정하나: 좌석이 몇 개가 어떤 순서로 와도 대표(master) 컬럼은 pane 영역의 1/3 이고, 대표 칸은
//   master(위)/cso(아래) 모양이며, 나머지는 2/3 를 균등히 나눈다. 사용자가 끈 탭(수동)은 재배치하지 않되
//   새 좌석 때문에 대표가 줄지는 않는다. 어떤 입력에도 던지지 않고 sid 를 잃거나 겹치게 하지 않는다.
// 반박 보고서(phase1/U3-master-width-third.refute.md)의 반례 R1~R9 · D1~D12 를 여기서 검체로 박제한다.
import { describe, it, expect } from "bun:test";
import {
  MASTER_FRAC,
  HEAD_COL_MASTER,
  anchorHead,
  anchorHeadSafe,
  holdPane,
  isAnchored,
  isAutoRatio,
  isCsoRole,
  isHeadRole,
  isMasterRole,
  isValidTree,
  legacyAppend,
  liveSidsOf,
  looksManual,
  placeSeat,
  placeSeatSafe,
  roleLayout,
  sanitizeRole,
  seatPriority,
  type LNode,
  type RoleOf,
} from "./seatlayout";

// ---------- 오라클 (렌더러와 같은 규칙: 안 보이는 구멍 쪽은 접힌다) ----------
const shown = (n: LNode, hs: (s: number) => boolean): boolean =>
  n.type === "pane" ? n.sid > 0 || hs(n.sid) : shown(n.a, hs) || shown(n.b, hs);
function share(t: LNode, sid: number, axis: "row" | "col", hs: (s: number) => boolean = () => false, acc = 1): number | null {
  if (t.type === "pane") return t.sid === sid ? acc : null;
  const sa = shown(t.a, hs);
  const sb = shown(t.b, hs);
  if (!sa) return share(t.b, sid, axis, hs, acc);
  if (!sb) return share(t.a, sid, axis, hs, acc);
  const r = t.ratio ?? 0.5;
  const [ra, rb] = t.dir === axis ? [r, 1 - r] : [1, 1];
  return share(t.a, sid, axis, hs, acc * ra) ?? share(t.b, sid, axis, hs, acc * rb);
}
const W = (t: LNode, sid: number) => share(t, sid, "row");
const H = (t: LNode, sid: number) => share(t, sid, "col");
const close = (a: number | null, b: number, eps = 1e-9) => {
  expect(a).not.toBeNull();
  expect(Math.abs((a as number) - b)).toBeLessThan(eps);
};
const sids = (t: LNode | null): number[] => {
  const out: number[] = [];
  const walk = (n: LNode | null) => {
    if (!n) return;
    if (n.type === "pane") out.push(n.sid);
    else {
      walk(n.a);
      walk(n.b);
    }
  };
  walk(t);
  return out;
};
const inSub = (n: LNode, sid: number): boolean => sids(n).includes(sid);
const mapRole = (m: Map<number, string | null>): RoleOf => (s) => (m.has(s) ? m.get(s) : undefined);
function perms<T>(xs: T[]): T[][] {
  if (xs.length <= 1) return [xs];
  return xs.flatMap((x, i) => perms([...xs.slice(0, i), ...xs.slice(i + 1)]).map((p) => [x, ...p]));
}
const rm = (n: LNode, sid: number): LNode | null =>
  n.type === "pane" ? (n.sid === sid ? null : n) : ((a, b) => (a && b ? { ...n, a, b } : a ?? b))(rm(n.a, sid), rm(n.b, sid));
function build(roles: [number, string | null][], manual = false): { t: LNode; role: Map<number, string | null> } {
  const role = new Map<number, string | null>();
  let t: LNode | null = null;
  for (const [s, r] of roles) {
    role.set(s, r);
    t = placeSeat(t, s, mapRole(role), manual);
  }
  return { t: t as LNode, role };
}
const ROLES6 = ["master", "cso", "worker", "worker-2", "reviewer-codex", "reviewer-gemini"];

// ────────────────────────────────────────────────────────────────────────────
describe("역할 술어 — 대표 칸 소속은 하나의 술어다(반박 D5·D6)", () => {
  it("master 는 정확 일치 — master-fresh-* 는 대표가 아니다", () => {
    expect(isMasterRole("master")).toBe(true);
    expect(isMasterRole("master-fresh-1790000000")).toBe(false);
    expect(isMasterRole("Master")).toBe(false);
  });
  it("cso · cso-<숫자> 는 대표 칸, cso-fresh-<epoch> 은 아니다", () => {
    for (const r of ["cso", "cso-2", "cso-10"]) expect(isCsoRole(r)).toBe(true);
    for (const r of ["cso-fresh-1790000000", "cso2", "csox", "cso-", "xcso"]) expect(isCsoRole(r)).toBe(false);
    expect(isHeadRole("cso-2")).toBe(true);
    expect(isHeadRole("worker")).toBe(false);
  });
  it("sanitizeRole — 저장·비교를 망가뜨릴 모양과 일회용 좌석만 거른다", () => {
    for (const r of ["master", "cso-2", "worker-3", "reviewer-gemini", "dept-1-master", "한글역할"]) expect(sanitizeRole(r)).toBe(r);
    for (const r of ["", " master", "master ", "a".repeat(65), "x\ny", "worker-fresh-1790000000", 7, null, undefined, {}, ["master"]])
      expect(sanitizeRole(r)).toBeUndefined();
  });
  it("입양 순서 master < cso < cso-N < worker < reviewer < 그 밖", () => {
    const order = ["reviewer-codex", null, "worker-2", "cso-2", "master", "cso", "worker"].sort(
      (a, b) => seatPriority(a) - seatPriority(b),
    );
    expect(order.slice(0, 1)).toEqual(["master"]);
    expect(seatPriority("cso")).toBe(1);
    expect(seatPriority("cso-2")).toBe(1);
    expect(seatPriority("worker-2")).toBe(2);
    expect(seatPriority("reviewer-codex")).toBe(3);
    expect(seatPriority(null)).toBe(4);
  });
});

// ────────────────────────────────────────────────────────────────────────────
describe("정렬(roleLayout) — 대표 컬럼 1/3 · 나머지 2/3 균등", () => {
  for (let w = 0; w <= 8; w++) {
    it(`worker ${w}명 + 리뷰어 2 → master 폭 1/3, 나머지 컬럼 (2/3)/${w + 1}`, () => {
      const role = new Map<number, string | null>([[1, "master"], [2, "cso"], [90, "reviewer-gemini"], [91, "reviewer-codex"]]);
      for (let k = 0; k < w; k++) role.set(10 + k, k ? `worker-${k + 1}` : "worker");
      const t = roleLayout([...role.keys()], mapRole(role)) as LNode;
      close(W(t, 1), MASTER_FRAC);
      close(H(t, 1), HEAD_COL_MASTER);
      for (let k = 0; k < w; k++) close(W(t, 10 + k), 2 / 3 / (w + 1));
      close(W(t, 90), 2 / 3 / (w + 1));
      close(W(t, 91), 2 / 3 / (w + 1)); // 리뷰어 둘은 한 컬럼(agy 위 / codex 아래)
    });
  }
  it("master 만 → 전체 폭 · master+cso-2 → 대표 컬럼만(전체 폭, cso-2 는 대표 칸 아래)", () => {
    close(W(roleLayout([1], () => "master") as LNode, 1), 1);
    const r = new Map<number, string | null>([[1, "master"], [2, "cso-2"]]);
    const t = roleLayout([1, 2], mapRole(r)) as LNode;
    close(W(t, 1), 1);
    close(H(t, 2), 1 - HEAD_COL_MASTER);
  });
  it("역할이 하나도 없으면 종전과 같다(모든 컬럼 균등)", () => {
    const t = roleLayout([1, 2, 3, 4], () => null) as LNode;
    for (const s of [1, 2, 3, 4]) close(W(t, s), 1 / 4);
  });
  it("cso-fresh-* 는 대표 칸이 아니라 가운데 — master 높이는 전체", () => {
    const r = new Map<number, string | null>([[1, "master"], [2, "cso-fresh-1790000000"], [3, "worker"]]);
    const t = roleLayout([1, 2, 3], mapRole(r)) as LNode;
    close(H(t, 1), 1);
    close(W(t, 1), MASTER_FRAC);
    close(W(t, 2), 1 / 3);
  });
  it("cso·cso-2 는 **자동 배치와 같은 술어로** 대표 칸에 모인다(D6)", () => {
    const r = new Map<number, string | null>([[1, "master"], [2, "cso"], [3, "cso-2"], [4, "worker"]]);
    const t = roleLayout([1, 2, 3, 4], mapRole(r)) as LNode;
    expect(t.type === "split" && inSub(t.a, 2) && inSub(t.a, 3)).toBe(true);
    close(W(t, 4), 2 / 3);
  });
  it("정렬 결과 칸에는 역할이 적힌다(다음 재부팅의 자리 기억)", () => {
    const r = new Map<number, string | null>([[1, "master"], [2, "worker"]]);
    const t = roleLayout([1, 2], mapRole(r)) as LNode;
    expect(JSON.stringify(t)).toContain('"role":"master"');
  });
});

// ────────────────────────────────────────────────────────────────────────────
describe("자동 배치 — 표준 6석의 도착 순서 720가지 전수(비수동)", () => {
  const all = perms(ROLES6);
  it(`${all.length}가지 × 매 단계: sid 중복 0·손실 0 · 대표 외 좌석이 생긴 뒤 master 폭 = 1/3`, () => {
    expect(all.length).toBe(720);
    let checked = 0;
    for (const order of all) {
      const role = new Map<number, string | null>();
      let t: LNode | null = null;
      order.forEach((r, i) => {
        role.set(i + 1, r);
        t = placeSeat(t, i + 1, mapRole(role), false);
        const ids = sids(t);
        expect(new Set(ids).size).toBe(ids.length);
        expect(ids.length).toBe(i + 1);
        const m = order.indexOf("master") + 1;
        const nonHead = order.slice(0, i + 1).some((x) => !isHeadRole(x));
        if (order.indexOf("master") <= i && nonHead) {
          close(W(t as LNode, m), MASTER_FRAC);
          checked++;
        }
      });
    }
    expect(checked).toBeGreaterThan(2000); // 오라클이 실제로 돌았다
  });
  it(`${all.length}가지 최종 모양: cso 는 대표 칸(root.a) 안 · master 위 3/4 · 나머지 4컬럼 각 1/6`, () => {
    let bad = 0;
    for (const order of all) {
      const { t } = build(order.map((r, i) => [i + 1, r] as [number, string]));
      const m = order.indexOf("master") + 1;
      const c = order.indexOf("cso") + 1;
      const ok =
        t.type === "split" &&
        t.dir === "row" &&
        inSub(t.a, m) &&
        inSub(t.a, c) &&
        Math.abs((H(t, m) ?? 0) - HEAD_COL_MASTER) < 1e-9 &&
        order.every((r, i) => isHeadRole(r) || Math.abs((W(t, i + 1) ?? 0) - 1 / 6) < 1e-9);
      if (!ok) bad++;
    }
    expect(bad).toBe(0); // 프로토타입은 336/720 에서 cso 가 대표 칸 밖이었다(반박 D1)
  });
});

// ────────────────────────────────────────────────────────────────────────────
describe("수동 탭(layoutManual) — 재배치하지 않되 대표는 줄지 않는다", () => {
  it("720가지 × 수동: master 와 대표 외 좌석이 함께 생긴 뒤로 master 폭 불변", () => {
    for (const order of perms(ROLES6)) {
      const role = new Map<number, string | null>();
      let t: LNode | null = null;
      let fixed: number | null = null;
      order.forEach((r, i) => {
        role.set(i + 1, r);
        t = placeSeat(t, i + 1, mapRole(role), true);
        const m = order.indexOf("master") + 1;
        const nonHead = order.slice(0, i + 1).some((x) => !isHeadRole(x));
        if (order.indexOf("master") <= i && nonHead) {
          const w = W(t as LNode, m) as number;
          if (fixed === null) fixed = w;
          else expect(Math.abs(w - fixed)).toBeLessThan(1e-12);
        }
      });
    }
  });
  it("사용자가 끈 루트 비율 0.5 → 좌석 7개가 더 와도 master 0.5", () => {
    const role = new Map<number, string | null>([[1, "master"], [2, "worker"]]);
    let t: LNode = { type: "split", dir: "row", ratio: 0.5, a: { type: "pane", sid: 1 }, b: { type: "pane", sid: 2 } };
    for (let k = 3; k < 10; k++) {
      role.set(k, "worker-" + k);
      t = placeSeat(t, k, mapRole(role), true);
      close(W(t, 1), 0.5);
    }
  });
  it("★master 옆 ⌘D(대표 칸에 셸이 섞임) 뒤 좌석 4개 — master 폭 불변(반박 D2: 종전 0.111→0.056)", () => {
    const { t: t0, role } = build([[1, "master"], [2, "cso"], [3, "worker"]]);
    const rep = (n: LNode): LNode =>
      n.type === "pane" ? (n.sid === 1 ? { type: "split", dir: "row", a: n, b: { type: "pane", sid: 50 } } : n) : { ...n, a: rep(n.a), b: rep(n.b) };
    role.set(50, null);
    let t = rep(t0);
    const before = W(t, 1) as number;
    for (const s of [4, 5, 6, 7]) {
      role.set(s, "worker-" + s);
      t = placeSeat(t, s, mapRole(role), true);
      close(W(t, 1), before);
    }
    // 비수동이어도 셸은 대표 칸에 남고(역할 없는 셸 허용) 대표 컬럼은 1/3
    let u = rep(build([[1, "master"], [2, "cso"], [3, "worker"]]).t);
    for (const s of [4, 5]) u = placeSeat(u, s, mapRole(role), false);
    expect(isAnchored(u, mapRole(role))).toBe(true);
    close((W(u, 1) as number) + (W(u, 50) as number), MASTER_FRAC);
  });
  it("사용자가 master 를 오른쪽으로 옮긴 수동 탭 — 절반이 아니라 균등 몫으로만 준다", () => {
    const role = new Map<number, string | null>([[1, "master"], [2, "worker"], [3, "worker-2"]]);
    let t: LNode = { type: "split", dir: "row", ratio: 0.5, a: { type: "pane", sid: 2 }, b: { type: "split", dir: "row", ratio: 0.5, a: { type: "pane", sid: 3 }, b: { type: "pane", sid: 1 } } };
    let prev = W(t, 1) as number;
    for (const s of [4, 5, 6]) {
      role.set(s, "worker-" + s);
      const n = 2 + (s - 4) + 1; // 삽입 전 보이는 컬럼 수
      t = placeSeat(t, s, mapRole(role), true);
      close(W(t, 1), (prev * n) / (n + 1));
      prev = W(t, 1) as number;
    }
  });
});

// ────────────────────────────────────────────────────────────────────────────
describe("재기동 — master·cso 가 새 sid 로 돌아온다(반박 D1·D2)", () => {
  it("비수동: 옛 master 칸이 접힌 뒤 새 master → 대표 칸 위 · 1/3 · cso 는 대표 칸 안", () => {
    const { t: t0, role } = build([[1, "master"], [2, "cso"], [3, "worker"], [4, "worker-2"]]);
    let t = rm(t0, 1) as LNode;
    role.delete(1);
    role.set(9, "master");
    t = placeSeat(t, 9, mapRole(role), false);
    close(W(t, 9), MASTER_FRAC);
    close(H(t, 9), HEAD_COL_MASTER);
    expect(t.type === "split" && inSub(t.a, 2)).toBe(true);
    close(W(t, 3), 1 / 3);
    close(W(t, 4), 1 / 3);
  });
  it("수동: 루트 0.5 로 끈 탭에서 master 재기동 → 같은 컬럼 위로(0.5 유지 · 반박 R2 의 1/3 덮어쓰기 없음)", () => {
    const { t: t0, role } = build([[1, "master"], [2, "cso"], [3, "worker"], [4, "worker-2"]]);
    let t = rm({ ...(t0 as LNode & { type: "split" }), ratio: 0.5 }, 1) as LNode;
    role.delete(1);
    role.set(9, "master");
    t = placeSeat(t, 9, mapRole(role), true);
    close(W(t, 9), 0.5);
    close(W(t, 2), 0.5);
    expect(t.type === "split" && inSub(t.a, 9) && inSub(t.a, 2)).toBe(true);
  });
  it("cso 재기동 → 대표 칸 아래로(비수동·수동 모두)", () => {
    for (const manual of [false, true]) {
      const { t: t0, role } = build([[1, "master"], [2, "cso"], [3, "worker"]], manual);
      let t = rm(t0, 2) as LNode;
      role.delete(2);
      role.set(8, "cso");
      t = placeSeat(t, 8, mapRole(role), manual);
      expect(t.type === "split" && inSub(t.a, 8) && inSub(t.a, 1)).toBe(true);
      close(H(t, 1), HEAD_COL_MASTER);
    }
  });
  it("종료된 옛 master(역할 맵에서 null)는 대표로 세지 않는다 — 대표 칸에 master 가 둘이 되지 않는다(반박 D4)", () => {
    const { t: t0, role } = build([[1, "master"], [2, "cso"], [3, "worker"]]);
    role.set(1, null); // 입양 틱은 `s.exited ? null : s.role` 로 맵을 만든다
    role.set(9, "master");
    const t = placeSeat(t0, 9, mapRole(role), false);
    const masters = sids(t).filter((s) => role.get(s) === "master");
    expect(masters).toEqual([9]);
    close(H(t, 9), HEAD_COL_MASTER);
    close(W(t, 9), MASTER_FRAC);
  });
});

// ────────────────────────────────────────────────────────────────────────────
describe("일회용·캐시 공백·대표 없는 탭", () => {
  it("cso-fresh 가 붙어도 master 높이 3/4 유지 · TTL 로 닫히면 원래 폭(반박 D5·R6)", () => {
    const { t: t0, role } = build([[1, "master"], [2, "cso"], [3, "worker"]]);
    role.set(5, "cso-fresh-1790000000");
    const t = placeSeat(t0, 5, mapRole(role), false);
    close(H(t, 1), HEAD_COL_MASTER);
    close(W(t, 1), MASTER_FRAC);
    const back = anchorHead(rm(t, 5), mapRole(role)) as LNode;
    close(W(back, 1), MASTER_FRAC);
    close(W(back, 3), 2 / 3);
  });
  it("오너 실사용 CEO 트리(정렬 0.25) + worker-fresh 1개 → master 1/3 · 닫힌 뒤에도 1/3(반박 R6)", () => {
    const role = new Map<number, string | null>([[18, "master"], [17, "cso"], [14, "worker"], [16, "worker-2"], [19, "reviewer-gemini"], [15, "reviewer-codex"], [30, "worker-fresh-1790000000"]]);
    const live: LNode = {
      type: "split", dir: "row", ratio: 0.25,
      a: { type: "split", dir: "col", ratio: 0.75, a: { type: "pane", sid: 18 }, b: { type: "pane", sid: 17 } },
      b: { type: "split", dir: "row", ratio: 1 / 3, a: { type: "pane", sid: 14 },
        b: { type: "split", dir: "row", ratio: 0.5, a: { type: "pane", sid: 16 },
          b: { type: "split", dir: "col", ratio: 0.5, a: { type: "pane", sid: 19 }, b: { type: "pane", sid: 15 } } } },
    };
    const t = placeSeat(live, 30, mapRole(role), false);
    close(W(t, 18), MASTER_FRAC);
    for (const s of [14, 16, 19, 30]) close(W(t, s), 2 / 3 / 4);
    const back = anchorHead(rm(t, 30), mapRole(role)) as LNode;
    close(W(back, 18), MASTER_FRAC);
  });
  it("+New 직후 역할 캐시가 비어도 칸의 기억(role)으로 대표를 알아본다(반박 D10 개선)", () => {
    const { t } = build([[1, "master"], [2, "cso"], [3, "worker"], [4, "worker-2"]]);
    const t2 = placeSeat(t, 20, (s) => (s === 20 ? null : undefined), false); // 기존 칸은 '모름'
    close(W(t2, 1), MASTER_FRAC);
    close(W(t2, 20), 2 / 3 / 3);
  });
  it("대표가 없는 탭은 종전 규칙 그대로 — +New 5회가 옛 오른쪽 부착과 **동일 트리**(반박 D12)", () => {
    let t: LNode | null = null;
    let legacy: LNode | null = null;
    for (const s of [1, 2, 3, 4, 5]) {
      t = placeSeat(t, s, () => null, false);
      legacy = legacyAppend(legacy, s);
    }
    expect(t).toEqual(legacy as LNode);
    close(W(t as LNode, 1), 1 / 16);
  });
  it("대표만 있는 탭(master+cso) → 전체 폭, master 위 3/4", () => {
    for (const order of [["master", "cso"], ["cso", "master"]]) {
      const { t } = build(order.map((r, i) => [i + 1, r] as [number, string]));
      const m = order.indexOf("master") + 1;
      close(W(t, m), 1);
      close(H(t, m), HEAD_COL_MASTER);
    }
  });
  it("중복 sid → 같은 객체(멱등 · 중복은 DOM 한 칸을 비운다)", () => {
    const t: LNode = { type: "pane", sid: 1 };
    expect(placeSeat(t, 1, () => "master", false)).toBe(t);
    expect(placeSeatSafe(t, 1, () => "master", false)).toBe(t);
  });
});

// ────────────────────────────────────────────────────────────────────────────
describe("구버전 저장본 — 기동 즉시 1/3(S5) · 수동 추정 시드(반박 D3)", () => {
  const CEO: LNode = {
    type: "split", dir: "row", ratio: 0.25,
    a: { type: "split", dir: "col", ratio: 0.75, a: { type: "pane", sid: 18 }, b: { type: "pane", sid: 17 } },
    b: { type: "split", dir: "row", ratio: 0.3333333333333333, a: { type: "pane", sid: 14 },
      b: { type: "split", dir: "row", ratio: 0.5, a: { type: "pane", sid: 16 },
        b: { type: "split", dir: "col", ratio: 0.5, a: { type: "pane", sid: 19 }, b: { type: "pane", sid: 15 } } } },
  };
  const ROLE = new Map<number, string | null>([[18, "master"], [17, "cso"], [14, "worker"], [16, "worker-2"], [19, "reviewer-gemini"], [15, "reviewer-codex"]]);
  it("★오너 CEO 탭(0.25·0.75·1/3·0.5·0.5)은 전부 자동값 → 수동 아님 → 기동 즉시 master 1/3 · 오른쪽 3컬럼 각 2/9", () => {
    expect(looksManual(CEO)).toBe(false);
    const t = anchorHeadSafe(CEO, mapRole(ROLE)) as LNode;
    close(W(t, 18), MASTER_FRAC);
    for (const s of [14, 16, 19]) close(W(t, s), 2 / 9);
    expect(sids(t).sort()).toEqual(sids(CEO).sort());
  });
  it("종전 반감 사슬(1/8) → anchorHead → 1/3 · cso 는 대표 칸 · sid 보존", () => {
    const role = new Map<number, string | null>([[1, "master"], [2, "cso"], [3, "worker"], [4, "worker-2"]]);
    let t: LNode = { type: "pane", sid: 1 };
    for (const s of [2, 3, 4]) t = { type: "split", dir: "row", a: t, b: { type: "pane", sid: s } };
    close(W(t, 1), 1 / 8);
    const u = anchorHead(t, mapRole(role)) as LNode;
    close(W(u, 1), MASTER_FRAC);
    expect(u.type === "split" && inSub(u.a, 2)).toBe(true);
    expect(sids(u).sort()).toEqual([1, 2, 3, 4]);
  });
  it("사용자가 끈 비율(임의 실수·클램프 0.15/0.85)은 수동으로 추정 — 자동값(1/k·3/4·n/(n+1))은 아님", () => {
    for (const r of [0.55, 0.15, 0.85, 0.4127]) expect(isAutoRatio(r)).toBe(false);
    for (const r of [undefined, null, 0.5, 1 / 3, 0.25, 0.75, 2 / 3, 5 / 6, 1 / 7, 15 / 16]) expect(isAutoRatio(r)).toBe(true);
    expect(looksManual({ type: "split", dir: "row", ratio: 0.55, a: { type: "pane", sid: 1 }, b: { type: "pane", sid: 3 } })).toBe(true);
    expect(looksManual(null)).toBe(false);
    expect(looksManual({ type: "pane", sid: 1 })).toBe(false);
  });
});

// ────────────────────────────────────────────────────────────────────────────
describe("유령 sid 가 섞인 틱 — 정리 뒤에도 오른쪽 균등(반박 D7·r9)", () => {
  it("데몬 재기동: 옛 1..5 가 남은 채 새 11..15 입양 → 옛 것 제거 → 재정렬하면 master 1/3 · 나머지 균등", () => {
    const role = new Map<number, string | null>([[11, "master"], [12, "cso"], [13, "worker"], [14, "worker-2"], [15, "reviewer-codex"]]);
    const old = new Map<number, string | null>([[1, "master"], [2, "cso"], [3, "worker"], [4, "worker-2"], [5, "reviewer-codex"]]);
    let t: LNode | null = null;
    for (const s of [1, 2, 3, 4, 5]) t = placeSeat(t, s, mapRole(old), false);
    for (const s of [11, 12, 13, 14, 15]) t = placeSeat(t, s, mapRole(role), false); // 옛 sid 는 '모름' → 칸의 기억
    for (const s of [1, 2, 3, 4, 5]) t = rm(t as LNode, s);
    const u = anchorHeadSafe(t, mapRole(role)) as LNode;
    close(W(u, 11), MASTER_FRAC);
    for (const s of [13, 14, 15]) close(W(u, s), 2 / 9);
    expect(u.type === "split" && inSub(u.a, 12)).toBe(true);
  });
});

// ────────────────────────────────────────────────────────────────────────────
describe("안전 래퍼 — 어떤 입력에도 던지지 않고 잃지 않는다(③·④ 방어)", () => {
  it("roleOf 가 던지면 종전 오른쪽 부착으로 떨어진다(sid 보존)", () => {
    const { t } = build([[1, "master"], [2, "worker"]]);
    const boom: RoleOf = () => {
      throw new Error("boom");
    };
    const out = placeSeatSafe(t, 7, boom, false);
    expect(out).toEqual(legacyAppend(t, 7));
    expect(anchorHeadSafe(t, boom)).toBe(t);
  });
  it("손상 트리(임의 JSON)에서 공개 연산이 던지지 않는다", () => {
    const junk: unknown[] = [
      null, undefined, 0, 1, "x", [], [1, 2], {}, { type: "pane" }, { type: "pane", sid: "3" }, { type: "pane", sid: 1.5 },
      { type: "pane", sid: 0 }, { type: "pane", sid: NaN }, { type: "split" }, { type: "split", dir: "diag", a: {}, b: {} },
      { type: "split", dir: "row", ratio: 7, a: { type: "pane", sid: 1 }, b: { type: "pane", sid: 2 } },
      { type: "split", dir: "row", a: { type: "pane", sid: 1 }, b: { type: "pane", sid: 1 } }, // 중복 sid
      { type: "split", dir: "row", a: { type: "pane", sid: 1 } }, // b 누락
      { type: "pane", sid: 1, role: 5 },
    ];
    let deep: unknown = { type: "pane", sid: 1 };
    for (let i = 0; i < 400; i++) deep = { type: "split", dir: "row", a: deep, b: { type: "pane", sid: i + 2 } };
    junk.push(deep);
    for (const j of junk) {
      const t = j as LNode | null;
      expect(() => placeSeatSafe(t, 99, () => "worker", false)).not.toThrow();
      expect(() => placeSeatSafe(t, 99, () => "master", true)).not.toThrow();
      expect(() => placeSeat(t, 99, () => "cso", false)).not.toThrow();
      expect(() => anchorHeadSafe(t, () => "master")).not.toThrow();
      expect(() => anchorHead(t, () => "master")).not.toThrow();
      expect(() => looksManual(j)).not.toThrow();
      expect(() => isAnchored(j, () => "master")).not.toThrow();
      expect(() => liveSidsOf(j)).not.toThrow();
      expect(() => isValidTree(j)).not.toThrow();
      expect(() => holdPane(t, 1, -1)).not.toThrow();
      if (!isValidTree(j) && j !== null) {
        expect(anchorHeadSafe(t, () => "master")).toBe(t); // 손상 트리는 손대지 않는다
      }
    }
  });
  it("속성: 무작위 유효 트리 2000개 × 무작위 좌석 — 결과는 유효 · sid 집합 = 이전 ∪ {새 sid}", () => {
    let seed = 12345;
    const rnd = () => {
      seed = (seed * 1103515245 + 12345) & 0x7fffffff;
      return seed / 0x7fffffff;
    };
    const ROLES = ["master", "cso", "cso-2", "worker", "worker-2", "reviewer-codex", "reviewer-gemini", "cso-fresh-1", null];
    for (let n = 0; n < 2000; n++) {
      const role = new Map<number, string | null>();
      let next = 1;
      let hole = -1;
      const gen = (d: number): LNode => {
        if (d > 3 || rnd() < 0.35) {
          if (rnd() < 0.15) return { type: "pane", sid: hole--, role: ROLES[Math.floor(rnd() * 7)] as string };
          const s = next++;
          role.set(s, ROLES[Math.floor(rnd() * ROLES.length)]);
          return rnd() < 0.5 ? { type: "pane", sid: s, role: role.get(s) ?? undefined } : { type: "pane", sid: s };
        }
        const r = rnd() < 0.3 ? undefined : 0.1 + rnd() * 0.8;
        return { type: "split", dir: rnd() < 0.5 ? "row" : "col", ...(r === undefined ? {} : { ratio: r }), a: gen(d + 1), b: gen(d + 1) };
      };
      const t = gen(0);
      if (!isValidTree(t)) continue;
      const add = next;
      role.set(add, ROLES[Math.floor(rnd() * ROLES.length)]);
      const manual = rnd() < 0.5;
      const out = placeSeatSafe(t, add, mapRole(role), manual);
      expect(isValidTree(out)).toBe(true);
      const want = [...sids(t), add].sort((a, b) => a - b);
      expect(sids(out).sort((a, b) => a - b)).toEqual(want);
      const anch = anchorHeadSafe(out, mapRole(role)) as LNode;
      expect(sids(anch).sort((a, b) => a - b)).toEqual(want);
    }
  });
});
