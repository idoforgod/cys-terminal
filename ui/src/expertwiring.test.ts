// U17 「전문가용」 칸 · 팀 직접 만들기 **배선** 회귀 핀 — main.ts·index.html·style.css 를 데이터로 읽는다
// (wswiring.test.ts 관례 · 런타임 코드 0줄 · 주석 제거 본문 기준).
//
// 무엇을 지키는가(설계 §3 U17 · 반박 D1·D3·D4·D9 · 조사 F1·F3·F4·F6):
//   · 팀 생성은 **확인 창 1회**를 반드시 지난다 — 버튼·팔레트 모두 같은 흐름이고, addDeptWorkspace 를
//     직접 부르는 우회로가 없다(F1: 팔레트가 확인·가드 없이 `void addDeptWorkspace()` 로 실패를 삼키던 것).
//   · 재진입 가드는 **첫 await 앞**(반박 D1 major) — 목록 조회 await 동안 Enter 재진입으로 확인 창이
//     겹쳐 뜨고 팀이 2개 생기는 창을 닫는다(clipath.test.ts 관례와 같은 계열).
//   · 진행 중 가드는 DOM 버튼이 아니라 모듈 변수(F4) — 버튼이 옮겨지거나 없어도 조용한 no-op 이 되지 않는다.
//   · 카탈로그 부재(exit 3)는 자동 생성하지 않고 **새 대상에 대한** 재확인 창(F3·설계 D5).
//   · 접힌 칸은 `hidden` + 짝 규칙(반박 D4) · 팔레트로 들어와 버튼 rect 가 0 이면 메뉴를 가운데에(반박 D3).
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
const css = read("./style.css");

function fnBody(name: string): string {
  const i = code.indexOf(`function ${name}(`);
  expect({ 함수: name, 존재: i >= 0 }).toEqual({ 함수: name, 존재: true });
  const end = code.indexOf("\n}\n", i);
  return code.slice(i, end > i ? end + 3 : undefined);
}
function enclosingFns(needle: string): string[] {
  const out: string[] = [];
  let at = code.indexOf(needle);
  while (at >= 0) {
    const h = Math.max(code.lastIndexOf("\nfunction ", at), code.lastIndexOf("\nasync function ", at));
    const closed = h >= 0 ? code.indexOf("\n}\n", h) : -1;
    if (h < 0 || (closed >= 0 && closed < at)) out.push("<top>");
    else out.push(/function\s+([A-Za-z0-9_$]+)\s*\(/.exec(code.slice(h, h + 200))?.[1] ?? "<?>");
    at = code.indexOf(needle, at + needle.length);
  }
  return out;
}
/** 가드 대입이 본문의 첫 await 보다 앞인가 + finally 가 가드를 푼다. */
function guardBeforeFirstAwait(name: string) {
  const b = fnBody(name);
  const g = b.indexOf("teamFlowBusy = true;");
  const aw = b.indexOf("await ");
  const fin = b.indexOf("finally");
  expect({ 함수: name, 가드: g >= 0, 첫_await: aw >= 0, 가드가_먼저: g >= 0 && aw >= 0 && g < aw, finally_해제: fin > g && b.indexOf("teamFlowBusy = false;", fin) > fin }).toEqual({
    함수: name, 가드: true, 첫_await: true, 가드가_먼저: true, finally_해제: true,
  });
  expect(b.slice(0, g)).toContain("if (teamFlowBusy) return");
}

describe("전문가용 칸 — 버튼 위치·기본 접힘", () => {
  it("＋부서 버튼이 사이드바 머리줄에서 빠졌다(index.html 에 btn-ws-dept 없음 — 전문가용 칸이 JS 로 만든다)", () => {
    expect(html.includes("btn-ws-dept")).toBe(false);
    const head = html.split("\n").find((l) => l.includes('id="wsbar-head"'))!;
    expect(head.includes(">＋부서<")).toBe(false); // 버튼 글자(＋ 버튼 툴팁의 '부서장' 설명은 허용)
  });
  it("＋ 버튼 안내가 사라진 ＋부서를 가리키지 않고, 두 번째 선언 '거부' 오안내도 없다(반박 D9)", () => {
    const newBtn = html.split("\n").find((l) => l.includes('id="btn-ws-new"'))!;
    expect(newBtn.includes("＋부서")).toBe(false);
    expect(newBtn.includes("거부")).toBe(false);
    expect(newBtn).toContain("전문가용");
  });
  it("mountExpertSection: 토글 + 기본 hidden 본문 + id 유지(btn-ws-dept) · 널 안전 조회", () => {
    const b = fnBody("mountExpertSection");
    expect(b).toContain('getElementById("wsbar-expert")');
    expect(b.includes('getElementById("wsbar-expert")!')).toBe(false);
    expect(b).toContain('"wsbar-expert-body"');
    expect(b).toContain(".hidden = true");
    expect(b).toContain('"btn-ws-dept"');
    expect(b).toContain('"btn-expert-toggle"');
    expect(b).toContain("aria-expanded");
  });
  it("배선부에서 한 번 마운트된다(최상위 호출 · 예외 격리)", () => {
    const tops = enclosingFns("mountExpertSection();");
    expect(tops.includes("<top>")).toBe(true);
  });
  it("펼침 상태는 앱이 켜져 있는 동안만 기억(저장소 미사용 — 설계 D2)", () => {
    expect(fnBody("setExpertOpen").includes("localStorage")).toBe(false);
    expect(fnBody("mountExpertSection").includes("localStorage")).toBe(false);
  });
  it("짝 규칙: #wsbar-expert-body[hidden] { display: none } (display 명시가 hidden 을 무력화하지 않게)", () => {
    expect(/#wsbar-expert-body\[hidden\]\s*\{\s*display:\s*none;?\s*\}/.test(css)).toBe(true);
  });
});

describe("팀 생성 흐름 — 확인 창 1회를 우회하는 길이 없다", () => {
  it("addDeptWorkspace 호출은 launchDept 안 1곳뿐", () => {
    const where = enclosingFns("addDeptWorkspace(").filter((f) => f !== "addDeptWorkspace");
    expect(where).toEqual(["launchDept"]);
  });
  it("launchDept 호출은 확인 창 함수(와 exit 3 재확인 재귀)뿐 — 클릭 처리기·팔레트가 직접 부르지 않는다", () => {
    // 정의 줄 자신(`function launchDept(`)은 launchDept 로 세어진다 — 재귀(exit 3 재확인)와 같은 이름이라 합집합으로 본다.
    const where = enclosingFns("launchDept(");
    expect([...new Set(where)].sort()).toEqual(["confirmAndCreateTeam", "launchDept"]);
  });
  it("confirmAndCreateTeam 의 첫 await 는 확인 창이다", () => {
    const b = fnBody("confirmAndCreateTeam");
    const aw = b.indexOf("await ");
    expect(b.slice(aw, aw + 40)).toContain("confirmModal(");
    expect(b).toContain("buildDeptCreatePlan(");
  });
  it("★재진입 가드가 첫 await 앞 + finally 해제 — 흐름 두 단계 모두(반박 D1)", () => {
    guardBeforeFirstAwait("openTeamCreateFlow");
    guardBeforeFirstAwait("confirmAndCreateTeam");
  });
  it("팔레트 act:dept 는 같은 흐름을 탄다(void addDeptWorkspace 로 실패를 삼키지 않는다 — F1)", () => {
    const line = code.split("\n").find((l) => l.includes('id: "act:dept"'));
    expect(line === undefined).toBe(false);
    const i = code.indexOf('id: "act:dept"');
    const seg = code.slice(i, code.indexOf("},", i) + 2);
    expect(seg).toContain("openTeamCreateFlow(");
    expect(seg.includes("addDeptWorkspace")).toBe(false);
    expect(seg.includes("void openTeamCreateFlow")).toBe(false);
  });
  it("진행 중 가드는 모듈 변수(F4) — DOM 버튼 disabled 에 기대지 않는다", () => {
    const b = fnBody("launchDept");
    expect(b).toContain("deptLaunchInFlight");
    expect(b.includes("deptBtn.disabled) return")).toBe(false);
    expect(b.includes("disabled) return")).toBe(false);
  });
  it("exit 3(카탈로그 부재)는 자동 생성하지 않고 새 대상 재확인 창을 거친다(F3)", () => {
    const b = fnBody("launchDept");
    const f = b.indexOf("fallbackFrom");
    const c = b.indexOf("confirmModal(", f);
    const l = b.indexOf("launchDept(undefined", f);
    expect({ 재확인_문구: f >= 0, 확인창: c > f, 확인창이_먼저: c > f && l > c }).toEqual({
      재확인_문구: true, 확인창: true, 확인창이_먼저: true,
    });
  });
  it("복원 중(!started)이면 조용히 무시하지 않고 안내한다(F6) · 리셋/교대 중 차단", () => {
    const b = fnBody("openTeamCreateFlow");
    expect(b).toContain("!started");
    expect(b).toContain("복원 중");
    expect(b).toContain("daemonActionBlocked()");
    expect(fnBody("confirmAndCreateTeam")).toContain("daemonActionBlocked()");
    expect(fnBody("launchDept")).toContain("daemonActionBlocked()");
  });
  it("접힌 칸(버튼 rect 0)에서 들어오면 메뉴를 창 가운데에(반박 D3 — (0,0) 상단바 위 표시 차단)", () => {
    const b = fnBody("openTeamCreateFlow");
    expect(b).toContain("width > 0");
    expect(b).toContain("innerWidth");
  });
  it("레지스트리·카탈로그 조회에 시간 상한 — 가드가 영구히 잡히지 않게", () => {
    const b = fnBody("openTeamCreateFlow");
    expect(b).toContain('rpcT(invoke("list_depts")');
    expect(b).toContain('rpcT(invoke("read_dept_catalog")');
  });
  it("메뉴 문구 '직접 입력(레거시 dept-N)'(입력칸 없음) 을 고쳤다", () => {
    expect(code.includes("직접 입력(레거시 dept-N)")).toBe(false);
  });
});
