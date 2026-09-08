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
import { destinationLooksAwake, pickLaunchedAgentSid, type SurfaceRow } from "./transfer";

const row = (o: Partial<SurfaceRow> & { surface_id: number }): SurfaceRow => ({
  role: null,
  agent: null,
  agent_alive: null,
  exited: false,
  created_at: 100,
  created_by: null,
  awakened_at: null,
  ...o,
});

/** 정상 목적지 한 줄 — 이 행에서 **한 축씩** 빼며 각 게이트를 잰다(음성 대조의 기준선). */
const good = (o: Partial<SurfaceRow> = {}): SurfaceRow =>
  row({
    surface_id: 8,
    role: "reviewer-codex",
    agent: "codex",
    agent_alive: true,
    created_by: 7,
    ...o,
  });

describe("pickLaunchedAgentSid", () => {
  const pick = (after: SurfaceRow[], before: number[] = [1, 2]) =>
    pickLaunchedAgentSid(before, 7, after, "reviewer-codex", "codex", 50);

  test("런치 후 그 런처 셸이 만든 그 역할·그 종류 좌석 하나를 고른다", () => {
    expect(pick([row({ surface_id: 7 }), good()])).toBe(8);
  });
  test("런처 셸(역할·에이전트 없음)은 절대 목적지가 아니다", () => {
    expect(pick([row({ surface_id: 7 })])).toBeNull();
  });
  test("역할만 붙고 에이전트 미관측이면 확정하지 않는다(각성 증거 요구)", () => {
    expect(pick([good({ agent: null, agent_alive: null })])).toBeNull();
  });
  // ★(R2 · codex blocking) 메타 등록은 생존이 아니다 — `agent_alive` 3값을 `true` 로만 좁힌다.
  test("메타 이름만 등록되고 생존 미관측(null)이면 확정하지 않는다", () => {
    expect(pick([good({ agent_alive: null })])).toBeNull();
  });
  test("기동 즉사(agent_alive=false)면 확정하지 않는다", () => {
    expect(pick([good({ agent_alive: false })])).toBeNull();
  });
  // ★(R2 · codex blocking) 런치 요청과의 연관 증거 — 데몬이 기록한 생성자 pane.
  test("다른 pane 이 만든 좌석은(생성자 불일치) 목적지가 아니다", () => {
    expect(pick([good({ created_by: 99 })])).toBeNull();
  });
  test("생성자 미상(null)은 '모름'이지 '맞음'이 아니다 — 확정하지 않는다", () => {
    expect(pick([good({ created_by: null })])).toBeNull();
  });
  // ★(R2 · codex blocking) 시킨 종류가 떠야 한다.
  test("다른 agent 종류의 유일 후보는 목적지가 아니다", () => {
    expect(pick([good({ agent: "claude" })])).toBeNull();
  });
  test("같은 역할 좌석이 둘이면 확정하지 않는다(모호는 승계 근거 아님)", () => {
    expect(pick([good(), good({ surface_id: 9 })])).toBeNull();
  });
  test("런치 전부터 있던 같은 역할 좌석은 후보가 아니다", () => {
    expect(pick([good()], [1, 8])).toBeNull();
  });
  test("런치 시각보다 먼저 생긴 좌석은 그 전출의 결과가 아니다", () => {
    expect(pick([good({ created_at: 10 })])).toBeNull();
  });
  test("다른 역할·종료 좌석은 후보가 아니다", () => {
    expect(pick([good({ role: "worker", agent: "claude" }), good({ surface_id: 9, exited: true })])).toBeNull();
  });
});

// ★(R2 · codex blocking) 원본 종료의 별도 관문 — 생존 관측은 '인계를 받을 수 있음'이 아니다.
describe("destinationLooksAwake", () => {
  test("각성 래치가 서면 참", () => {
    expect(destinationLooksAwake(good({ awakened_at: 1234 }))).toBe(true);
  });
  test("살아 있어도 래치가 없으면 거짓(신뢰 관문 대기 중일 수 있다)", () => {
    expect(destinationLooksAwake(good())).toBe(false);
  });
  test("종료된 좌석은 래치가 있어도 거짓", () => {
    expect(destinationLooksAwake(good({ awakened_at: 1234, exited: true }))).toBe(false);
  });
  test("행이 없으면 거짓(관측 실패는 통과가 아니다)", () => {
    expect(destinationLooksAwake(undefined)).toBe(false);
  });
  // ★독립 재유도(triage · codex blocking #3): 각성 래치는 **한 번 서면 내려가지 않는 표식**이다.
  //   원본을 닫는 결정은 "지금 이 좌석이 인계를 받을 수 있는가"인데, 래치만 보면 각성 뒤에
  //   에이전트가 죽었거나(agent_alive=false · 셸만 남아 exited=false) 역할을 잃은 좌석도 통과한다.
  //   그 상태에서 원본을 닫으면 인계는 아무도 읽지 않고 작업 세션만 사라진다.
  test("각성 뒤 에이전트가 죽으면 거짓(래치는 과거 사실이다)", () => {
    expect(destinationLooksAwake(good({ awakened_at: 1234, agent_alive: false }))).toBe(false);
  });
  test("각성 뒤 역할을 잃은 좌석은 거짓(인계 대상이 아니다)", () => {
    expect(destinationLooksAwake(good({ awakened_at: 1234, role: null }))).toBe(false);
  });
});
