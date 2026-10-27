// ctxrender.ts 행동 시험 (bun test · 신규 의존성 0) — P11 / C-30.
//
// 설계 P11-DESIGN-R2 §1.3 (R3 §2.1 로 문면 교정) 의 시험 목록 그대로다. 핵심은 **payload 별 진실**:
// 좌석 배지(surface.list)와 함대 행(control.dashboard)에는 status/가드 필드가 **없으므로**, 그 화면의
// 글에 자기보고·가드·보호 이야기가 섞이면 그것은 거짓이다. 아래 금지낱말 시험이 그것을 못박는다.
import { expect, test } from "bun:test";
import { badgeCtxPart, fleetCtxCell, barCtxNoValue } from "./ctxrender";
import { pickCtx, isHotCtx } from "./ctxpick";

// 이 두 화면이 **절대** 입에 담아서는 안 되는 낱말(자기 payload 에 근거가 없다).
const FORBIDDEN = ["자기보고", "self-report", "가드", "guard", "보호", "protection", "60%"];
const noForbidden = (s: string) => {
  for (const w of FORBIDDEN) expect(s).not.toContain(w);
};

// ── 1. 함대 행 (control.dashboard) ────────────────────────────────────────────
const FLEET_ROW_NULL = {
  surface_id: 94, role: "reviewer-gemini", agent: "gemini", state: "working",
  idle_secs: 3, agent_status: "working",
  usage: { ctx_pct: null, rate: [{ label: "5h", used_pct: 3.55, resets_at: null }] },
};

test("함대 행: 실측값이 null 이면 CTX — 를 그리고, 자기보고·가드·보호를 말하지 않는다", () => {
  const c = fleetCtxCell(FLEET_ROW_NULL);
  expect(c).not.toBeNull();
  expect(c!.text).toBe("CTX —");
  expect(c!.title).toBe("컨텍스트 실측값 없음 (이 화면은 실측값만 싣는다)");
  noForbidden(c!.text);
  noForbidden(c!.title);
});

test("함대 행은 자기보고를 볼 수 없다 — 같은 좌석이 org.status 에서 64 를 신고해도 CTX — 다", () => {
  // codex r1 재현 쌍: control.dashboard 에는 status 가 실리지 않는다.
  expect(fleetCtxCell(FLEET_ROW_NULL)!.text).toBe("CTX —");
  // 같은 좌석의 org.status 행(자기보고 64, 신선) — Tasks 막대는 ≈64% 를 그린다.
  const orgRow = { usage: { ctx_pct: null }, status: { context_pct: 64, age_secs: 12 } };
  expect(pickCtx(orgRow)).toEqual({ pct: 64, src: "self" });
});

// ── 2. 좌석 배지 (surface.list) ───────────────────────────────────────────────
test("배지: ctx_pct null 이면 중립 CTX — + 실측값 없음 툴팁(출처 포함)", () => {
  const p = badgeCtxPart({ ctx_pct: null, source: "statusline" });
  expect(p).not.toBeNull();
  expect(p!.text).toBe("CTX —");
  expect(p!.cls).toBe("unmeasured");          // 절대 warn/crit 이 아니다
  expect(p!.tip).toBe("컨텍스트 실측값 없음 (이 행에 실린 usage.ctx_pct = null · source statusline)");
  noForbidden(p!.tip!);
});

test("배지: source 가 :stale 로 끝나면 낡은 매핑임을 덧붙인다", () => {
  const p = badgeCtxPart({ ctx_pct: null, source: "transcript:heuristic:stale" });
  expect(p!.tip).toBe(
    "컨텍스트 실측값 없음 (이 행에 실린 usage.ctx_pct = null · source transcript:heuristic:stale) · 낡은 매핑",
  );
});

test("배지: usage 자체가 없으면 아무것도 그리지 않는다(종료 좌석의 호출자 경로)", () => {
  expect(badgeCtxPart(null)).toBeNull();
  expect(badgeCtxPart(undefined)).toBeNull();
});

// ── 3. Tasks 막대 (org.status) ────────────────────────────────────────────────
test("Tasks 막대: none 은 빈 칸이 아니라 — 이고, 문면은 '현재 쓸 수 있는 값 없음'이다", () => {
  const n = barCtxNoValue("none");
  expect(n).not.toBeNull();
  expect(n!.value).toBe("—");
  expect(n!.title).toBe("현재 쓸 수 있는 컨텍스트 값 없음");
});

test("Tasks 막대: stale 은 종전대로 ? 이고 문면이 바뀌지 않는다(두 상태를 섞지 않는다)", () => {
  const s = barCtxNoValue("stale");
  expect(s!.value).toBe("?");
  expect(s!.title).toContain("판정 불가");
  expect(s!.value).not.toBe(barCtxNoValue("none")!.value);
});

test("Tasks 막대 축: 신선 자기보고 / 낡음 / 사망 / 없음 네 갈래(pickCtx 불변 확인)", () => {
  expect(pickCtx({ usage: { ctx_pct: null }, status: { context_pct: 64, age_secs: 12 } }).src).toBe("self");
  expect(pickCtx({ usage: { ctx_pct: null }, status: { context_pct: 64, age_secs: 301 } }).src).toBe("stale");
  expect(pickCtx({ agent_alive: false, usage: { ctx_pct: 82 } }).src).toBe("stale");
  expect(pickCtx({ usage: { ctx_pct: null }, status: null }).src).toBe("none");
});

// ── 4. claude 양성 대조 — 측정되는 좌석의 경보는 그대로여야 한다 ──────────────
test("양성 대조: 실측 64 는 배지·함대에서 64% warn 이고 isHotCtx 가 참이다", () => {
  const b = badgeCtxPart({ ctx_pct: 64, source: "statusline" });
  expect(b!.text).toBe("CTX 64%");
  expect(b!.cls).toBe("warn");
  expect(fleetCtxCell({ usage: { ctx_pct: 64 } })!.text).toBe("CTX 64%");
  expect(isHotCtx({ usage: { ctx_pct: 64 } })).toBe(true);
});

test("양성 대조: 실측 0 은 '없음'이 아니라 0% 다", () => {
  expect(badgeCtxPart({ ctx_pct: 0, source: "statusline" })!.text).toBe("CTX 0%");
  expect(fleetCtxCell({ usage: { ctx_pct: 0 } })!.text).toBe("CTX 0%");
});
