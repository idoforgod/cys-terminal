// setFocus 모달 층 가드 **배선** 회귀 핀 — 소스를 데이터로 읽어 계약을 단언한다
// (wswiring.test.ts 관례: 런타임 코드 0줄 · 본체에서 import 되지 않는다 · DOM/Tauri 불요).
//
// ★무엇을 지키는가(U6 · 0.14.41 반박 D2 blocking): 3초 자가치유 틱의 자동 입양과 `surface.exited` 가 부르는
//   setFocus → term.focus() 가, 사용자가 모달 입력칸이나 ⌘K 팔레트 검색칸에 글을 쓰는 도중 **뒤에 가려진 pane**
//   의 xterm 으로 키보드 포커스를 옮기지 못하게 하는 가드다(계속 친 글자와 Enter 가 마스터 PTY 로 들어가는 경로).
// ★왜 이 파일이 따로 있는가: 판정 모듈(modalguard.ts)이 옳아도 main.ts 가 그것을 부르지 않으면 결함은 그대로
//   산다. modalguard.test.ts 는 순수 함수만 돌리고 **배선은 보지 않는다** — setFocus 가 그 판정을 실제로 거치는지는
//   여기서만 센다. 여기 실패는 "고쳐 두었다고 믿는 것이 코드에 없다"는 뜻이다.
// ★이 가드는 어느 한 창의 것이 아니다: 확인·입력·업데이트 창과 팔레트 전부를 지킨다. 창 하나가 제품에서 빠져도
//   이 핀은 남아야 한다 — 그래서 특정 창의 검체 파일에 얹어 두지 않는다.
import { describe, it, expect } from "bun:test";
import { readFileSync } from "node:fs";

const read = (rel: string): string => readFileSync(new URL(rel, import.meta.url), "utf-8");
const main = read("./main.ts");
const guardSrc = read("./modalguard.ts");

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

/**
 * 모듈 최상위(괄호 깊이 0) **문장**들 — '최상위 부수효과 0' 판정용. 여러 줄에 걸친 상수 초기화
 * (`const X = "…" +\n "…";`)는 한 문장이다. 문장 경계 = 깊이 0 의 `;` 또는 줄 끝에서 깊이 0 으로 돌아오는 `}`.
 * 함수·클래스 본문(깊이 > 0)은 모으지 않는다 — 그 안의 호출은 부수효과가 아니라 정의다.
 */
function topLevelStatements(src: string): string[] {
  const code = stripComments(src)
    .split("\n")
    .map((l) => l.replace(/"(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'|`(?:[^`\\]|\\.)*`/g, '""'))
    .join("\n")
    .replace(/\/\*[\s\S]*?\*\//g, ""); // 블록 주석(JSDoc) — 문자열을 걷은 뒤라 문자열 속 `/*` 에 속지 않는다
  const out: string[] = [];
  let depth = 0;
  let cur = "";
  const flush = () => {
    const t = cur.replace(/\s+/g, " ").trim();
    if (t && t !== ";") out.push(t);
    cur = "";
  };
  for (let i = 0; i < code.length; i++) {
    const ch = code[i];
    if (ch === "{" || ch === "(" || ch === "[") {
      if (depth === 0) cur += ch;
      depth++;
    } else if (ch === "}" || ch === ")" || ch === "]") {
      depth--;
      if (depth === 0) {
        cur += ch;
        // 줄 끝에서 닫히는 `}` 만 문장 끝(함수·인터페이스 선언). `import { a } from` 의 `}` 는 아니다.
        if (ch === "}" && /^[ \t]*(\n|$)/.test(code.slice(i + 1, i + 40))) flush();
      }
    } else if (depth === 0) {
      cur += ch;
      if (ch === ";") flush();
    }
  }
  flush();
  return out;
}

describe("① setFocus 포커스 가드 (반박 D2 · blocking)", () => {
  const body = stripComments(fnBody(main, "function setFocus(sid: number)"));
  it("term.focus() 는 모달 층 판정 뒤에서만 불린다 — 가드 줄은 글자 그대로(`|| true` 같은 무력화도 RED)", () => {
    const lines = body.split("\n").filter((l) => l.includes("term.focus()"));
    expect(lines.length).toBe(1);
    expect(lines[0].trim()).toBe("if (!modalLayerOpen(document)) panes.get(key)?.term.focus();");
    // xterm 포커스를 주는 곳은 파일 전체에서 이 한 줄뿐이다(다른 자리에서 우회하면 가드가 무의미).
    expect(stripComments(main).split("term.focus()").length - 1).toBe(1);
  });
  it("건너뛴 포커스는 모달이 닫힌 뒤 되살린다(리뷰 minor #3) — 가드 바로 다음 줄이 else 분기", () => {
    const lines = body.split("\n").map((l) => l.trim());
    const i = lines.indexOf("if (!modalLayerOpen(document)) panes.get(key)?.term.focus();");
    expect(i).toBeGreaterThanOrEqual(0);
    expect(lines[i + 1]).toBe("else deferPaneFocusUntilModalsClose();");
  });
  it("되살리기는 호이스팅되는 함수 선언이고 절대 던지지 않는다(setFocus·3초 틱 보호) — body 직계 자식만 지켜본다", () => {
    const fn = stripComments(fnBody(main, "function deferPaneFocusUntilModalsClose() {"));
    expect(/^function deferPaneFocusUntilModalsClose\(\) \{\s*try \{\s*armDeferredPaneFocus\(\{/.test(fn)).toBe(true);
    expect(fn).toContain("} catch {");
    expect(fn).toContain("layerOpen: () => modalLayerOpen(document),");
    expect(fn).toContain("return a == null || a === document.body;"); // 다른 칸이 가진 포커스는 빼앗지 않는다
    expect(fn).toContain("if (focusedSid != null) setFocus(focusedSid);");
    expect(fn).toContain("mo.observe(document.body, { childList: true });");
    expect(fn).toContain("return () => mo.disconnect();");
    expect(fn).toContain("later: (fn) => void setTimeout(fn, 0),"); // 닫는 키의 남은 이벤트가 pane 으로 가지 않게
    expect(/import \{[^}]*armDeferredPaneFocus[^}]*\} from "\.\/modalguard";/.test(main)).toBe(true);
  });
  it("focused 표시·focusedSid 갱신은 가드와 무관하게 유지된다(탭 강조가 사라지지 않게)", () => {
    expect(body).toContain("focusedSid = sid;");
    expect(body).toContain('classList.toggle("focused"');
  });
  it("판정은 modalguard 모듈에서 들여온다(인라인 복제 금지)", () => {
    expect(/import \{[^}]*modalLayerOpen[^}]*\} from "\.\/modalguard";/.test(main)).toBe(true);
  });
});

describe("modalguard.ts 모듈 위생", () => {
  const mods: [string, string][] = [["modalguard.ts", guardSrc]];
  it("최상위 부수효과 0 — 최상위 문장은 import·선언만", () => {
    for (const [name, src] of mods) {
      const stmts = topLevelStatements(src);
      expect(stmts.length).toBeGreaterThan(3); // 수집기가 아무것도 못 봤다 = 핀이 무력
      for (const st of stmts) {
        const ok = /^(import |export |const |let |function |async function |type |interface |declare )/.test(st);
        expect(`${name}: ${ok ? "ok" : st}`).toBe(`${name}: ok`);
        const fx = /\b(document|window|navigator)\.|setInterval\(|setTimeout\(|addEventListener\(|listen\(|invoke\(/.test(st);
        expect(`${name}: ${fx ? st : "no-fx"}`).toBe(`${name}: no-fx`);
      }
    }
  });
  it("수집기 자기검증 — 최상위 호출·DOM 접근을 실제로 잡는다", () => {
    const bad = 'import { a } from "./x";\nconst A = "x" +\n  "y";\nfunction f() {\n  document.body;\n}\nf();\nconst B = document.getElementById("z");\n';
    const st = topLevelStatements(bad);
    expect(st).toContain("f();");
    expect(st.some((x) => x.startsWith("const B") && x.includes("document."))).toBe(true);
    expect(st.some((x) => x.includes("document.body"))).toBe(false); // 함수 본문은 정의
  });
  it("구형 WKWebView 비호환 문법 0", () => {
    for (const [name, src] of mods) {
      const code = stripComments(src);
      for (const bad of ["(?<=", "(?<!", ".at(", "findLast", "structuredClone", "Object.hasOwn", "replaceAll("]) {
        expect(`${name}:${code.includes(bad) ? bad : ""}`).toBe(`${name}:`);
      }
    }
  });
});
