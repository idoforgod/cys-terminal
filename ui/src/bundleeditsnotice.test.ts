// ★(0.15.0 · 「스스로 해결」 M16 · RULINGS AG3) 지난 업데이트의 앱 안쪽 수정 보관 알림 — 기동 때 UI 가 백엔드 명령
// `bundle_edits_pending_notices` 를 **update-error 리스너를 붙인 뒤** 한 번 당겨 기존 토스트로 보인다. 윈도우에서는 앱이 설치 프로그램을
// 띄우고 곧바로 닫혀 그 자리에서 알릴 수 없어, 백엔드가 보관소 안(`bundle-edits/.notices/`)에 기록만 남긴다. 종전(AA3)은 리스너 등록
// **전에** 도는 다른 pull 안에서 이벤트를 쐈기 때문에 화면 알림이 늘 사라졌다 — 이 시험이 그 순서를 고정한다.
import { describe, it, expect } from "bun:test";
import { readFileSync } from "node:fs";

const read = (rel: string) => readFileSync(new URL(rel, import.meta.url), "utf-8");
/** 주석 제거본 — 주석에만 적힌 호출로 핀이 통과하지 않게 한다. */
const stripComments = (s: string): string =>
  s
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .split("\n")
    .map((l) => (l.trimStart().startsWith("//") ? "" : l.replace(/\s\/\/.*$/, "")))
    .join("\n");
const count = (hay: string, needle: string): number => hay.split(needle).length - 1;

const main = stripComments(read("./main.ts"));
const PULL = 'invoke("bundle_edits_pending_notices")';

describe("앱 안쪽 수정 보관 알림 pull (RULINGS AG3)", () => {
  it("UI 는 이 명령을 정확히 한 번 부른다", () => {
    expect(count(main, PULL)).toBe(1);
  });

  it("start() 안에서, update-error 리스너를 등록한 **뒤에** 부른다(먼저 부르면 화면 알림이 사라진다)", () => {
    const start = main.indexOf("async function start() {");
    const listener = main.indexOf('await listen("update-error"');
    const pull = main.indexOf(PULL);
    expect(start).toBeGreaterThan(0);
    expect(listener).toBeGreaterThan(start);
    expect(pull).toBeGreaterThan(listener);
    // 리스너 등록 문장 바로 뒤 — 그 사이에 다른 문장이 끼지 않는다(주석은 제거본에서 빈 줄이다)
    const between = main.slice(listener, pull);
    const listenerEnd = between.indexOf("});") + "});".length;
    expect(between.slice(listenerEnd).trim()).toBe("void");
  });

  it("기다리지 않고(void) · 받은 항목을 기존 toast 로 보이고 · 실패는 조용히 삼킨다(부팅 무영향)", () => {
    const at = main.indexOf(PULL);
    const stmt = main.slice(main.lastIndexOf("void", at), main.indexOf("});", main.indexOf(".catch(", at)) + 3);
    expect(stmt.startsWith("void invoke(")).toBe(true);
    expect(stmt).toContain(".then((list) => {");
    expect(stmt).toContain("Array.isArray(list)");
    expect(stmt).toContain('toast(n.warn === true ? "health" : "watchdog", n.title, n.body)');
    expect(stmt).toContain(".catch(() => {");
    for (const bad of ["await ", "stickyToast(", "osBanner(", "setTimeout(", "setInterval("]) expect({ bad, present: stmt.includes(bad) }).toEqual({ bad, present: false });
  });
});

describe("백엔드 계약 — 명령이 정의·등재돼 있고 UI 가 읽는 키(warn·title·body)를 싣는다", () => {
  const rust = read("../../src-tauri/src/main.rs");
  const prod = rust.slice(0, rust.indexOf("#[cfg(test)]\nmod tests {"));
  const be = read("../../src-tauri/src/bundle_edits.rs");
  const beProd = be.slice(0, be.indexOf("#[cfg(test)]\nmod tests {"));

  it("#[tauri::command] 로 정의되고 invoke_handler 에 한 번 등재된다", () => {
    expect(prod.includes("#[tauri::command]\nasync fn bundle_edits_pending_notices() -> Vec<Value> {")).toBe(true);
    const i = prod.indexOf("tauri::generate_handler![");
    const reg = prod.slice(i, prod.indexOf("\n        ])", i));
    expect(reg.split("\n").filter((l) => l.trim() === "bundle_edits_pending_notices,").length).toBe(1);
  });

  it("넘기는 항목의 키가 UI 가 읽는 n.warn · n.title · n.body 와 같다", () => {
    const f = beProd.slice(beProd.indexOf("pub(crate) fn take_pending_notices_for_ui()"));
    expect(f.slice(0, f.indexOf("\n}\n"))).toContain('json!({"warn": n.warn, "title": n.title, "body": n.body})');
    for (const key of ["warn", "title", "body"]) expect({ key, read: new RegExp(`\\bn\\??\\.${key}\\b`).test(main) }).toEqual({ key, read: true });
  });
});
