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
  evenComb,
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
  placeSeatFallbackCount,
  placeSeatSafe,
  roleLayout,
  sanitizeRole,
  seatPriority,
  type LNode,
  type RoleOf,
} from "./seatlayout";
// ★S1 수정1 의 새 공개 이름(isSideRole · moveCsoToSide)은 이름 가져오기가 아니라 모듈 객체로 읽는다 — 그 이름이
//   없는 판(v0.15.1)에서 파일 전체가 적재 실패로 죽지 않고, 해당 검체만 제 이유로 붉어지게 하려는 것이다.
import * as SL from "./seatlayout";
const sl = SL as unknown as Record<string, (...a: any[]) => any>;

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

// ---------- ★S1 수정1 오라클: 칸의 사각형(왼쪽 위 원점 · 화면 전체 = 1×1 · 안 보이는 구멍 쪽은 접힌다) ----------
type Rect = { x: number; y: number; w: number; h: number };
function rect(t: LNode, sid: number, hs: (s: number) => boolean = () => false, r: Rect = { x: 0, y: 0, w: 1, h: 1 }): Rect | null {
  if (t.type === "pane") return t.sid === sid ? r : null;
  const sa = shown(t.a, hs);
  const sb = shown(t.b, hs);
  if (!sa) return rect(t.b, sid, hs, r);
  if (!sb) return rect(t.a, sid, hs, r);
  const q = t.ratio ?? 0.5;
  const ra: Rect = t.dir === "row" ? { ...r, w: r.w * q } : { ...r, h: r.h * q };
  const rb: Rect = t.dir === "row" ? { x: r.x + r.w * q, y: r.y, w: r.w * (1 - q), h: r.h } : { x: r.x, y: r.y + r.h * q, w: r.w, h: r.h * (1 - q) };
  return rect(t.a, sid, hs, ra) ?? rect(t.b, sid, hs, rb);
}
const near = (a: number, b: number) => Math.abs(a - b) < 1e-9;
/**
 * 새 표준 모양인가: master 는 왼쪽 1/3 을 위아래 전부 **혼자** 쓰고, side(위→아래 순서 · 맨 아래가 CSO)는 **오른쪽 끝
 * 한 열**을 같은 높이로 나누며, mids(가운데 좌석)는 각자 한 컬럼 전체 높이이고, 오른쪽 2/3 의 컬럼 폭은 전부 같다.
 * 맞으면 "" · 어긋나면 어긋난 첫 칸과 그 사각형.
 */
function stdShape(t: LNode, master: number, side: number[], mids: number[], hs: (s: number) => boolean = () => false): string {
  const m = rect(t, master, hs);
  if (!m || !near(m.x, 0) || !near(m.y, 0) || !near(m.w, MASTER_FRAC) || !near(m.h, 1)) return "master " + JSON.stringify(m);
  const cols = mids.length + (side.length ? 1 : 0);
  const cw = cols ? (1 - MASTER_FRAC) / cols : 0;
  for (const s of mids) {
    const r = rect(t, s, hs);
    if (!r || !near(r.w, cw) || !near(r.h, 1) || !near(r.y, 0)) return "mid " + s + " " + JSON.stringify(r);
  }
  for (let i = 0; i < side.length; i++) {
    const r = rect(t, side[i], hs);
    if (!r || !near(r.x, 1 - cw) || !near(r.w, cw) || !near(r.h, 1 / side.length) || !near(r.y, i / side.length)) {
      return "side " + side[i] + " " + JSON.stringify(r);
    }
  }
  return "";
}
const P = (sid: number, role: string): LNode => ({ type: "pane", sid, role });
/** v0.15.1 이 만들던 모양(구버전 저장본): 왼쪽 1/3 = master 위(3/4)·CSO 아래 / 가운데 worker·그 밖 / 오른쪽 agy 위·codex 아래(1:1). */
function oldShape(seats: [number, string][]): LNode {
  const by = (r: string) => seats.filter((s) => s[1] === r).map((s) => P(s[0], s[1]));
  const head: LNode = { type: "split", dir: "col", ratio: HEAD_COL_MASTER, a: by("master")[0], b: by("cso")[0] };
  const stacked = ["reviewer-gemini", "reviewer-codex"];
  const rest = seats.filter((s) => s[1] !== "master" && s[1] !== "cso" && stacked.indexOf(s[1]) < 0).map((s) => P(s[0], s[1]));
  const agy = by("reviewer-gemini")[0];
  const codex = by("reviewer-codex")[0];
  if (agy && codex) rest.push({ type: "split", dir: "col", ratio: 1 / 2, a: agy, b: codex });
  else if (agy || codex) rest.push(agy ?? codex);
  return { type: "split", dir: "row", ratio: MASTER_FRAC, a: head, b: evenComb(rest, "row") };
}

// ────────────────────────────────────────────────────────────────────────────
describe("역할 술어 — 대표 칸 소속은 하나의 술어다(반박 D5·D6)", () => {
  it("master 는 정확 일치 — master-fresh-* 는 대표가 아니다", () => {
    expect(isMasterRole("master")).toBe(true);
    expect(isMasterRole("master-fresh-1790000000")).toBe(false);
    expect(isMasterRole("Master")).toBe(false);
  });
  it("★S1 수정1: 대표 칸 소속은 master 뿐 — cso · cso-<숫자> 와 리뷰어는 리뷰어 열 소속, 일회용 좌석은 어느 쪽도 아니다", () => {
    for (const r of ["cso", "cso-2", "cso-10"]) expect(isCsoRole(r)).toBe(true);
    for (const r of ["cso-fresh-1790000000", "cso2", "csox", "cso-", "xcso"]) expect(isCsoRole(r)).toBe(false);
    expect(isHeadRole("master")).toBe(true);
    for (const r of ["cso", "cso-2", "worker", "reviewer-gemini"]) expect(isHeadRole(r)).toBe(false);
    for (const r of ["cso", "cso-2", "reviewer-gemini", "reviewer-codex", "reviewer-grok", "reviewer-claude-2"]) expect(sl.isSideRole(r)).toBe(true);
    for (const r of ["master", "worker", "worker-2", null, undefined, "cso-fresh-1790000000", "reviewer-codex-fresh-1790000000"]) {
      expect(sl.isSideRole(r)).toBe(false);
    }
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
    it(`worker ${w}명 + 리뷰어 2 + CSO → master 혼자 1/3 · 나머지 컬럼 (2/3)/${w + 1} · 오른쪽 끝 열 = agy·codex·CSO 1/3 씩`, () => {
      const role = new Map<number, string | null>([[1, "master"], [2, "cso"], [90, "reviewer-gemini"], [91, "reviewer-codex"]]);
      const mids: number[] = [];
      for (let k = 0; k < w; k++) {
        role.set(10 + k, k ? `worker-${k + 1}` : "worker");
        mids.push(10 + k);
      }
      const t = roleLayout([...role.keys()], mapRole(role)) as LNode;
      expect(stdShape(t, 1, [90, 91, 2], mids)).toBe("");
    });
  }
  it("master 만 → 전체 폭 · master+cso-2 → master 왼쪽 1/3(전체 높이) | cso-2 오른쪽 열(★S1 수정1: master 아래가 아니다)", () => {
    close(W(roleLayout([1], () => "master") as LNode, 1), 1);
    const r = new Map<number, string | null>([[1, "master"], [2, "cso-2"]]);
    const t = roleLayout([1, 2], mapRole(r)) as LNode;
    expect(stdShape(t, 1, [2], [])).toBe("");
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
  it("cso·cso-2 는 **자동 배치와 같은 술어로** 리뷰어 열에 모인다(D6 · ★S1 수정1)", () => {
    const r = new Map<number, string | null>([[1, "master"], [2, "cso"], [3, "cso-2"], [4, "worker"]]);
    const t = roleLayout([1, 2, 3, 4], mapRole(r)) as LNode;
    expect(stdShape(t, 1, [2, 3], [4])).toBe("");
    expect(stdShape(build([[1, "master"], [2, "cso"], [3, "cso-2"], [4, "worker"]]).t, 1, [2, 3], [4])).toBe("");
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
  it(`${all.length}가지 최종 모양(★S1 수정1): master 혼자 왼쪽 1/3 · worker 2컬럼 · 오른쪽 끝 열 = agy·codex·CSO 1/3 씩`, () => {
    let bad = "";
    for (const order of all) {
      const { t } = build(order.map((r, i) => [i + 1, r] as [number, string]));
      const at = (r: string) => order.indexOf(r) + 1;
      const got = stdShape(t, at("master"), [at("reviewer-gemini"), at("reviewer-codex"), at("cso")], [at("worker"), at("worker-2")]);
      if (got && !bad) bad = order.join(",") + " → " + got;
    }
    expect(bad).toBe(""); // 도착 순서가 어떻든 같은 모양 — 흩어진 좌석은 매 배치 뒤 제자리로 모인다(반박 D1)
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
  it("사용자가 master 를 오른쪽으로 옮긴 수동 탭 — 첫 좌석만 균등 몫(×3/4)으로 줄고 그 뒤로는 불변(절반 연쇄 없음)", () => {
    // 루트 a 쪽에 master 가 없으면(사용자가 옮김) 첫 좌석은 루트에 균등 몫으로 붙는다. 그 순간부터 master 는
    // 루트 a 쪽에 있으므로 이후 좌석은 root.b 로만 간다 — 설계서 §3 "새 좌석은 항상 root.b로(대표 폭 불변)".
    const role = new Map<number, string | null>([[1, "master"], [2, "worker"], [3, "worker-2"]]);
    let t: LNode = { type: "split", dir: "row", ratio: 0.5, a: { type: "pane", sid: 2 }, b: { type: "split", dir: "row", ratio: 0.5, a: { type: "pane", sid: 3 }, b: { type: "pane", sid: 1 } } };
    const start = W(t, 1) as number;
    role.set(4, "worker-4");
    t = placeSeat(t, 4, mapRole(role), true);
    close(W(t, 1), (start * 3) / 4); // 보이는 컬럼 3 → 새 컬럼 1/4
    for (const s of [5, 6, 7]) {
      role.set(s, "worker-" + s);
      t = placeSeat(t, s, mapRole(role), true);
      close(W(t, 1), (start * 3) / 4);
    }
    close(W(t, 2), 0.5 * 0.75); // 사용자가 둔 다른 컬럼 비율도 그대로
  });
});

// ────────────────────────────────────────────────────────────────────────────
describe("재기동 — master·cso 가 새 sid 로 돌아온다(반박 D1·D2)", () => {
  it("비수동: 옛 master 칸이 접힌 뒤 새 master → 왼쪽 1/3 을 혼자 · cso 는 오른쪽 끝 열(★S1 수정1)", () => {
    const { t: t0, role } = build([[1, "master"], [2, "cso"], [3, "worker"], [4, "worker-2"]]);
    let t = rm(t0, 1) as LNode;
    role.delete(1);
    role.set(9, "master");
    t = placeSeat(t, 9, mapRole(role), false);
    expect(stdShape(t, 9, [2], [3, 4])).toBe("");
  });
  it("수동: master 칸이 사라진 탭에 master 재기동 → 왼쪽 1/3 새 컬럼 · 사용자가 둔 다른 컬럼의 상대 비율은 그대로", () => {
    // ★S1 수정1 뒤로는 CSO 가 대표 칸에 없으므로, master 칸이 **통째로 사라진** 탭에는 대표 칸의 흔적이 남지 않는다
    //   (실제 재기동은 master 구멍에 결속한다 — seatbind.test.ts). 그 경우 새 master 는 왼쪽 1/3 을 새로 받는다.
    const { t: t0, role } = build([[1, "master"], [2, "cso"], [3, "worker"], [4, "worker-2"]], true);
    const dragged = { ...(t0 as LNode & { type: "split" }), ratio: 0.5 };
    let t = rm(dragged, 1) as LNode;
    const before = [2, 3, 4].map((s) => W(t, s) as number);
    role.delete(1);
    role.set(9, "master");
    t = placeSeat(t, 9, mapRole(role), true);
    close(W(t, 9), MASTER_FRAC);
    close(H(t, 9), 1);
    [2, 3, 4].forEach((s, i) => close(W(t, s), before[i] * (1 - MASTER_FRAC)));
    expect(t.type === "split" && inSub(t.a, 9) && inSub(t.b, 2)).toBe(true);
  });
  it("cso 재기동 → 리뷰어 열로(비수동·수동 모두 · ★S1 수정1: master 아래가 아니다)", () => {
    for (const manual of [false, true]) {
      const { t: t0, role } = build([[1, "master"], [2, "cso"], [3, "worker"], [5, "reviewer-codex"]], manual);
      let t = rm(t0, 2) as LNode;
      role.delete(2);
      role.set(8, "cso");
      t = placeSeat(t, 8, mapRole(role), manual);
      expect(t.type === "split" && inSub(t.b, 8) && t.a.type === "pane" && t.a.sid === 1).toBe(true);
      close(H(t, 1), 1);
      const codex = rect(t, 5) as Rect;
      const cso = rect(t, 8) as Rect;
      expect(near(cso.x, codex.x) && near(cso.w, codex.w) && cso.y > codex.y).toBe(true); // 같은 열 · codex 아래
      close(cso.h, 1 / 2);
      close(codex.h, 1 / 2);
    }
  });
  it("종료된 옛 master(역할 맵에서 null)는 대표로 세지 않는다 — 대표 칸에 master 가 둘이 되지 않는다(반박 D4)", () => {
    const { t: t0, role } = build([[1, "master"], [2, "cso"], [3, "worker"]]);
    role.set(1, null); // 입양 틱은 `s.exited ? null : s.role` 로 맵을 만든다
    role.set(9, "master");
    const t = placeSeat(t0, 9, mapRole(role), false);
    const masters = sids(t).filter((s) => role.get(s) === "master");
    expect(masters).toEqual([9]);
    close(H(t, 9), 1); // ★S1 수정1: 대표 칸은 master 혼자(끝난 옛 칸은 역할 없는 셸로 오른쪽에 남는다)
    close(W(t, 9), MASTER_FRAC);
  });
});

// ────────────────────────────────────────────────────────────────────────────
describe("일회용·캐시 공백·대표 없는 탭", () => {
  it("cso-fresh 가 붙어도 master·CSO 칸은 그대로(제 컬럼을 받는다) · TTL 로 닫히면 원래 폭(반박 D5·R6)", () => {
    const { t: t0, role } = build([[1, "master"], [2, "cso"], [3, "worker"]]);
    role.set(5, "cso-fresh-1790000000");
    const t = placeSeat(t0, 5, mapRole(role), false);
    expect(stdShape(t, 1, [2], [3, 5])).toBe(""); // 일회용은 리뷰어 열에 끼지 않는다 — CSO 높이 1 그대로
    const back = anchorHead(rm(t, 5), mapRole(role)) as LNode;
    expect(stdShape(back, 1, [2], [3])).toBe("");
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
    expect(stdShape(t2, 1, [2], [3, 4, 20])).toBe(""); // 기억한 역할로 master·CSO 를 알아본다 — 새 셸은 CSO 열 앞 컬럼
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
  it("master+cso 뿐인 탭 → master 왼쪽 1/3(전체 높이) | CSO 오른쪽 열(★S1 수정1: master 아래가 아니다)", () => {
    for (const order of [["master", "cso"], ["cso", "master"]]) {
      const { t } = build(order.map((r, i) => [i + 1, r] as [number, string]));
      expect(stdShape(t, order.indexOf("master") + 1, [order.indexOf("cso") + 1], [])).toBe("");
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
    expect(stdShape(u, 1, [2], [3, 4])).toBe(""); // ★S1 수정1: cso 는 오른쪽 끝 열
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
    expect(stdShape(u, 11, [15, 12], [13, 14])).toBe(""); // master 1/3 · 나머지 균등 · 오른쪽 끝 열 = codex 위 / CSO 아래
  });
});

// ────────────────────────────────────────────────────────────────────────────
// ★S1 수정1(오너 2026-11-22 · 합의 나-15): 본부(CEO) 탭을 포함한 모든 탭에서 CSO 는 master 아래를 떠나 리뷰어 열의
//   맨 아래에 선다. 리뷰어들과 CSO 는 그 열을 똑같이 나누고(리뷰어 2 → 1/3 씩 · 1 → 1/2 씩 · 3 → 1/4 씩) master 는
//   제 열을 혼자 쓴다. 자동 배치 · 정렬 버튼 · 앱 기동이 전부 이 모양을 만들고, 옛 모양으로 저장된 탭은 업데이트 뒤
//   첫 기동에 사용자가 할 일 없이 옮겨진다.
describe("★S1 수정1 — CSO 는 리뷰어 열 맨 아래 · 리뷰어와 CSO 는 같은 높이 · master 는 제 열을 혼자", () => {
  const CASES: { name: string; reviewers: string[] }[] = [
    { name: "리뷰어 2명 → 1/3 씩", reviewers: ["reviewer-gemini", "reviewer-codex"] },
    { name: "리뷰어 1명 → 1/2 씩", reviewers: ["reviewer-codex"] },
    { name: "리뷰어 3명 → 1/4 씩", reviewers: ["reviewer-gemini", "reviewer-codex", "reviewer-grok"] },
    { name: "리뷰어 0명 → CSO 혼자 한 열", reviewers: [] },
  ];
  for (const c of CASES) {
    // sid: master 1 · cso 2 · worker 3 · worker-2 4 · 리뷰어 11..
    const seats: [number, string][] = [[1, "master"], [2, "cso"], [3, "worker"], [4, "worker-2"]];
    c.reviewers.forEach((r, i) => seats.push([11 + i, r]));
    const side = [...c.reviewers.map((_, i) => 11 + i), 2]; // 위→아래: 리뷰어들 · CSO
    const role = new Map<number, string | null>(seats);
    it(`정렬 버튼 — ${c.name}`, () => {
      const ids = seats.map((s) => s[0]);
      for (const order of [ids, [...ids].reverse()]) {
        expect(stdShape(roleLayout(order, mapRole(role)) as LNode, 1, side, [3, 4])).toBe(""); // 넘기는 순서와 무관
      }
    });
    it(`자동 배치 — ${c.name} · 도착 순서 전수`, () => {
      let bad = "";
      let n = 0;
      for (const order of perms(seats)) {
        n++;
        const got = stdShape(build(order).t, 1, side, [3, 4]);
        if (got && !bad) bad = order.map((s) => s[1]).join(",") + " → " + got;
      }
      expect(bad).toBe("");
      expect(n).toBeGreaterThanOrEqual(24); // 오라클이 실제로 돌았다(4석 24가지 ~ 7석 5040가지)
    });
    it(`앱 기동 — ${c.name} · 옛 모양(master 아래 CSO)으로 저장된 탭이 첫 기동에 새 모양으로 · 두 번째 기동은 그대로`, () => {
      const old = oldShape(seats);
      close(H(old, 2), 1 - HEAD_COL_MASTER); // 검체가 정말 옛 모양이다: CSO 는 master 아래 1/4
      for (const roleOf of [mapRole(role), (() => undefined) as RoleOf]) {
        // 역할을 데몬이 말해 줄 때 · 아직 모를 때(칸에 적힌 기억으로) 둘 다
        const t = anchorHeadSafe(old, roleOf) as LNode;
        expect(stdShape(t, 1, side, [3, 4])).toBe("");
        expect(sids(t).sort((a, b) => a - b)).toEqual(sids(old).sort((a, b) => a - b));
        expect(isAnchored(t, roleOf)).toBe(true);
        expect(anchorHeadSafe(t, roleOf)).toEqual(t);
      }
    });
    it(`앱 기동 — ${c.name} · 데몬이 재기동해 칸이 전부 '기다리는 자리'(구멍)여도 자리는 새 모양으로 옮겨진다`, () => {
      const neg = (n: LNode): LNode => (n.type === "pane" ? { ...n, sid: -n.sid } : { ...n, a: neg(n.a), b: neg(n.b) });
      const t = anchorHeadSafe(neg(oldShape(seats)), () => undefined) as LNode;
      expect(stdShape(t, -1, side.map((s) => -s), [-3, -4], () => true)).toBe("");
    });
  }
  it("본부(CEO) 탭 — 오너 실사용 저장본(0.25 · 0.75 · 1/3 · 0.5 · 0.5)이 첫 기동에 새 모양으로", () => {
    const CEO: LNode = {
      type: "split", dir: "row", ratio: 0.25,
      a: { type: "split", dir: "col", ratio: 0.75, a: { type: "pane", sid: 18 }, b: { type: "pane", sid: 17 } },
      b: { type: "split", dir: "row", ratio: 0.3333333333333333, a: { type: "pane", sid: 14 },
        b: { type: "split", dir: "row", ratio: 0.5, a: { type: "pane", sid: 16 },
          b: { type: "split", dir: "col", ratio: 0.5, a: { type: "pane", sid: 19 }, b: { type: "pane", sid: 15 } } } },
    };
    const ROLE = new Map<number, string | null>([[18, "master"], [17, "cso"], [14, "worker"], [16, "worker-2"], [19, "reviewer-gemini"], [15, "reviewer-codex"]]);
    expect(looksManual(CEO)).toBe(false);
    const t = anchorHeadSafe(CEO, mapRole(ROLE)) as LNode;
    expect(stdShape(t, 18, [19, 15, 17], [14, 16])).toBe("");
    expect(looksManual(t)).toBe(false); // 새 모양의 비율도 전부 자동값 — 다음 기동이 수동 탭으로 오인하지 않는다
  });
  it("살아 있는 칸끼리 같은 높이 — 끝난 리뷰어의 자리(구멍)가 접혀도 남은 리뷰어와 CSO 는 1/2 씩, 구멍이 보이는 동안은 이웃 몫을 빌린다", () => {
    const seats: [number, string][] = [[1, "master"], [2, "cso"], [3, "worker"], [11, "reviewer-gemini"], [12, "reviewer-codex"]];
    const role = new Map<number, string | null>(seats);
    const held = holdPane(build(seats).t, 12, -7) as LNode; // codex 가 끝났다 — 자리는 기억으로 남는다
    role.delete(12);
    const t = anchorHeadSafe(held, mapRole(role)) as LNode;
    expect(stdShape(t, 1, [11, 2], [3])).toBe(""); // 구멍이 접힌 화면: agy 1/2 · CSO 1/2
    const shownHole = (s: number) => s === -7;
    close(H(t, 11), 1 / 2);
    close((rect(t, -7, shownHole) as Rect).h + (rect(t, 2, shownHole) as Rect).h, 1 / 2); // 보류 스피너는 CSO 몫을 나눠 쓴다
    // 돌아온 codex 는 그 자리(agy 아래 · CSO 위)에 다시 선다
    const back = anchorHeadSafe(SL.fillHole(t, -7, 13, "reviewer-codex"), mapRole(new Map<number, string | null>([...role, [13, "reviewer-codex"]]))) as LNode;
    expect(stdShape(back, 1, [11, 13, 2], [3])).toBe("");
  });
  it("그 밖의 리뷰어(reviewer-claude-2 등)·같은 역할 둘도 리뷰어 열에 서고 CSO 가 맨 아래다 · 일회용 리뷰어는 제 컬럼", () => {
    const seats: [number, string][] = [[1, "master"], [2, "cso"], [3, "worker"], [11, "reviewer-claude-2"], [12, "reviewer-gemini"], [13, "reviewer-gemini"], [14, "reviewer-codex"]];
    expect(stdShape(build(seats).t, 1, [12, 13, 14, 11, 2], [3])).toBe("");
    const fresh: [number, string][] = [[1, "master"], [2, "cso"], [3, "worker"], [12, "reviewer-gemini"], [20, "reviewer-codex-fresh-1790000000"]];
    expect(stdShape(build(fresh).t, 1, [12, 2], [3, 20])).toBe("");
  });
  it("master 가 없는 탭은 손대지 않는다(자동 배치 · 기동) — 정렬 버튼만 리뷰어 열을 세운다", () => {
    const seats: [number, string][] = [[2, "cso"], [3, "worker"], [11, "reviewer-gemini"]];
    const role = new Map<number, string | null>(seats);
    let legacy: LNode | null = null;
    for (const [s] of seats) legacy = legacyAppend(legacy, s);
    const got = build(seats).t;
    expect(JSON.parse(JSON.stringify(got, (k, v) => (k === "role" ? undefined : v)))).toEqual(legacy as LNode);
    expect(anchorHeadSafe(got, mapRole(role))).toBe(got);
    const t = roleLayout([2, 3, 11], mapRole(role)) as LNode;
    close(W(t, 3), 1 / 2);
    close((rect(t, 11) as Rect).h, 1 / 2);
    close((rect(t, 2) as Rect).y, 1 / 2);
  });
  it("수동 탭 — 새 좌석: 리뷰어·CSO 는 오른쪽 끝 열에 끼고(CSO 가 맨 아래 · 기존 칸의 상대 비율 유지) 그 밖의 좌석은 그 열 앞 컬럼 · 대표 폭 불변", () => {
    const role = new Map<number, string | null>([[1, "master"], [3, "worker"], [11, "reviewer-gemini"], [12, "reviewer-codex"]]);
    // 사용자가 끈 탭: 대표 0.4 · worker 0.55 · 리뷰어 열 안 agy:codex = 3:7
    let t: LNode = {
      type: "split", dir: "row", ratio: 0.4, a: P(1, "master"),
      b: { type: "split", dir: "row", ratio: 0.55, a: P(3, "worker"), b: { type: "split", dir: "col", ratio: 0.3, a: P(11, "reviewer-gemini"), b: P(12, "reviewer-codex") } },
    };
    role.set(2, "cso");
    t = placeSeat(t, 2, mapRole(role), true);
    close(W(t, 1), 0.4);
    close(W(t, 3), 0.6 * 0.55);
    const [g, c, s] = [11, 12, 2].map((x) => rect(t, x) as Rect);
    expect(near(g.x, c.x) && near(c.x, s.x) && near(s.w, 0.6 * 0.45)).toBe(true);
    close(g.h, (0.3 * 2) / 3);
    close(c.h, (0.7 * 2) / 3);
    close(s.h, 1 / 3);
    close(s.y, 2 / 3); // CSO 가 맨 아래
    role.set(13, "reviewer-grok");
    t = placeSeat(t, 13, mapRole(role), true);
    expect((rect(t, 13) as Rect).y < (rect(t, 2) as Rect).y && (rect(t, 13) as Rect).y > (rect(t, 12) as Rect).y).toBe(true); // 늦게 온 리뷰어도 CSO 위
    close(W(t, 1), 0.4);
    role.set(4, "worker-2");
    t = placeSeat(t, 4, mapRole(role), true);
    close(W(t, 1), 0.4);
    expect((rect(t, 4) as Rect).x < (rect(t, 2) as Rect).x && (rect(t, 4) as Rect).x > (rect(t, 3) as Rect).x).toBe(true); // 리뷰어 열 앞
    close((rect(t, 2) as Rect).x + (rect(t, 2) as Rect).w, 1); // 리뷰어 열은 계속 오른쪽 끝
  });
  it("수동 탭 1회 이관(moveCsoToSide) — CSO 만 리뷰어 열 맨 아래로 · 대표·가운데 폭은 사용자가 끈 그대로 · 리뷰어 열은 같은 높이(★S1 M3) · 두 번째는 아무것도 안 한다", () => {
    const role = mapRole(new Map<number, string | null>([[1, "master"], [2, "cso"], [3, "worker"], [11, "reviewer-gemini"], [12, "reviewer-codex"]]));
    const manual: LNode = {
      type: "split", dir: "row", ratio: 0.4,
      a: { type: "split", dir: "col", ratio: 0.6, a: P(1, "master"), b: P(2, "cso") },
      b: { type: "split", dir: "row", ratio: 0.55, a: P(3, "worker"), b: { type: "split", dir: "col", ratio: 0.3, a: P(11, "reviewer-gemini"), b: P(12, "reviewer-codex") } },
    };
    expect(looksManual(manual)).toBe(true);
    const t = sl.moveCsoToSide(manual, role) as LNode;
    expect(sids(t).sort((a, b) => a - b)).toEqual([1, 2, 3, 11, 12]);
    expect(rect(t, 1)).toEqual({ x: 0, y: 0, w: 0.4, h: 1 }); // 대표 폭은 사용자가 둔 0.4 그대로 · 이제 혼자
    close(W(t, 3), 0.6 * 0.55);
    const [g, c, s] = [11, 12, 2].map((x) => rect(t, x) as Rect);
    expect(near(g.x, c.x) && near(c.x, s.x) && near(s.w, 0.6 * 0.45)).toBe(true);
    close(g.h, 1 / 3); // ★S1 M3(R1): 이 1회 이관은 리뷰어 열의 산 칸을 같은 높이로 세운다 — 주인이 끌어 둔 agy:codex = 3:7 도 1/3 씩이 된다
    close(c.h, 1 / 3);
    close(s.h, 1 / 3);
    close(s.y, 2 / 3);
    expect(sl.moveCsoToSide(t, role)).toBe(t);
  });
  it("수동 탭 1회 이관 — 리뷰어 열이 없으면 오른쪽 끝 새 컬럼 · master+CSO 뿐인 탭은 master | CSO · master 가 왼쪽에 없거나 CSO 가 대표 칸에 없으면 그대로 · 손상 트리도 그대로", () => {
    const role = mapRole(new Map<number, string | null>([[1, "master"], [2, "cso"], [3, "worker"]]));
    const noSide: LNode = { type: "split", dir: "row", ratio: 0.4, a: { type: "split", dir: "col", ratio: 0.6, a: P(1, "master"), b: P(2, "cso") }, b: P(3, "worker") };
    const a = sl.moveCsoToSide(noSide, role) as LNode;
    expect(rect(a, 1)).toEqual({ x: 0, y: 0, w: 0.4, h: 1 });
    close((rect(a, 2) as Rect).x + (rect(a, 2) as Rect).w, 1);
    close(H(a, 2), 1);
    const only: LNode = { type: "split", dir: "col", ratio: 0.6, a: P(1, "master"), b: P(2, "cso") };
    expect(stdShape(sl.moveCsoToSide(only, role) as LNode, 1, [2], [])).toBe("");
    const moved: LNode = { type: "split", dir: "row", ratio: 0.5, a: P(3, "worker"), b: { type: "split", dir: "col", ratio: 0.6, a: P(1, "master"), b: P(2, "cso") } };
    expect(sl.moveCsoToSide(moved, role)).toBe(moved); // 사용자가 master 를 오른쪽으로 옮긴 탭 — 어디가 대표 칸인지 단정하지 않는다
    const already: LNode = { type: "split", dir: "row", ratio: 0.4, a: P(1, "master"), b: { type: "split", dir: "row", ratio: 0.5, a: P(3, "worker"), b: P(2, "cso") } };
    expect(sl.moveCsoToSide(already, role)).toBe(already);
    for (const junk of [null, undefined, 7, {}, { type: "pane", sid: 0 }, { type: "split", dir: "row", a: { type: "pane", sid: 1 }, b: { type: "pane", sid: 1 } }]) {
      expect(() => sl.moveCsoToSide(junk, role)).not.toThrow();
      expect(sl.moveCsoToSide(junk, role)).toBe(junk);
    }
    const boom: RoleOf = () => {
      throw new Error("boom");
    };
    expect(sl.moveCsoToSide(noSide, boom)).toBe(noSide);
  });
});

// ────────────────────────────────────────────────────────────────────────────
// ★S1 M3(리뷰 지적) — 수동 탭 1회 이관이 CSO 를 **이미 있던 리뷰어 열**에 넣을 때 그 열의 산 칸은 같은 높이다.
//   종전: addToSide → spineInsert 가 옛 상대 비율을 그대로 두어, 0.8 : 0.2 로 끌어 둔 열이 0.5333 / 0.1333 / 0.3333 이 됐다
//   (오너의 말은 「같은 높이」). R1 같은 높이(구멍은 sideColumn 의 규칙 그대로) · R2 열 안의 위→아래 순서 유지 · CSO 맨 아래 ·
//   R3 칸 보존 · 그 열 밖은 그대로 · R4 나중에 오는 좌석(addToSide)은 종전 그대로 · R5 이관의 다른 갈래는 종전 그대로.
describe("★S1 M3 — 수동 탭 1회 이관: 리뷰어 열의 산 칸은 같은 높이다", () => {
  const ROLE = new Map<number, string | null>([[1, "master"], [2, "cso"], [3, "worker"], [4, "reviewer-gemini"], [5, "reviewer-codex"], [6, "reviewer-claude-2"], [7, "worker-2"]]);
  const role = mapRole(ROLE);
  const col = (ratio: number, a: LNode, b: LNode): LNode => ({ type: "split", dir: "col", ratio, a, b });
  const row = (ratio: number, a: LNode, b: LNode): LNode => ({ type: "split", dir: "row", ratio, a, b });
  /** 리뷰어의 증인(입력 그대로): 대표 칸 0.41 = master 위(0.75)·CSO 아래 / worker / 리뷰어 열 agy 0.8 : codex 0.2 */
  const witness = (): LNode => row(0.41, col(0.75, P(1, "master"), P(2, "cso")), row(0.5, P(3, "worker"), col(0.8, P(4, "reviewer-gemini"), P(5, "reviewer-codex"))));

  // ---------- 오라클(코드가 계산한다): 트리 → 칸마다 사각형 → 맨 오른쪽 열의 산 칸(위→아래) ----------
  /** 맨 오른쪽 열(x 가 가장 큰 칸들)의 산 칸과 그 사각형 — 위에서 아래로. 안 보이는 구멍 쪽은 접힌다(rect 와 같은 규칙). */
  function lastColumn(t: LNode): { sid: number; r: Rect }[] {
    const all = sids(t).filter((s) => s > 0).map((sid) => ({ sid, r: rect(t, sid) as Rect }));
    const x = Math.max(...all.map((b) => b.r.x));
    return all.filter((b) => near(b.r.x, x)).sort((p, q) => p.r.y - q.r.y);
  }
  /** 그 열의 산 칸이 전부 같은 높이로 열을 빈틈없이 채우는가 — 맞으면 "" · 어긋나면 칸마다의 높이. */
  function unequal(t: LNode): string {
    const c = lastColumn(t);
    return c.every((b, i) => near(b.r.h, 1 / c.length) && near(b.r.y, i / c.length)) ? "" : c.map((b) => `sid ${b.sid} h=${b.r.h.toFixed(4)}`).join(" · ");
  }
  const heights = (t: LNode) => lastColumn(t).map((b) => [b.sid, Math.round(b.r.h * 10000) / 10000]);

  it("R1 · 리뷰어의 증인 — 0.8 : 0.2 로 끌어 둔 리뷰어 열에 CSO 가 들어가면 agy · codex · CSO 가 1/3 씩이다(종전 0.5333 / 0.1333 / 0.3333)", () => {
    const t = sl.moveCsoToSide(witness(), role) as LNode;
    expect(heights(t)).toEqual([[4, 0.3333], [5, 0.3333], [2, 0.3333]]);
    expect(unequal(t)).toBe("");
    close(W(t, 1), 0.41); // 대표 폭은 주인이 끈 그대로
    close(H(t, 1), 1); // 대표 칸은 master 혼자
    close(W(t, 3), 0.295); // worker 폭 그대로(0.59 × 0.5)
    expect(sids(t).sort((a, b) => a - b)).toEqual([1, 2, 3, 4, 5]); // 다섯 칸 그대로
  });

  it("R1·R2·R3 · 오라클 — 리뷰어 1·2·3명 × 씨앗 고정 옛 비율 20벌씩: 열의 산 칸은 같은 높이 · 순서 그대로 · CSO 맨 아래 · 그 열 밖은 한 글자도 안 바뀐다", () => {
    let seed = 20261122;
    const rnd = () => {
      seed = (seed * 1103515245 + 12345) & 0x7fffffff;
      return seed / 0x7fffffff;
    };
    const ratio = () => 0.1 + rnd() * 0.8; // 주인이 끌어 둘 수 있는 범위
    const HOLD: LNode = { type: "pane", sid: 999 }; // 「리뷰어 열이 있던 자리」 — 열 밖만 비교하려고 열을 이것으로 가린다
    const maskLast = (n: LNode): LNode => (n.type === "split" && n.dir === "row" ? { ...n, b: maskLast(n.b) } : HOLD);
    const outside = (t: LNode): LNode => (t.type === "split" ? { ...t, b: maskLast(t.b) } : t);
    let cases = 0;
    for (const k of [1, 2, 3]) {
      for (let n = 0; n < 20; n++) {
        const order = [4, 5, 6].slice(0, k); // 열 안의 위→아래 순서도 섞는다(주인이 codex 를 위에 둔 탭)
        for (let i = order.length - 1; i > 0; i--) {
          const j = Math.floor(rnd() * (i + 1));
          [order[i], order[j]] = [order[j], order[i]];
        }
        // 열의 모양도 섞는다(위로 기운 분할·아래로 기운 분할) — 위→아래 순서는 order 그대로다
        const stack = (xs: number[]): LNode => {
          if (xs.length === 1) return P(xs[0], ROLE.get(xs[0]) as string);
          const cut = 1 + Math.floor(rnd() * (xs.length - 1));
          return col(ratio(), stack(xs.slice(0, cut)), stack(xs.slice(cut)));
        };
        const mids: LNode[] = rnd() < 0.5 ? [P(3, "worker")] : [P(3, "worker"), P(7, "worker-2")];
        let rest: LNode = stack(order);
        for (let i = mids.length - 1; i >= 0; i--) rest = row(ratio(), mids[i], rest);
        const input = row(ratio(), col(ratio(), P(1, "master"), P(2, "cso")), rest) as Extract<LNode, { type: "split" }>;
        const frozen = JSON.stringify(input);
        const out = sl.moveCsoToSide(input, role) as LNode;
        const tag = `리뷰어 ${k}명 #${n} `;
        expect(JSON.stringify(input)).toBe(frozen); // 입력은 바꾸지 않는다
        expect(tag + unequal(out)).toBe(tag); // R1: 같은 높이
        expect(lastColumn(out).map((b) => b.sid)).toEqual([...order, 2]); // R2: 순서 그대로 · CSO 맨 아래
        expect(sids(out).sort((a, b) => a - b)).toEqual(sids(input).sort((a, b) => a - b)); // R3: 칸 보존
        expect(outside(out)).toEqual(outside({ ...input, a: P(1, "master") })); // R3: 대표 칸은 master 혼자 · 그 열 밖의 모든 비율이 그대로
        close(W(out, 1), input.ratio as number); // R3: 대표 폭
        for (const m of mids) if (m.type === "pane") expect(rect(out, m.sid)).toEqual(rect(input, m.sid)); // R3: 가운데 컬럼의 자리와 폭
        cases++;
      }
    }
    expect(cases).toBe(60);
  });

  it("R2 · 대조 — 주인이 codex 를 agy 위에 둔 열: 그 순서 그대로이고 CSO 가 맨 아래다(등급 순으로 다시 세우지 않는다)", () => {
    const t0 = row(0.41, col(0.75, P(1, "master"), P(2, "cso")), row(0.5, P(3, "worker"), col(0.8, P(5, "reviewer-codex"), P(4, "reviewer-gemini"))));
    expect(lastColumn(sl.moveCsoToSide(t0, role) as LNode).map((b) => b.sid)).toEqual([5, 4, 2]);
  });

  it("R1 · 구멍 — 열에 복원을 기다리는 자리가 있으면 sideColumn 의 규칙 그대로다: 산 칸끼리 같은 높이 · 구멍은 이웃 몫을 빌린다", () => {
    const t0 = row(0.41, col(0.75, P(1, "master"), P(2, "cso")), row(0.5, P(3, "worker"), col(0.8, P(4, "reviewer-gemini"), P(-5, "reviewer-codex"))));
    const t = sl.moveCsoToSide(t0, role) as LNode;
    expect(unequal(t)).toBe(""); // 구멍이 접힌 화면: agy 1/2 · CSO 1/2
    expect(lastColumn(t).map((b) => b.sid)).toEqual([4, 2]);
    const hole = (s: number) => s === -5; // 구멍이 보이는 동안(보류 기한 안)
    close((rect(t, 4, hole) as Rect).h, 1 / 2); // agy 는 그대로 1/2
    close((rect(t, -5, hole) as Rect).h + (rect(t, 2, hole) as Rect).h, 1 / 2); // 구멍은 CSO 몫을 나눠 쓴다
    expect((rect(t, -5, hole) as Rect).y < (rect(t, 2, hole) as Rect).y).toBe(true); // 자리(순서)는 그대로 — agy · codex 자리 · CSO
    expect(sids(t).sort((a, b) => a - b)).toEqual([-5, 1, 2, 3, 4]);
    // `정렬`·자동 배치가 쓰는 정의처(sideColumn)가 같은 칸들로 세운 열과 높이가 같다 — 규칙은 하나다
    const std = anchorHeadSafe(t0, role) as LNode;
    for (const s of [4, -5, 2]) close((rect(t, s, hole) as Rect).h, (rect(std, s, hole) as Rect).h);
  });

  it("R4 · 대조 — 나중에 오는 좌석(수동 탭의 placeSeat → addToSide)은 주인의 비율을 그대로 둔다: 같은 0.8 : 0.2 열에 CSO 가 오면 0.5333 / 0.1333 / 0.3333", () => {
    const noCso = row(0.41, P(1, "master"), row(0.5, P(3, "worker"), col(0.8, P(4, "reviewer-gemini"), P(5, "reviewer-codex"))));
    const t = placeSeat(noCso, 2, role, true);
    expect(heights(t)).toEqual([[4, 0.5333], [5, 0.1333], [2, 0.3333]]);
    close(W(t, 1), 0.41);
  });

  it("R5 · 대조 — 오른쪽 끝 컬럼이 리뷰어만의 열이 아니면(다른 좌석이 섞였다) CSO 는 맨 오른쪽에 제 컬럼으로 선다 · 섞인 컬럼의 비율은 그대로", () => {
    const mixedCol = (): LNode => col(0.8, P(4, "reviewer-gemini"), P(7, "worker-2"));
    const t = sl.moveCsoToSide(row(0.41, col(0.75, P(1, "master"), P(2, "cso")), row(0.5, P(3, "worker"), mixedCol())), role) as LNode;
    expect(t).toEqual(row(0.41, P(1, "master"), row(2 / 3, row(0.5, P(3, "worker"), mixedCol()), P(2, "cso"))));
  });

  it("R5 · 대조 — 리뷰어 열이 없던 탭의 대표 칸에 CSO 가 여럿(하나는 복원을 기다리는 자리): 제 컬럼에 받은 순서대로 1/3 씩 선다(종전 그대로 — 여기는 다시 나누지 않는다)", () => {
    const r5 = mapRole(new Map<number, string | null>([[1, "master"], [2, "cso"], [3, "worker"], [8, "cso-3"]]));
    const multi = row(0.41, col(0.6, P(1, "master"), col(0.5, P(2, "cso"), col(0.5, P(-6, "cso-2"), P(8, "cso-3")))), P(3, "worker"));
    const t = sl.moveCsoToSide(multi, r5) as LNode;
    const all = () => true; // 구멍도 보이는 화면
    for (const [s, y] of [[2, 0], [-6, 1 / 3], [8, 2 / 3]] as [number, number][]) {
      const r = rect(t, s, all) as Rect;
      close(r.x, 0.705);
      close(r.y, y);
      close(r.h, 1 / 3);
    }
    close(W(t, 1), 0.41);
  });
});

// ★S1 M3(R6 · 리뷰 지적) — 표식 csoSideDone 을 세워도 되는지의 판정(순수부). 배선(실제 기동 S5 루프)은 csosidewiring.test.ts.
//   규칙: 수동 탭의 대표 칸 안에 CSO 가 **아직 있거나 있을 수 있으면** 남았다(참) — 그 기동은 표식을 쓰지 않는다.
describe("★S1 M3(R6) — csoMovePending: 이 수동 탭의 1회 이관이 아직 남았는가", () => {
  const role = mapRole(new Map<number, string | null>([[1, "master"], [2, "cso"], [3, "worker"], [4, "reviewer-gemini"], [5, "reviewer-codex"], [9, null]]));
  const silent: RoleOf = () => undefined; // 그 탭의 데몬이 답하지 않았다 — 칸의 기억(role)만 남는다
  const col = (ratio: number, a: LNode, b: LNode): LNode => ({ type: "split", dir: "col", ratio, a, b });
  const row = (ratio: number, a: LNode, b: LNode): LNode => ({ type: "split", dir: "row", ratio, a, b });
  const B = (sid: number): LNode => ({ type: "pane", sid }); // 역할 기억이 없는 칸
  const pending = (t: unknown, r: RoleOf): unknown => sl.csoMovePending(t, r);

  it("대표 칸 안에 CSO 가 있다 → 남았다 · 옮긴 뒤에는 남지 않았다(왼쪽 master 컬럼 · 한 열로 쌓인 두 좌석 탭 · 기다리는 자리인 CSO)", () => {
    const shapes: LNode[] = [
      row(0.41, col(0.75, P(1, "master"), P(2, "cso")), row(0.5, P(3, "worker"), col(0.8, P(4, "reviewer-gemini"), P(5, "reviewer-codex")))),
      col(0.75, P(1, "master"), P(2, "cso")),
      row(0.41, col(0.75, P(1, "master"), P(-2, "cso")), P(3, "worker")),
    ];
    for (const t of shapes) {
      expect(pending(t, role)).toBe(true);
      expect(pending(t, silent)).toBe(true); // 데몬이 답하지 않아도 칸의 기억으로 알아본다
      expect(pending(sl.moveCsoToSide(t, role), role)).toBe(false);
    }
  });
  it("옮길 것이 없다 → 남지 않았다: 빈 탭(null) · CSO 없음 · CSO 가 이미 리뷰어 열 · CSO 가 master 옆 · master 를 오른쪽으로 옮긴 탭(이관이 손대지 않는 모양)", () => {
    expect(pending(null, role)).toBe(false);
    expect(pending(row(0.41, P(1, "master"), P(3, "worker")), role)).toBe(false);
    expect(pending(row(0.41, P(1, "master"), row(0.5, P(3, "worker"), col(0.5, P(4, "reviewer-gemini"), P(2, "cso")))), role)).toBe(false);
    expect(pending(row(0.3, P(2, "cso"), P(1, "master")), role)).toBe(false);
    expect(pending(row(0.5, P(3, "worker"), col(0.6, P(1, "master"), P(2, "cso"))), role)).toBe(false);
  });
  it("판정할 수 없다 → 남았다고 본다: 손상 트리(같은 sid 두 번 · 비율 범위 밖) · 역할 조회가 던진다", () => {
    expect(pending(row(0.41, col(0.75, P(1, "master"), P(2, "cso")), P(1, "master")), role)).toBe(true);
    expect(pending(row(7, col(0.75, P(1, "master"), P(2, "cso")), P(3, "worker")), role)).toBe(true);
    const boom: RoleOf = () => {
      throw new Error("boom");
    };
    expect(() => pending(row(0.41, col(0.75, P(1, "master"), P(2, "cso")), P(3, "worker")), boom)).not.toThrow();
    expect(pending(row(0.41, col(0.75, P(1, "master"), P(2, "cso")), P(3, "worker")), boom)).toBe(true);
  });
  it("역할을 모르는 칸(데몬 무응답 + 칸의 기억 없음)이 대표 칸 안에 있다 → CSO 일 수 있다 → 남았다고 본다", () => {
    expect(pending(row(0.41, col(0.75, B(1), B(2)), B(3)), silent)).toBe(true); // 구버전 저장본: 역할 기억이 하나도 없다
    expect(pending(col(0.75, B(1), B(2)), silent)).toBe(true);
    expect(pending(row(0.41, col(0.75, P(1, "master"), B(9)), P(3, "worker")), silent)).toBe(true); // master 아래의 모르는 칸
    expect(pending(row(0.41, col(0.75, B(1), P(2, "cso")), P(3, "worker")), silent)).toBe(true); // 왼쪽 칸의 master 를 알아보지 못한다 — 어디가 대표 칸인지 모른다
  });
  it("대조 — 모르는 칸이 대표 칸 밖에만 있으면 막지 않는다 · 데몬이 「역할 없음」이라고 답한 셸은 아는 칸이다", () => {
    expect(pending(row(0.41, P(1, "master"), row(0.5, P(3, "worker"), B(9))), silent)).toBe(false); // master 는 기억으로 왼쪽 칸에 혼자
    expect(pending(row(0.41, col(0.75, P(1, "master"), B(9)), P(3, "worker")), role)).toBe(false); // 9 = 데몬이 답한 plain 셸(null)
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

// ────────────────────────────────────────────────────────────────────────────
// ★리뷰1 F8(minor) — 순수부 생존 뮤테이션 6종 중 이 파일 소관 4종(P9·P15·P17 · F6 진단 카운터).
// P12·P35 는 seatbind.test.ts(restoreTree·adoptSeat 문맥). P31 은 실제로 트리거되는 입력을 찾지
// 못해(placeSeat 이 멤버를 보존하도록 구성돼 있어 sameMembers 가 거짓이 되는 도달 가능한 경로가
// 없다) 제외한다 — WORKLOG "제외" 절에 이유를 남긴다.
describe("리뷰1 F8 — 생존 뮤테이션 보강", () => {
  it("isValidTree: 같은 sid 가 두 번 있으면 거짓이다(중복 방어 · P17)", () => {
    expect(
      isValidTree({ type: "split", dir: "row", a: { type: "pane", sid: 1 }, b: { type: "pane", sid: 1 } }),
    ).toBe(false);
  });

  it("master 없이 cso 만 있으면, 두 번째 cso 는 대표 컬럼(세로 쌓기)을 세우지 않는다(WORKLOG #13 · P9)", () => {
    const roleOf = mapRole(new Map<number, string | null>([[1, "cso"], [2, "cso"]]));
    const out = placeSeat({ type: "pane", sid: 1, role: "cso" }, 2, roleOf, false) as any;
    // dir:"col" 이면 master 없이 cso 가 대표 컬럼을 세운 것(appendCell) — 마스터 재기동 공백을
    // cso 가 가로채지 않는다는 약속이 깨진다. 종전 규칙은 가로(row) 부착이다.
    expect(out.type).toBe("split");
    expect(out.dir).toBe("row");
  });

  it("colsVis 는 구멍만 있는 컬럼을 보이는 컬럼으로 세지 않는다(rebalanceRow 가 폭을 뭉개지 않는다 · P15)", () => {
    const roleOf = mapRole(new Map<number, string | null>([[1, "master"], [5, "worker"]]));
    const t: LNode = {
      type: "split", dir: "row", ratio: 0.5,
      a: { type: "pane", sid: 1, role: "master" }, // 대표 컬럼 — anchoredCore 가 참이 되게 한다
      b: {
        type: "split", dir: "row", ratio: 0.9, // 의도적으로 균등이 아닌 값 — 재분배되면 값이 바뀐다
        a: { type: "pane", sid: 5, role: "worker" }, // 산 칸 — 보이는 컬럼 1
        b: { type: "pane", sid: -7, role: "worker-2" }, // 구멍뿐인 컬럼 — 보이는 컬럼 0 이어야 한다
      },
    };
    const out = anchorHeadSafe(t, roleOf) as any;
    // 구멍뿐인 컬럼을 1로 세면(P15) ca=cb=1 → 균등(0.5)으로 재분배돼 0.9 가 사라진다.
    expect(out.b.ratio).toBe(0.9);
  });

  it("appendColumn: 오른쪽이 구멍뿐이면(n=0) 절반을 주지 않는다 — 기억한 비율을 짓누르지 않는다(리뷰1 F4)", () => {
    // probe M(review1-probe-out.txt) 축소판 — 수동 탭이 경로 B(복원 중 앱 시작)로 재기동한 모양.
    const tree: LNode = {
      type: "split", dir: "row", ratio: 0.25,
      a: { type: "pane", sid: -1, role: "master" },
      b: {
        type: "split", dir: "row", ratio: 0.5,
        a: { type: "pane", sid: -2, role: "worker" },
        b: { type: "pane", sid: -3, role: "worker-2" },
      },
    };
    const roleOf = mapRole(new Map<number, string | null>([[99, null]]));
    const out = placeSeat(tree, 99, roleOf, true) as any; // manual=true — 충전 셸이 한 칸 곁에 붙는다
    // 고쳐지기 전(0.5)엔 방금 붙는 빈 셸이 보류 구멍들의 자리를 절반이나 가져간다(probe M: worker
    // 0.25→0.125). 구멍 쪽(기존)이 0.85, 셸이 0.15 — 결속되면 제 비율로 돌아온다.
    expect(out.b.ratio).toBeCloseTo(0.85, 9);
  });

  it("손상 트리 → legacyAppend 폴백은 조용히 지나가지 않는다 — 진단 카운터가 오른다(리뷰1 F6)", () => {
    const before = placeSeatFallbackCount();
    const dup: LNode = { type: "split", dir: "row", a: { type: "pane", sid: 1 }, b: { type: "pane", sid: 1 } };
    placeSeatSafe(dup, 2, () => undefined, false);
    expect(placeSeatFallbackCount()).toBe(before + 1);
  });
});
