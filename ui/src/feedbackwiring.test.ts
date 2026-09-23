// U6(0.14.41) 피드백 **배선** 회귀 핀 — 소스를 데이터로 읽어 계약을 단언한다
// (wswiring.test.ts 관례: 런타임 코드 0줄 · 본체에서 import 되지 않는다 · DOM/Tauri 불요).
//
// 판정 모듈(modalguard·feedback)이 옳아도 main.ts 가 그것을 부르지 않으면 결함은 그대로 산다.
// 여기 실패는 "고쳐 두었다고 믿는 것이 코드에 없다"는 뜻이다.
import { describe, it, expect } from "bun:test";
import { readFileSync } from "node:fs";

const read = (rel: string): string => readFileSync(new URL(rel, import.meta.url), "utf-8");
const main = read("./main.ts");
const html = read("../index.html");
const css = read("./style.css");
const modalSrc = read("./feedbackmodal.ts");
const pureSrc = read("./feedback.ts");
const guardSrc = read("./modalguard.ts");
const mainRs = read("../../src-tauri/src/main.rs");
const fbRs = read("../../src-tauri/src/feedback.rs");

/** 주석을 걷어낸 코드 본문(wswiring 과 같은 규칙) — 설명문이 핀을 속이지 못하게. */
const stripComments = (s: string): string =>
  s
    .split("\n")
    .map((l) => (l.trimStart().startsWith("//") ? "" : l.replace(/\s\/\/.*$/, "")))
    .join("\n");

/** `function name(` 부터 짝이 맞는 닫는 중괄호까지(문자열 속 중괄호는 이 파일들에서 쓰지 않는다). */
function fnBody(src: string, header: string): string {
  const a = src.indexOf(header);
  expect(a).toBeGreaterThanOrEqual(0); // 앵커가 사라졌다 = 핀이 아무것도 지키지 않는다
  const open = src.indexOf("{", a);
  let depth = 0;
  for (let i = open; i < src.length; i++) {
    if (src[i] === "{") depth++;
    else if (src[i] === "}") {
      depth--;
      if (depth === 0) return src.slice(a, i + 1);
    }
  }
  throw new Error(`unbalanced: ${header}`);
}

/** 모듈 최상위(중괄호 깊이 0)에 놓인 줄만 — '최상위 부수효과 0' 판정용. */
function topLevelLines(src: string): string[] {
  const out: string[] = [];
  let depth = 0;
  for (const raw of stripComments(src).split("\n")) {
    const line = raw.replace(/"(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'|`(?:[^`\\]|\\.)*`/g, '""');
    if (depth === 0 && line.trim()) out.push(line.trim());
    for (const ch of line) {
      if (ch === "{" || ch === "(" || ch === "[") depth++;
      else if (ch === "}" || ch === ")" || ch === "]") depth--;
    }
  }
  return out;
}

describe("① setFocus 포커스 가드 (반박 D2 · blocking)", () => {
  const body = stripComments(fnBody(main, "function setFocus(sid: number)"));
  it("term.focus() 는 모달 층 판정 뒤에서만 불린다", () => {
    const lines = body.split("\n").filter((l) => l.includes("term.focus()"));
    expect(lines.length).toBe(1);
    expect(lines[0]).toContain("!modalLayerOpen(document)");
  });
  it("focused 표시·focusedSid 갱신은 가드와 무관하게 유지된다(탭 강조가 사라지지 않게)", () => {
    expect(body).toContain("focusedSid = sid;");
    expect(body).toContain('classList.toggle("focused"');
  });
  it("판정은 modalguard 모듈에서 들여온다(인라인 복제 금지)", () => {
    expect(main).toMatch(/import \{[^}]*modalLayerOpen[^}]*\} from "\.\/modalguard";/);
  });
});

describe("② pane 드롭 리스너 첫 줄 가드", () => {
  it("tauri://drag-drop pane 리스너의 첫 문장이 피드백 창 가드다", () => {
    const a = main.indexOf('await listen("tauri://drag-drop", (e) => {');
    expect(a).toBeGreaterThan(0);
    const firstStmt = stripComments(main.slice(a).split("\n").slice(1).join("\n"))
      .split("\n")
      .map((l) => l.trim())
      .find((l) => l.length > 0);
    expect(firstStmt).toBe("if (feedbackOverlayOpen(document)) return;");
  });
});

describe("③ 사이드바 바닥 공용 칸(A2·A3 공통 마크업) + 단추 배선", () => {
  it("#wsbar-foot 은 #wsbar 안, #ws-tabs 바로 다음 형제이고 자식 순서는 사용량 → 피드백 → 전문가용", () => {
    const compact = html.replace(/\s+/g, " ");
    expect(compact).toContain(
      '<div id="ws-tabs"></div> <div id="wsbar-foot"> <section id="wsbar-usage" aria-label="사용량"></section> <div id="wsbar-feedback-slot"></div> <section id="wsbar-expert"></section> </div> </nav>',
    );
  });
  it("단추는 index.html 이 아니라 JS 가 슬롯에 단다(공용 마크업을 A2 와 바이트 동일하게 유지)", () => {
    expect(html.includes("btn-feedback")).toBe(false);
  });
  it("모듈 최상위 배선은 null 단정(!) 없이 슬롯을 넘긴다 — null 이면 main.js 전체가 죽어 모든 pane 백지(④)", () => {
    const code = stripComments(main);
    expect(code).toContain('mountFeedbackButton(document.getElementById("wsbar-feedback-slot"),');
    expect(code.includes('getElementById("wsbar-feedback-slot")!')).toBe(false);
  });
  it("배선은 start() 밖에 있다 — start() 가 멈춘 기계에서도 피드백을 보낼 수 있게", () => {
    const a = main.indexOf("async function start() {");
    const b = main.indexOf("started = true;", a);
    expect(a).toBeGreaterThan(0);
    expect(main.slice(a, b).includes("mountFeedbackButton(")).toBe(false);
  });
});

describe("④ 피드백 창 수명 — 리스너는 finally 에서 반드시 걷힌다(반박 D13)", () => {
  const code = stripComments(modalSrc);
  it("오버레이는 modal-overlay 층을 쓴다(전역 단축키 차단 상속 · z 1000 — 위에 뜨는 확인 창이 밑에 깔리지 않게)", () => {
    expect(code).toContain('"modal-overlay feedback-overlay"');
    expect(css).not.toMatch(/\.feedback-overlay\s*\{[^}]*z-index/);
  });
  it("keydown 캡처·focusin 되찾기·드롭 구독은 finally 에서 해제된다", () => {
    const fin = code.slice(code.lastIndexOf("} finally {"));
    expect(code).toContain('window.addEventListener("keydown", onKey, true)');
    expect(fin).toContain('window.removeEventListener("keydown", onKey, true)');
    expect(code).toContain('document.addEventListener("focusin", onFocusIn, true)');
    expect(fin).toContain('document.removeEventListener("focusin", onFocusIn, true)');
    expect(fin).toContain("unlistenAll()");
    expect(fin).toContain("ov.remove()");
  });
  it("열 때 재진입 가드가 첫 await 앞에 있다 + 다른 모달·팔레트가 떠 있으면 열지 않는다(중첩 금지)", () => {
    const body = stripComments(fnBody(modalSrc, "export async function openFeedbackModal("));
    const guard = body.indexOf("feedbackOpen = true");
    const firstAwait = body.indexOf("await ");
    expect(guard).toBeGreaterThan(0);
    expect(firstAwait).toBeGreaterThan(guard);
    expect(body).toContain("modalLayerOpen(document)");
    expect(body).toMatch(/finally \{\s*feedbackOpen = false;/);
  });
});

describe("⑤ 새 모듈 위생", () => {
  const mods: [string, string][] = [
    ["feedback.ts", pureSrc],
    ["modalguard.ts", guardSrc],
    ["feedbackmodal.ts", modalSrc],
  ];
  it("최상위 부수효과 0 — 최상위에는 import·선언만", () => {
    for (const [name, src] of mods) {
      for (const l of topLevelLines(src)) {
        const ok = /^(import |export |const |let |function |async function |type |interface |\}|\)|\])/.test(l);
        expect(`${name}: ${ok ? "ok" : l}`).toBe(`${name}: ok`);
        expect(/\b(document|window)\.|setInterval\(|setTimeout\(|addEventListener\(|listen\(|invoke\(/.test(l)).toBe(false);
      }
    }
  });
  it("구형 WKWebView 비호환 문법 0", () => {
    for (const [name, src] of mods) {
      const code = stripComments(src);
      for (const bad of ["(?<=", "(?<!", ".at(", "findLast", "structuredClone", "Object.hasOwn", "replaceAll("]) {
        expect(`${name}:${code.includes(bad) ? bad : ""}`).toBe(`${name}:`);
      }
    }
  });
  it("피드백 창은 feedback_* 커맨드만 부른다 — 데몬·에이전트 큐·PTY 로 가는 경로 0(원문 자동 주입 금지)", () => {
    const code = stripComments(modalSrc);
    const cmds = [...code.matchAll(/invoke(?:Raw)?\("([a-z_]+)"/g)].map((m) => m[1]);
    expect(cmds.length).toBeGreaterThan(5);
    for (const c of cmds) expect(c.startsWith("feedback_")).toBe(true);
    for (const bad of ["send_input", "send_text", "channel", "org_status", "feed_"]) {
      expect(code.includes(bad)).toBe(false);
    }
  });
  it("innerHTML 에는 사용자 값이 들어가지 않는다(고정 틀만 · 값은 textContent)", () => {
    const code = stripComments(modalSrc);
    for (const m of code.matchAll(/innerHTML\s*=\s*([^;]+);/g)) {
      expect(m[1].includes("${")).toBe(false);
    }
  });
});

describe("⑥ Rust 배선 — 등록·창 정책·전송 부재", () => {
  const rs = stripComments(fbRs);
  const cmdNames = [...fbRs.matchAll(/#\[tauri::command\]\s*(?:pub(?:\(crate\))? )?(?:async )?fn ([a-z_]+)/g)].map((m) => m[1]);
  it("feedback.rs 의 모든 커맨드가 invoke_handler 에 등재된다(누락 = 런타임 'command not found')", () => {
    expect(cmdNames.length).toBeGreaterThan(5);
    for (const n of cmdNames) {
      expect(n.startsWith("feedback_")).toBe(true);
      expect(`${n}:${mainRs.includes(`feedback::${n},`)}`).toBe(`${n}:true`);
    }
    expect(mainRs).toMatch(/^mod feedback;/m);
  });
  it("자식 프로세스를 만드는 함수는 전부 창 정책(no_console)을 건다 — 윈도우 검은 창 0", () => {
    const chunks = rs.split(/\n(?=(?:pub(?:\(crate\))? )?(?:async )?fn )/);
    let spawners = 0;
    for (const c of chunks) {
      if (c.includes("Command::new(")) {
        spawners++;
        expect(c).toContain("no_console(&mut");
      }
    }
    expect(spawners).toBeGreaterThan(0);
  });
  it("서버 전송 없음(2단계 전까지) — curl·http·데몬 소켓 호출 0", () => {
    for (const bad of ['"curl"', "reqwest", "http://", "https://", "rpc_on(", "connect_to(", "send_text", "cmd.exe", '"cmd"', "rundll32"]) {
      expect(`${bad}:${rs.includes(bad)}`).toBe(`${bad}:false`);
    }
  });
  it("받는 주소는 README·SECURITY 의 공식 연락 주소와 같고 상수 한 곳에만 있다", () => {
    const readme = read("../../README.md");
    const security = read("../../SECURITY.md");
    const m = fbRs.match(/pub const FEEDBACK_TO: &str = "([^"]+)";/);
    expect(m).not.toBeNull();
    const addr = (m as RegExpMatchArray)[1];
    expect(readme).toContain(addr);
    expect(security).toContain(addr);
    expect(fbRs.split(addr).length - 1).toBe(1); // 상수 정의 1곳
    expect(modalSrc.includes(addr)).toBe(false); // UI 는 Rust 보고에서 받는다
  });
});
