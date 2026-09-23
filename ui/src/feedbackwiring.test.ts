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
const flowSrc = read("./feedbackflow.ts");
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
    expect(/\.feedback-overlay\s*\{[^}]*z-index/.test(css)).toBe(false);
  });
  it("keydown 캡처·focusin 되찾기·드롭 구독은 finally 에서 해제된다", () => {
    const fin = code.slice(code.lastIndexOf("} finally {"));
    expect(code).toContain('window.addEventListener("keydown", onKey, true)');
    expect(fin).toContain('window.removeEventListener("keydown", onKey, true)');
    expect(code).toContain('document.addEventListener("focusin", onFocusIn, true)');
    expect(fin).toContain('document.removeEventListener("focusin", onFocusIn, true)');
    expect(fin).toContain("listeners.dispose()");
    expect(fin).toContain("ov.remove()");
  });
  // ── 행동은 feedbackflow.ts(행동 검체 feedbackflow.test.ts)에 있다. 여기서는 창이 그것을 **실제로** 쓰는지 본다.
  //    한 줄이라도 다른 것으로 바뀌면(무력화·우회) RED — 리뷰1 변이 M5·M7·M11·M13·M14 가 GREEN 이던 자리.
  it("Esc: IME 가드·맨 위 층 판정을 가진 makeEscHandler 를 쓴다(M14 · 리뷰 minor #4)", () => {
    expect(code).toContain("const onKey = makeEscHandler(() => isTopModalLayer(document, ov), requestClose);");
  });
  it("focusin 되찾기: makeFocusReclaimer 로 모달 층 밖 포커스를 창의 기본 칸으로(M5 · 반박 D2 이중 방어)", () => {
    expect(code).toContain(
      "const onFocusIn = makeFocusReclaimer({ closed: () => closedFlag, home: () => (done ? closeBtn : desc) });",
    );
  });
  it("드롭 구독: 창 수명 구독(scopedListener)만 쓴다 — 늦게 풀린 구독도 해제(M11)", () => {
    expect(code).toContain("const listeners = scopedListener(deps.listen);");
    const subs = [...code.matchAll(/listeners\.sub\("([a-z:/-]+)"/g)].map((m) => m[1]);
    expect(subs.sort()).toEqual(["tauri://drag-drop", "tauri://drag-enter", "tauri://drag-leave"]);
    expect(code.includes("deps.listen(")).toBe(false); // 창 수명 밖 구독 우회 0
  });
  it("버리기: 창을 닫으면(묶음 전) 줄 선 첨부 뒤 초안을 지운다(M7)", () => {
    const fin = stripComments(fnBody(modalSrc, "const finish = () =>"));
    expect(fin).toContain("chain = discardDraftAfter(chain, { draft: draftP, bundled: done }, deps.invoke);");
    expect(fin.indexOf("discardDraftAfter(")).toBeLessThan(fin.indexOf("resolveClose();"));
  });
  it("보내기: 폴더 → 메일 순서는 openBundleForMail 한 곳(M13)", () => {
    const sub = stripComments(fnBody(modalSrc, "const submit = async () =>"));
    expect(sub).toContain("const { folderErr, mailErr } = await openBundleForMail(rep, tryInvoke);");
    expect(sub).toContain("showDone(rep, mailErr, folderErr);");
    expect(sub.includes('"feedback_reveal"') || sub.includes('"feedback_open_mail"')).toBe(false); // 순서 우회 0
  });
  it("열 때 재진입 가드가 첫 await 앞에 있다 + 다른 모달·팔레트가 떠 있으면 열지 않는다(중첩 금지)", () => {
    const body = stripComments(fnBody(modalSrc, "export async function openFeedbackModal("));
    const guard = body.indexOf("feedbackOpen = true");
    const firstAwait = body.indexOf("await ");
    expect(guard).toBeGreaterThan(0);
    expect(firstAwait).toBeGreaterThan(guard);
    expect(body).toContain("modalLayerOpen(document)");
    expect(/finally \{\s*feedbackOpen = false;/.test(body)).toBe(true);
  });
});

describe("⑤ 새 모듈 위생", () => {
  const mods: [string, string][] = [
    ["feedback.ts", pureSrc],
    ["modalguard.ts", guardSrc],
    ["feedbackmodal.ts", modalSrc],
    ["feedbackflow.ts", flowSrc],
  ];
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
  it("피드백 창은 feedback_* 커맨드만 부른다 — 데몬·에이전트 큐·PTY 로 가는 경로 0(원문 자동 주입 금지)", () => {
    // 창과 그 수명 동작 모듈 둘 다 — invoke·invokeRaw·tryInvoke 호출을 모두 센다.
    const code = stripComments(modalSrc) + "\n" + stripComments(flowSrc);
    const cmds = [...code.matchAll(/[iI]nvoke(?:Raw)?\("([a-z_]+)"/g)].map((m) => m[1]);
    expect(cmds.length).toBeGreaterThan(8);
    for (const c of cmds) expect(c.startsWith("feedback_")).toBe(true);
    for (const bad of ["send_input", "send_text", "channel", "org_status", "feed_"]) {
      expect(code.includes(bad)).toBe(false);
    }
  });
  it("창이 찾는 모든 클래스가 고정 틀(MODAL_HTML)에 있다 — 오타 하나면 여는 순간 예외로 창이 안 뜬다", () => {
    const code = stripComments(modalSrc);
    const tplStart = code.indexOf("const MODAL_HTML =");
    const tplEnd = code.indexOf("function errText(");
    expect(tplStart).toBeGreaterThan(0);
    expect(tplEnd).toBeGreaterThan(tplStart);
    const tpl = code.slice(tplStart, tplEnd);
    const sels = [...code.matchAll(/q<[A-Za-z]+>\("\.([a-z-]+)"\)/g)].map((m) => m[1]);
    expect(sels.length).toBeGreaterThan(15);
    for (const cls of new Set(sels)) {
      expect(`${cls}:${new RegExp(`class="[^"]*\\b${cls}\\b`).test(tpl)}`).toBe(`${cls}:true`);
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
    expect(/^mod feedback;/m.test(mainRs)).toBe(true);
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
