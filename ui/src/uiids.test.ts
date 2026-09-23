// index.html ↔ main.ts **id 계약** 핀 (U17 조사 F5 · 반박 §5 ③④).
//
// main.ts 최상위의 `document.getElementById("x")!` 는 x 가 index.html 에 없으면 **모듈 평가 중 TypeError**
// 를 던진다 — 그러면 파일 끝 start() 에 도달하지 못해 `started` 가 false 로 남고, 3초 자가치유 틱은 매번
// 첫 줄에서 빠지며, 화면은 백지가 된다(데몬·에이전트는 살아 있는데 글자가 하나도 안 보이는 ④).
// 요소를 옮기거나 지우는 작업(이번 U17 의 ＋부서 이동)이 가장 밟기 쉬운 함정이라 계열로 닫는다:
// **non-null 단언으로 조회하는 모든 id 는 index.html 에 실재해야 한다.**
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

describe("id 계약 — `getElementById(...)!` 로 조회하는 id 는 index.html 에 있다", () => {
  const ids = new Set<string>();
  for (const m of code.matchAll(/getElementById\(\s*["']([^"']+)["']\s*\)!/g)) ids.add(m[1]);
  it("검사 대상이 실제로 잡힌다(정규식이 0건이면 핀이 아무것도 지키지 않는다)", () => {
    expect(ids.size).toBeGreaterThan(50);
  });
  it("누락 0", () => {
    const missing = [...ids].filter((id) => !html.includes(`id="${id}"`));
    expect(missing).toEqual([]);
  });
  it("querySelector('#…')! 형태의 우회 단언도 같은 계약", () => {
    const qs = [...code.matchAll(/querySelector\(\s*["']#([A-Za-z0-9_-]+)["']\s*\)!/g)].map((m) => m[1]);
    const missing = qs.filter((id) => !html.includes(`id="${id}"`));
    expect(missing).toEqual([]);
  });
});
