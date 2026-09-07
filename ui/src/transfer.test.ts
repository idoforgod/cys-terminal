// transfer.ts 순수 함수 회귀 테스트 (bun test — 신규 의존성 0).
import { describe, expect, test } from "bun:test";
import { appendPane, transferTrees, treeSids, type TNode } from "./transfer";

const pane = (sid: number): TNode => ({ type: "pane", sid });
const split = (a: TNode, b: TNode): TNode => ({ type: "split", dir: "row", a, b });

describe("transferTrees", () => {
  test("단일 pane 트리에서 떼어내면 src=null, dest에 편입", () => {
    const r = transferTrees(pane(1), null, 1)!;
    expect(r.src).toBeNull();
    expect(treeSids(r.dest)).toEqual([1]);
  });
  test("분할 트리에서 떼면 형제로 붕괴 + dest 말단 분할", () => {
    const r = transferTrees(split(pane(1), pane(2)), pane(9), 2)!;
    expect(treeSids(r.src)).toEqual([1]);
    expect(treeSids(r.dest)).toEqual([9, 2]);
    expect(r.dest.type).toBe("split");
  });
  test("src에 없는 sid는 null(무변경 신호)", () => {
    expect(transferTrees(pane(1), null, 7)).toBeNull();
    expect(transferTrees(null, pane(1), 1)).toBeNull();
  });
  test("dest에 같은 sid가 이미 있으면 거부(유령 pane 차단)", () => {
    expect(transferTrees(split(pane(1), pane(2)), pane(1), 1)).toBeNull();
  });
  test("깊은 트리에서도 제거·유일성 보존", () => {
    const src = split(split(pane(1), pane(2)), pane(3));
    const r = transferTrees(src, split(pane(8), pane(9)), 2)!;
    expect(treeSids(r.src)).toEqual([1, 3]);
    expect(treeSids(r.dest)).toEqual([8, 9, 2]);
    // 원본 불변(순수성)
    expect(treeSids(src)).toEqual([1, 2, 3]);
  });
});

describe("appendPane", () => {
  test("빈 트리는 pane 단독", () => {
    expect(appendPane(null, 5)).toEqual({ type: "pane", sid: 5 });
  });
});

// ── ★(0.14.31 · WP-4 R1) 전출 목적지 좌석 식별 ──
import { pickLaunchedAgentSid, type SurfaceRow } from "./transfer";

const row = (o: Partial<SurfaceRow> & { surface_id: number }): SurfaceRow => ({
  role: null,
  agent: null,
  exited: false,
  created_at: 100,
  ...o,
});

describe("pickLaunchedAgentSid", () => {
  test("런치 후 새로 생긴 그 역할 좌석 하나를 고른다(런처 셸이 아니라)", () => {
    const after = [row({ surface_id: 7 }), row({ surface_id: 8, role: "reviewer-codex", agent: "codex" })];
    expect(pickLaunchedAgentSid([1, 2], 7, after, "reviewer-codex", 50)).toBe(8);
  });
  test("런처 셸(역할·에이전트 없음)은 절대 목적지가 아니다", () => {
    const after = [row({ surface_id: 7 })];
    expect(pickLaunchedAgentSid([1, 2], 7, after, "reviewer-codex", 50)).toBeNull();
  });
  test("역할만 붙고 에이전트 미관측이면 확정하지 않는다(각성 증거 요구)", () => {
    const after = [row({ surface_id: 8, role: "reviewer-codex", agent: null })];
    expect(pickLaunchedAgentSid([1], 7, after, "reviewer-codex", 50)).toBeNull();
  });
  test("같은 역할 좌석이 둘이면 확정하지 않는다(모호는 승계 근거 아님)", () => {
    const after = [
      row({ surface_id: 8, role: "reviewer-codex", agent: "codex" }),
      row({ surface_id: 9, role: "reviewer-codex", agent: "codex" }),
    ];
    expect(pickLaunchedAgentSid([1], 7, after, "reviewer-codex", 50)).toBeNull();
  });
  test("런치 전부터 있던 같은 역할 좌석은 후보가 아니다", () => {
    const after = [row({ surface_id: 8, role: "reviewer-codex", agent: "codex" })];
    expect(pickLaunchedAgentSid([1, 8], 7, after, "reviewer-codex", 50)).toBeNull();
  });
  test("런치 시각보다 먼저 생긴 좌석은 그 전출의 결과가 아니다", () => {
    const after = [row({ surface_id: 8, role: "reviewer-codex", agent: "codex", created_at: 10 })];
    expect(pickLaunchedAgentSid([1], 7, after, "reviewer-codex", 50)).toBeNull();
  });
  test("다른 역할·종료 좌석은 후보가 아니다", () => {
    const after = [
      row({ surface_id: 8, role: "worker", agent: "claude" }),
      row({ surface_id: 9, role: "reviewer-codex", agent: "codex", exited: true }),
    ];
    expect(pickLaunchedAgentSid([1], 7, after, "reviewer-codex", 50)).toBeNull();
  });
});
