// seatsig.ts 순수 판정 회귀 테스트 + main.ts 배선 핀 (bun test — 신규 의존성 0).
// 0.14.41 · U12(빈 자리를 살아 있음으로 오판 — 표시 정직화) · U4 A5①(작업공간 탭 점 조용한 통과).
//
// 근거: _evidence/impl-9items-20260923/phase1/U12-hollow-seat-alive{,.refute}.md (MRC-2·MRC-3·D4·D6·D11)
//       _evidence/impl-9items-20260923/phase1/U4c-silentpass-pack-ui{,.refute}.md (F7·D7)
// 실패 방향(이 파일이 막는 것): 데몬이 '모름'이라고 말한 좌석, 응답이 끊긴 소켓, 셸만 남은 역할 자리가
//   사이드바에 **초록 점**으로 그려지는 것. 반대로 정직화가 기존 3값(working·idle·error)을 망가뜨리거나,
//   agent_alive=null 을 '사망'으로 접는 것(M1 3상 계약 파손)도 여기서 먼저 깨진다.
import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import {
  HOLLOW_GRACE,
  HOLLOW_MIN_AGE_SECS,
  HOLLOW_QUIET_SECS,
  SIG_POLL_MS,
  SIG_STALE_MS,
  STATUS_STATE_FRESH_SECS,
  ceoActiveFromSigs,
  ceoSigActivity,
  hollowGraceScaled,
  isHollowSeat,
  isUnregisteredRoleSeat,
  seatState,
  selfReportFresh,
  summarizeWsSigs,
  summarizeWsSigsSafe,
  taskSeatIsWorking,
  taskSeatView,
  wsSubBits,
  type SeatSig,
} from "./seatsig";

const NOW = 1_800_000_000_000;
/** 신선한(방금 받은) 산 좌석 신호 — 필요한 축만 덮어쓴다. */
const sig = (o: Partial<SeatSig> = {}): SeatSig => ({
  role: "worker",
  state: "working",
  ctx_pct: 10,
  idle_secs: 1,
  agent_alive: true,
  hollow: false,
  at: NOW - 1_000,
  ...o,
});
/** org.status surfaces[] 한 행(데몬 handlers.rs org.status 가 싣는 키만). 기본값 = 빈 자리(10분 전에 생긴 좌석). */
const row = (o: Record<string, unknown> = {}) => ({
  surface_id: 7,
  role: "master",
  exited: false,
  agent: "claude",
  agent_alive: null,
  seat: "empty",
  idle_secs: HOLLOW_QUIET_SECS + 40,
  status: null,
  created_at: NOW / 1000 - 600,
  ...o,
});
/** 테스트 기본 시각으로 판정(isHollowSeat 의 now 기본값은 실제 시계라 created_at 하한이 흔들린다). */
const hollow = (n: any, grace = HOLLOW_GRACE) => isHollowSeat(n, NOW, grace);

describe("상수 — 주기·임계는 정본과 같은 값", () => {
  test("낡음 창 = 3×폴링 주기(설계 §3 U4 A5①)", () => {
    expect(SIG_POLL_MS).toBe(10_000);
    expect(SIG_STALE_MS).toBe(3 * SIG_POLL_MS);
  });
  test("자기보고 신선도 = javis_boot_node.STATUS_FRESH_SECS(600)", () => {
    const py = readFileSync(new URL("../../cysjavis-pack/bin/javis_boot_node.py", import.meta.url), "utf-8");
    const m = py.match(/^STATUS_FRESH_SECS\s*=\s*(\d+)/m);
    expect(m).not.toBeNull();
    expect(STATUS_STATE_FRESH_SECS).toBe(Number(m![1]));
  });
});

describe("U4 A5① — 신호 없음·낡음은 초록이 아니라 회색 '미확인'", () => {
  test("첫 성공 전(신호 0건)이면 회색 — 종전은 dead=0·idle=0 이라 초록이었다", () => {
    const s = summarizeWsSigs([undefined, undefined], NOW);
    expect(s.dot).toBe("unknown");
    expect(s.fresh).toBe(0);
    expect(s.unknownN).toBe(2);
    expect(s.lastOkAt).toBeNull();
    expect(wsSubBits(s, "")).toEqual(["2 pane", "상태 수집 전"]);
    expect(s.title).toContain("상태 수집 전");
  });
  test("마지막 성공이 3×주기를 넘으면 회색 — 낡은 ❌·CTX·💤 는 그리지 않는다", () => {
    const old = NOW - SIG_STALE_MS - 1_000;
    const s = summarizeWsSigs(
      [sig({ at: old, agent_alive: false }), sig({ at: old, ctx_pct: 91 }), sig({ at: old, state: "idle" })],
      NOW,
    );
    expect(s.dot).toBe("unknown");
    expect(s.dead).toBe(0);
    expect(s.idleN).toBe(0);
    expect(s.worst).toBe(0);
    const bits = wsSubBits(s, "master-claude");
    expect(bits).toEqual(["3 pane", "master-claude", "상태 미확인"]);
    expect(s.title).toContain(`마지막 성공 ${Math.round((NOW - old) / 1000)}초 전`);
  });
  test("경계: 정확히 3×주기는 아직 신선, 1ms 넘으면 미확인", () => {
    expect(summarizeWsSigs([sig({ at: NOW - SIG_STALE_MS })], NOW).dot).toBe("working");
    expect(summarizeWsSigs([sig({ at: NOW - SIG_STALE_MS - 1 })], NOW).dot).toBe("unknown");
  });
  test("낡음 창은 호출자가 넓힐 수 있다(윈도우 winScaled ×2)", () => {
    const at = NOW - SIG_STALE_MS - 5_000;
    expect(summarizeWsSigs([sig({ at })], NOW).dot).toBe("unknown");
    expect(summarizeWsSigs([sig({ at })], NOW, 2 * SIG_STALE_MS).dot).toBe("working");
  });
  test("시계가 뒤로 뛰어 at 이 창보다 먼 미래면 신선으로 보지 않는다", () => {
    expect(summarizeWsSigs([sig({ at: NOW + SIG_STALE_MS + 1 })], NOW).dot).toBe("unknown");
  });
  test("일부 좌석만 신호가 없으면(자리표시자·폴링 전) 회색 + '?미확인 N'", () => {
    const s = summarizeWsSigs([sig(), undefined, sig()], NOW);
    expect(s.dot).toBe("unknown");
    expect(s.unknownN).toBe(1);
    expect(wsSubBits(s, "")).toEqual(["3 pane", "?미확인 1"]);
  });
  test("pane 0개(신호 0) — 초록으로 그리지 않고, '데몬 응답 없음'이 아니라 '좌석 없음'이라고 쓴다(리뷰1 #5)", () => {
    const s = summarizeWsSigs([], NOW);
    expect(s.dot).toBe("unknown");
    expect(s.title).toContain("좌석 없음");
    expect(s.title).not.toContain("데몬 응답");
    expect(wsSubBits(s, "")).toEqual(["0 pane", "좌석 없음"]);
    expect(wsSubBits(s, "t")).toEqual(["0 pane", "t", "좌석 없음"]);
  });
});

describe("회귀 — 신선한 신호의 종전 3값은 그대로", () => {
  test("전부 산 좌석 → working(초록)", () => {
    const s = summarizeWsSigs([sig(), sig({ ctx_pct: 30 })], NOW);
    expect(s.dot).toBe("working");
    expect(wsSubBits(s, "t")).toEqual(["2 pane", "t"]);
  });
  test("idle(자기보고 idle 또는 60초 초과 무출력) → idle 💤", () => {
    const s = summarizeWsSigs([sig(), sig({ state: "idle" }), sig({ idle_secs: 61 })], NOW);
    expect(s.dot).toBe("idle");
    expect(s.idleN).toBe(2);
    expect(wsSubBits(s, "")).toEqual(["3 pane", "💤2"]);
  });
  test("사망 확정(agent_alive=false) → error ❌ — idle 보다 우선", () => {
    const s = summarizeWsSigs([sig({ state: "idle" }), sig({ agent_alive: false })], NOW);
    expect(s.dot).toBe("error");
    expect(s.dead).toBe(1);
    expect(wsSubBits(s, "")).toContain("❌1");
  });
  test("CTX 60% 이상은 표기, worst 는 신선한 산 좌석의 최댓값", () => {
    const s = summarizeWsSigs([sig({ ctx_pct: 62 }), sig({ ctx_pct: null }), sig({ ctx_pct: 85 })], NOW);
    expect(s.worst).toBe(85);
    expect(wsSubBits(s, "")).toContain("CTX 85%");
  });
});

describe("U12 — 이름표만 남은 빈 자리는 초록이 아니라 ○빈자리", () => {
  test("빈 자리(role·등록 에이전트·agent_alive null·seat empty·무출력 유예 경과) → hollow", () => {
    expect(hollow(row())).toBe(true);
    expect(hollow(row({ agent: "agy", role: "reviewer-gemini" }))).toBe(true);
  });
  test("등록 에이전트가 없는 역할 좌석은 판정하지 않는다 — 루트가 셸이 아닌 pane 의 거짓 빈 자리 차단", () => {
    // `cys new-surface --cmd <watcher> --role cycle-verifier` → zsh -lc 암묵 exec → 루트=워처 · 자손 0 · agent 없음.
    //   살아 일하는 pane 인데 seat=empty 다. 데몬 응답엔 '루트가 셸인가'가 없으므로 UI 는 이 부류를 빈 자리로 부르지 않는다.
    expect(hollow(row({ role: "cycle-verifier", agent: null, idle_secs: 999 }))).toBe(false);
    // 온보딩 CLI 미설치 master 셸(formation new-surface --role master)도 같은 부류 — 종전 표시(반박 D4).
    expect(hollow(row({ agent: null }))).toBe(false);
    expect(hollow(row({ agent: "" }))).toBe(false);
    expect(hollow(row({ agent: undefined }))).toBe(false); // 구 데몬
  });
  test("진리표 — 한 항이라도 어긋나면 빈 자리가 아니다(종전 동작 유지 · 구 데몬 무해)", () => {
    expect(hollow(row({ role: null }))).toBe(false); // 역할 없는 '내 자리' 빈 창
    expect(hollow(row({ role: "" }))).toBe(false);
    expect(hollow(row({ exited: true }))).toBe(false); // pane 종료는 다른 사실
    expect(hollow(row({ agent_alive: true }))).toBe(false); // 산 에이전트 관측(exec 루트 포함)
    expect(hollow(row({ agent_alive: false }))).toBe(false); // 사망 확정 — ❌ 축(섞지 않는다)
    expect(hollow(row({ seat: "occupied" }))).toBe(false); // 셸 아래 프로세스 있음(윈도우 EDR null 포함)
    expect(hollow(row({ seat: "unknown" }))).toBe(false); // 콜드스타트 첫 틱 전
    expect(hollow(row({ seat: undefined }))).toBe(false); // 구 데몬(seat 키 부재)
    expect(hollow(row({ idle_secs: HOLLOW_QUIET_SECS - 1 }))).toBe(false); // 기동 직후·사람 입력 중
    expect(hollow(row({ idle_secs: undefined }))).toBe(false);
    expect(hollow(null)).toBe(false);
    expect(hollow("x")).toBe(false);
  });
  test("빈 자리 좌석이 있으면 초록 금지 — dot hollow · '○빈자리 N' · 툴팁에 역할", () => {
    const s = summarizeWsSigs([sig(), sig({ role: "master", hollow: true, agent_alive: null, state: "working" })], NOW);
    expect(s.dot).toBe("hollow");
    expect(s.hollowN).toBe(1);
    expect(s.hollowRoles).toEqual(["master"]);
    expect(wsSubBits(s, "")).toEqual(["2 pane", "○빈자리 1"]);
    expect(s.title).toContain("빈 자리 1");
    expect(s.title).toContain("master");
  });
  test("빈 자리는 빨간 사망 점이 아니다 — ❌ 와 섞지 않는다(반박 D4·D6)", () => {
    const s = summarizeWsSigs([sig({ hollow: true, agent_alive: null })], NOW);
    expect(s.dot).toBe("hollow");
    expect(s.dead).toBe(0);
    expect(wsSubBits(s, "").join(" ")).not.toContain("❌");
  });
  test("우선순위: 사망 확정 > 빈 자리 > 미확인 > idle > working", () => {
    const H = sig({ hollow: true, agent_alive: null });
    expect(summarizeWsSigs([H, sig({ agent_alive: false })], NOW).dot).toBe("error");
    expect(summarizeWsSigs([H, undefined], NOW).dot).toBe("hollow");
    expect(summarizeWsSigs([undefined, sig({ state: "idle" })], NOW).dot).toBe("unknown");
  });
  test("빈 자리의 동결 CTX·idle 은 집계에 넣지 않는다(낡은 상태 비표시)", () => {
    const s = summarizeWsSigs([sig({ ctx_pct: 20 }), sig({ hollow: true, agent_alive: null, ctx_pct: 95, idle_secs: 999 })], NOW);
    expect(s.worst).toBe(20);
    expect(s.idleN).toBe(0);
  });
  test("hollow 플래그가 붙어도 agent_alive=false 면 사망 축(AgentDead > Hollow)", () => {
    const s = summarizeWsSigs([sig({ hollow: true, agent_alive: false })], NOW);
    expect(s.dot).toBe("error");
    expect(s.hollowN).toBe(0);
    expect(s.dead).toBe(1);
  });
});

describe("U12 — 빈 자리 유예: Windows 배율 · 자기보고 반증 · 좌석 나이 하한(리뷰1 #4)", () => {
  test("상수 — 무출력 20초 · 좌석 나이 60초", () => {
    expect(HOLLOW_QUIET_SECS).toBe(20);
    expect(HOLLOW_MIN_AGE_SECS).toBe(60);
    expect(HOLLOW_GRACE).toEqual({ quietSecs: 20, minAgeSecs: 60 });
  });
  test("hollowGraceScaled — 플랫폼 배율(winScaled ×2)을 두 유예에 같이 건다 · 이상한 배율은 기본값", () => {
    expect(hollowGraceScaled((ms) => ms)).toEqual({ quietSecs: 20, minAgeSecs: 60 });
    expect(hollowGraceScaled((ms) => ms * 2)).toEqual({ quietSecs: 40, minAgeSecs: 120 });
    expect(hollowGraceScaled(() => NaN)).toEqual({ quietSecs: 20, minAgeSecs: 60 });
    expect(hollowGraceScaled(() => 0)).toEqual({ quietSecs: 20, minAgeSecs: 60 }); // 좁히는 배율은 받지 않는다
  });
  test("Windows 배율 유예: 무출력 30초는 맥에선 빈 자리, Windows(40초)에선 아직 아니다", () => {
    const win = hollowGraceScaled((ms) => ms * 2);
    expect(hollow(row({ idle_secs: 30 }))).toBe(true);
    expect(hollow(row({ idle_secs: 30 }), win)).toBe(false);
    expect(hollow(row({ idle_secs: 40 }), win)).toBe(true);
  });
  test("방금 온 자기보고(age < 무출력 유예)는 산 에이전트의 반증 — 빈 자리 아님", () => {
    expect(hollow(row({ status: { state: "working", age_secs: 3 } }))).toBe(false);
    expect(hollow(row({ status: { state: "working", age_secs: HOLLOW_QUIET_SECS - 1 } }))).toBe(false);
    expect(hollow(row({ status: { state: "working", age_secs: HOLLOW_QUIET_SECS } }))).toBe(true);
    expect(hollow(row({ status: { state: "working" } }))).toBe(true); // 나이 미상은 반증이 아니다
  });
  test("좌석 나이 하한 — 생긴 지 60초 전이면 빈 자리 아님(느린 셸 초기화) · 경계 · 구 데몬(created_at 부재)", () => {
    expect(hollow(row({ created_at: NOW / 1000 - 10 }))).toBe(false);
    expect(hollow(row({ created_at: NOW / 1000 - (HOLLOW_MIN_AGE_SECS - 1) }))).toBe(false);
    expect(hollow(row({ created_at: NOW / 1000 - HOLLOW_MIN_AGE_SECS }))).toBe(true);
    expect(hollow(row({ created_at: NOW / 1000 + 3600 }))).toBe(false); // 시계 역행(미래) = 모름 → 종전 표시
    expect(hollow(row({ created_at: undefined }))).toBe(true);
    expect(hollow(row({ created_at: "x" }))).toBe(true);
    const win = hollowGraceScaled((ms) => ms * 2);
    expect(hollow(row({ created_at: NOW / 1000 - 90 }), win)).toBe(false); // Windows 하한 120초
  });
  test("기본 인자(now = 실제 시계)로도 판정한다 — 오래전에 생긴 좌석", () => {
    expect(isHollowSeat(row({ created_at: Date.now() / 1000 - 3600 }))).toBe(true);
  });
});

describe("U12 — 등록 에이전트 없는 역할 좌석은 '미등록 N' 중립 표식(리뷰1 #3 · 반박 D4)", () => {
  test("진리표 — role 있음 ∧ agent 없음 ∧ 살아 있음 ∧ agent_alive≠true ∧ seat empty", () => {
    expect(isUnregisteredRoleSeat(row({ agent: null }))).toBe(true);
    expect(isUnregisteredRoleSeat(row({ agent: "", role: "cycle-verifier" }))).toBe(true);
    expect(isUnregisteredRoleSeat(row({ agent: undefined }))).toBe(true);
    expect(isUnregisteredRoleSeat(row())).toBe(false); // 등록 에이전트 있음 → 빈 자리 축(isHollowSeat)
    expect(isUnregisteredRoleSeat(row({ agent: null, role: null }))).toBe(false); // 역할 없는 '내 자리'
    expect(isUnregisteredRoleSeat(row({ agent: null, exited: true }))).toBe(false);
    expect(isUnregisteredRoleSeat(row({ agent: null, agent_alive: true }))).toBe(false); // exec 루트 관측
    expect(isUnregisteredRoleSeat(row({ agent: null, seat: "occupied" }))).toBe(false); // 손으로 띄운 에이전트 등
    expect(isUnregisteredRoleSeat(row({ agent: null, seat: "unknown" }))).toBe(false);
    expect(isUnregisteredRoleSeat(null)).toBe(false);
  });
  test("빈 자리와 겹치지 않는다(같은 행이 둘 다 참일 수 없다)", () => {
    for (const agent of ["claude", null])
      expect(hollow(row({ agent })) && isUnregisteredRoleSeat(row({ agent }))).toBe(false);
  });
  test("점 색은 그대로 · 둘째 줄 '미등록 N' · 툴팁에 역할과 이유", () => {
    const s = summarizeWsSigs([sig(), sig({ role: "cycle-verifier", unregistered: true })], NOW);
    expect(s.dot).toBe("working");
    expect(s.unregN).toBe(1);
    expect(s.unregRoles).toEqual(["cycle-verifier"]);
    expect(wsSubBits(s, "")).toEqual(["2 pane", "미등록 1"]);
    expect(s.title).toContain("미등록 1");
    expect(s.title).toContain("cycle-verifier");
    expect(s.title).toContain("판정하지 않습니다");
    // idle 은 종전대로 idle(점 색 규칙 불변)
    expect(summarizeWsSigs([sig({ unregistered: true, state: "idle" })], NOW).dot).toBe("idle");
  });
  test("낡은 신호의 미등록은 세지 않는다", () => {
    const s = summarizeWsSigs([sig(), sig({ unregistered: true, at: NOW - SIG_STALE_MS - 1 })], NOW);
    expect(s.unregN).toBe(0);
  });
});

describe("U12 — CEO 활성 판정 루프(ceoActiveFromSigs · 리뷰1 #1 M24)", () => {
  const OLD = NOW - SIG_STALE_MS - 1;
  test("CEO 신호 없음 → false(즉시 사람 소환)", () => {
    expect(ceoActiveFromSigs([], NOW, SIG_STALE_MS)).toBe(false);
    expect(ceoActiveFromSigs([sig({ role: "worker" })], NOW, SIG_STALE_MS)).toBe(false);
    expect(ceoActiveFromSigs([null, undefined], NOW, SIG_STALE_MS)).toBe(false);
  });
  test("낡은 CEO 신호만 있으면 false — 낡은 신호가 유예를 끌지 않는다", () => {
    expect(ceoActiveFromSigs([sig({ role: "ceo", at: OLD })], NOW, SIG_STALE_MS)).toBe(false);
  });
  test("낡은 CEO 는 건너뛰고 다음 신선한 CEO 로 판정", () => {
    expect(ceoActiveFromSigs([sig({ role: "ceo", at: OLD }), sig({ role: "ceo" })], NOW, SIG_STALE_MS)).toBe(true);
    expect(
      ceoActiveFromSigs([sig({ role: "ceo", at: OLD }), sig({ role: "ceo", idle_secs: 999, state: "idle" })], NOW, SIG_STALE_MS),
    ).toBe(false);
  });
  test("빈 자리 CEO 는 비활성 · 산 CEO working 은 활성 · Map.values() 를 그대로 받는다", () => {
    expect(ceoActiveFromSigs([sig({ role: "ceo", hollow: true, agent_alive: null })], NOW, SIG_STALE_MS)).toBe(false);
    const m = new Map<string, SeatSig>([["a#1", sig({ role: "ceo" })]]);
    expect(ceoActiveFromSigs(m.values(), NOW, SIG_STALE_MS)).toBe(true);
  });
  test("낡음 창은 호출자가 넓힐 수 있다(Windows)", () => {
    const at = NOW - SIG_STALE_MS - 5_000;
    expect(ceoActiveFromSigs([sig({ role: "ceo", at })], NOW, SIG_STALE_MS)).toBe(false);
    expect(ceoActiveFromSigs([sig({ role: "ceo", at })], NOW, 2 * SIG_STALE_MS)).toBe(true);
  });
});

describe("U12 — Control Center Tasks 행도 같은 규칙(taskSeatView · 리뷰1 #2)", () => {
  const V = (o: Record<string, unknown>) => taskSeatView(row(o), NOW, HOLLOW_GRACE);
  test("selfReportFresh — 숫자 age ≤ 600 만 신선", () => {
    expect(selfReportFresh({ state: "working", age_secs: 0 })).toBe(true);
    expect(selfReportFresh({ state: "working", age_secs: STATUS_STATE_FRESH_SECS })).toBe(true);
    expect(selfReportFresh({ state: "working", age_secs: STATUS_STATE_FRESH_SECS + 1 })).toBe(false);
    expect(selfReportFresh({ state: "working" })).toBe(false);
    expect(selfReportFresh(null)).toBe(false);
  });
  test("좌석: 종료 > 빈 자리 > 산 좌석", () => {
    expect(V({ exited: true })).toEqual({ seat: "offline", report: "none" });
    expect(V({})).toEqual({ seat: "hollow", report: "none" });
    expect(V({ seat: "occupied", agent_alive: true })).toEqual({ seat: "live", report: "none" });
    expect(V({ agent: null })).toEqual({ seat: "live", report: "none" }); // 무메타는 빈 자리로 부르지 않는다
  });
  test("자기보고: 신선 · 낡음 · 나이 미상(=낡음) · 없음", () => {
    const live = { seat: "occupied", agent_alive: true };
    expect(V({ ...live, status: { state: "working", age_secs: 30 } }).report).toBe("fresh");
    expect(V({ ...live, status: { state: "working", age_secs: 5000 } }).report).toBe("stale");
    expect(V({ ...live, status: { state: "working" } }).report).toBe("stale");
    expect(V({ ...live, status: null }).report).toBe("none");
  });
  test("'작업중' 계수 — 낡은 working 자기보고는 출력 기준 · 빈 자리·종료는 0", () => {
    const live = { seat: "occupied", agent_alive: true };
    const w = (o: Record<string, unknown>) => {
      const r = row({ ...live, ...o });
      return taskSeatIsWorking(r, taskSeatView(r, NOW, HOLLOW_GRACE));
    };
    expect(w({ status: { state: "working", age_secs: 10 }, idle_secs: 999 })).toBe(true);
    expect(w({ status: { state: "waiting", age_secs: 10 }, idle_secs: 0 })).toBe(false);
    expect(w({ status: { state: "working", age_secs: 5000 }, idle_secs: 999 })).toBe(false); // 종전: 영구 작업중
    expect(w({ status: { state: "working", age_secs: 5000 }, idle_secs: 5 })).toBe(true);
    expect(w({ status: null, idle_secs: 60 })).toBe(true);
    expect(w({ status: null, idle_secs: undefined })).toBe(false);
    expect(w({ exited: true, status: { state: "working", age_secs: 1 } })).toBe(false);
    const h = row({ status: { state: "working", age_secs: 100 } });
    expect(taskSeatIsWorking(h, taskSeatView(h, NOW, HOLLOW_GRACE))).toBe(false); // 빈 자리
  });
});

describe("U12 — 자기보고 state 는 신선할 때만(MRC-2 낡은 working 영구 초록)", () => {
  test("낡은 자기보고(age > 600초)는 쓰지 않고 출력 기반 산식으로", () => {
    expect(seatState({ status: { state: "working", age_secs: STATUS_STATE_FRESH_SECS + 1 }, idle_secs: 300 })).toBe("idle");
    expect(seatState({ status: { state: "working", age_secs: 5000 }, idle_secs: 3 })).toBe("working");
  });
  test("신선한 자기보고는 그대로", () => {
    expect(seatState({ status: { state: "blocked", age_secs: 10 }, idle_secs: 300 })).toBe("blocked");
    expect(seatState({ status: { state: "idle", age_secs: STATUS_STATE_FRESH_SECS }, idle_secs: 0 })).toBe("idle");
  });
  test("나이 미상은 '방금'이 아니라 '모른다' — 산식으로", () => {
    expect(seatState({ status: { state: "working" }, idle_secs: 120 })).toBe("idle");
  });
  test("자기보고 없음 → 종전 산식(idle_secs > 60 → idle)", () => {
    expect(seatState({ status: null, idle_secs: 61 })).toBe("idle");
    expect(seatState({ status: null, idle_secs: 60 })).toBe("working");
  });
});

describe("U12 — CEO 활성 판정(승인 전환 유예)이 빈 자리·낡은 신호를 '생성 중'으로 읽지 않는다", () => {
  test("산 CEO 가 working → 활성", () => {
    expect(ceoSigActivity(sig({ role: "ceo" }), NOW)).toBe(true);
  });
  test("agent_alive=null(모름)이고 빈 자리가 아니면 종전대로 활성 가능(M1: null ≠ 사망)", () => {
    expect(ceoSigActivity(sig({ role: "ceo", agent_alive: null }), NOW)).toBe(true);
  });
  test("빈 자리 CEO 는 활성 아님 → 사람에게 전환(보여 주는 쪽으로 실패)", () => {
    expect(ceoSigActivity(sig({ role: "ceo", agent_alive: null, hollow: true, idle_secs: 1 }), NOW)).toBe(false);
  });
  test("낡은 신호는 판정에 쓰지 않는다(null = 건너뜀)", () => {
    expect(ceoSigActivity(sig({ role: "ceo", at: NOW - SIG_STALE_MS - 1 }), NOW)).toBeNull();
  });
  test("사망 확정 CEO 는 활성 아님(종전 유지)", () => {
    expect(ceoSigActivity(sig({ role: "ceo", agent_alive: false }), NOW)).toBe(false);
  });
});

describe("④ 방어 — 렌더 경로의 판정 예외가 탭 바를 멈추지 않는다", () => {
  test("이상한 입력에도 throw 없이 회색 요약을 낸다", () => {
    const weird: any[] = [null, 0, "x", { at: "soon" }, { at: NaN }, Object.create(null)];
    expect(() => summarizeWsSigsSafe(weird, NOW, SIG_STALE_MS)).not.toThrow();
    expect(summarizeWsSigsSafe(weird, NOW, SIG_STALE_MS).dot).toBe("unknown");
    expect(summarizeWsSigsSafe(null as any, NOW, SIG_STALE_MS).dot).toBe("unknown");
  });
});

// ---------- 배선 핀: main.ts 가 이 한 벌을 실제로 쓰는가 ----------
// (wswiring.test.ts 관례 — 순수 함수가 옳아도 본체가 부르지 않으면 화면은 그대로다.)
const src = readFileSync(new URL("./main.ts", import.meta.url), "utf-8");
const css = readFileSync(new URL("./style.css", import.meta.url), "utf-8");
const code = src
  .split("\n")
  .map((l) => (l.trimStart().startsWith("//") ? "" : l.replace(/\s\/\/.*$/, "")))
  .join("\n");
/** 바늘은 조각으로 조립한다 — 이 파일 자체가 소스 grep 에 옛 형태로 잡히지 않게. */
const j = (...p: string[]) => p.join("");

/** 공백을 지운 비교 — 줄바꿈·들여쓰기 변경엔 둔감, 인자·호출형 변경엔 민감. */
const ns = (x: string) => x.replace(/\s+/g, "");
function sliceBody(start: string, end: string): string {
  const a = code.indexOf(start);
  expect(a).toBeGreaterThan(0);
  const b = code.indexOf(end, a + start.length);
  expect(b).toBeGreaterThan(a);
  return code.slice(a, b);
}

describe("배선 핀 — main.ts", () => {
  test("seatsig 를 들여온다", () => {
    expect(/from "\.\/seatsig"/.test(code)).toBe(true);
  });
  test("폴링 주기 리터럴 = SIG_POLL_MS(바꾸면 둘 다)", () => {
    expect(code.includes(j("setInterval(refreshSidebarStatus, ", String(SIG_POLL_MS), ")"))).toBe(true);
  });
  test("플랫폼 배율 창 두 개 — 낡음 창·빈 자리 유예 모두 winScaled", () => {
    expect(code.includes("const SIG_STALE_MS_UI = winScaled(SIG_STALE_MS);")).toBe(true);
    expect(code.includes("const HOLLOW_GRACE_UI = hollowGraceScaled(winScaled);")).toBe(true);
  });
  test("nodeSig 기록: 신선한 자기보고만 · 빈 자리(시각·배율 유예) · 미등록 · 수신 시각", () => {
    const body = ns(sliceBody("async function refreshSidebarStatus", "function updatePendingBadges"));
    expect(body.includes(ns("state: seatState(n),"))).toBe(true);
    expect(body.includes(ns("hollow: isHollowSeat(n, Date.now(), HOLLOW_GRACE_UI),"))).toBe(true);
    expect(body.includes(ns("unregistered: isUnregisteredRoleSeat(n),"))).toBe(true);
    expect(body.includes(ns("at: Date.now(),"))).toBe(true);
    // 옛 형태: 자기보고 state 를 나이와 무관하게 쓰던 산식
    expect(code.includes(j("n.status?.state ?", "? (n.idle_secs"))).toBe(false);
  });
  test("탭 점: 한 벌 판정(summarizeWsSigsSafe) — 트리 좌석 신호 · 현재 시각 · **Windows 배율 낡음 창** 정확히", () => {
    const body = sliceBody("function buildTab(", "tab.append(titleRow, sub)");
    expect(
      ns(body).includes(
        ns("summarizeWsSigsSafe(sids.map((id) => nodeSig.get(`${ws.socket}#${id}`)), Date.now(), SIG_STALE_MS_UI,);"),
      ),
    ).toBe(true);
    expect(body.includes("wsSubBits(")).toBe(true);
    expect(code.includes(j('(dead ? "error" : idleN ? "idle" : ', '"working")'))).toBe(false);
  });
  test("CEO 활성 판정: 신호 재갱신 뒤 ceoActiveFromSigs(nodeSig, 지금, 배율 창) 한 줄", () => {
    const body = sliceBody("async function ceoIsActivelyGenerating", "\n}\n");
    expect(ns(body)).toBe(
      ns(
        "async function ceoIsActivelyGenerating(): Promise<boolean> {\n  await refreshSidebarStatus().catch(() => {});\n  return ceoActiveFromSigs(nodeSig.values(), Date.now(), SIG_STALE_MS_UI);",
      ),
    );
    expect(src.includes(j("const alive = sig.agent_alive !", "== false;"))).toBe(false);
  });
  test("CC Tasks 행·부서 머리: taskSeatView 한 벌(빈 자리 · 낡은 자기보고)", () => {
    const row = sliceBody("function taskRow(", "\nfunction ");
    expect(row.includes("const view = taskSeatView(s, Date.now(), HOLLOW_GRACE_UI);")).toBe(true);
    expect(row.includes('const freshReport = view.report === "fresh";')).toBe(true);
    expect(ns(row).includes(ns('} else if (view.seat === "hollow") {\n    cls = "hollow";\n    label = "빈 자리";'))).toBe(true);
    expect(ns(row).includes(ns("} else if (freshReport) {"))).toBe(true);
    expect(row.includes('(view.report === "stale" ? " (자기보고 낡음)" : "")')).toBe(true);
    expect(ns(row).includes(ns("const trust = freshReport"))).toBe(true);
    // 옛 형태: 자기보고가 있기만 하면(나이 무관) 라벨·신뢰 배지로 쓰던 분기
    expect(ns(row).includes(ns("} else if (selfReport) {"))).toBe(false);
    expect(ns(row).includes(ns("const trust = selfReport"))).toBe(false);
    const tasks = ns(sliceBody("function renderTasks(", "\nfunction taskRow("));
    expect(tasks.includes(ns("taskSeatIsWorking(s, taskSeatView(s, tNow, HOLLOW_GRACE_UI))"))).toBe(true);
  });
  test("새 pane·입양 직후 신호를 1회 당겨 온다(회색 깜빡임 차단 · 리뷰1 #6)", () => {
    const ns1 = sliceBody("async function newSurface(", "\n}\n");
    expect(ns1.includes("void refreshSidebarStatus().catch(() => {});")).toBe(true);
    const adopt = sliceBody("async function refreshPaneTitles", "setInterval(refreshPaneTitles, 3000);");
    expect(adopt.includes("if (adopted) void refreshSidebarStatus().catch(() => {});")).toBe(true);
  });
  test("style.css — 회색 미확인·빈 자리 점 클래스가 있다(사이드바·Tasks)", () => {
    expect(/\.ws-dot\.unknown\s*\{/.test(css)).toBe(true);
    expect(/\.ws-dot\.hollow\s*\{/.test(css)).toBe(true);
    expect(/\.cc-dot\.hollow\s*\{/.test(css)).toBe(true);
  });
});
