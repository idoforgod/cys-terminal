// U2(재부팅 후 대표 자리 기억) — 역할 결속 순수부(seatlayout.ts)의 동작 핀.
//
// ★무엇을 고정하나: 칸에 역할을 적어 두면, 데몬이 모든 창을 **새 sid** 로 되살려도(재부팅·업데이트)
//   같은 역할의 새 창이 **원래 칸**에 들어간다. 세대가 바뀌면 옛 sid 는 살아 보여도 믿지 않는다(재사용).
//   주인이 아직 안 온 칸(구멍)은 기한 동안만 보이고, 기한이 지나도 기억은 남는다. 역할 하나에 칸 하나.
// 반박 보고서(phase1/U2-master-seat-persist.refute.md)의 major/minor 보정을 여기서 검체로 박제한다.
import { describe, it, expect } from "bun:test";
import {
  MASTER_FRAC,
  HEAD_COL_MASTER,
  MAX_HOLES_PER_TREE,
  ROLE_SLOT_GRACE_MS,
  adoptSeat,
  anchorHeadSafe,
  annotateRoles,
  daemonIdentOf,
  dropHole,
  fillHole,
  generationChanged,
  holdPane,
  holeSidsOf,
  isValidTree,
  legacyAppend,
  liveSidsOf,
  nextHoleSid,
  nodeShown,
  pickAdoptIndex,
  placeSeatSafe,
  reserveDeadline,
  restoreTree,
  roleSlotText,
  seatPriority,
  tidyHoles,
  type LNode,
  type RoleOf,
  type WsView,
} from "./seatlayout";

// ---------- 오라클 ----------
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
const mapRole = (m: Map<number, string | null>): RoleOf => (s) => (m.has(s) ? m.get(s) : undefined);
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
/** sid 를 뺀 **모양** — 결속이 칸·비율을 그대로 두는지 비교한다(역할 기준으로 칸을 이름 붙인다). */
function shape(t: LNode | null, roleOfSid: (s: number) => string | undefined): unknown {
  if (!t) return null;
  if (t.type === "pane") return `pane:${t.sid < 0 ? "hole" : "live"}:${roleOfSid(t.sid) ?? t.role ?? "-"}`;
  return { dir: t.dir, ratio: t.ratio ?? 0.5, a: shape(t.a, roleOfSid), b: shape(t.b, roleOfSid) };
}
function mkAlloc(trees: (LNode | null)[]): () => number {
  let h = nextHoleSid(trees);
  return () => h--;
}

/** 오너 실측 저장본(phase1 U2 §3-1) — 이번 판부터 칸에 역할이 적혀 있다. 세대 E1. */
const CEO_SAVED = (): LNode => ({
  type: "split", dir: "row", ratio: 0.25,
  a: { type: "split", dir: "col", ratio: 0.75, a: { type: "pane", sid: 18, role: "master" }, b: { type: "pane", sid: 17, role: "cso" } },
  b: { type: "split", dir: "row", ratio: 1 / 3, a: { type: "pane", sid: 14, role: "worker" },
    b: { type: "split", dir: "row", ratio: 0.5, a: { type: "pane", sid: 16, role: "worker-2" },
      b: { type: "split", dir: "col", ratio: 0.5, a: { type: "pane", sid: 19, role: "reviewer-gemini" }, b: { type: "pane", sid: 15, role: "reviewer-codex" } } } },
});
/** 재부팅 뒤 새 데몬(E2)이 되살린 좌석 — 스폰 순서는 무작위(실측: master 가 6개 중 5번째). */
const REBOOT_LIVE: [number, string][] = [[26, "worker"], [27, "reviewer-codex"], [28, "worker-2"], [29, "cso"], [30, "master"], [31, "reviewer-gemini"]];

/** 기동 복원 한 번(main.ts start() 의 순수 골격): restoreTree → 고아 입양(결속 우선) → 구멍 위생. */
function simulateStart(
  wss: WsView[],
  live: [number, string | null][],
  opts: { genChanged: boolean; roleConflictIsDead: boolean; order?: "priority" | "given" },
): WsView[] {
  const liveRole = new Map<number, string | null>(live);
  const roleOf = mapRole(liveRole);
  const alloc = mkAlloc(wss.map((w) => w.tree));
  const out = wss.map((w) => ({
    ...w,
    tree: restoreTree(w.tree, {
      genChanged: opts.genChanged,
      isLive: (s) => liveRole.has(s),
      liveRole: roleOf,
      roleConflictIsDead: opts.roleConflictIsDead,
      allocHole: alloc,
    }),
  }));
  const orphans = [...live];
  if ((opts.order ?? "priority") === "priority") orphans.sort((a, b) => seatPriority(a[1]) - seatPriority(b[1]) || a[0] - b[0]);
  for (const [sid, role] of orphans) {
    if (out.some((w) => sids(w.tree).includes(sid))) continue;
    const plan = adoptSeat(out, undefined, sid, role, roleOf);
    if (!plan) continue;
    out[plan.idx] = { ...out[plan.idx], tree: plan.tree };
  }
  const tidy = tidyHoles(out.map((w) => w.tree), roleOf);
  return out.map((w, i) => ({ ...w, tree: tidy[i] }));
}

// ────────────────────────────────────────────────────────────────────────────
describe("★재부팅 실측 재현 — 새 sid 로 되살아난 6석이 원래 칸으로 돌아간다", () => {
  const ROLE_BY_NEW = new Map<number, string>(REBOOT_LIVE);
  for (const manual of [true, false]) {
    it(`${manual ? "수동" : "비수동"} 탭: 모양 동일 · 좌상단 = 새 master(30)${manual ? " · 비율도 저장본 그대로(0.25)" : " · 대표 폭은 단일 상수 1/3"}`, () => {
      for (const order of ["priority", "given"] as const) {
        const [ws] = simulateStart([{ tree: CEO_SAVED(), layoutManual: manual }], REBOOT_LIVE, { genChanged: true, roleConflictIsDead: false, order });
        const t = ws.tree as LNode;
        expect(holeSidsOf(t)).toEqual([]); // 전부 결속 — 구멍이 남지 않는다
        expect(sids(t).sort()).toEqual([26, 27, 28, 29, 30, 31]);
        // 좌상단 = master
        expect(t.type === "split" && t.a.type === "split" && t.a.a.type === "pane" && t.a.a.sid === 30).toBe(true);
        if (manual) {
          expect(shape(t, (s) => ROLE_BY_NEW.get(s))).toEqual(shape(CEO_SAVED(), () => undefined));
          close(W(t, 30), 0.25);
        } else {
          close(W(t, 30), MASTER_FRAC); // U2↔U3 충돌은 1/3 한 상수로 해소
          close(H(t, 30), HEAD_COL_MASTER);
          for (const s of [26, 28, 31]) close(W(t, s), 2 / 9);
        }
      }
    });
  }
  it("대조(수정 전 규칙): 역할을 무시하고 sid 순으로 오른쪽에 붙이면 master 는 6개 중 5번째·폭 1/4 · worker 1/32 — 이 결함이 재현된다(U2 보고 §2 경로 A)", () => {
    let t: LNode | null = null;
    for (const [sid] of [...REBOOT_LIVE].sort((a, b) => a[0] - b[0])) t = legacyAppend(t, sid);
    const order = sids(t);
    expect(order.indexOf(30)).toBe(4);
    close(W(t as LNode, 30), 1 / 4);
    close(W(t as LNode, 26), 1 / 32);
    close(W(t as LNode, 31), 1 / 2);
  });
});

// ────────────────────────────────────────────────────────────────────────────
describe("기억이 없는 경우 · 부분 복원 · 1:1", () => {
  it("옛 저장본(역할 없음) → 구멍 0 · U3 배치로 master 왼쪽 1/3 · cso 는 대표 칸(업그레이드 첫 기동 = 폴백)", () => {
    const legacy: LNode = JSON.parse(JSON.stringify(CEO_SAVED()).replace(/,"role":"[^"]+"/g, ""));
    const [ws] = simulateStart([{ tree: legacy }], REBOOT_LIVE, { genChanged: false, roleConflictIsDead: true });
    const t = ws.tree as LNode;
    expect(holeSidsOf(t)).toEqual([]);
    close(W(t, 30), MASTER_FRAC);
    expect(t.type === "split" && sids(t.a).includes(29)).toBe(true);
  });
  it("복원 진행 중: worker 만 살아 있으면 worker 결속 · master 칸은 구멍으로 남고 · 다음 틱에 오면 **같은 칸**에 결속", () => {
    const [ws0] = simulateStart([{ tree: CEO_SAVED(), layoutManual: true }], [[40, "worker"]], { genChanged: true, roleConflictIsDead: false });
    const t0 = ws0.tree as LNode;
    expect(liveSidsOf(t0)).toEqual([40]);
    expect(holeSidsOf(t0).length).toBe(5);
    const masterHole = (t0 as any).a.a;
    expect(masterHole.sid < 0 && masterHole.role === "master").toBe(true);
    const roleOf = mapRole(new Map<number, string | null>([[40, "worker"], [41, "master"]]));
    const plan = adoptSeat([ws0], undefined, 41, "master", roleOf) as NonNullable<ReturnType<typeof adoptSeat>>;
    expect(plan.boundHole).toBe(masterHole.sid);
    expect((plan.tree as any).a.a).toEqual({ type: "pane", sid: 41, role: "master" });
    close(W(plan.tree, 41), 0.25); // 수동 탭: 저장 비율 그대로
  });
  it("1:1 — master 구멍 둘 + 산 master 하나 → DFS 첫 칸에만 결속, 나머지 구멍은 위생에서 제거", () => {
    const t: LNode = { type: "split", dir: "row", ratio: 0.5, a: { type: "pane", sid: -1, role: "master" }, b: { type: "pane", sid: -2, role: "master" } };
    const roleOf = mapRole(new Map([[5, "master"]]));
    const plan = adoptSeat([{ tree: t }], undefined, 5, "master", roleOf) as NonNullable<ReturnType<typeof adoptSeat>>;
    expect(plan.boundHole).toBe(-1);
    const [tidy] = tidyHoles([plan.tree], roleOf);
    expect(sids(tidy)).toEqual([5]);
  });
  it("역할 불일치 — master 구멍에 cso 를 넣지 않는다(cso 는 U3 규칙으로 배치 · 구멍은 남음)", () => {
    const t: LNode = { type: "split", dir: "row", ratio: 1 / 3, a: { type: "pane", sid: -1, role: "master" }, b: { type: "pane", sid: 3, role: "worker" } };
    const roleOf = mapRole(new Map<number, string | null>([[3, "worker"], [7, "cso"]]));
    const plan = adoptSeat([{ tree: t }], undefined, 7, "cso", roleOf) as NonNullable<ReturnType<typeof adoptSeat>>;
    expect(plan.boundHole).toBeNull();
    expect(holeSidsOf(plan.tree)).toEqual([-1]);
    expect(plan.tree.type === "split" && sids(plan.tree.a).includes(7)).toBe(true); // master 구멍 = 대표 칸 → cso 는 그 아래
  });
});

// ────────────────────────────────────────────────────────────────────────────
describe("세대(daemonEpoch) — 옛 sid 를 믿지 않는다(반박 U2 major ⓒ · §3-1 sid 재사용)", () => {
  const SAVED = (): LNode => ({ type: "split", dir: "row", ratio: 1 / 3, a: { type: "pane", sid: 5, role: "master" }, b: { type: "pane", sid: 6, role: "worker" } });
  it("세대 변경 + 같은 번호 재사용: 새 데몬의 sid 5 가 worker 여도 master 칸에 남지 않는다 → master 칸엔 새 master(9)", () => {
    const [ws] = simulateStart([{ tree: SAVED(), layoutManual: true }], [[5, "worker"], [9, "master"]], { genChanged: true, roleConflictIsDead: false });
    const t = ws.tree as any;
    expect(t.a).toEqual({ type: "pane", sid: 9, role: "master" });
    expect(t.b).toEqual({ type: "pane", sid: 5, role: "worker" });
  });
  it("같은 세대: 산 칸은 절대 빼앗지 않는다 — 역할이 달라도 칸은 그대로 두고 기억만 갱신(annotate)", () => {
    const t = SAVED();
    const out = restoreTree(t, {
      genChanged: false,
      isLive: () => true,
      liveRole: () => "worker-9",
      roleConflictIsDead: false,
      allocHole: () => -99,
    }) as LNode;
    expect(out).toBe(t);
    expect(annotateRoles(out, () => "worker-9")).toBe(true);
    expect((out as any).a.role).toBe("worker-9");
  });
  it("세대 모름(구저장본·식별 실패): 기억한 역할과 지금 역할이 다르면 다른 surface 로 본다(재사용 방어)", () => {
    const [ws] = simulateStart([{ tree: SAVED(), layoutManual: true }], [[5, null], [6, "worker"], [9, "master"]], { genChanged: false, roleConflictIsDead: true });
    const t = ws.tree as LNode;
    expect((t as any).a).toEqual({ type: "pane", sid: 9, role: "master" });
    expect(sids(t).sort()).toEqual([5, 6, 9]); // 역할 없는 5 는 사라지지 않고 오른쪽에 붙는다
  });
  it("generationChanged / daemonIdentOf", () => {
    expect(generationChanged(undefined, "1-2")).toBeNull();
    expect(generationChanged("1-2", null)).toBeNull();
    expect(generationChanged(5, "1-2")).toBeNull();
    expect(generationChanged("1-2", "1-2")).toBe(false);
    expect(generationChanged("1-2", "3-4")).toBe(true);
    expect(daemonIdentOf({ started_at: 1790147949.358786, daemon_pid: 72876 })).toEqual({ epoch: "1790147949.358786-72876", startedAtMs: 1790147949.358786 * 1000 });
    expect(daemonIdentOf({ started_at: 1790147949.5 })).toEqual({ epoch: null, startedAtMs: 1790147949500 });
    for (const j of [null, undefined, 3, "x", [], {}, { started_at: "1", daemon_pid: 1 }, { started_at: -1, daemon_pid: 2 }])
      expect(daemonIdentOf(j)).toEqual({ epoch: null, startedAtMs: null });
  });
});

// ────────────────────────────────────────────────────────────────────────────
describe("보류 기한 — 데몬이 젊을 때만 · 기한 뒤엔 화면에서만 접는다(반박 U2 major ⓑⓒ · minor)", () => {
  it("reserveDeadline: 젊은 데몬 = 기동+grace · 늙은 데몬 = null(보류 없이 폴백) · 모름 = null · 미래 시각은 now+grace 상한", () => {
    const now = 1_000_000_000;
    expect(reserveDeadline(now - 10_000, now, ROLE_SLOT_GRACE_MS)).toBe(now - 10_000 + ROLE_SLOT_GRACE_MS);
    expect(reserveDeadline(now - ROLE_SLOT_GRACE_MS - 1, now, ROLE_SLOT_GRACE_MS)).toBeNull();
    expect(reserveDeadline(null, now, ROLE_SLOT_GRACE_MS)).toBeNull();
    expect(reserveDeadline(NaN, now, ROLE_SLOT_GRACE_MS)).toBeNull();
    expect(reserveDeadline(now + 3_600_000, now, ROLE_SLOT_GRACE_MS)).toBe(now + ROLE_SLOT_GRACE_MS);
    expect(ROLE_SLOT_GRACE_MS).toBe(240_000);
  });
  it("기한이 지난 구멍은 안 보이지만(nodeShown=false) 트리에 남아 — 늦게 온 역할도 그 칸에 결속", () => {
    const t: LNode = { type: "split", dir: "row", ratio: 1 / 3, a: { type: "pane", sid: -3, role: "master" }, b: { type: "pane", sid: 4, role: "worker" } };
    expect(nodeShown(t.a, () => false)).toBe(false);
    expect(nodeShown(t, () => false)).toBe(true);
    expect(nodeShown(t.a, (s) => s === -3)).toBe(true);
    close(W(t, 4), 1); // 접힌 구멍의 몫은 형제가 가져간다
    const plan = adoptSeat([{ tree: t }], undefined, 9, "master", mapRole(new Map<number, string | null>([[4, "worker"], [9, "master"]])));
    expect(plan?.boundHole).toBe(-3);
    close(W(plan!.tree, 9), MASTER_FRAC);
  });
  it("구멍만 남은 트리는 '보이는 칸 0' — 렌더러는 idle 패널(셸 손잡이)을 그린다(④ 백지 금지)", () => {
    const t: LNode = { type: "split", dir: "col", ratio: 0.75, a: { type: "pane", sid: -1, role: "master" }, b: { type: "pane", sid: -2, role: "cso" } };
    expect(nodeShown(t, () => false)).toBe(false);
    expect(liveSidsOf(t)).toEqual([]);
    // 셸을 붙여도 기억은 유지된다(placeSeatSafe — 역할 없는 셸은 대표 칸 밖으로)
    const u = placeSeatSafe(t, 50, (s) => (s === 50 ? null : undefined), false);
    expect(holeSidsOf(u).sort((a, b) => a - b)).toEqual([-2, -1]);
    expect(nodeShown(u, () => false)).toBe(true);
    close(W(u, 50), 1);
  });
});

// ────────────────────────────────────────────────────────────────────────────
describe("구멍 만들기·채우기·비우기 · 역할 기억(annotate)", () => {
  it("holdPane: 역할을 기억한 칸 → 같은 자리 구멍 · 셸·일회용 → null(호출측이 종전대로 뗀다)", () => {
    const t: LNode = { type: "split", dir: "row", a: { type: "pane", sid: 1, role: "master" }, b: { type: "split", dir: "col", a: { type: "pane", sid: 2 }, b: { type: "pane", sid: 3, role: "worker-fresh-1" } } };
    const h = holdPane(t, 1, -7) as any;
    expect(h.a).toEqual({ type: "pane", sid: -7, role: "master" });
    expect(h.b).toBe((t as any).b);
    expect(holdPane(t, 2, -7)).toBeNull();
    expect(holdPane(t, 3, -7)).toBeNull();
    expect(holdPane(t, 99, -7)).toBeNull();
    expect(holdPane(t, 1, 5)).toBeNull(); // 구멍 sid 는 음수만
    expect(holdPane(h, 2, -7)).toBeNull(); // 이미 있는 구멍 sid 재사용 금지
  });
  it("fillHole([새 셸 열기]) · dropHole([칸 비우기])", () => {
    const t: LNode = { type: "split", dir: "row", ratio: 0.3, a: { type: "pane", sid: -1, role: "master" }, b: { type: "pane", sid: 4 } };
    expect(fillHole(t, -1, 9)).toEqual({ type: "split", dir: "row", ratio: 0.3, a: { type: "pane", sid: 9 }, b: { type: "pane", sid: 4 } });
    expect(fillHole(t, -5, 9)).toBeNull();
    expect(fillHole(t, -1, 4)).toBeNull(); // 이미 있는 sid 로 채우면 중복
    expect(dropHole(t, -1)).toEqual({ type: "pane", sid: 4 });
    expect(dropHole(t, -5)).toEqual(t);
  });
  it("annotateRoles: 바뀐 것만 쓰고(변경 여부 반환) · 모름(undefined)·구멍은 건드리지 않고 · 일회용은 기억하지 않는다", () => {
    const t: LNode = { type: "split", dir: "row", a: { type: "pane", sid: 1 }, b: { type: "split", dir: "col", a: { type: "pane", sid: 2, role: "worker" }, b: { type: "pane", sid: -1, role: "cso" } } };
    const live = new Map<number, string | null>([[1, "master"], [2, "worker"]]);
    expect(annotateRoles(t, mapRole(live))).toBe(true);
    expect(annotateRoles(t, mapRole(live))).toBe(false); // 두 번째는 쓰기 0
    expect((t as any).a.role).toBe("master");
    live.set(2, null); // 역할 해제(plain 셸)
    expect(annotateRoles(t, mapRole(live))).toBe(true);
    expect((t as any).b.a.role).toBeUndefined();
    expect(annotateRoles(t, () => undefined)).toBe(false); // 종료·모름은 기억 보존
    expect((t as any).a.role).toBe("master");
    expect(annotateRoles(t, (s) => (s === 1 ? "master-fresh-17" : undefined))).toBe(true);
    expect((t as any).a.role).toBeUndefined();
    expect((t as any).b.b).toEqual({ type: "pane", sid: -1, role: "cso" });
  });
  it("nextHoleSid: 모든 탭의 구멍보다 작은 음수", () => {
    expect(nextHoleSid([null, { type: "pane", sid: 3 }])).toBe(-1);
    expect(nextHoleSid([{ type: "pane", sid: -4, role: "x" }, { type: "split", dir: "row", a: { type: "pane", sid: -9, role: "y" }, b: { type: "pane", sid: 2 } }])).toBe(-10);
  });
  it("roleSlotText", () => {
    expect(roleSlotText("master")).toBe("대표(master) 자리 — 복원을 기다리는 중입니다");
    expect(roleSlotText("worker-2")).toBe("worker-2 자리 — 복원을 기다리는 중입니다");
    expect(roleSlotText(undefined)).toBe("역할 자리 — 복원을 기다리는 중입니다");
  });
});

// ────────────────────────────────────────────────────────────────────────────
describe("입양 대상 탭 · 소켓 격리(반박 U2 §2-4·§3-3 · U3 M1)", () => {
  const m = (sid: number, role?: string): LNode => (role ? { type: "pane", sid, role } : { type: "pane", sid });
  it("master 가 있는 탭 우선(두 번째 탭이어도) · master 구멍도 master 로 센다 · 없으면 첫 탭", () => {
    const roleOf = mapRole(new Map<number, string | null>([[1, null], [2, "master"]]));
    const wss: WsView[] = [{ tree: m(1) }, { tree: m(2, "master") }];
    expect(pickAdoptIndex(wss, undefined, roleOf)).toBe(1);
    expect(pickAdoptIndex([{ tree: m(1) }, { tree: m(-4, "master") }], undefined, roleOf)).toBe(1);
    expect(pickAdoptIndex([{ tree: m(1) }, { tree: null }], undefined, roleOf)).toBe(0);
  });
  it("pending·deleting 탭은 제외 · 소켓은 정확 일치(Windows 파이프 표기 차이를 합치지 않는다 — 입양과 같은 판정)", () => {
    const P = "\\\\.\\pipe\\cys-dept-A";
    const wss: WsView[] = [
      { tree: m(2, "master"), pending: true },
      { tree: m(3, "master"), deleting: true },
      { tree: m(4, "master"), socket: P.toLowerCase() },
      { tree: m(5), socket: P },
    ];
    expect(pickAdoptIndex(wss, undefined, () => undefined)).toBe(-1);
    expect(pickAdoptIndex(wss, P, () => undefined)).toBe(3);
  });
  it("★교차 소켓: 부서 탭의 master 구멍에 본부 master 를 넣지 않는다", () => {
    const wss: WsView[] = [
      { socket: "/d1.sock", tree: m(-1, "master") },
      { tree: { type: "split", dir: "row", a: m(-2, "master"), b: m(7, "worker") } },
    ];
    const plan = adoptSeat(wss, undefined, 30, "master", mapRole(new Map<number, string | null>([[7, "worker"], [30, "master"]])));
    expect(plan?.idx).toBe(1);
    expect(plan?.boundHole).toBe(-2);
    const dept = adoptSeat(wss, "/d1.sock", 11, "master", () => "master");
    expect(dept?.idx).toBe(0);
    expect(dept?.boundHole).toBe(-1);
    expect(adoptSeat(wss, "/nope.sock", 12, "master", () => "master")).toBeNull();
  });
});

// ────────────────────────────────────────────────────────────────────────────
describe("구멍 위생(tidyHoles)", () => {
  it("역할이 이미 산 칸으로 있으면 구멍 제거 · 중복 역할은 첫 칸만 · 역할 없는 구멍 제거 · 트리당 상한", () => {
    const roleOf = mapRole(new Map<number, string | null>([[1, "master"]]));
    const t1: LNode = { type: "split", dir: "row", a: { type: "pane", sid: 1, role: "master" }, b: { type: "split", dir: "row", a: { type: "pane", sid: -1, role: "master" }, b: { type: "pane", sid: -2, role: "worker" } } };
    const t2: LNode = { type: "split", dir: "row", a: { type: "pane", sid: -3, role: "worker" }, b: { type: "pane", sid: -4 } };
    const [a, b] = tidyHoles([t1, t2], roleOf);
    expect(sids(a)).toEqual([1, -2]);
    expect(b).toBeNull(); // 중복 worker 구멍·역할 없는 구멍만 있던 탭 → 비었다
    let big: LNode = { type: "pane", sid: 1, role: "master" };
    for (let i = 1; i <= MAX_HOLES_PER_TREE + 5; i++) big = { type: "split", dir: "row", a: big, b: { type: "pane", sid: -i, role: `worker-${i}` } };
    const [c] = tidyHoles([big], roleOf);
    expect(holeSidsOf(c).length).toBe(MAX_HOLES_PER_TREE);
    const same: LNode = { type: "pane", sid: 1, role: "master" };
    expect(tidyHoles([same], roleOf)[0]).toBe(same); // 바뀐 게 없으면 같은 객체
  });
});

// ────────────────────────────────────────────────────────────────────────────
describe("속성 — 무작위 저장본 × 무작위 생존 집합 × 세대: 모든 산 좌석이 정확히 한 번", () => {
  it("1500회: 결과 트리는 유효 · 산 sid 는 전부 정확히 한 번 · 죽은 양수 sid 0 · 구멍 역할 중복 0", () => {
    let seed = 777;
    const rnd = () => {
      seed = (seed * 1103515245 + 12345) & 0x7fffffff;
      return seed / 0x7fffffff;
    };
    const ROLES = ["master", "cso", "cso-2", "worker", "worker-2", "worker-3", "reviewer-codex", "reviewer-gemini", null];
    for (let n = 0; n < 1500; n++) {
      let next = 1;
      let hole = -1;
      const gen = (d: number): LNode => {
        if (d > 3 || rnd() < 0.35) {
          if (rnd() < 0.1) return { type: "pane", sid: hole--, role: ROLES[Math.floor(rnd() * 8)] as string };
          const r = ROLES[Math.floor(rnd() * ROLES.length)];
          return r ? { type: "pane", sid: next++, role: r } : { type: "pane", sid: next++ };
        }
        return { type: "split", dir: rnd() < 0.5 ? "row" : "col", ratio: 0.1 + rnd() * 0.8, a: gen(d + 1), b: gen(d + 1) };
      };
      const trees = [gen(0), rnd() < 0.5 ? gen(1) : null];
      const live: [number, string | null][] = [];
      const used = new Set<string>();
      for (let s = 1; s < next + 6; s++) {
        if (rnd() < 0.5) continue;
        const r = ROLES[Math.floor(rnd() * ROLES.length)];
        if (r && used.has(r)) continue; // 데몬은 역할당 한 좌석
        if (r) used.add(r);
        live.push([s, r]);
      }
      const gen2 = rnd() < 0.5;
      const res = simulateStart(trees.map((t) => ({ tree: t, layoutManual: rnd() < 0.5 })), live, { genChanged: gen2, roleConflictIsDead: !gen2 });
      const all = res.flatMap((w) => sids(w.tree));
      for (const w of res) if (w.tree) expect(isValidTree(w.tree)).toBe(true);
      const liveSet = new Set(live.map(([s]) => s));
      for (const [s] of live) expect(all.filter((x) => x === s).length).toBe(1);
      expect(all.filter((x) => x > 0 && !liveSet.has(x))).toEqual([]);
      const holeRoles = res.flatMap((w) => {
        const out: string[] = [];
        const walk = (t: LNode | null) => {
          if (!t) return;
          if (t.type === "pane") {
            if (t.sid < 0) out.push(t.role as string);
          } else {
            walk(t.a);
            walk(t.b);
          }
        };
        walk(w.tree);
        return out;
      });
      expect(new Set(holeRoles).size).toBe(holeRoles.length);
      const liveRoles = new Set(live.map(([, r]) => r).filter((r): r is string => !!r));
      for (const r of holeRoles) expect(liveRoles.has(r)).toBe(false); // 산 역할의 구멍은 남지 않는다(1:1)
    }
  });
});

// ────────────────────────────────────────────────────────────────────────────
describe("손상 노드 — U2 연산도 던지지 않는다", () => {
  it("임의 JSON 에서 restoreTree·annotateRoles·holdPane·fillHole·dropHole·tidyHoles·adoptSeat·pickAdoptIndex·nodeShown·nextHoleSid", () => {
    const junk: unknown[] = [
      null, undefined, 0, "x", [], {}, { type: "pane" }, { type: "pane", sid: 0 }, { type: "pane", sid: 2.5 },
      { type: "split", dir: "row", a: { type: "pane", sid: 1 } }, { type: "split", dir: "row", ratio: -1, a: { type: "pane", sid: 1 }, b: { type: "pane", sid: 2 } },
      { type: "split", dir: "row", a: { type: "pane", sid: 1 }, b: { type: "pane", sid: 1 } },
      { type: "pane", sid: -1, role: 3 },
    ];
    const boom: RoleOf = () => {
      throw new Error("boom");
    };
    for (const j of junk) {
      const t = j as LNode | null;
      const opts = { genChanged: true, isLive: () => true, liveRole: () => "master", roleConflictIsDead: true, allocHole: () => -50 };
      expect(() => restoreTree(t, opts)).not.toThrow();
      if (t !== null && !isValidTree(t)) expect(restoreTree(t, opts)).toBe(t); // 손상 트리는 종전 경로가 처리
      expect(() => annotateRoles(t, () => "master")).not.toThrow();
      expect(() => annotateRoles(t, boom)).not.toThrow();
      expect(() => holdPane(t, 1, -3)).not.toThrow();
      expect(() => fillHole(t, -1, 3)).not.toThrow();
      expect(() => dropHole(t, -1)).not.toThrow();
      expect(() => tidyHoles([t, t], boom)).not.toThrow();
      expect(() => adoptSeat([{ tree: t }], undefined, 9, "master", boom)).not.toThrow();
      expect(adoptSeat([{ tree: t }], undefined, 9, "master", boom)).not.toBeNull();
      expect(() => pickAdoptIndex([{ tree: t }], undefined, boom)).not.toThrow();
      expect(() => nodeShown(j, () => true)).not.toThrow();
      expect(() => nodeShown(j, () => {
        throw new Error("x");
      })).not.toThrow();
      expect(() => nextHoleSid([j, j])).not.toThrow();
    }
    // 결속 계획이 실패하면 종전 오른쪽 부착 — 그래도 좌석은 화면에 붙는다(③)
    const p = adoptSeat([{ tree: { type: "pane", sid: 1, role: "worker" } }], undefined, 9, "worker-2", boom);
    expect(p && sids(p.tree).sort()).toEqual([1, 9]);
    // anchorHeadSafe 가 결속 뒤 재정렬에 실패해도 결속 자체는 유지
    expect(anchorHeadSafe({ type: "pane", sid: 9, role: "master" }, boom)).toEqual({ type: "pane", sid: 9, role: "master" });
  });
});
