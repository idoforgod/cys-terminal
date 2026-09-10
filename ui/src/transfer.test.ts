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
import {
  destinationCanReceive,
  destinationGone,
  handoffAckPath,
  handoffInstruction,
  originCloseVerdict,
  parseEnqueueReceipt,
  pickLaunchedAgentSid,
  transferRetryAction,
  type SurfaceRow,
  type TransferRecord,
} from "./transfer";
import { readFileSync } from "node:fs";

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

// ★(0.14.31 · 성찰 C2 · blocking) 좌석 수신 가능 술어 — `awakened_at` 을 **요구하지 않는다**.
//   리뷰어 좌석(codex/gemini)은 그 래치를 구조적으로 낼 수 없다(유일 write path = `cys set-status` ·
//   REVIEWER_DIRECTIVE 금지 · 훅 미가동). 각성과 인수는 다른 축이다.
describe("destinationCanReceive", () => {
  test("역할·생존 관측·종류·미종료면 참 — 각성 래치 null 이어도 참(리뷰어 좌석)", () => {
    expect(destinationCanReceive(good({ awakened_at: null }), "reviewer-codex", "codex")).toBe(true);
  });
  test("종료된 좌석은 거짓", () => {
    expect(destinationCanReceive(good({ exited: true }))).toBe(false);
  });
  test("행이 없으면 거짓(관측 실패는 통과가 아니다)", () => {
    expect(destinationCanReceive(undefined)).toBe(false);
  });
  test("에이전트가 죽으면 거짓(셸만 남아 exited=false)", () => {
    expect(destinationCanReceive(good({ agent_alive: false }))).toBe(false);
  });
  test("생존 미관측(null)은 근거가 아니다", () => {
    expect(destinationCanReceive(good({ agent_alive: null }))).toBe(false);
  });
  test("역할을 잃은 좌석·다른 역할·다른 종류는 거짓", () => {
    expect(destinationCanReceive(good({ role: null }))).toBe(false);
    expect(destinationCanReceive(good(), "worker")).toBe(false);
    expect(destinationCanReceive(good(), "reviewer-codex", "claude")).toBe(false);
  });
});

describe("destinationGone", () => {
  test("관측 실패는 소멸이 아니다(모름은 보류 사유)", () => {
    expect(destinationGone(undefined, false)).toBe(false);
    expect(destinationGone(good({ agent_alive: null }), true)).toBe(false);
  });
  test("행 부재·종료·사망 통지는 소멸", () => {
    expect(destinationGone(undefined, true)).toBe(true);
    expect(destinationGone(good({ exited: true }), true)).toBe(true);
    expect(destinationGone(good({ agent_alive: false }), true)).toBe(true);
  });
});

// ★(0.14.31 · 성찰 C1 · blocking) 데몬 적재 응답은 소실 없이 GUI 에 닿고, 결측은 값이 아니다.
describe("parseEnqueueReceipt", () => {
  test("queue_entry_id·durable 을 그대로 읽는다", () => {
    expect(parseEnqueueReceipt({ surface_id: 8, queued: true, depth: 1, queue_entry_id: "q-1", durable: false }))
      .toEqual({ entryId: "q-1", durable: false });
    expect(parseEnqueueReceipt({ queue_entry_id: 17, durable: true })).toEqual({ entryId: "17", durable: true });
  });
  test("결측·비객체·비불리언은 null(구 데몬 · 말할 것 없음)", () => {
    expect(parseEnqueueReceipt({})).toEqual({ entryId: null, durable: null });
    expect(parseEnqueueReceipt(null)).toEqual({ entryId: null, durable: null });
    expect(parseEnqueueReceipt(undefined)).toEqual({ entryId: null, durable: null });
    expect(parseEnqueueReceipt("ok")).toEqual({ entryId: null, durable: null });
    expect(parseEnqueueReceipt({ queue_entry_id: "", durable: "yes" })).toEqual({ entryId: null, durable: null });
  });
});

describe("handoff ack contract", () => {
  test("인수 파일은 인계 문서 옆 같은 이름 + .received", () => {
    expect(handoffAckPath("/p/_round/handoffs/transfer-3-1.md")).toBe("/p/_round/handoffs/transfer-3-1.md.received");
  });
  test("지시문이 인계 문서와 인수 파일 쓰기를 모두 요구한다(원본 종료의 유일한 신호)", () => {
    const t = handoffInstruction("reviewer-codex", "/p/h.md");
    expect(t).toContain("reviewer-codex");
    expect(t).toContain("/p/h.md 를 읽고");
    expect(t).toContain("/p/h.md.received");
    expect(t).toContain('"received"');
    expect(t).toContain("쓰지 않으면 원본은 보존된다");
  });
});

// ★원본 종료 결정 — 인수 확인 ∧ 좌석 수신 가능. 실패 방향은 언제나 '전출 안 함'.
describe("originCloseVerdict", () => {
  const facts = (o: Partial<Parameters<typeof originCloseVerdict>[0]> = {}) =>
    originCloseVerdict({
      row: good(),
      rowsObserved: true,
      wantRole: "reviewer-codex",
      wantAgent: "codex",
      entryId: "q-1",
      durable: true,
      acked: false,
      ...o,
    });
  // ★C1 검체: 내구 미확정 응답에서 원본을 닫지 않는다 — 사유에 durable:false 가 실린다.
  test("C1: durable:false + 인수 없음 → 닫지 않음(보류 · 사유에 내구 미확정)", () => {
    const v = facts({ durable: false });
    expect(v.close).toBe(false);
    if (!v.close) {
      expect(v.hold).toBe("not-acked");
      expect(v.gone).toBe(false);
      expect(v.note).toContain("durable:false");
    }
  });
  test("C1: 인수 확인은 내구보다 강한 사실(소비) — durable:false 여도 인수가 있으면 닫는다", () => {
    expect(facts({ durable: false, acked: true })).toEqual({ close: true });
  });
  test("구 데몬(entryId null)은 인수 없이는 닫지 않고 사유에 적재 미확인이 실린다", () => {
    const v = facts({ entryId: null, durable: null });
    expect(v.close).toBe(false);
    if (!v.close) expect(v.note).toContain("적재 미확인");
  });
  // ★C2 검체: awakened_at:null + agent_alive:true 인 리뷰어 행 — 인수 전에는 close 0, 인수 뒤에는
  //   닫힌다(영구 보류가 아니다). 각성 래치는 결정에 관여하지 않는다.
  test("C2: 리뷰어 행(awakened_at:null · agent_alive:true)은 인수 전엔 close 0", () => {
    const v = facts({ row: good({ awakened_at: null, agent_alive: true }) });
    expect(v.close).toBe(false);
    if (!v.close) expect(v.hold).toBe("not-acked");
  });
  test("C2: 같은 리뷰어 행이 인수하면 닫힌다 — 영구 보류가 아닌 결말", () => {
    expect(facts({ row: good({ awakened_at: null, agent_alive: true }), acked: true })).toEqual({ close: true });
  });
  test("인수가 있어도 좌석이 수신 가능 상태가 아니면 닫지 않는다(닫기 직전 재평가)", () => {
    const dead = facts({ acked: true, row: good({ agent_alive: false }) });
    expect(dead.close).toBe(false);
    if (!dead.close) {
      expect(dead.hold).toBe("destination");
      expect(dead.gone).toBe(true);
    }
    const lost = facts({ acked: true, row: good({ role: null }) });
    expect(lost.close).toBe(false);
    if (!lost.close) expect(lost.gone).toBe(false);
    const unknown = facts({ acked: true, row: good({ agent_alive: null }) });
    expect(unknown.close).toBe(false);
    if (!unknown.close) expect(unknown.gone).toBe(false); // 모름 ≠ 소멸(기록 보존 · 새 기동 금지)
  });
  test("관측 실패는 보류이고 소멸이 아니다", () => {
    const v = facts({ acked: true, row: undefined, rowsObserved: false });
    expect(v.close).toBe(false);
    if (!v.close) {
      expect(v.hold).toBe("destination");
      expect(v.gone).toBe(false);
      expect(v.note).toContain("관측하지 못했다");
    }
    const gone = facts({ acked: true, row: undefined, rowsObserved: true });
    expect(gone.close).toBe(false);
    if (!gone.close) expect(gone.gone).toBe(true);
  });
});

// ★C2 검체: 120s 내 재시도가 pane 을 늘리지 않는다 — 인수 대기 중인 기록이 있으면 관측만 재개한다.
describe("transferRetryAction", () => {
  const rec = (o: Partial<TransferRecord> = {}): TransferRecord => ({
    state: "awaiting-ack",
    destSocket: "/tmp/dept-2.sock",
    destSid: 8,
    launcherSid: 7,
    handoffPath: "/p/h.md",
    role: "reviewer-codex",
    wantAgent: "codex",
    entryId: "q-1",
    durable: true,
    sinceMs: 1000,
    ...o,
  });
  test("기록 없음 → 새 전출(fresh)", () => {
    expect(transferRetryAction(undefined, "/tmp/dept-2.sock")).toBe("fresh");
  });
  test("진행 중 → busy(중복 클릭 무동작)", () => {
    expect(transferRetryAction(rec({ state: "in-progress" }), "/tmp/dept-2.sock")).toBe("busy");
  });
  test("같은 부서로 재시도 → resume(기동 0 · 적재 0 · 관측만)", () => {
    expect(transferRetryAction(rec(), "/tmp/dept-2.sock")).toBe("resume");
  });
  test("다른 부서로 재시도 → other-destination(앞선 인수 대기를 먼저 매듭)", () => {
    expect(transferRetryAction(rec(), "/tmp/dept-3.sock")).toBe("other-destination");
    expect(transferRetryAction(rec({ destSocket: undefined }), "/tmp/dept-3.sock")).toBe("other-destination");
  });
});

// ★배선 핀 — main.ts 의 전출 흐름이 이 계약을 실제로 쓴다(순수 함수만 초록이고 배선이 종전이면 무의미).
//   ★두 함수를 **따로** 잰다: `transferCrossDept`(적재까지)와 `awaitHandoffAck`(인수 확인 → 원본 종료).
//   한 덩어리로 재면 셸 pane 의 정당한 원본 정리("인계할 맥락이 없다" — 같은 경로의 새 셸을 이미 만들었다)와
//   에이전트의 인계 종료가 한 이름으로 섞여, 둘 중 어느 쪽이 어디에 있는지를 이 핀이 말할 수 없게 된다.
describe("transferCrossDept wiring (source pin)", () => {
  const MAIN_URL = new URL("./main.ts", import.meta.url);
  const src = readFileSync(MAIN_URL, "utf-8");
  const a = src.indexOf("async function transferCrossDept(");
  const m = src.indexOf("/** 같은 원본 pane 의 진행 중 전출 기록");
  const c = src.indexOf("async function awaitHandoffAck(");
  const b = src.indexOf("/// sid pane을 트리에서 떼어 target pane의 side 쪽에 분할 삽입한다.");
  const xfer = src.slice(a, m); // 전출 본체 — 목적지 기동부터 큐 적재까지
  const ack = src.slice(c, b); // 인수 확인 대기 → 원본 종료 결정
  const region = src.slice(a, b); // 둘 다 — "어디에도 없다" 를 재는 축
  const CLOSE_ORIGIN = 'invoke("close_surface", { socket: srcSock, surfaceId: sid })';
  const count = (hay: string, needle: string) => hay.split(needle).length - 1;
  test("두 함수가 이 순서로 존재한다", () => {
    expect(a).toBeGreaterThan(0);
    expect(m).toBeGreaterThan(a);
    expect(c).toBeGreaterThan(m);
    expect(b).toBeGreaterThan(c);
  });
  test("C1: 큐 적재는 정확히 한 번이고 그 응답을 영수증으로 읽는다(중복 enqueue 0 · 응답 소실 0)", () => {
    expect(count(xfer, "queued: true")).toBe(1);
    expect(xfer).toContain("parseEnqueueReceipt(");
  });
  test("C1·C2: 에이전트 전출은 적재 뒤 곧바로 인수 확인으로 넘어가고 **여기서** 원본을 닫지 않는다", () => {
    const handOff = xfer.indexOf("await awaitHandoffAck(sid, srcWs, destWs, rec, recKey);");
    expect(handOff).toBeGreaterThan(0);
    // 이 함수에 남은 단 하나의 원본 종료는 **셸 pane** 의 것이고, 에이전트 경로는 그 앞에서 return 한다.
    expect(count(xfer, CLOSE_ORIGIN)).toBe(1);
    expect(xfer.indexOf(CLOSE_ORIGIN)).toBeGreaterThan(handOff);
    expect(xfer.slice(handOff, xfer.indexOf(CLOSE_ORIGIN))).toContain("return;");
  });
  test("C1·C2: 원본 종료는 originCloseVerdict 가 close 를 낸 **뒤에만** 있다(한 자리)", () => {
    expect(ack).toContain("originCloseVerdict(");
    expect(count(ack, CLOSE_ORIGIN)).toBe(1);
    expect(ack.indexOf(CLOSE_ORIGIN)).toBeGreaterThan(ack.indexOf("originCloseVerdict("));
    expect(ack.slice(0, ack.indexOf(CLOSE_ORIGIN))).toContain("if (verdict.close) {");
  });
  test("C2: 각성 래치 술어는 이 흐름 어디에도 없다", () => {
    expect(region).not.toContain("destinationLooksAwake(");
    // 속성 접근으로만 잰다(주석의 언급은 허용 — 왜 안 보는지를 적어 두는 것이 이 수정의 일부다).
    expect(region).not.toContain(".awakened_at");
    expect(region).not.toContain("awakened_at !=");
  });
  test("C2: 인수 확인 파일 · 지시문 · 전출 기록 · 재시도 계획이 배선돼 있다", () => {
    expect(ack).toContain("handoffAckPath(");
    expect(xfer).toContain("handoffInstruction(");
    expect(xfer).toContain("transferRetryAction(");
    expect(region).toContain("transfersInFlight");
  });
  test("보류 토스트는 한 자리(한 번)다", () => {
    expect(count(region, '"전출 보류"')).toBe(1);
  });
});
