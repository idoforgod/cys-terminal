// U1 사이드바 사용량 패널 **배선** 회귀 핀 — main.ts·index.html·style.css·새 모듈 소스를 데이터로 읽어
// 계약을 단언한다(wswiring.test.ts 관례: 런타임 코드 0줄 · DOM/Tauri 불요 · 주석 제거 본문 기준).
//
// 왜 배선을 기계로 세는가(설계 §3 U1 · 반박 D6·D7):
//   · 사용량 패널은 **표시 전용**이다. 새 타이머·새 폴링 루프·3초 입양 틱 개입·await 대기가 하나라도
//     생기면 자가치유 틱(③)과 승인 자동전환 판정(ceoIsActivelyGenerating 이 refreshSidebarStatus 를 await)
//     에 부하·지연이 번진다. 개수 핀(setInterval 9개)은 병렬 항목과 결합하므로 **의미 핀**으로 막는다.
//   · 새 DOM id 에 `!` 단언을 쓰면 index.html 과 번들이 어긋날 때 main.js 평가가 중단돼 전 pane 이
//     백지가 된다(④). 새 모듈의 구형 WKWebView 비호환 문법·최상위 부수효과도 같은 치명도다(반박 D6).
import { describe, it, expect } from "bun:test";
import { readFileSync } from "node:fs";

const read = (rel: string) => readFileSync(new URL(rel, import.meta.url), "utf-8");
const stripComments = (s: string): string =>
  s
    .split("\n")
    .map((l) => (l.trimStart().startsWith("//") ? "" : l.replace(/\s\/\/.*$/, "")))
    .join("\n");
const src = read("./main.ts");
const code = stripComments(src);
const html = read("../index.html");
const css = read("./style.css");

/** 열 0 의 `function name(` 부터 첫 열 0 닫는 중괄호까지(clipath.test.ts mainFnBody 와 같은 규칙). */
function fnBody(name: string): string {
  const i = code.indexOf(`function ${name}(`);
  expect({ 함수: name, 존재: i >= 0 }).toEqual({ 함수: name, 존재: true });
  const end = code.indexOf("\n}\n", i);
  return code.slice(i, end > i ? end + 3 : undefined);
}
/** 호출 지점마다 그 호출을 감싼 최상위 함수 이름(없으면 "<top>"). */
function enclosingFns(needle: string): string[] {
  const out: string[] = [];
  let at = code.indexOf(needle);
  while (at >= 0) {
    const head = code.lastIndexOf("\nfunction ", at);
    const headA = code.lastIndexOf("\nasync function ", at);
    const h = Math.max(head, headA);
    const closed = h >= 0 ? code.indexOf("\n}\n", h) : -1;
    if (h < 0 || (closed >= 0 && closed < at)) out.push("<top>");
    else {
      const m = /function\s+([A-Za-z0-9_$]+)\s*\(/.exec(code.slice(h, h + 200));
      out.push(m ? m[1] : "<?>");
    }
    at = code.indexOf(needle, at + needle.length);
  }
  return out;
}
const tagsOnly = (s: string) => s.replace(/>\s+</g, "><");

describe("사이드바 바닥 공용 꼬리 컨테이너(#wsbar-foot) — A2·A3 공통 마크업", () => {
  const h = tagsOnly(html);
  it("#ws-tabs 바로 다음 형제로 사용량 → 피드백 슬롯 → 전문가용 순서", () => {
    expect(
      h.includes(
        '<div id="ws-tabs"></div><div id="wsbar-foot"><section id="wsbar-usage" aria-label="사용량"></section><div id="wsbar-feedback-slot">',
      ),
    ).toBe(true);
    expect(h.includes('<section id="wsbar-expert"></section></div></nav>')).toBe(true);
    const u = h.indexOf('id="wsbar-usage"'), f = h.indexOf('id="wsbar-feedback-slot"'), e = h.indexOf('id="wsbar-expert"');
    expect(u >= 0 && u < f && f < e).toBe(true);
  });
  it("꼬리는 #wsbar 안에 있다", () => {
    const nav = h.indexOf('<nav id="wsbar">'), foot = h.indexOf('id="wsbar-foot"'), end = h.indexOf("</nav>", nav);
    expect(nav >= 0 && nav < foot && foot < end).toBe(true);
  });
  it("목록(#ws-tabs)에 최소 높이 · 꼬리 묶음에 상한 + 자체 스크롤 (목록이 0px 로 눌리지 않게 — 반박 U17 D5)", () => {
    const flat = css.replace(/\s+/g, " ");
    expect(/#ws-tabs \{[^}]*min-height:/.test(flat)).toBe(true);
    const foot = /#wsbar-foot \{([^}]*)\}/.exec(flat);
    expect(foot).not.toBeNull();
    expect(foot![1]).toContain("max-height:");
    expect(foot![1]).toContain("overflow-y: auto");
    expect(foot![1]).toContain("min-height: 0");
  });
});

describe("사용량 조회 — 공유 fetcher 하나 · in-flight 가드 · 전용 상한", () => {
  it("usage_accounts_all 호출은 fetcher 본문 1곳뿐", () => {
    const n = code.split('invoke("usage_accounts_all"').length - 1;
    expect(n).toBe(1);
    expect(fnBody("refreshAccountsShared")).toContain('invoke("usage_accounts_all"');
  });
  it("fetcher 는 claimFlight + releaseFlightWhenSettled + rpcT(T_ACCT) 규약을 쓴다", () => {
    const b = fnBody("refreshAccountsShared");
    for (const needle of ["claimFlight(", "releaseFlightWhenSettled(", "rpcT(", "T_ACCT", "shouldFetchAccounts("])
      expect({ 배선: needle, 있음: b.includes(needle) }).toEqual({ 배선: needle, 있음: true });
  });
  it("T_ACCT 는 winScaled 명명 상수(넘기면 무엇이 일어나는지 주석)", () => {
    expect(/const T_ACCT = winScaled\(\d[\d_]*\);/.test(code)).toBe(true);
    const line = src.split("\n").find((l) => l.includes("const T_ACCT = winScaled("))!;
    expect(line).toContain("넘기면");
  });
  it("호출 지점은 사이드바 10초 틱과 Control Center 두 곳뿐(반박 D7 의미 핀)", () => {
    const where = enclosingFns("refreshAccountsShared(").filter((f) => f !== "refreshAccountsShared");
    expect([...new Set(where)].sort()).toEqual(["refreshControlCenter", "refreshSidebarStatus"]);
  });
  it("사이드바 틱에서는 void(비대기)로 renderWsTabs() 뒤에 — await 금지(승인 자동전환 판정 지연 차단)", () => {
    const b = fnBody("refreshSidebarStatus");
    expect(b.includes("await refreshAccountsShared")).toBe(false);
    const v = b.indexOf("void refreshAccountsShared(false)");
    const r = b.indexOf("renderWsTabs();");
    expect(v > r && r >= 0).toBe(true);
  });
  it("Control Center Live 는 공유 fetcher 를 force 로 부른다(직접 invoke 제거 → 겹침 가드 획득)", () => {
    expect(fnBody("refreshControlCenter")).toContain("await refreshAccountsShared(true)");
  });
  it("3초 입양 틱(refreshPaneTitles)에는 아무것도 얹지 않는다", () => {
    const b = fnBody("refreshPaneTitles");
    for (const needle of ["refreshAccountsShared", "usage_accounts_all", "renderUsageBar"])
      expect({ 금지: needle, 있음: b.includes(needle) }).toEqual({ 금지: needle, 있음: false });
  });
  it("어떤 setInterval/setTimeout 도 사용량 조회·렌더를 직접 돌리지 않는다(새 타이머 0)", () => {
    const lines = code.split("\n");
    lines.forEach((l, i) => {
      if (!/set(Interval|Timeout)\(/.test(l)) return;
      const win = l + (lines[i + 1] ?? "");
      for (const needle of ["refreshAccountsShared", "usage_accounts_all", "renderUsageBar"])
        expect({ 줄: i + 1, 금지: needle, 있음: win.includes(needle) }).toEqual({ 줄: i + 1, 금지: needle, 있음: false });
    });
  });
  it("start() 복원 구간(머리 ~ started = true)에는 조회가 없다(부트 체인 비개입)", () => {
    const a = src.indexOf("async function start() {");
    const b = src.indexOf("started = true;", a);
    expect(a > 0 && b > a).toBe(true);
    expect(stripComments(src.slice(a, b)).includes("refreshAccountsShared")).toBe(false);
  });
  it("노드 신호(nodeSig) 폴백을 새로 만들지 않는다(설계 금지 — 제공자 혼합 최대값)", () => {
    expect(fnBody("refreshAccountsShared").includes("nodeSig")).toBe(false);
    expect(fnBody("renderUsageBar").includes("nodeSig")).toBe(false);
  });
});

describe("사용량 렌더 — 백지(④) 차단", () => {
  it("새 id 에 non-null 단언(`!`)을 쓰지 않는다", () => {
    for (const id of ["wsbar-usage", "wsbar-foot", "wsbar-expert", "wsbar-feedback-slot"])
      expect({ id, 단언: code.includes(`getElementById("${id}")!`) }).toEqual({ id, 단언: false });
  });
  it("렌더는 textContent 로만(innerHTML 금지 — 라벨은 로컬 폴더·파일에서 온다)", () => {
    expect(fnBody("renderUsageBar").includes("innerHTML")).toBe(false);
    expect(fnBody("renderUsageBar")).toContain("buildUsageBarModel(");
  });
  it("렌더는 스스로 오류를 삼킨다(표시 전용 — 호출측 틱으로 새지 않는다)", () => {
    expect(fnBody("renderUsageBar")).toContain("catch");
  });
  it("초기 렌더는 start() 와 무관한 배선부에서 1회(복원 중·start 실패에도 빈 섹션이 남지 않게 — 반박 D4·D5)", () => {
    const at = src.indexOf("// ---------- ui wiring ----------");
    expect(at).toBeGreaterThan(0);
    const wiring = stripComments(src.slice(at));
    expect(/\nrenderUsageBar\(\);/.test(wiring)).toBe(true);
  });
  it("🔒 계정 가림 토글이 사이드바도 다시 그린다(같은 키 공유)", () => {
    const i = code.indexOf('acctRedactBtn.addEventListener("click"');
    expect(i).toBeGreaterThan(0);
    expect(code.slice(i, code.indexOf("});", i))).toContain("renderUsageBar()");
  });
});

describe("새 순수 모듈 — 구형 WKWebView 파싱 실패·최상위 부수효과 0(반박 D6)", () => {
  for (const mod of ["./usagebar.ts", "./deptcreate.ts"]) {
    const m = stripComments(read(mod));
    it(`${mod}: 비호환 문법 0`, () => {
      for (const bad of ["(?<=", "(?<!", ".at(", "findLast", "structuredClone", "Object.hasOwn", "replaceAll("])
        expect({ 모듈: mod, 문법: bad, 있음: m.includes(bad) }).toEqual({ 모듈: mod, 문법: bad, 있음: false });
    });
    it(`${mod}: 전역 부수효과 표면(localStorage·document·window·navigator·타이머) 0`, () => {
      for (const bad of ["localStorage", "document.", "window.", "navigator", "setInterval", "setTimeout", "__TAURI__"])
        expect({ 모듈: mod, 표면: bad, 있음: m.includes(bad) }).toEqual({ 모듈: mod, 표면: bad, 있음: false });
    });
    it(`${mod}: 최상위 문장은 선언뿐`, () => {
      const bad = m
        .split("\n")
        .filter((l) => l.length > 0 && !/^\s/.test(l))
        .filter((l) => !/^(import |export |const |function |interface |type |\}|\)|\]|;)/.test(l));
      expect({ 모듈: mod, 최상위_비선언: bad }).toEqual({ 모듈: mod, 최상위_비선언: [] });
    });
  }
});
