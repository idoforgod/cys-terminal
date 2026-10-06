// 0.14.44 B5·B6 — 오피스 탭 안내 판정(officetab.ts) 회귀 핀. 순수 모듈이라 DOM·Tauri 불요.
import { describe, it, expect } from "bun:test";
import { readFileSync } from "node:fs";
import {
  planOfficeTab,
  officePollDelay,
  repairOutcomeOf,
  OFFICE_POLL_FAST_MS,
  OFFICE_POLL_SLOW_MS,
  OFFICE_POLL_FAST_WINDOW_MS,
  OFFICE_TEXT_PREPARING,
  OFFICE_TEXT_LONG,
  OFFICE_TEXT_REPAIRING,
  OFFICE_TEXT_REPAIR_FAILED,
  OFFICE_TEXT_ASSETS_BUTTON,
  OFFICE_TEXT_ASSETS_NONE,
  type OfficeCtx,
} from "./officetab";

const ctx = (o: Partial<OfficeCtx> = {}): OfficeCtx => ({ elapsedMs: 0, repairing: false, repairOutcome: "none", ...o });
const healthy = { ok: true, reachable: true, reason: "ok", assets_missing: [], repair: "auto" };
const noAssets = (repair: string) => ({ ok: true, reachable: true, reason: "ok", assets_missing: ["web/vendor/three.module.js"], repair });

describe("확인 간격 — 3분 동안 3초 · 그 뒤 15초", () => {
  it("경계", () => {
    expect(officePollDelay(0)).toBe(OFFICE_POLL_FAST_MS);
    expect(officePollDelay(OFFICE_POLL_FAST_WINDOW_MS - 1)).toBe(OFFICE_POLL_FAST_MS);
    expect(officePollDelay(OFFICE_POLL_FAST_WINDOW_MS)).toBe(OFFICE_POLL_SLOW_MS);
    expect(officePollDelay(NaN)).toBe(OFFICE_POLL_FAST_MS);
    expect([OFFICE_POLL_FAST_MS, OFFICE_POLL_SLOW_MS, OFFICE_POLL_FAST_WINDOW_MS]).toEqual([3000, 15000, 180000]);
  });
});

describe("planOfficeTab — 브리지가 없거나 느릴 때", () => {
  it("응답 없음·호출 실패·이상한 값 → 3분 안은 준비 중(터미널 명령 없음) · 자동으로 다시 확인", () => {
    for (const h of [null, undefined, {}, "x", 3, { ok: false, reason: "no_connect" }, { ok: false, reachable: true, reason: "no_response" }]) {
      const p = planOfficeTab(h, ctx({ elapsedMs: 10_000 }));
      expect({ stage: p.stage, text: p.text, load: p.loadFrame, poll: p.nextPollMs, btn: p.buttonLabel }).toEqual({ stage: "preparing", text: OFFICE_TEXT_PREPARING, load: false, poll: 3000, btn: "" });
    }
  });
  it("3분이 지나도 안 되면 긴 안내(컴퓨터를 다시 시작·피드백) · 15초 간격 · 앱만 다시 켜라고 하지 않는다", () => {
    const p = planOfficeTab({ ok: false }, ctx({ elapsedMs: OFFICE_POLL_FAST_WINDOW_MS }));
    expect({ stage: p.stage, text: p.text, poll: p.nextPollMs }).toEqual({ stage: "long", text: OFFICE_TEXT_LONG, poll: 15000 });
    expect(p.text.includes("컴퓨터를 다시 시작")).toBe(true);
    expect(p.text.includes("피드백")).toBe(true);
    expect(p.text.includes("앱을 다시")).toBe(false);
  });
});

describe("planOfficeTab — 화면을 싣는다", () => {
  it("건강하고 자산이 다 있으면 화면을 싣고 안내를 숨기고 확인을 멈춘다(그 뒤는 화면 안 배너의 몫)", () => {
    const p = planOfficeTab(healthy, ctx({ elapsedMs: 5000 }));
    expect({ stage: p.stage, text: p.text, load: p.loadFrame, poll: p.nextPollMs, auto: p.autoRepair }).toEqual({ stage: "loaded", text: "", load: true, poll: 0, auto: false });
  });
  it("자산을 '모른다'(키 없음·모양 이상)는 없다는 뜻이 아니다 — 화면을 싣는다", () => {
    for (const m of [undefined, null, "x", 5, [1, null, ""]]) expect(planOfficeTab({ ok: true, assets_missing: m }, ctx()).stage).toBe("loaded");
  });
});

describe("planOfficeTab — 화면 자산이 없을 때(B6)", () => {
  it("맥(auto): 처음엔 자동 복구를 시작 · 문구 '복구하고 있습니다' · 안내와 화면은 겹치지 않는다", () => {
    const p = planOfficeTab(noAssets("auto"), ctx());
    expect({ stage: p.stage, text: p.text, auto: p.autoRepair, load: p.loadFrame, btn: p.buttonLabel }).toEqual({ stage: "repairing", text: OFFICE_TEXT_REPAIRING, auto: true, load: false, btn: "" });
  });
  it("복구 중에는 다시 시작하지 않는다(autoRepair=false) · 짧은 간격으로 다시 확인", () => {
    const p = planOfficeTab(noAssets("auto"), ctx({ repairing: true }));
    expect({ stage: p.stage, auto: p.autoRepair, poll: p.nextPollMs }).toEqual({ stage: "repairing", auto: false, poll: 3000 });
  });
  it("실패하면 '복구하지 못했습니다' + [다시 시도] 단추 · 자동으로 또 시작하지 않는다", () => {
    const p = planOfficeTab(noAssets("auto"), ctx({ repairOutcome: "failed" }));
    expect({ stage: p.stage, text: p.text, btn: p.buttonLabel, auto: p.autoRepair }).toEqual({ stage: "repair_failed", text: OFFICE_TEXT_REPAIR_FAILED, btn: "다시 시도", auto: false });
  });
  it("button(정책이 자동을 끔 · 윈도우에서 단추가 켜짐): 안내 + [복구] 단추 · 자동 시작 없음", () => {
    const p = planOfficeTab(noAssets("button"), ctx());
    expect({ stage: p.stage, text: p.text, btn: p.buttonLabel, auto: p.autoRepair }).toEqual({ stage: "assets_button", text: OFFICE_TEXT_ASSETS_BUTTON, btn: "복구", auto: false });
  });
  it("none(윈도우 기본): 단추도 자동도 없이 피드백 안내만", () => {
    for (const repair of ["none", undefined, "weird"]) {
      const p = planOfficeTab(noAssets(repair as string), ctx());
      expect({ stage: p.stage, text: p.text, btn: p.buttonLabel, auto: p.autoRepair }).toEqual({ stage: "assets_none", text: OFFICE_TEXT_ASSETS_NONE, btn: "", auto: false });
    }
  });
  it("앱이 손대지 않는 상태(원장 항목 · 팩 버전 불일치)는 피드백 안내만 — 단추를 내지 않는다", () => {
    const p = planOfficeTab(noAssets("auto"), ctx({ repairOutcome: "blocked" }));
    expect({ stage: p.stage, btn: p.buttonLabel, auto: p.autoRepair }).toEqual({ stage: "assets_none", btn: "", auto: false });
  });
  it("복구 명령의 답 해석 — repaired·partial 은 다시 확인 · 손대지 않는 상태는 blocked · 나머지(실패·상한·정책)는 failed", () => {
    expect(["repaired", "partial"].map(repairOutcomeOf)).toEqual(["retry", "retry"]);
    expect(["ledger_entry", "version_mismatch", "unavailable", "nothing_missing"].map(repairOutcomeOf)).toEqual(["blocked", "blocked", "blocked", "blocked"]);
    expect(["failed", "already_tried", "auto_disabled", undefined, 3].map(repairOutcomeOf)).toEqual(["failed", "failed", "failed", "failed", "failed"]);
  });
});

describe("터미널 명령 0 · 모듈 불변식", () => {
  const src = readFileSync(new URL("./officetab.ts", import.meta.url), "utf-8");
  const strip = (s: string) => s.replace(/\/\*[\s\S]*?\*\//g, "").split("\n").map((l) => (l.trimStart().startsWith("//") ? "" : l.replace(/\s\/\/.*$/, ""))).join("\n");
  it("화면 문구 전부에 명령이 한 글자도 없다", () => {
    const all = [OFFICE_TEXT_PREPARING, OFFICE_TEXT_LONG, OFFICE_TEXT_REPAIRING, OFFICE_TEXT_REPAIR_FAILED, OFFICE_TEXT_ASSETS_BUTTON, OFFICE_TEXT_ASSETS_NONE].join("\n");
    for (const bad of ["python", "cys ", "init-pack", "pack-heal", "~/", "`", "--force", "sudo", "bash", ".py", "터미널"]) expect({ 금지: bad, 있음: all.includes(bad) }).toEqual({ 금지: bad, 있음: false });
  });
  it("구형 WKWebView 비호환 문법 0 · 전역 부수효과 표면 0 · 최상위 문장은 선언뿐", () => {
    const m = strip(src);
    for (const bad of ["(?<=", "(?<!", ".at(", "findLast", "structuredClone", "Object.hasOwn", "replaceAll("]) expect({ 문법: bad, 있음: m.includes(bad) }).toEqual({ 문법: bad, 있음: false });
    for (const bad of ["localStorage", "document.", "window.", "navigator", "setInterval", "setTimeout", "__TAURI__", "invoke("]) expect({ 표면: bad, 있음: m.includes(bad) }).toEqual({ 표면: bad, 있음: false });
    const top = m.split("\n").filter((l) => l.length > 0 && !/^\s/.test(l)).filter((l) => !/^(import |export |const |function |interface |type |\}|\)|\]|;|\| )/.test(l));
    expect(top).toEqual([]);
  });
});
