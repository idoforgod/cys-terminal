// 폴더 접근 안내(U14)·좌석 막힘 안내(U18) **배선** 회귀 핀 — 소스를 데이터로 읽어 계약을 단언한다
// (wswiring.test.ts·typegate.test.ts 관례: 런타임 코드 0줄 · 본체에서 import 되지 않는다 · DOM/Tauri 불요).
//
// ★왜 판정 테스트(folderaccess.test.ts)만으로 부족한가: 순수 문구가 옳아도
//   · 백엔드가 버퍼에 쌓기 **전에** emit 하거나(emit-before-listen 유실 — 조사 R5 · 반박 §1-2)
//   · 프런트가 listen 뒤 pull 을 **await** 해서 뒤따르는 리스너 등록을 막거나(④ 화면 기능 단절)
//   · stickyToast 가 클릭 처리기를 addEventListener 로 **누적**하면(같은 id 재표시마다 설정 창 n회 — 반박 M6)
// 안내는 여전히 안 보이거나 해가 된다. 그래서 배선을 기계가 센다.
import { describe, it, expect } from "bun:test";
import { readFileSync } from "node:fs";

const main = readFileSync(new URL("./main.ts", import.meta.url), "utf-8");
const rust = readFileSync(new URL("../../src-tauri/src/main.rs", import.meta.url), "utf-8");
/** 주석을 걷어낸 코드 본문 — '코드에 있는가'를 묻는 핀은 주석에 속으면 안 된다. */
const stripComments = (s: string): string =>
  s
    .split("\n")
    .map((l) => (l.trimStart().startsWith("//") ? "" : l.replace(/\s\/\/.*$/, "")))
    .join("\n");
const mainCode = stripComments(main);
const rustCode = stripComments(rust);

/** `start` 에서 시작하는 함수 본문(다음 최상위 `\nfunction `/`\nfn ` 전까지). */
function body(src: string, start: string, next: RegExp): string {
  const i = src.indexOf(start);
  expect(i).toBeGreaterThanOrEqual(0);
  const rest = src.slice(i + start.length);
  const m = rest.search(next);
  return m < 0 ? rest : rest.slice(0, m);
}

describe("백엔드(Rust) — 쌓고 나서 쏜다 · 설정 열기는 고정 목록", () => {
  it("nudge_folder_permissions 는 bare emit 을 하지 않고 버퍼 경유 함수를 부른다", () => {
    const nudge = body(rustCode, "fn nudge_folder_permissions(", /\n(#\[|fn |\/\/\/)/);
    expect(nudge).toContain("push_perm_warning(");
    expect(nudge).not.toContain('emit("perm-warning"');
  });
  it("push_perm_warning: 저장소 push 가 emit 보다 먼저다(순서 핀)", () => {
    const push = body(rustCode, "fn push_perm_warning(", /\n(#\[|fn |\/\/\/)/);
    const store = push.indexOf("PERM_WARNINGS");
    const emit = push.indexOf('emit("perm-warning"');
    expect(store).toBeGreaterThanOrEqual(0);
    expect(emit).toBeGreaterThan(store);
  });
  it("perm_warnings·open_privacy_settings 가 invoke 핸들러에 등록돼 있다", () => {
    const i = rustCode.indexOf("generate_handler![");
    expect(i).toBeGreaterThan(0);
    const reg = rustCode.slice(i, rustCode.indexOf("]", i));
    expect(/\bperm_warnings\b/.test(reg)).toBe(true);
    expect(/\bopen_privacy_settings\b/.test(reg)).toBe(true);
  });
  it("open_privacy_settings 는 /usr/bin/open 절대경로 + 고정 URL 표만 쓴다(임의 URL 0)", () => {
    const f = body(rustCode, "fn open_privacy_settings(", /\n(#\[|fn |\/\/\/)/);
    expect(f).toContain('"/usr/bin/open"');
    expect(f).toContain("privacy_settings_url(");
    // 전체 디스크 접근 앵커는 확인되지 않았고(조사 §2-3) 제품 문구에서 뺐다 — 표에도 없다.
    expect(rustCode).not.toContain("Privacy_AllFiles");
  });
});

describe("프런트(main.ts) — listen 직후 비차단 pull · 클릭 1회 = 열기 1회", () => {
  it("perm-warning listen 뒤에 perm_warnings pull 이 있고 await 하지 않는다", () => {
    const li = mainCode.indexOf('listen("perm-warning"');
    expect(li).toBeGreaterThan(0);
    const pull = mainCode.indexOf('invoke("perm_warnings")', li);
    expect(pull).toBeGreaterThan(li);
    expect(/await\s+invoke\("perm_warnings"\)/.test(mainCode)).toBe(false);
    expect(/void\s+invoke\("perm_warnings"\)/.test(mainCode)).toBe(true);
  });
  it("perm-warning 렌더는 문구 SOT(folderaccess)를 쓴다 — 하드코딩 문구 0", () => {
    expect(mainCode).toContain("permWarningToast(");
    expect(mainCode).not.toContain("전체 디스크 접근 권한)에서 cys를 허용");
    expect(mainCode).not.toContain("로그인 항목에서 cys 관련 항목을 허용");
  });
  it("stickyToast 는 onClick 을 받아 **대입**으로 건다(addEventListener 누적 금지)", () => {
    const st = body(mainCode, "function stickyToast(", /\nfunction /);
    expect(/onClick\?\s*:/.test(st)).toBe(true);
    expect(st).toContain(".onclick =");
    expect(st).not.toContain('addEventListener("click"');
  });
  it("좌석 막힘(U18)은 3초 목록 루프가 cwd_blocked 를 모아 cwdBlockedNotices 로 넘긴다", () => {
    const loop = body(mainCode, "async function refreshPaneTitles(", /\nsetInterval\(refreshPaneTitles/);
    expect(loop).toContain("cwd_blocked");
    expect(loop).toContain("cwdBlockedNotices(");
  });
  it("설정 열기는 한 곳(타입이 고정 목록인 헬퍼)에서만 invoke 하고, 호출부는 고정 target 만 넘긴다", () => {
    const calls = mainCode.match(/invoke\("open_privacy_settings"/g) ?? [];
    expect(calls.length).toBe(1);
    const helper = body(mainCode, "function openPrivacySettings(", /\nfunction /);
    expect(mainCode).toContain("function openPrivacySettings(target: PrivacyTarget)");
    expect(helper).toContain('invoke("open_privacy_settings", { target })');
    const uses = mainCode.match(/openPrivacySettings\(([^)]*)\)/g) ?? [];
    const callers = uses.filter((u) => !u.includes("target: PrivacyTarget"));
    expect(callers.length).toBeGreaterThan(0);
    for (const c of callers) expect(/openPrivacySettings\((t\.target|n\.target|"files"|"login")\)/.test(c)).toBe(true);
  });
});
