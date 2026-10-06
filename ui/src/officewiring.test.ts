// 0.14.44 B5·B6 — 오피스 탭 배선 핀(main.ts · index.html 을 데이터로 읽는다 · wswiring.test.ts 관례: 런타임 코드 0줄 · 주석 제거 본문 기준).
// 판정(문구·간격·복구 방식 해석)은 officetab.test.ts 가 표로 잡는다 — 여기는 main.ts 가 그 판정을 어떻게 옮기는가.
import { describe, it, expect } from "bun:test";
import { readFileSync } from "node:fs";

const read = (rel: string) => readFileSync(new URL(rel, import.meta.url), "utf-8");
const stripComments = (s: string): string =>
  s
    .split("\n")
    .map((l) => (l.trimStart().startsWith("//") ? "" : l.replace(/\s\/\/.*$/, "")))
    .join("\n");
const code = stripComments(read("./main.ts"));
const html = read("../index.html");
function fnBody(name: string): string {
  const i = code.search(new RegExp(`(async )?function ${name}\\(`));
  expect({ 함수: name, 존재: i >= 0 }).toEqual({ 함수: name, 존재: true });
  const end = code.indexOf("\n}\n", i);
  return code.slice(i, end > i ? end + 3 : undefined);
}

describe("B5 — 탭 안내 마크업(index.html)에 터미널 명령이 한 글자도 없다", () => {
  const hint = html.slice(html.indexOf('id="cc-office-hint"'), html.indexOf('<iframe id="cc-office-frame"'));
  it("안내 칸은 문구 span + 복구 단추(처음엔 숨김) — 옛 수동 기동 문구·<code> 없음", () => {
    expect(hint.includes('id="cc-office-hint-text"')).toBe(true);
    expect(/<button id="cc-office-repair" type="button" hidden>/.test(hint)).toBe(true);
    for (const bad of ["python", "~/.cys", "javis_hud_bridge", "<code>", "수동 기동", "60초 내", "cys "]) expect({ 금지: bad, 있음: hint.includes(bad) }).toEqual({ 금지: bad, 있음: false });
  });
  it("main.ts 에도 옛 안내 문구가 없다", () => {
    for (const bad of ["브리지가 꺼져 있습니다", "수동 기동", "javis_hud_bridge.py"]) expect({ 금지: bad, 있음: code.includes(bad) }).toEqual({ 금지: bad, 있음: false });
  });
});

describe("B5 — 건강 확인 루프의 배선", () => {
  it("탭을 열면 루프를 새 세대로 시작한다(openOfficeView → officeStopWatch · officeWatchTick)", () => {
    const b = fnBody("openOfficeView");
    expect(b.indexOf("officeStopWatch();") >= 0 && b.indexOf("officeStopWatch();") < b.indexOf("officeWatchTick(")).toBe(true);
    expect(code.includes('if (view === "office") openOfficeView();')).toBe(true);
  });
  it("탭을 떠나면(setCcTab) · 패널을 닫으면(setCcOpen false) 멈춘다", () => {
    expect(fnBody("setCcTab").includes('if (view !== "office") officeStopWatch();')).toBe(true);
    const o = fnBody("setCcOpen");
    expect(o.slice(o.indexOf("} else {")).includes("officeStopWatch();")).toBe(true);
  });
  it("틱은 백엔드 office_health 만 부르고(fetch no-cors 프로브 폐기) · 기다린 뒤 세대를 다시 확인한다 · 간격은 판정 모듈의 nextPollMs", () => {
    const b = fnBody("officeWatchTick");
    expect(b.includes('invoke("office_health")')).toBe(true);
    expect(b.includes("fetch(")).toBe(false);
    const iAwait = b.indexOf('await invoke("office_health")');
    const iGen = b.indexOf("if (gen !== officeWatchGen) return;", iAwait);
    expect(iAwait >= 0 && iGen > iAwait).toBe(true);
    expect(b.includes("planOfficeTab(h, {")).toBe(true);
    expect(b.includes("plan.nextPollMs")).toBe(true);
    expect(b.includes("if (plan.loadFrame) {")).toBe(true);
  });
  it("새 setInterval 0 — 확인은 setTimeout 연쇄(탭 틱 수 핀 불변)", () => {
    for (const f of ["officeWatchTick", "officeRunRepair", "openOfficeView", "officeStopWatch"]) expect({ 함수: f, setInterval: fnBody(f).includes("setInterval(") }).toEqual({ 함수: f, setInterval: false });
    expect(fnBody("officeWatchTick").includes("setTimeout(")).toBe(true);
  });
  it("안내 문구는 textContent 로만(innerHTML 금지)", () => {
    for (const f of ["officeRenderHint", "officeWatchTick", "officeRunRepair"]) expect({ 함수: f, innerHTML: fnBody(f).includes("innerHTML") }).toEqual({ 함수: f, innerHTML: false });
  });
  it("화면이 실리면 안내를 숨기고 확인을 멈춘다 · 안 실렸을 때는 안내를 보인다(두 안내가 동시에 보이는 순간이 없다)", () => {
    const b = fnBody("officeWatchTick");
    const load = b.slice(b.indexOf("if (plan.loadFrame) {"), b.indexOf("frame.removeAttribute"));
    expect(load.includes('officeRenderHint("", "");')).toBe(true);
    expect(load.includes("return;")).toBe(true);
    expect(b.includes("officeRenderHint(plan.text, plan.buttonLabel);")).toBe(true);
  });
});

describe("B6 — 자산 복구 호출의 배선", () => {
  it("복구는 repair_office_assets 만 부르고 manual 을 넘긴다 · 동시에 둘이 돌지 않는다 · 끝나면 한 번 다시 확인", () => {
    const b = fnBody("officeRunRepair");
    expect(b.includes('invoke("repair_office_assets", { manual })')).toBe(true);
    expect(b.includes("if (officeRepairing) return;")).toBe(true);
    expect(b.includes("officeWatchTick(gen)")).toBe(true);
    expect(b.includes("finally")).toBe(true);
  });
  it("자동 복구는 판정이 autoRepair 일 때만 · 단추는 manual=true 로(정책이 자동을 꺼도 사람은 누를 수 있다)", () => {
    expect(fnBody("officeWatchTick").includes("if (plan.autoRepair && !officeRepairing) void officeRunRepair(false, gen);")).toBe(true);
    expect(code.includes('document.getElementById("cc-office-repair")?.addEventListener("click"')).toBe(true);
    expect(code.includes("void officeRunRepair(true, officeWatchGen);")).toBe(true);
  });
  it("탭을 다시 열면 복구 실패의 잔상을 지운다", () => {
    expect(fnBody("openOfficeView").includes('officeRepairOutcome = "none";')).toBe(true);
  });
});
