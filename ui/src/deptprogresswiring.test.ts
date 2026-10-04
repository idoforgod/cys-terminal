// 0.14.43 GU — 「팀 직접 만들기」 대기 문구(경과·단계)·팀원 부팅 안내의 main.ts **배선** 회귀 핀 + Tauri 계약 대조.
//
// ★왜 판정 모듈 검체(deptprogress.test.ts)만으로 부족한가(wswiring.test.ts 와 같은 이유): 문구·열쇠가 옳아도 main.ts 가 그 함수를 안 부르거나, 타이머가 엘리먼트 수명에
//   안 묶이거나, 성공 분기 밖에서 createdAt 을 세우면 화면은 그대로 틀린다. 그래서 배선을 기계가 센다 — 두 층으로:
//   ① 소스 문자열 핀(기존 *wiring.test.ts 관례) — 옛 문구 부재 · invoke 의 progressId · listen 1곳 · 새 setInterval 없음(대기 화면 엘리먼트 수명 타이머 1개만) · 저장본 제외 ...
//   ② **실제 본문 실행**(starvednotice.test.ts 관례) — main.ts 의 함수 본문을 그대로 떼어(bun 변환기로 타입을 벗겨) 가짜 DOM·타이머·시계 위에서 돌린다. 문자열 핀을 우회하는
//      변형(핀 문구는 남기고 동작만 바꾸기)을 여기서 잡는다. 함수가 새로운 외부 이름을 쓰기 시작하면 ReferenceError 로 적색이 된다 — 그때 이 대역을 갱신한다.
//
// 지키는 것(티켓 GU §2):
//   · renderDeptPending(ws): deptPendingText 로 주 문구·단계 줄을 **따로** · 1초 타이머는 엘리먼트 수명에 묶임(isConnected 가 아니면 clearInterval · 누수 0) ·
//     aria-live 는 단계 줄에만(주 문구는 off) · 단계 이벤트는 문구 노드만 고친다(전체 render() 안 부름 → 스피너 재생성 없음).
//   · addDeptWorkspace: pendingSince·progressId · createdAt 은 성공 분기에서만 · 첫 안내는 try/catch 안(표시 실패가 생성 성공을 뒤집지 않는다) · 3분기·회수 로직은 그대로.
//   · renderIdleWorkspace: 방금 만든 팀(60초 안)의 빈 탭만 '첫 자리를 붙이는 중' · 종료 실패 탭은 그대로.
//   · 팀원 부팅 안내(R1F-UB 개정): 성공 직후 1회 + 값(열쇠)이 바뀔 때만 갱신(새 setInterval 0 — refreshPaneTitles 끝에서 점검) · **완료 판정은 그 팀 소켓의 좌석 목록(3초 틱이 이미 받는 list_surfaces)의
//     역할 다섯**이다(편성 결과 feed 는 본부 데몬으로 가서 화면이 부서 탭과 대응시킬 수 없다 — S4 B1 · 그 배선은 걷었다) · 15분 상한에 다 안 붙었으면 '확인 필요' 1회 · 목록을 못 받은 틱은 판정을 건너뛴다 ·
//     닫힌 탭은 거둔다 · **× 로 닫은 안내만** 주기 갱신으로 되살리지 않는다(수명 만료는 닫음이 아니다 — S4 m3) · 새 팀 표지(createdAt)는 이 호출이 데몬을 띄웠을 때만(S4 m4) · 표시 전용(어떤 명령도 보내지 않는다).
//   · Tauri 계약: invoke 인자 progressId ↔ allocate_dept_daemon 의 progress_id · 이벤트 'dept-create-progress' `{id, stage}`.
import { describe, it, expect } from "bun:test";
import { readFileSync } from "node:fs";
import {
  deptPendingText,
  deptFormationText,
  deptFormationNoticeKey,
  deptFormationCapped,
  deptFormationToastId,
  deptFirstSeatPending,
  deptFirstSeatRemainingMs,
  deptProgressId,
  parseDeptProgressPayload,
  DEPT_FORMATION_TOAST_PREFIX,
  DEPT_FIRST_SEAT_TEXT,
} from "./deptprogress";
import { toastTtl, needsExpiryBanner, expiryBannerText, VOLATILE_TTL_MS, STICKY_TTL_MS } from "./toastttl";
import { seatPriority, annotateRoles, tidyHoles } from "./seatlayout";
import { advanceGhostStrikes } from "./wsreconcile";
import { feedCreatedToastTitle } from "./feedclass";
import { TEAM_CREATE_KIND } from "./teamproposal";
// ★R1F-UB: 자리 판정 도우미(deptLiveRoles · deptFormationVerdict)는 네임스페이스로 받는다 — 이름이 모듈에 없으면 그 검체만 개별로 적색이 된다(import 연결 오류로 파일 전체가 죽지 않는다).
import * as DP from "./deptprogress";

// bun 변환기(테스트 전용) — main.ts 의 함수 본문에서 타입 표기를 벗겨 실행 가능한 JS 로 만든다. 타입 게이트(tsc)가 이 이름을 알도록 선언한다.
declare const Bun: { Transpiler: new (o: { loader: "ts" }) => { transformSync(code: string): string } };

const read = (rel: string): string => readFileSync(new URL(rel, import.meta.url), "utf-8");
const SRC = read("./main.ts");
const RS = read("../../src-tauri/src/main.rs");
const CSS = read("./style.css");
/** 주석을 걷어낸 코드 본문 — '코드에 있는가'를 묻는 핀은 주석에 속으면 안 된다. */
const strip = (s: string): string =>
  s
    .split("\n")
    .map((l) => (l.trimStart().startsWith("//") ? "" : l.replace(/\s\/\/.*$/, "")))
    .join("\n");
const CODE = strip(SRC);

/** 최상위 함수 하나의 본문(머리 `function name(` 또는 `async function name(` 부터 첫 `\n}\n` 까지). stripped=true 면 주석을 걷은 본문. */
function fnText(name: string, stripped = false): string {
  const hay = stripped ? CODE : SRC;
  const m = new RegExp(`^(?:async )?function ${name}\\(`, "m").exec(hay);
  expect({ 함수: name, 존재: m !== null }).toEqual({ 함수: name, 존재: true });
  const a = (m as RegExpExecArray).index;
  const b = hay.indexOf("\n}\n", a);
  expect({ 함수: name, 끝: b > a }).toEqual({ 함수: name, 끝: true });
  return hay.slice(a, b + 2);
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type AnyFn = (...a: any[]) => any;

/**
 * main.ts 의 함수들을 **그 본문 그대로** 떼어 실행한다 — deps 의 이름이 그 함수들의 자유 변수(문서·타이머·시계·모듈 상태·순수 도우미)가 된다.
 * prelude = 함수들이 **다시 대입하는** 모듈 변수(`let activeWs` 같은 것)를 선언하는 코드, access = 그 변수를 밖에서 읽는 식(예: `getActive: () => activeWs`).
 */
function load(names: string[], deps: Record<string, unknown>, prelude = "", access = ""): Record<string, AnyFn> {
  const tr = new Bun.Transpiler({ loader: "ts" });
  const parts = names.map((n) => tr.transformSync(fnText(n)));
  const keys = Object.keys(deps);
  const body = `${prelude}\n${parts.join("\n")}\nreturn { ${names.join(", ")}${access ? `, ${access}` : ""} };`;
  return new Function(...keys, body)(...keys.map((k) => deps[k])) as Record<string, AnyFn>;
}

/**
 * load() 의 변형 — 함수 본문을 **그대로** 떼어 `with (deps)` 범위 안에서 연다. 자유 변수가 수십 개인 큰 함수(refreshPaneTitles)를 실제로 돌릴 때 쓴다:
 * 이름 하나하나를 인자로 나열하는 대신 deps 의 이름이 곧 그 함수들의 모듈 변수가 된다(함수가 다시 대입하는 `let` 변수도 deps 의 속성으로 읽고 쓴다).
 * deps 에 없는 이름을 쓰면 ReferenceError 로 적색이 된다(조용히 undefined 가 되지 않는다). 시험 코드 전용 — 제품 코드에는 `with` 가 없다.
 */
function loadWith(names: string[], deps: Record<string, unknown>): { fns: Record<string, AnyFn>; scope: Record<string, unknown> } {
  const tr = new Bun.Transpiler({ loader: "ts" });
  const code = names.map((n) => tr.transformSync(fnText(n))).join("\n");
  const scope: Record<string, unknown> = Object.assign(Object.create(null), deps);
  const fns = new Function("__scope", `with (__scope) {\n${code}\nreturn { ${names.join(", ")} };\n}`)(scope) as Record<string, AnyFn>;
  return { fns, scope };
}

// ── 대역: 가짜 DOM · 타이머 · 시계 ──
class FakeEl {
  className = "";
  textContent = "";
  isConnected = false;
  disabled = false;
  attrs: Record<string, string> = {};
  children: FakeEl[] = [];
  constructor(public tag: string) {}
  setAttribute(k: string, v: string): void {
    this.attrs[k] = v;
  }
  append(...kids: FakeEl[]): void {
    this.children.push(...kids);
  }
  appendChild(k: FakeEl): FakeEl {
    this.children.push(k);
    return k;
  }
  addEventListener(): void {}
  /** className 에 cls 토큰이 있는 첫 자손(자기 자신 포함). */
  find(cls: string): FakeEl | undefined {
    if (this.className.split(" ").includes(cls)) return this;
    for (const c of this.children) {
      const r = c.find(cls);
      if (r) return r;
    }
    return undefined;
  }
}
function fakeDoc(): { createElement(tag: string): FakeEl; created: number } {
  const d = {
    created: 0,
    createElement(tag: string): FakeEl {
      d.created++;
      return new FakeEl(tag);
    },
  };
  return d;
}
function fakeTimers() {
  let next = 1;
  const intervals = new Map<number, { fn: () => void; ms: number }>();
  const timeouts: { fn: () => void; ms: number }[] = [];
  const cleared: number[] = [];
  return {
    intervals,
    timeouts,
    cleared,
    setInterval: (fn: () => void, ms: number): number => {
      const id = next++;
      intervals.set(id, { fn, ms });
      return id;
    },
    clearInterval: (id: number): void => {
      cleared.push(id);
      intervals.delete(id);
    },
    setTimeout: (fn: () => void, ms: number): number => {
      timeouts.push({ fn, ms });
      return timeouts.length;
    },
    fire(id: number): void {
      const e = intervals.get(id);
      if (e) e.fn();
    },
  };
}

const NORMAL = (e: string): string => `팀을 만드는 중입니다 — 보통 30초 안팎, 컴퓨터에 따라 1분 넘게 걸릴 수 있어요 · 경과 ${e}`;
const SLOW = (e: string): string => `평소보다 오래 걸리고 있습니다 — 그대로 기다려 주세요(중간에 닫으면 만들던 팀이 정리됩니다) · 경과 ${e}`;

// ════════════════════════════════════════════════════════════════════════════
// ① 소스 문자열 핀
// ════════════════════════════════════════════════════════════════════════════
describe("main.ts 소스 핀 — 대기 문구(renderDeptPending)", () => {
  it("★옛 문구 「최대 십여 초 걸릴 수 있어요」 가 main.ts 에 없다(주석 포함 원문 전체)", () => {
    expect(SRC.includes("최대 십여 초 걸릴 수 있어요")).toBe(false);
    expect(SRC.includes("십여 초")).toBe(false);
    expect(SRC.includes("부서를 준비하고 있습니다")).toBe(false);
  });
  it("render() 는 대기 탭 정보를 넘겨 그린다 — renderDeptPending(ws) · 인자 없는 옛 호출 0", () => {
    expect(CODE.includes("root.appendChild(renderDeptPending(ws))")).toBe(true);
    expect(CODE.includes("renderDeptPending()")).toBe(false);
    expect(CODE.includes("function renderDeptPending(ws: Workspace): HTMLElement")).toBe(true);
  });
  it("renderDeptPending 은 deptPendingText 로 문구를 만들고 주 문구·단계 줄을 따로 그린다 · aria-live 는 단계 줄에만(주 문구는 off · 호스트에는 없음)", () => {
    const f = fnText("renderDeptPending", true);
    expect(f).toContain("deptPendingText(");
    expect(f).toContain('className = "dept-pending-msg"');
    expect(f).toContain('className = "dept-pending-stage"');
    expect(f).toContain('msg.setAttribute("aria-live", "off")');
    expect(f).toContain('stage.setAttribute("aria-live", "polite")');
    expect(f.includes('host.setAttribute("aria-live"')).toBe(false);
    expect(f).toContain('host.setAttribute("aria-busy", "true")');
    expect(f.includes("innerHTML")).toBe(false);
    expect(f).toContain("msg.textContent");
    expect(f).toContain("stage.textContent");
  });
  it("★1초 타이머는 이 엘리먼트 수명에 묶인다 — setInterval 1개 · isConnected 가드 · clearInterval 이 함께(누수 0)", () => {
    const f = fnText("renderDeptPending", true);
    expect(f.split("setInterval(").length - 1).toBe(1);
    const iv = f.indexOf("setInterval(");
    const guard = f.indexOf("host.isConnected", iv);
    const clr = f.indexOf("clearInterval(", iv);
    expect(guard).toBeGreaterThan(iv);
    expect(clr).toBeGreaterThan(guard); // 가드 뒤에서 멈춘다
    expect(f).toContain("}, 1000);");
  });
  it("style.css — 단계 줄 클래스(비어 있으면 자리를 차지하지 않는다)", () => {
    expect(CSS).toContain(".dept-pending-stage {");
    expect(CSS).toContain(".dept-pending-stage:empty {");
  });
});

describe("main.ts 소스 핀 — addDeptWorkspace(진행 id · createdAt · 3분기 불변)", () => {
  const f = fnText("addDeptWorkspace", true);
  it("placeholder 는 pendingSince 를 갖고 진행 id 를 만들어 invoke 에 넘긴다 · invoke 호출은 한 곳", () => {
    expect(f).toContain('const ws: Workspace = { id: wsCounter++, name: "…", tree: null, pending: true, pendingSince: Date.now() };');
    expect(f).toContain("const progressId = deptProgressId(ws.id);");
    expect(f).toContain('invoke("allocate_dept_daemon", { catalogKey, teamSpec, progressId })');
    expect(CODE.split('invoke("allocate_dept_daemon"').length - 1).toBe(1);
  });
  it("★createdAt 은 성공 분기(자기 placeholder 가 그대로 탭이 되는 분기)에서만 — 정확히 1곳 · pending 해제 뒤 render() 앞 · dup·취소·실패 분기에는 없다", () => {
    expect(CODE.split("createdAt = Date.now()").length - 1).toBe(1);
    expect(f.split("createdAt = Date.now()").length - 1).toBe(1);
    const at = f.indexOf("ws.createdAt = Date.now();");
    const dupEnd = f.indexOf("return dup;"); // 멱등 합류 분기의 끝
    const cancel = f.indexOf("return dup ?? null;"); // 취소(회수) 분기의 끝
    const pend = f.indexOf("ws.pending = false;");
    const rend = f.indexOf("render();", pend);
    const refresh = f.indexOf("await refreshPaneTitles();");
    const catchAt = f.indexOf("} catch (e) {");
    expect(dupEnd).toBeGreaterThan(0);
    expect(cancel).toBeGreaterThan(0);
    expect(at).toBeGreaterThan(dupEnd);
    expect(at).toBeGreaterThan(cancel);
    expect(at).toBeGreaterThan(pend);
    expect(at).toBeLessThan(rend); // render() 보다 먼저 서야 빈 탭이 '아직 켜지 않았습니다' 를 비추지 않는다
    expect(rend).toBeLessThan(refresh);
    expect(at).toBeLessThan(catchAt);
    // 실패(catch) 분기에는 createdAt 이 없다
    expect(f.slice(catchAt).includes("createdAt")).toBe(false);
  });
  it("★첫 안내는 try/catch 안 — 표시 실패가 바깥 catch 의 롤백(성공한 팀 삭제)으로 새지 않는다", () => {
    const at = f.indexOf("ws.createdAt = Date.now();");
    const show = f.indexOf('showDeptFormation(ws, "booting");');
    const refresh = f.indexOf("await refreshPaneTitles();");
    expect(show).toBeGreaterThan(at);
    expect(show).toBeLessThan(refresh);
    const seg = f.slice(at, refresh);
    expect(/try \{\s*showDeptFormation\(ws, "booting"\);\s*\} catch \{/.test(seg)).toBe(true);
  });
  it("★새 팀 표지(createdAt)와 첫 안내는 **이 호출이 데몬을 띄웠을 때만**(S4 m4) — `spawn` 단계 표지를 받았거나(pendingSpawned) 표지를 아예 못 받았을(pendingStage 없음 = 구 팩·스트리밍 끔) 때. 둘 다 같은 판정 하나(spawned)를 쓴다", () => {
    const spawned = f.indexOf("const spawned = ws.pendingSpawned === true || ws.pendingStage === undefined;");
    expect(spawned).toBeGreaterThan(f.indexOf("ws.pending = false;")); // 성공 분기 안(pending 해제 뒤)
    expect(f.includes("if (spawned) ws.createdAt = Date.now();")).toBe(true);
    expect(f.indexOf("if (spawned) ws.createdAt = Date.now();")).toBeGreaterThan(spawned);
    const show = f.indexOf('showDeptFormation(ws, "booting");');
    expect(f.slice(f.lastIndexOf("if (spawned)", show), show)).toContain("if (spawned)"); // 첫 안내도 그 판정 아래
    expect(fnText("onDeptCreateProgress", true)).toContain('if (p.stage === "spawn") ws.pendingSpawned = true;');
  });
  it("기존 3분기·회수 로직은 그대로다 — 멱등 합류·취소 회수·실패 롤백의 핵심 줄", () => {
    for (const needle of [
      "const dup = workspaces.find((w) => w !== ws && w.socket && w.socket === info.socket);",
      "if (workspaces.indexOf(ws) < 0) {",
      'if (!dup && info.socket) await invoke("stop_dept_daemon_by_socket", { socket: info.socket }).catch(() => {});',
      "return dup ?? null;",
      "if (dup) {",
      "if (pi >= 0) workspaces.splice(pi, 1);",
      "ws.socket = info.socket;",
      "ws.pending = false;",
      "await refreshPaneTitles();",
      "if (i >= 0) workspaces.splice(i, 1);",
      'if (ws.socket) await invoke("stop_dept_daemon_by_socket", { socket: ws.socket }).catch(() => {});',
      "throw e;",
    ]) {
      expect({ 줄: needle, 있음: f.includes(needle) }).toEqual({ 줄: needle, 있음: true });
    }
    // 순서: 멱등 합류 분기 → 취소 분기가 아니라, 원본 순서(취소 판정 → dup 판정 → 성공 분기)
    expect(f.indexOf("if (workspaces.indexOf(ws) < 0) {")).toBeLessThan(f.indexOf("if (dup) {"));
    expect(f.indexOf("if (dup) {")).toBeLessThan(f.indexOf("ws.socket = info.socket;"));
  });
});

describe("main.ts 소스 핀 — 이벤트·점검 배선(새 setInterval 0 · 표시 전용)", () => {
  it("listen 은 'dept-create-progress' 한 곳 — 핸들러는 onDeptCreateProgress(e.payload)", () => {
    expect(CODE.split('listen("dept-create-progress"').length - 1).toBe(1);
    expect(CODE).toContain('await listen("dept-create-progress", (e) => onDeptCreateProgress(e.payload));');
    // daemon-event listen 바로 뒤(start() 의 같은 구간)
    expect(CODE.indexOf('listen("dept-create-progress"')).toBeGreaterThan(CODE.indexOf('await listen("daemon-event"'));
  });
  it("★새 setInterval 은 대기 화면의 엘리먼트 수명 타이머 1개뿐 — 팀원 부팅 안내 함수들에는 setInterval 이 없다", () => {
    for (const name of ["onDeptCreateProgress", "showDeptFormation", "checkDeptFormationNotices", "noteToastClosedByUser"]) {
      const g = fnText(name, true);
      expect({ 함수: name, setInterval: g.includes("setInterval(") }).toEqual({ 함수: name, setInterval: false });
    }
    expect(CODE.split("setInterval(refreshPaneTitles, 3000);").length - 1).toBe(1); // 기존 3초 틱은 그대로 하나
    // ★총수 핀: v0.14.42 기준 main.ts 의 `setInterval(` 은 9곳이었고, 이 티켓이 더한 것은 renderDeptPending 의 엘리먼트 수명 타이머 1곳(= 10)뿐이다.
    //   다른 티켓이 합법적으로 타이머를 더하면 이 수를 올리면서 사유를 남긴다 — 팀원 부팅 안내용 새 타이머가 슬쩍 들어오는 길을 막는 핀이다.
    expect(CODE.split("setInterval(").length - 1).toBe(10);
    // 어디에 있든 setInterval 의 첫 인자가 팀원 부팅 안내 함수면 금지
    expect(/setInterval\([^;]{0,120}(checkDeptFormationNotices|showDeptFormation|noteToastClosedByUser|deptFormation)/.test(CODE)).toBe(false);
    // 새 코드가 쓰는 setTimeout 은 renderIdleWorkspace 의 일회성 하나뿐(60초 창이 끝날 때 문구를 한 번 고친다)
    for (const name of ["onDeptCreateProgress", "showDeptFormation", "checkDeptFormationNotices", "noteToastClosedByUser", "renderDeptPending"]) {
      expect({ 함수: name, setTimeout: fnText(name, true).includes("setTimeout(") }).toEqual({ 함수: name, setTimeout: false });
    }
  });
  it("★팀원 부팅 안내 점검은 기존 3초 틱(refreshPaneTitles)의 끝에서 try/catch 로 부른다 — 이번 틱에 받은 좌석 역할(seatRolesTick)을 넘긴다 · updateFtRoot 앞", () => {
    const f = fnText("refreshPaneTitles", true);
    const call = f.indexOf("checkDeptFormationNotices(seatRolesTick);");
    expect(call).toBeGreaterThan(0);
    expect(call).toBeGreaterThan(f.indexOf("} finally {")); // 본 루프(try/finally)가 끝난 뒤
    expect(call).toBeLessThan(f.indexOf("updateFtRoot();"));
    expect(/try \{\s*checkDeptFormationNotices\(seatRolesTick\);\s*\} catch \{/.test(f)).toBe(true);
    expect(CODE.split("checkDeptFormationNotices(").length - 1).toBe(2); // 정의 1 + 호출 1(다른 호출 지점이 판정 입력 없이 부르지 않는다)
  });
  it("★B1: 완료 판정의 입력은 이 틱이 **이미 받는** 좌석 목록이다 — 소켓별로 받은 뒤에만 기록하고(시간 초과·오류·진행 중 요청이면 키 없음 = 판정 건너뜀) 새 RPC 는 없다", () => {
    const f = fnText("refreshPaneTitles", true);
    const decl = f.indexOf("const seatRolesTick = new Map<string, string[] | null>();");
    expect(decl).toBeGreaterThan(0);
    expect(decl).toBeLessThan(f.indexOf("try {")); // 소켓 루프(try) 밖에서 선언 — 끝의 점검이 읽는다
    const rpc = f.indexOf("await rpcT(listCall, T_LIST)");
    const rec = f.indexOf('seatRolesTick.set(sk ?? "", deptLiveRoles(r.surfaces));');
    expect(rpc).toBeGreaterThan(0);
    expect(rec).toBeGreaterThan(rpc); // 목록을 **받은 뒤에만** 적는다(받지 못하면 이 줄에 닿지 않는다)
    expect(rec).toBeGreaterThan(f.indexOf("if (!claimFlight(flightKey)) continue;")); // 진행 중 요청으로 건너뛴 소켓도 기록 없음
    expect(rec).toBeLessThan(f.indexOf("pruneHoleUntil();")); // 소켓 루프 안
    expect(f.split("seatRolesTick.set(").length - 1).toBe(1);
    expect((f.match(/invoke\("list_surfaces"/g) ?? []).length).toBe(1); // 새 RPC 0 — 종전 호출 하나가 어차피 받는 결과를 쓴다
    expect(f.split("invoke(").length - 1).toBe(1); // 이 함수의 invoke 는 그 하나뿐
  });
  it("★onDaemonEvent 의 feed.item.created 분기에는 팀원 안내 배선이 없다(S4 B1) — 편성 도구의 feed 는 본부 데몬 소속이라 화면이 부서 탭과 대응시킬 수 없다 · 그 함수·판정 도우미도 없다 · 일반 알림(ℹ 알림)은 종전 그대로", () => {
    const i = CODE.indexOf('if (name === "feed.item.created") {');
    expect(i).toBeGreaterThan(0);
    const seg = CODE.slice(i, CODE.indexOf("refreshFeed();", i));
    expect(seg).toContain("feedCreatedToastTitle(payload.kind)"); // 일반 알림(정보성은 'ℹ 알림')은 그대로
    for (const bad of ["ormation", "stickyToast", "socketForSlug", "createdAt"]) expect({ 낱말: bad, 있음: seg.includes(bad) }).toEqual({ 낱말: bad, 있음: false });
    for (const gone of ["onDeptFormationFeed", "deptFormationStateOfKind", "deptFormationDetail"]) expect({ 이름: gone, main_ts: CODE.includes(gone) }).toEqual({ 이름: gone, main_ts: false });
    expect(seg.includes("confirmModal")).toBe(false);
    expect(seg.includes("runTeamProposalFlow")).toBe(false);
  });
  it("새 함수들은 입력을 신뢰하지 않는 순수 도우미를 지난다 · HTML 삽입 0 · 명령 전송 0 — 단계 표지는 parseDeptProgressPayload · 좌석 목록은 deptLiveRoles(문자열 역할만) · 판정은 deptFormationVerdict", () => {
    expect(fnText("onDeptCreateProgress", true)).toContain("parseDeptProgressPayload(");
    expect(fnText("checkDeptFormationNotices", true)).toContain("deptFormationVerdict(");
    expect(fnText("refreshPaneTitles", true)).toContain("deptLiveRoles(r.surfaces)");
    for (const name of ["onDeptCreateProgress", "showDeptFormation", "checkDeptFormationNotices", "noteToastClosedByUser", "renderDeptPending"]) {
      const g = fnText(name, true);
      for (const bad of ["innerHTML", "outerHTML", "insertAdjacentHTML", "invoke(", "send_input", "send_key", "feed_reply", "cys send"]) {
        expect({ 함수: name, 낱말: bad, 있음: g.includes(bad) }).toEqual({ 함수: name, 낱말: bad, 있음: false });
      }
    }
  });
  it("안내 토스트 id 는 순수 모듈이 정한다 — main.ts 에 `dept-formation:` 리터럴 0", () => {
    expect(CODE.includes('"dept-formation:')).toBe(false);
    expect(CODE.includes("'dept-formation:")).toBe(false);
    expect(CODE.includes("`dept-formation:")).toBe(false);
    expect(fnText("showDeptFormation", true)).toContain("deptFormationToastId(ws.socket)");
  });
});

describe("main.ts 소스 핀 — Workspace 표시 전용 필드 · 저장본 제외 · renderIdleWorkspace", () => {
  it("Workspace 에 표시 전용 필드 6개(pendingSince·pendingStage·pendingSpawned·createdAt·formationDone·formationView) — 안내 상태에는 사유(detail)가 없다(feed 본문을 싣던 배선을 걷었다)", () => {
    const a = CODE.indexOf("interface Workspace {");
    const iface = CODE.slice(a, CODE.indexOf("\n}\n", a));
    for (const field of ["pendingSince?: number;", "pendingStage?: string;", "pendingSpawned?: boolean;", "createdAt?: number;", "formationDone?: boolean;", "formationView?: {"]) {
      expect({ 필드: field, 있음: iface.includes(field) }).toEqual({ 필드: field, 있음: true });
    }
    expect(iface).toContain("formationView?: { state: DeptFormationState; key: string; muted?: boolean };");
  });
  it("★저장본에는 실리지 않는다 — saveLayout 이 여섯 필드를 직렬화에서 턴다(기존 첫 map 한 줄은 그대로)", () => {
    const f = fnText("saveLayout", true);
    expect(f).toContain("norm.map(({ deleting: _d, stopFailed: _s, ...w }) => w)");
    for (const strip of ["pendingSince: _ps", "pendingStage: _pg", "pendingSpawned: _sp", "createdAt: _ca", "formationDone: _fd", "formationView: _fv"]) {
      expect({ 제외: strip, 있음: f.includes(strip) }).toEqual({ 제외: strip, 있음: true });
    }
    expect(f).toContain("JSON.stringify({ workspaces: persisted,");
    // 순서: 두 map 모두 persisted 를 만드는 한 문장 안
    expect(f.indexOf("deleting: _d")).toBeLessThan(f.indexOf("pendingSince: _ps"));
    expect(f.indexOf("pendingSince: _ps")).toBeLessThan(f.indexOf("JSON.stringify("));
  });
  it("renderIdleWorkspace — 방금 만든 팀의 문구 분기 1개 · 기존 문구·버튼 구조는 그대로", () => {
    const f = fnText("renderIdleWorkspace", true);
    expect(f).toContain("deptFirstSeatPending(ws.createdAt, Date.now())");
    expect(f).toContain("DEPT_FIRST_SEAT_TEXT");
    expect(f).toContain("ws.stopFailed");
    expect(f).toContain("종료 실패 — 탭을 다시 닫아 재시도 / 지금 켜기를 누르면 삭제가 취소됩니다");
    expect(f).toContain("이 부서는 아직 켜지 않았습니다 — 아래 버튼을 누르거나, 앱을 다시 켜면 준비됩니다.");
    expect(f).toContain("이 워크스페이스에 아직 열린 창이 없습니다 — 아래 버튼을 누르면 새 셸이 열립니다.");
    expect(f).toContain("box.append(msg, btn);");
    expect(f.split("setTimeout(").length - 1).toBe(1);
  });
});

describe("addDeptWorkspace — 실제 본문 실행(진행 id · createdAt 은 성공 분기에서만 · 3분기·회수 로직 불변 · 표시 실패가 성공을 뒤집지 않는다)", () => {
  type Inv = { cmd: string; args: Record<string, unknown> };
  const T = 7_000_000;
  const INFO = { socket: "/s/new.sock", socket_slug: "slug-new", name: "dept-9", display_name: "영업팀" };
  /** markers = 이 호출이 도는 동안 Tauri 가 올리는 팩의 단계 표지(`dept-create-progress` `{id, stage}`) — **실제 onDeptCreateProgress 본문**으로 흘려 넣는다(S4 m4: 표지로 새 팀을 가린다). */
  function setup(over: { invoke?: (cmd: string, args: Record<string, unknown>, ctx: Ctx) => Promise<unknown>; showThrows?: boolean; markers?: string[] } = {}) {
    const invokes: Inv[] = [];
    const workspaces: Record<string, unknown>[] = [];
    const socketForSlug = new Map<string, string>();
    const events: string[] = [];
    const shown: unknown[] = [];
    const ctx: Ctx = { workspaces, events };
    let fns: Record<string, AnyFn> = {};
    const deps = {
      workspaces,
      socketForSlug,
      Date: { now: () => T },
      deptProgressId,
      parseDeptProgressPayload,
      deptPendingPainters: new Map<number, () => void>(),
      invoke: (cmd: string, args: Record<string, unknown>): Promise<unknown> => {
        invokes.push({ cmd, args });
        if (cmd === "allocate_dept_daemon") for (const stage of over.markers ?? []) fns.onDeptCreateProgress({ id: String(args.progressId), stage });
        return over.invoke ? over.invoke(cmd, args, ctx) : Promise.resolve(INFO);
      },
      render: (): void => void events.push("render"),
      refreshPaneTitles: (): Promise<void> => {
        events.push("refresh");
        return Promise.resolve();
      },
      collectSids: (tree: unknown): number[] => (tree ? [1] : []),
      setFocus: (): void => void events.push("focus"),
      showDeptFormation: (ws: unknown, state: string, seated: unknown): void => {
        events.push("show");
        shown.push({ ws, state, seated });
        if (over.showThrows) throw new Error("표시 실패(검체)");
      },
    };
    fns = load(["addDeptWorkspace", "onDeptCreateProgress"], deps, "let wsCounter = 41; let activeWs = 0;", "getActive: () => activeWs, getCounter: () => wsCounter");
    return { fns, invokes, workspaces, socketForSlug, events, shown };
  }
  type Ctx = { workspaces: Record<string, unknown>[]; events: string[] };

  it("★성공 — 대기 탭이 그대로 탭이 된다: 진행 id 가 invoke 에 실리고(dp-<탭 번호>) pendingSince 가 서고, createdAt 은 성공 분기에서 한 번 선다 · 첫 안내는 render 뒤·refresh 앞", async () => {
    const { fns, invokes, workspaces, socketForSlug, events, shown } = setup();
    const ws = (await fns.addDeptWorkspace("sales", undefined)) as Record<string, unknown>;
    expect(invokes.length).toBe(1);
    expect(invokes[0].cmd).toBe("allocate_dept_daemon");
    expect(invokes[0].args).toEqual({ catalogKey: "sales", teamSpec: undefined, progressId: "dp-41" });
    expect(invokes[0].args.progressId).toBe(deptProgressId(ws.id as number));
    expect(ws.id).toBe(41);
    expect(workspaces).toEqual([ws]);
    expect(ws.name).toBe("영업팀");
    expect(ws.socket).toBe("/s/new.sock");
    expect(ws.pending).toBe(false);
    expect(ws.pendingSince).toBe(T);
    expect(ws.createdAt).toBe(T);
    expect(socketForSlug.get("slug-new")).toBe("/s/new.sock");
    expect(shown.length).toBe(1);
    expect((shown[0] as { state: string; seated: unknown }).state).toBe("booting");
    expect((shown[0] as { state: string; seated: unknown }).seated).toBeUndefined(); // 첫 안내는 '켜는 중' — 자리 판정 인자 없음
    // 순서: 대기 탭 render → (성공) render → 첫 안내 → 즉시 입양(refresh)
    expect(events).toEqual(["render", "render", "show", "refresh"]);
    expect(fns.getActive()).toBe(0);
  });
  it("진행 id 는 호출마다 다르다(탭 번호가 호출마다 새로 나온다)", async () => {
    const { fns, invokes } = setup();
    await fns.addDeptWorkspace(undefined, undefined);
    await fns.addDeptWorkspace(undefined, undefined);
    expect(invokes[0].args.progressId).toBe("dp-41");
    expect(invokes[1].args.progressId).toBe("dp-42");
    expect(fns.getCounter()).toBe(43);
  });
  it("★멱등 합류(dup) — 같은 소켓의 다른 탭이 이미 있으면 대기 탭을 버리고 그 탭을 돌려준다 · createdAt·첫 안내 없음", async () => {
    const dup: Record<string, unknown> = { id: 5, name: "기존", socket: "/s/new.sock", tree: { type: "pane", sid: 3 } };
    const { fns, workspaces, events, shown } = setup({
      invoke: (cmd, _a, ctx) => {
        if (cmd === "allocate_dept_daemon") ctx.workspaces.unshift(dup); // 같은 소켓의 기존 탭(연타·재호출)
        return Promise.resolve(INFO);
      },
    });
    const r = (await fns.addDeptWorkspace(undefined, undefined)) as Record<string, unknown>;
    expect(r === dup).toBe(true);
    expect(workspaces).toEqual([dup]); // 대기 탭은 폐기됐다
    expect(dup.createdAt).toBeUndefined();
    expect(shown.length).toBe(0);
    expect(events.includes("focus")).toBe(true); // 기존 탭의 첫 pane 으로 포커스
    expect(fns.getActive()).toBe(0);
  });
  it("★취소(생성 중 탭 ×) — 같은 소켓의 다른 탭이 없으면 방금 만든 데몬을 회수하고 null · createdAt 없음", async () => {
    const { fns, invokes, workspaces, shown } = setup({
      invoke: (cmd, _a, ctx) => {
        if (cmd === "allocate_dept_daemon") ctx.workspaces.length = 0; // 사용자가 대기 탭을 닫았다
        return Promise.resolve(cmd === "allocate_dept_daemon" ? INFO : undefined);
      },
    });
    const r = await fns.addDeptWorkspace(undefined, undefined);
    expect(r).toBeNull();
    expect(invokes.map((i) => i.cmd)).toEqual(["allocate_dept_daemon", "stop_dept_daemon_by_socket"]);
    expect(invokes[1].args).toEqual({ socket: "/s/new.sock" });
    expect(workspaces.length).toBe(0);
    expect(shown.length).toBe(0);
  });
  it("취소했지만 같은 소켓의 다른 탭이 있으면 회수하지 않고 그 탭을 돌려준다", async () => {
    const other: Record<string, unknown> = { id: 5, name: "기존", socket: "/s/new.sock", tree: null };
    const { fns, invokes } = setup({
      invoke: (cmd, _a, ctx) => {
        if (cmd === "allocate_dept_daemon") ctx.workspaces.splice(0, ctx.workspaces.length, other);
        return Promise.resolve(INFO);
      },
    });
    const r = (await fns.addDeptWorkspace(undefined, undefined)) as Record<string, unknown>;
    expect(r === other).toBe(true);
    expect(invokes.map((i) => i.cmd)).toEqual(["allocate_dept_daemon"]);
  });
  it("★실패 — 대기 탭을 롤백하고(유령 탭 없음) 오류를 그대로 다시 던진다 · socket 이 미정이면 회수 호출 없음 · createdAt 없음", async () => {
    const boom = new Error("dept-create:3:실패 사유");
    const { fns, invokes, workspaces, events, shown } = setup({ invoke: () => Promise.reject(boom) });
    let caught: unknown = null;
    try {
      await fns.addDeptWorkspace(undefined, undefined);
    } catch (e) {
      caught = e;
    }
    expect(caught === boom).toBe(true);
    expect(workspaces.length).toBe(0);
    expect(invokes.map((i) => i.cmd)).toEqual(["allocate_dept_daemon"]);
    expect(events).toEqual(["render", "render"]); // 대기 탭 render + 롤백 render
    expect(shown.length).toBe(0);
  });
  it("★표시 실패(첫 안내가 던져도)는 성공한 팀을 뒤집지 않는다 — 롤백·회수 없이 탭을 돌려준다(refresh 도 그대로 돈다)", async () => {
    const { fns, invokes, workspaces, events } = setup({ showThrows: true });
    const ws = (await fns.addDeptWorkspace(undefined, undefined)) as Record<string, unknown>;
    expect(workspaces).toEqual([ws]);
    expect(ws.pending).toBe(false);
    expect(ws.createdAt).toBe(T);
    expect(invokes.map((i) => i.cmd)).toEqual(["allocate_dept_daemon"]); // 회수(stop) 없음
    expect(events).toEqual(["render", "render", "show", "refresh"]);
  });
  it("팀 제안 경로(teamSpec)도 같은 invoke 에 진행 id 를 싣는다 — 새 호출 경로 0", async () => {
    const { fns, invokes } = setup();
    const spec = { id: "t1", display: "팀", purpose: "일" };
    await fns.addDeptWorkspace(undefined, spec);
    expect(invokes[0].args).toEqual({ catalogKey: undefined, teamSpec: spec, progressId: "dp-41" });
  });

  // ── ★S4 m4 — '이 세션에서 새로 만든 팀' 표지가 재사용 반환에도 섰다. 기존 팀을 돌려받은 호출(create 의 REUSE_UP·allocate 의 멱등 반환·이미 가동 중 재사용)에도
  //    대기 탭이 그대로 탭이 되므로 createdAt 이 서서 「첫 자리를 붙이는 중」·「팀원을 켜는 중」이 떴다(문서는 '새로 만든 팀에만'이라 적었다). 이제 그 호출에서 **데몬을 띄웠다는 표지(`spawn`)** 를
  //    받았을 때만 선다. 표지 시퀀스는 cys-dept 의 dept_stage 호출 위치 그대로다(create 2541~2604 · allocate 2191~2299): 신규·재기동 = reserve probe spawn wait up seat done ·
  //    이미 가동 중 재사용 = reserve probe up seat done · REUSE_UP/BOOTING = reserve done · allocate 멱등 반환 = done.
  describe("★S4 m4 — 새 팀 표지(createdAt)와 첫 안내는 이 호출이 데몬을 띄웠을 때만", () => {
    const NEW_SEQ = ["reserve", "probe", "spawn", "wait", "up", "seat", "done"];
    const REUSE_SEQS: [string, string[]][] = [
      ["create REUSE_UP·REUSE_BOOTING(reserve 뒤 곧바로 done)", ["reserve", "done"]],
      ["allocate 멱등 반환(제안 id 가 같은 팀 — done 만)", ["done"]],
      ["이미 가동 중인 데몬 재사용(spawn·wait 없이 up·seat)", ["reserve", "probe", "up", "seat", "done"]],
    ];
    it("★기존 팀을 돌려받은 호출(spawn 표지 없음 · 다른 표지는 옴) — 대기 탭은 그대로 탭이 되지만 createdAt·첫 안내는 없다", async () => {
      for (const [이름, markers] of REUSE_SEQS) {
        const { fns, workspaces, shown, events } = setup({ markers });
        const ws = (await fns.addDeptWorkspace("sales", undefined)) as Record<string, unknown>;
        expect({ 사례: 이름, createdAt: ws.createdAt, 안내: shown.length }).toEqual({ 사례: 이름, createdAt: undefined, 안내: 0 });
        expect(ws.pending).toBe(false); // 탭 자체는 종전대로 선다
        expect(ws.socket).toBe("/s/new.sock");
        expect(workspaces).toEqual([ws]);
        expect(events).toEqual(["render", "render", "refresh"]); // 첫 안내(show)가 없다
        expect(ws.pendingStage).toBe(markers[markers.length - 1]);
      }
    });
    it("★이 호출이 데몬을 띄운 경우(신규·죽은 팀 재기동 — spawn 표지를 받음) — createdAt 과 첫 안내가 선다 · render 앞에 선다", async () => {
      const { fns, shown, events } = setup({ markers: NEW_SEQ });
      const ws = (await fns.addDeptWorkspace("sales", undefined)) as Record<string, unknown>;
      expect(ws.createdAt).toBe(T);
      expect(ws.pendingSpawned).toBe(true);
      expect(shown.length).toBe(1);
      expect((shown[0] as { state: string }).state).toBe("booting");
      expect(events).toEqual(["render", "render", "show", "refresh"]);
    });
    it("spawn 이 중간에 한 번만 와도(다른 표지가 빠져도) 이 호출이 띄운 것이다 · 표지 순서와 무관하다", async () => {
      for (const markers of [["spawn"], ["probe", "spawn"], ["spawn", "done"], ["done", "spawn"]]) {
        const { fns, shown } = setup({ markers });
        const ws = (await fns.addDeptWorkspace(undefined, undefined)) as Record<string, unknown>;
        expect({ markers, createdAt: ws.createdAt, 안내: shown.length }).toEqual({ markers, createdAt: T, 안내: 1 });
      }
    });
    it("★표지를 아예 못 받았으면(진행 표지를 내지 않는 구 팩 · 스트리밍 끔 · 이벤트 유실) 구분할 수 없으므로 현행 유지 — 새 팀으로 보고 안내를 낸다", async () => {
      const { fns, shown } = setup({ markers: [] });
      const ws = (await fns.addDeptWorkspace("sales", undefined)) as Record<string, unknown>;
      expect(ws.pendingStage).toBeUndefined(); // 구분 근거: 표지가 한 번도 안 왔다
      expect(ws.pendingSpawned).toBeUndefined();
      expect(ws.createdAt).toBe(T);
      expect(shown.length).toBe(1);
    });
    it("신뢰할 수 없는 표지(모양 오류·다른 호출의 id·잘못된 키)는 판정에 끼지 않는다 — 그런 표지만 온 호출은 '표지 없음'(구 팩)과 같다", async () => {
      const { fns, shown, invokes } = setup({
        invoke: (cmd, args) => {
          if (cmd === "allocate_dept_daemon") {
            const id = String(args.progressId);
            for (const bad of [{ id: "dp-999", stage: "spawn" }, { id, stage: "Spawn" }, { id, stage: 7 }, { stage: "spawn" }, null, "spawn"]) fns.onDeptCreateProgress(bad);
          }
          return Promise.resolve(INFO);
        },
      });
      const ws = (await fns.addDeptWorkspace("sales", undefined)) as Record<string, unknown>;
      expect(invokes.length).toBe(1);
      expect(ws.pendingSpawned).toBeUndefined();
      expect(ws.pendingStage).toBeUndefined();
      expect(ws.createdAt).toBe(T); // '표지 없음' 경로(현행 유지)
      expect(shown.length).toBe(1);
    });
    it("멱등 합류(dup)·취소·실패 분기는 이 판정과 무관하게 종전 그대로 — createdAt·첫 안내 없음", async () => {
      const dup: Record<string, unknown> = { id: 5, name: "기존", socket: "/s/new.sock", tree: { type: "pane", sid: 3 } };
      const a = setup({
        markers: NEW_SEQ,
        invoke: (cmd, _a, ctx) => {
          if (cmd === "allocate_dept_daemon") ctx.workspaces.unshift(dup);
          return Promise.resolve(INFO);
        },
      });
      const r = (await a.fns.addDeptWorkspace(undefined, undefined)) as Record<string, unknown>;
      expect(r === dup).toBe(true);
      expect(dup.createdAt).toBeUndefined();
      expect(a.shown.length).toBe(0);
      const b = setup({ markers: NEW_SEQ, invoke: () => Promise.reject(new Error("dept-create:1:x")) });
      let caught: unknown = null;
      try {
        await b.fns.addDeptWorkspace(undefined, undefined);
      } catch (e) {
        caught = e;
      }
      expect(caught === null).toBe(false); // 실패는 그대로 다시 던진다
      expect(b.shown.length).toBe(0);
    });
  });
});

describe("onDaemonEvent feed.item.created 분기 — 실제 본문 실행(실제 꼴: 본부 데몬이 낸 편성 feed 는 일반 알림만 띄우고 팀원 안내는 건드리지 않는다 · S4 B1)", () => {
  const start = CODE.indexOf('    if (name === "feed.item.created") {');
  expect({ 분기: "feed.item.created", 존재: start >= 0 }).toEqual({ 분기: "feed.item.created", 존재: true });
  const branch = CODE.slice(start, CODE.indexOf("\n    }\n", start) + 6);
  /** 분기 본문을 **실제 도우미**(feedCreatedToastTitle · TEAM_CREATE_KIND)와 함께 돌린다 — 분기가 이 밖의 이름(stickyToast 등)을 쓰면 ReferenceError 로 적색이 된다. */
  function run(payload: Record<string, unknown>, event: Record<string, unknown>) {
    const calls: { fn: string; args: unknown[] }[] = [];
    const rec = (fn: string) => (...args: unknown[]) => {
      calls.push({ fn, args });
    };
    const js = new Bun.Transpiler({ loader: "ts" }).transformSync(branch);
    const f = new Function("name", "event", "payload", "toast", "feedCreatedToastTitle", "TEAM_CREATE_KIND", "scheduleFeedSwitchIfStillPending", js);
    f("feed.item.created", event, payload, rec("toast"), feedCreatedToastTitle, TEAM_CREATE_KIND, rec("scheduleFeedSwitchIfStillPending"));
    return calls;
  }
  /** 편성 도구(javis_formation.py _feed_for_state)가 실제로 내는 완결 feed — 본부 데몬 소속이다(소켓 지정 없는 `cys feed push`). */
  const HQ_COMPLETE = { kind: "formation-complete", title: "부서 팀 편성 완결", body: "master + 4종 의무 노드(cso·worker·reviewer-gemini·reviewer-codex) 전부 기동 완료.", request_id: "feed-17", wait: false };
  const HQ_SLUG = "9f3a07c1d2e4b856"; // sock_slug(본부 소켓) 꼴(16자리 16진) — 부서 소켓만 socketForSlug 에 들어 있다
  it("★본부 slug 의 formation-complete feed → 일반 알림 1건(ℹ 알림 · 부서 팀 편성 완결)만 — 다른 호출 0(팀원 안내는 좌석 목록의 역할로 판정한다)", () => {
    const calls = run(HQ_COMPLETE, { socket_slug: HQ_SLUG });
    expect(calls).toEqual([{ fn: "toast", args: ["feed", "ℹ 알림", "부서 팀 편성 완결"] }]);
  });
  it("formation-partial·pending·failed 도 같다 — 일반 알림만(자동 전환 예약은 wait/auto_route 가 있을 때만 · 종전 그대로)", () => {
    for (const [kind, title] of [["formation-partial", "부서 팀 부분 편성"], ["formation-pending", "부서 팀 편성 대기(CLI 미설치)"], ["formation-failed", "부서 팀 편성 실패"]]) {
      const calls = run({ ...HQ_COMPLETE, kind, title }, { socket_slug: HQ_SLUG });
      expect({ kind, calls }).toEqual({ kind, calls: [{ fn: "toast", args: ["feed", "ℹ 알림", title] }] });
    }
    const wait = run({ ...HQ_COMPLETE, kind: "formation-failed", wait: true, request_id: "feed-18" }, { socket_slug: HQ_SLUG });
    expect(wait.map((c) => c.fn)).toEqual(["toast", "scheduleFeedSwitchIfStillPending"]);
    expect(wait[1].args).toEqual(["feed-18", false]);
  });
  it("승인 요청 종류는 종전 토스트 그대로('📥 승인 요청') · 팀 제안 종류(team-create-request)는 종전 안내 그대로 — 이 분기가 편성 배선을 가졌던 자리는 비었다", () => {
    const approval = run({ kind: "approval", title: "권한 요청", request_id: "r1" }, { socket_slug: HQ_SLUG });
    expect(approval).toEqual([{ fn: "toast", args: ["feed", "📥 승인 요청", "권한 요청"] }]);
    const team = run({ kind: TEAM_CREATE_KIND, title: "영업팀", request_id: "r2" }, { socket_slug: HQ_SLUG });
    expect(team.length).toBe(1);
    expect(team[0].args[1]).toBe("팀 만들기 제안 1건");
  });
});

describe("saveLayout — 실제 본문 실행(저장본에는 런타임 전용·표시 전용 필드가 실리지 않는다)", () => {
  it("★deleting·stopFailed 와 이번에 더한 여섯 필드(pendingSince·pendingStage·pendingSpawned·createdAt·formationDone·formationView)는 직렬화에서 빠지고 나머지는 그대로 저장된다 · 대기 탭은 저장 제외", () => {
    let stored = "";
    const workspaces: Record<string, unknown>[] = [
      {
        id: 1,
        name: "dept-1",
        socket: "/s/1.sock",
        tree: { type: "pane", sid: 4 },
        groupId: 3,
        layoutManual: true,
        daemonEpoch: "e1",
        // 런타임 전용 — 저장되면 안 된다
        createdAt: 1234,
        formationDone: true,
        formationView: { state: "booting", key: "k", muted: true },
        pendingSince: 99,
        pendingStage: "wait",
        pendingSpawned: true,
        deleting: true,
        stopFailed: true,
      },
      { id: 2, name: "…", tree: null, socket: undefined, pending: true, pendingSince: 5, pendingStage: "probe" },
      { id: 3, name: "본부", tree: { type: "pane", sid: 7 } },
    ];
    const groups = [{ id: 3, name: "g", collapsed: false, pinned: false }];
    const fns = load(["collectSids", "normalizeWorkspaces", "normalizeGroups", "saveLayout"], {
      layoutLoaded: true,
      workspaces,
      groups,
      activeWs: 0,
      wsCounter: 4,
      groupCounter: 4,
      LAYOUT_KEY: "cys-layout-v2",
      localStorage: { setItem: (_k: string, v: string): void => void (stored = v) },
    });
    fns.saveLayout();
    const saved = JSON.parse(stored) as { workspaces: Record<string, unknown>[]; counter: number };
    expect(saved.workspaces.map((w) => w.id)).toEqual([1, 3]); // 대기 탭(id 2) 제외
    const w1 = saved.workspaces[0];
    for (const gone of ["createdAt", "formationDone", "formationView", "pendingSince", "pendingStage", "pendingSpawned", "deleting", "stopFailed", "pending"]) {
      expect({ 필드: gone, 저장됨: gone in w1 }).toEqual({ 필드: gone, 저장됨: false });
    }
    expect({ id: w1.id, name: w1.name, socket: w1.socket, groupId: w1.groupId, layoutManual: w1.layoutManual, daemonEpoch: w1.daemonEpoch, tree: w1.tree }).toEqual({
      id: 1,
      name: "dept-1",
      socket: "/s/1.sock",
      groupId: 3,
      layoutManual: true,
      daemonEpoch: "e1",
      tree: { type: "pane", sid: 4 },
    });
    expect(saved.counter).toBe(4);
    // 원본 객체는 건드리지 않는다(런타임 상태가 사라지면 안 된다 — 복사본만 걸렀다)
    expect(workspaces[0].createdAt).toBe(1234);
    expect(workspaces[0].formationDone).toBe(true);
    expect(workspaces[0].stopFailed).toBe(true);
  });
});

describe("Tauri 계약 대조 — invoke 인자 · 이벤트 이름 · payload", () => {
  it("invoke 의 progressId ↔ allocate_dept_daemon 의 progress_id(기존 catalogKey↔catalog_key·teamSpec↔team_spec 와 같은 camelCase→snake_case)", () => {
    expect(RS).toContain("progress_id: Option<String>,");
    expect(RS).toContain("catalog_key: Option<String>,");
    expect(RS).toContain("team_spec: Option<cys::team_spec::TeamSpec>,");
    expect(CODE).toContain("{ catalogKey, teamSpec, progressId }");
  });
  it("이벤트 이름 'dept-create-progress' 와 payload 모양 {id, stage} 가 양쪽이 같다", () => {
    expect(RS).toContain('emit_app.emit("dept-create-progress", json!({"id": emit_id, "stage": key}))');
    expect(CODE).toContain('listen("dept-create-progress"');
    // 이벤트를 받는 쪽이 읽는 키(id·stage)는 순수 파서가 정한다
    expect(parseDeptProgressPayload({ id: "dp-1", stage: "probe" })).toEqual({ id: "dp-1", stage: "probe" });
  });
});

// ════════════════════════════════════════════════════════════════════════════
// ② 실제 본문 실행 — 가짜 DOM·타이머·시계 위에서
// ════════════════════════════════════════════════════════════════════════════
describe("renderDeptPending — 실제 본문 실행(문구·타이머 수명·단계 갱신)", () => {
  function setup(ws: Record<string, unknown>) {
    const doc = fakeDoc();
    const t = fakeTimers();
    const clock = { now: 1_000_000 };
    const painters = new Map<number, () => void>();
    const fns = load(["renderDeptPending"], {
      document: doc,
      setInterval: t.setInterval,
      clearInterval: t.clearInterval,
      Date: { now: () => clock.now },
      deptPendingText,
      deptPendingPainters: painters,
    });
    const run = (): FakeEl => fns.renderDeptPending(ws) as FakeEl;
    return { doc, t, clock, painters, run };
  }
  const nodes = (h: FakeEl) => ({
    msg: h.find("dept-pending-msg") as FakeEl,
    stage: h.find("dept-pending-stage") as FakeEl,
    spin: h.find("dept-spinner") as FakeEl,
  });

  it("구조·접근성 — 호스트는 aria-busy 만 · 주 문구는 aria-live off · 단계 줄만 aria-live polite · 스피너는 aria-hidden", () => {
    const ws = { id: 7, name: "…", tree: null, pending: true, pendingSince: 1_000_000 };
    const { run } = setup(ws);
    const host = run();
    const { msg, stage, spin } = nodes(host);
    expect(host.className).toBe("pane dept-pending");
    expect(host.attrs["aria-busy"]).toBe("true");
    expect(host.attrs["aria-live"]).toBeUndefined();
    expect(msg.attrs["aria-live"]).toBe("off");
    expect(stage.attrs["aria-live"]).toBe("polite");
    expect(spin.attrs["aria-hidden"]).toBe("true");
  });
  it("첫 그림 — 경과 0초 · 단계 줄은 비어 있다(구 팩·표지 전)", () => {
    const ws = { id: 7, name: "…", tree: null, pending: true, pendingSince: 1_000_000 };
    const { run } = setup(ws);
    const { msg, stage } = nodes(run());
    expect(msg.textContent).toBe(NORMAL("0초"));
    expect(stage.textContent).toBe("");
  });
  it("pendingSince 가 없는 대기 탭도 방어적으로 지금부터 센다(경과 0초 · pendingSince 가 채워진다)", () => {
    const ws: Record<string, unknown> = { id: 8, name: "…", tree: null, pending: true };
    const { run, clock } = setup(ws);
    const { msg } = nodes(run());
    expect(msg.textContent).toBe(NORMAL("0초"));
    expect(ws.pendingSince).toBe(clock.now);
  });
  it("★1초 타이머가 문구를 고친다 — 12초 → 경과 12초 · 92초 → '평소보다 오래' 문구", () => {
    const ws = { id: 7, name: "…", tree: null, pending: true, pendingSince: 1_000_000 };
    const { run, t, clock } = setup(ws);
    const host = run();
    host.isConnected = true;
    const { msg } = nodes(host);
    expect(t.intervals.size).toBe(1);
    const [[id, iv]] = [...t.intervals.entries()];
    expect(iv.ms).toBe(1000);
    clock.now += 12_000;
    t.fire(id);
    expect(msg.textContent).toBe(NORMAL("12초"));
    clock.now += 80_000;
    t.fire(id);
    expect(msg.textContent).toBe(SLOW("1분 32초"));
    expect(t.cleared).toEqual([]); // 붙어 있는 동안은 멈추지 않는다
  });
  it("★엘리먼트가 화면에서 떨어지면(isConnected=false) 다음 틱에서 스스로 clearInterval · 문구 갱신기도 지운다 · 문구는 더 안 고친다(누수 0)", () => {
    const ws = { id: 7, name: "…", tree: null, pending: true, pendingSince: 1_000_000 };
    const { run, t, clock, painters } = setup(ws);
    const host = run();
    host.isConnected = true;
    const { msg } = nodes(host);
    const [[id]] = [...t.intervals.entries()];
    expect(painters.has(7)).toBe(true);
    host.isConnected = false; // render() 가 호스트를 갈아 끼웠거나 탭을 바꿨다
    clock.now += 30_000;
    t.fire(id);
    expect(t.cleared).toEqual([id]);
    expect(t.intervals.size).toBe(0);
    expect(painters.has(7)).toBe(false);
    expect(msg.textContent).toBe(NORMAL("0초")); // 갱신하지 않았다
  });
  it("아직 한 번도 붙지 않은 호스트(생성만 되고 버려짐)도 첫 틱에서 멈춘다", () => {
    const ws = { id: 9, name: "…", tree: null, pending: true, pendingSince: 1_000_000 };
    const { run, t } = setup(ws);
    run(); // isConnected 기본 false
    const [[id]] = [...t.intervals.entries()];
    t.fire(id);
    expect(t.cleared).toEqual([id]);
  });
  it("대기가 끝났으면(pending=false) 붙어 있어도 멈춘다", () => {
    const ws = { id: 7, name: "…", tree: null, pending: true, pendingSince: 1_000_000 };
    const { run, t } = setup(ws);
    const host = run();
    host.isConnected = true;
    const [[id]] = [...t.intervals.entries()];
    ws.pending = false;
    t.fire(id);
    expect(t.cleared).toEqual([id]);
  });
  it("★단계 이벤트는 문구 노드만 고친다 — 새 엘리먼트를 만들지 않고(스피너 재생성 0) 같은 노드의 텍스트만 바뀐다", () => {
    const ws: Record<string, unknown> = { id: 7, name: "…", tree: null, pending: true, pendingSince: 1_000_000 };
    const { run, painters, doc } = setup(ws);
    const host = run();
    host.isConnected = true;
    const before = nodes(host);
    const created = doc.created;
    ws.pendingStage = "wait";
    const paint = painters.get(7);
    expect(typeof paint).toBe("function");
    (paint as () => void)();
    const after = nodes(host);
    expect(after.msg === before.msg).toBe(true);
    expect(after.stage === before.stage).toBe(true);
    expect(after.spin === before.spin).toBe(true);
    expect(doc.created).toBe(created); // 새 DOM 노드 0
    expect(after.stage.textContent).toBe("지금: 데몬이 팩을 설치하는 중(파일 수백 개)");
    ws.pendingStage = "up";
    (paint as () => void)();
    expect(after.stage.textContent).toBe("지금: 데몬이 켜졌습니다 — 설정을 심는 중");
  });
  it("모르는 단계 키·HTML 모양 값은 단계 줄을 비운다(라벨 표에 없는 값은 그리지 않는다)", () => {
    const ws: Record<string, unknown> = { id: 7, name: "…", tree: null, pending: true, pendingSince: 1_000_000 };
    const { run, painters } = setup(ws);
    const host = run();
    host.isConnected = true;
    const { stage } = nodes(host);
    for (const bad of ["zzz", "<img src=x onerror=alert(1)>", "", "Reserve"]) {
      ws.pendingStage = bad;
      (painters.get(7) as () => void)();
      expect({ 값: bad, 줄: stage.textContent }).toEqual({ 값: bad, 줄: "" });
    }
  });
  it("같은 탭을 다시 그리면(render() 재생성) 새 갱신기가 등록되고, 옛 호스트의 타이머 정리가 새 갱신기를 지우지 않는다", () => {
    const ws = { id: 7, name: "…", tree: null, pending: true, pendingSince: 1_000_000 };
    const { run, t, painters } = setup(ws);
    const h1 = run();
    const [[id1]] = [...t.intervals.entries()];
    const p1 = painters.get(7);
    const h2 = run();
    h2.isConnected = true;
    const p2 = painters.get(7);
    expect(p1 === p2).toBe(false); // 새 갱신기
    h1.isConnected = false; // 옛 호스트는 떨어졌다
    t.fire(id1);
    expect(t.cleared).toEqual([id1]);
    expect(painters.get(7) === p2).toBe(true); // 새 갱신기는 살아 있다
    expect(t.intervals.size).toBe(1);
  });
});

describe("onDeptCreateProgress — 실제 본문 실행(진행 id 가 맞는 대기 탭의 단계만)", () => {
  function setup() {
    const calls: number[] = [];
    const painters = new Map<number, () => void>();
    const wsA: Record<string, unknown> = { id: 5, name: "…", tree: null, pending: true };
    const wsB: Record<string, unknown> = { id: 6, name: "…", tree: null, pending: true };
    const wsDone: Record<string, unknown> = { id: 7, name: "dept-1", tree: null, socket: "S", pending: false };
    painters.set(5, () => calls.push(5));
    painters.set(6, () => calls.push(6));
    painters.set(7, () => calls.push(7));
    const fns = load(["onDeptCreateProgress"], {
      workspaces: [wsA, wsB, wsDone],
      deptPendingPainters: painters,
      parseDeptProgressPayload,
      deptProgressId,
    });
    return { fn: fns.onDeptCreateProgress, calls, wsA, wsB, wsDone, painters };
  }
  it("id 가 맞는 대기 탭의 pendingStage 만 고치고 그 탭의 문구 갱신기만 부른다(다른 대기 탭·대기 아닌 탭은 그대로)", () => {
    const { fn, calls, wsA, wsB, wsDone } = setup();
    fn({ id: "dp-5", stage: "spawn" });
    expect(wsA.pendingStage).toBe("spawn");
    expect(wsB.pendingStage).toBeUndefined();
    expect(wsDone.pendingStage).toBeUndefined();
    expect(calls).toEqual([5]);
    fn({ id: "dp-6", stage: "seat" });
    expect(wsB.pendingStage).toBe("seat");
    expect(calls).toEqual([5, 6]);
  });
  it("대기가 아닌 탭(id 가 맞아도)은 건드리지 않는다 · 일치하는 id 가 없으면 아무 일도 없다", () => {
    const { fn, calls, wsDone } = setup();
    fn({ id: "dp-7", stage: "done" });
    expect(wsDone.pendingStage).toBeUndefined();
    fn({ id: "dp-99", stage: "done" });
    expect(calls).toEqual([]);
  });
  it("신뢰할 수 없는 payload(모양 오류·잘못된 키)는 던지지 않고 무시한다", () => {
    const { fn, calls, wsA } = setup();
    for (const bad of [null, undefined, "x", 7, [], {}, { id: "dp-5" }, { stage: "up" }, { id: 5, stage: "up" }, { id: "dp-5", stage: "Up" }, { id: "dp-5", stage: "<b>" }, { id: "dp-5", stage: 3 }]) {
      fn(bad);
    }
    expect(wsA.pendingStage).toBeUndefined();
    expect(calls).toEqual([]);
  });
  it("그 탭이 지금 안 보이면(갱신기 없음) 값만 적고 던지지 않는다 — 다시 그릴 때 보인다", () => {
    const { fn, wsA, painters } = setup();
    painters.delete(5);
    fn({ id: "dp-5", stage: "probe" });
    expect(wsA.pendingStage).toBe("probe");
  });
});

describe("renderIdleWorkspace — 실제 본문 실행(방금 만든 팀의 문구 분기 1개 · 60초 창이 끝나면 한 번 고친다)", () => {
  const T0 = 5_000_000;
  const OLD_DEPT = "이 부서는 아직 켜지 않았습니다 — 아래 버튼을 누르거나, 앱을 다시 켜면 준비됩니다.";
  const STOP_FAILED = "종료 실패 — 탭을 다시 닫아 재시도 / 지금 켜기를 누르면 삭제가 취소됩니다";
  const PLAIN = "이 워크스페이스에 아직 열린 창이 없습니다 — 아래 버튼을 누르면 새 셸이 열립니다.";
  function setup(nowMs: number) {
    const doc = fakeDoc();
    const t = fakeTimers();
    const clock = { now: nowMs };
    const fns = load(["renderIdleWorkspace"], {
      document: doc,
      setTimeout: t.setTimeout,
      Date: { now: () => clock.now },
      deptFirstSeatPending,
      deptFirstSeatRemainingMs,
      DEPT_FIRST_SEAT_TEXT,
    });
    const run = (ws: Record<string, unknown>): FakeEl => fns.renderIdleWorkspace(ws) as FakeEl;
    return { t, clock, run };
  }
  const msgOf = (h: FakeEl): string => (h.find("dept-pending-msg") as FakeEl).textContent;

  it("★방금 만든 팀(createdAt 이 60초 안)의 빈 탭 → '첫 자리를 붙이는 중' · 버튼은 그대로 있다", () => {
    const { run } = setup(T0 + 10_000);
    const host = run({ id: 1, name: "dept-1", tree: null, socket: "S", createdAt: T0 });
    expect(msgOf(host)).toBe("첫 자리를 붙이는 중입니다 — 잠시만 기다려 주세요");
    const btn = host.find("dept-idle-btn") as FakeEl;
    expect(btn.textContent).toBe("지금 켜기");
  });
  it("60초가 지났거나 createdAt 이 없는 부서 탭(복원된 탭 등)은 종전 문구", () => {
    const old = setup(T0 + 60_000);
    expect(msgOf(old.run({ id: 1, name: "dept-1", tree: null, socket: "S", createdAt: T0 }))).toBe(OLD_DEPT);
    const none = setup(T0);
    expect(msgOf(none.run({ id: 2, name: "dept-2", tree: null, socket: "S2" }))).toBe(OLD_DEPT);
  });
  it("종료 실패 탭은 방금 만들었어도 종전 '종료 실패' 문구 그대로", () => {
    const { run, t } = setup(T0 + 5_000);
    const host = run({ id: 1, name: "dept-1", tree: null, socket: "S", createdAt: T0, stopFailed: true });
    expect(msgOf(host)).toBe(STOP_FAILED);
    expect(t.timeouts.length).toBe(0);
  });
  it("부서가 아닌 워크스페이스는 종전 문구 그대로", () => {
    const { run, t } = setup(T0);
    expect(msgOf(run({ id: 3, name: "ws", tree: null }))).toBe(PLAIN);
    expect(t.timeouts.length).toBe(0);
  });
  it("★창이 끝나는 순간 문구를 한 번 고친다 — 남은 시간 + 50ms 뒤 · 붙어 있을 때만 · 안 붙었으면 아무것도 안 한다", () => {
    const a = setup(T0 + 20_000);
    const ws = { id: 1, name: "dept-1", tree: null, socket: "S", createdAt: T0 };
    const host = a.run(ws);
    host.isConnected = true;
    expect(a.t.timeouts.length).toBe(1);
    expect(a.t.timeouts[0].ms).toBe(40_000 + 50);
    a.clock.now = T0 + 60_100;
    a.t.timeouts[0].fn();
    expect(msgOf(host)).toBe(OLD_DEPT);
    // 떨어진 호스트는 건드리지 않는다
    const b = setup(T0 + 20_000);
    const host2 = b.run(ws);
    host2.isConnected = false;
    b.clock.now = T0 + 60_100;
    b.t.timeouts[0].fn();
    expect(msgOf(host2)).toBe("첫 자리를 붙이는 중입니다 — 잠시만 기다려 주세요");
  });
  it("창 밖에서 그리면 타이머를 걸지 않는다", () => {
    const { run, t } = setup(T0 + 61_000);
    run({ id: 1, name: "dept-1", tree: null, socket: "S", createdAt: T0 });
    expect(t.timeouts.length).toBe(0);
  });
});

describe("팀원 부팅 안내 — 실제 본문 실행(showDeptFormation · checkDeptFormationNotices · noteToastClosedByUser)", () => {
  type Call = { id: string; category: string; name: string; detail: string; at: number };
  const T0 = 10_000_000;
  const pane = (sid: number) => ({ type: "pane", sid });
  /** n 자리 트리(분할로 묶음) — 음수 sid 는 구멍(세지 않는다). */
  const treeOf = (n: number, holes = 0): unknown => {
    let t: unknown = null;
    for (let i = 1; i <= n; i++) t = t === null ? pane(i) : { type: "split", a: t, b: pane(i) };
    for (let h = 1; h <= holes; h++) t = t === null ? pane(-h) : { type: "split", a: t, b: pane(-h) };
    return t;
  };
  /** 데몬 surface.list 한 줄의 **실제 꼴**(handlers.rs surface.list 의 키 전부) — 값은 지어낸 값. 판정의 입력은 이 꼴이다(손으로 만든 이벤트가 아니다 — S4 B1 의 교훈). */
  const row = (sid: number, role: string | null, over: Record<string, unknown> = {}): Record<string, unknown> => ({
    surface_id: sid, surface_ref: `surface:${sid}`, title: "zsh", role, cmd: "zsh", cwd: "/Users/runner/work", live_cwd: "/Users/runner/work", pid: 41000 + sid, exited: false,
    created_at: 1_800_000_000, pending_input_bytes: 0, pending_input_human_bytes: 0, input_paste_open: false, seat: "occupied", env_injected: true, created_by: null,
    claude_config_dir: null, agent: null, agent_alive: null, awakened_at: null, directive_verified: null, ack_nonce_ok: null, ack_source: null, boot_nonce_generation: null,
    alt_screen: false, line_count: 12, cwd_blocked: null, usage: null, ...over,
  });
  const FIVE = ["master", "cso", "worker", "reviewer-gemini", "reviewer-codex"];
  /** 붙은 순서대로 앞 n 개 역할의 좌석 목록(list_surfaces 의 surfaces). */
  const seatsOf = (n: number): Record<string, unknown>[] => FIVE.slice(0, n).map((r, i) => row(i + 1, r));
  /** 이번 3초 틱이 받은 목록 → refreshPaneTitles 가 만드는 소켓별 역할표(`seatRolesTick.set(sk ?? "", deptLiveRoles(r.surfaces))` 와 같은 한 줄). 목록을 못 받은 소켓은 항목이 없다. */
  const tickOf = (entries: [string, unknown][]): Map<string, string[] | null> => new Map(entries.map(([sock, surfaces]) => [sock, DP.deptLiveRoles(surfaces)] as [string, string[] | null]));
  const idOf = (sock: string): string => `dept-formation:${sock}`;
  function setup() {
    const clock = { now: T0 };
    const calls: Call[] = [];
    const dismissed: string[] = [];
    const stickyToasts = new Map<string, { el: unknown; timer: unknown }>();
    const workspaces: Record<string, unknown>[] = [];
    const deps = {
      workspaces,
      stickyToasts,
      stickyToast: (id: string, category: string, name: string, detail: string): void => {
        calls.push({ id, category, name, detail, at: clock.now });
        stickyToasts.set(id, { el: {}, timer: 0 });
      },
      dismissToast: (id: string): void => {
        dismissed.push(id);
        stickyToasts.delete(id);
      },
      Date: { now: () => clock.now },
      deptFormationText,
      deptFormationNoticeKey,
      deptFormationCapped,
      deptFormationToastId,
      deptFormationVerdict: DP.deptFormationVerdict,
      DEPT_FORMATION_TOAST_PREFIX,
    };
    const fns = load(["collectSids", "showDeptFormation", "checkDeptFormationNotices", "noteToastClosedByUser"], deps);
    const newTeam = (id: number, socket: string): Record<string, unknown> => {
      const ws: Record<string, unknown> = { id, name: `dept-${id}`, tree: null, socket, pending: false, createdAt: clock.now };
      workspaces.push(ws);
      return ws;
    };
    return { clock, calls, dismissed, stickyToasts, workspaces, fns, newTeam };
  }

  it("collectSids(실제 본문) — 구멍(음수 sid)은 자리 수에 안 센다", () => {
    const { fns } = setup();
    expect(fns.collectSids(treeOf(3, 2))).toEqual([1, 2, 3]);
    expect(fns.collectSids(null)).toEqual([]);
  });

  it("★성공 직후 첫 안내 — 같은 id(dept-formation:<소켓>)·'feed' 등급·「팀원을 켜는 중」·자리 0·경과 1분 미만 · 탭에 열쇠가 적힌다(사유 필드는 없다)", () => {
    const { fns, calls, newTeam } = setup();
    const ws = newTeam(1, "/s/1.sock");
    fns.showDeptFormation(ws, "booting");
    expect(calls.length).toBe(1);
    expect(calls[0].id).toBe("dept-formation:/s/1.sock");
    expect(calls[0].category).toBe("feed");
    expect(calls[0].name).toBe("팀원을 켜는 중");
    expect(calls[0].detail).toBe("부서장·CSO·워커·리뷰어가 차례로 켜집니다(보통 5분 안팎) · 지금 0자리 · 경과 1분 미만");
    expect(ws.formationView).toEqual({ state: "booting", key: deptFormationNoticeKey({ seats: 0, elapsedSec: 0, state: "booting" }) });
  });
  it("createdAt 이 없는 탭(복원된 탭 · 기존 팀을 돌려받은 탭)·소켓이 없는 탭은 안내를 내지 않는다", () => {
    const { fns, calls, workspaces } = setup();
    const a: Record<string, unknown> = { id: 1, name: "x", tree: null, socket: "/s/a" };
    const b: Record<string, unknown> = { id: 2, name: "y", tree: null, createdAt: T0 };
    workspaces.push(a, b);
    fns.showDeptFormation(a, "booting");
    fns.showDeptFormation(b, "booting");
    fns.showDeptFormation(a, "check", 2);
    expect(calls.length).toBe(0);
  });

  it("★값(열쇠)이 바뀔 때만 다시 낸다 — 같은 틱 반복·같은 분 안 · 자리 수가 바뀌면 1회 · 분이 바뀌면 1회 · 45초 칸이 바뀌면 1회(자리 목록을 못 받은 틱에도 갱신은 계속된다)", () => {
    const { fns, calls, clock, newTeam } = setup();
    const ws = newTeam(1, "/s/1.sock");
    fns.showDeptFormation(ws, "booting"); // t=0
    expect(calls.length).toBe(1);
    // 같은 값으로 여러 번 점검해도 호출 0(3초 틱을 12번)
    for (let t = 3; t <= 36; t += 3) {
      clock.now = T0 + t * 1000;
      fns.checkDeptFormationNotices();
    }
    expect(calls.length).toBe(1);
    // 자리 수가 바뀌었다 → 1회(그 틱에서만)
    ws.tree = treeOf(1);
    clock.now = T0 + 39_000;
    fns.checkDeptFormationNotices();
    expect(calls.length).toBe(2);
    expect(calls[1].detail).toBe("부서장·CSO·워커·리뷰어가 차례로 켜집니다(보통 5분 안팎) · 지금 1자리 · 경과 1분 미만");
    fns.checkDeptFormationNotices(); // 같은 틱 반복
    expect(calls.length).toBe(2);
    // 45초 칸이 바뀌었다(수명 갱신) → 1회 · 본문은 같다
    clock.now = T0 + 45_000;
    fns.checkDeptFormationNotices();
    expect(calls.length).toBe(3);
    expect(calls[2].detail).toBe(calls[1].detail);
    // 같은 칸 안(46~59초)에서는 호출 0
    for (let t = 48; t <= 57; t += 3) {
      clock.now = T0 + t * 1000;
      fns.checkDeptFormationNotices();
    }
    expect(calls.length).toBe(3);
    // 경과 분이 바뀌었다(60초) → 1회 · '1분'
    clock.now = T0 + 60_000;
    fns.checkDeptFormationNotices();
    expect(calls.length).toBe(4);
    expect(calls[3].detail).toBe("부서장·CSO·워커·리뷰어가 차례로 켜집니다(보통 5분 안팎) · 지금 1자리 · 경과 1분");
  });

  it("★5분(3초 틱 100번)을 돌려도 호출은 한 자릿수~십여 회 · 두 호출 사이는 47초 이하(토스트 기본 수명 60초가 갱신 사이에 끝나지 않는다) — 매 틱 좌석 목록을 받아도(다섯 전에는) 같다", () => {
    const { fns, calls, clock, newTeam } = setup();
    const ws = newTeam(1, "/s/1.sock");
    fns.showDeptFormation(ws, "booting");
    for (let t = 3; t <= 240; t += 3) {
      clock.now = T0 + t * 1000;
      const seats = t >= 190 ? 4 : t >= 130 ? 3 : t >= 70 ? 2 : t >= 10 ? 1 : 0;
      ws.tree = treeOf(seats);
      fns.checkDeptFormationNotices(tickOf([["/s/1.sock", seatsOf(seats)]]));
    }
    expect(ws.formationDone).toBeUndefined(); // 다섯이 안 됐다 — 아직 켜는 중
    expect(calls.length).toBeLessThan(25);
    expect(calls.length).toBeGreaterThan(6);
    let maxGap = 0;
    for (let i = 1; i < calls.length; i++) maxGap = Math.max(maxGap, (calls[i].at - calls[i - 1].at) / 1000);
    expect(maxGap).toBeLessThan(48); // 47초 이하(틱이 3초 격자라 정수)
    expect(calls[calls.length - 1].detail).toContain("지금 4자리");
    expect(new Set(calls.map((c) => c.id)).size).toBe(1); // 모든 호출은 같은 id 하나(탭마다 하나 · 갱신)
  });

  it("★탭이 닫히면(어떤 경로로든 workspaces 에서 사라지면) 그 안내를 dismissToast 한다 — 다른 탭의 안내·무관한 토스트는 그대로", () => {
    const { fns, calls, dismissed, stickyToasts, workspaces, newTeam } = setup();
    const a = newTeam(1, "/s/1.sock");
    const b = newTeam(2, "/s/2.sock");
    fns.showDeptFormation(a, "booting");
    fns.showDeptFormation(b, "booting");
    stickyToasts.set("restore", { el: {}, timer: 0 }); // 무관한 토스트
    stickyToasts.set("dept-formation:/s/ghost.sock", { el: {}, timer: 0 }); // 탭 없는 소켓의 안내 잔재
    workspaces.splice(workspaces.indexOf(a), 1); // 탭 ×
    fns.checkDeptFormationNotices();
    expect(dismissed.sort()).toEqual(["dept-formation:/s/1.sock", "dept-formation:/s/ghost.sock"]);
    expect(stickyToasts.has("dept-formation:/s/2.sock")).toBe(true);
    expect(stickyToasts.has("restore")).toBe(true);
    expect(calls.length).toBe(2); // 닫힌 탭 정리는 새 토스트를 내지 않는다
  });

  it("첫 안내가 빠졌어도(formationView 없음) 점검이 한 번 내 준다(안전망)", () => {
    const { fns, calls, newTeam } = setup();
    const ws = newTeam(1, "/s/1.sock");
    fns.checkDeptFormationNotices();
    expect(calls.length).toBe(1);
    expect(calls[0].name).toBe("팀원을 켜는 중");
    expect(ws.formationView === undefined).toBe(false);
  });
  it("pending 탭·createdAt 없는 탭·formationDone 탭은 점검이 건드리지 않는다 — 자리 목록이 다섯이어도", () => {
    const { fns, calls, workspaces } = setup();
    workspaces.push(
      { id: 1, name: "…", tree: null, pending: true, socket: "/s/p", createdAt: T0 },
      { id: 2, name: "r", tree: null, socket: "/s/r" },
      { id: 3, name: "d", tree: null, socket: "/s/d", createdAt: T0, formationDone: true },
    );
    fns.checkDeptFormationNotices(tickOf([["/s/p", seatsOf(5)], ["/s/r", seatsOf(5)], ["/s/d", seatsOf(5)]]));
    expect(calls.length).toBe(0);
  });

  // ════ ★S4 B1 — 완료 판정은 편성 결과 feed 가 아니라 그 팀 소켓의 좌석 목록이다 ════
  describe("★S4 B1 — 완료 판정 = 그 팀 소켓의 좌석 목록(3초 틱이 이미 받는 list_surfaces)의 의무 역할 다섯", () => {
    const SOCK = "/s/1.sock";
    it("★다섯 역할이 모두 붙으면 안내가 「팀 자리가 모두 붙었습니다」로 **한 번** 바뀌고 더 갱신하지 않는다(formationDone) — 토스트는 수명대로 사라진다", () => {
      const { fns, calls, clock, newTeam } = setup();
      const ws = newTeam(1, SOCK);
      fns.showDeptFormation(ws, "booting");
      // 자리가 하나씩 붙는다(master → cso → worker → reviewer-gemini) — 다섯 전에는 완료가 아니다
      for (const [t, n] of [[10, 1], [40, 2], [100, 3], [160, 4]] as const) {
        clock.now = T0 + t * 1000;
        ws.tree = treeOf(n);
        fns.checkDeptFormationNotices(tickOf([[SOCK, seatsOf(n)]]));
        expect({ t, n, done: ws.formationDone }).toEqual({ t, n, done: undefined });
        expect(calls[calls.length - 1].name).toBe("팀원을 켜는 중");
      }
      const before = calls.length;
      clock.now = T0 + 252_000;
      ws.tree = treeOf(5);
      fns.checkDeptFormationNotices(tickOf([[SOCK, seatsOf(5)]]));
      expect(calls.length).toBe(before + 1);
      const last = calls[calls.length - 1];
      expect({ id: last.id, category: last.category, name: last.name, detail: last.detail }).toEqual({
        id: "dept-formation:/s/1.sock",
        category: "feed",
        name: "팀 자리가 모두 붙었습니다",
        detail: "자리 5개가 모두 붙었습니다 · 걸린 시간 4분",
      });
      expect(ws.formationDone).toBe(true);
      expect((ws.formationView as { state: string }).state).toBe("seated");
      // 더 갱신하지 않는다 — 시간이 흐르고 자리 수가 바뀌어도 · 15분을 넘겨도
      for (const t of [255, 300, 400, 899, 900, 1000, 5000]) {
        clock.now = T0 + t * 1000;
        ws.tree = treeOf(6);
        fns.checkDeptFormationNotices(tickOf([[SOCK, seatsOf(5)]]));
      }
      expect(calls.length).toBe(before + 1);
    });
    it("★실제 꼴: 두 팀이 동시에 켜지는 중이면 각자 **자기 소켓의** 좌석 목록으로만 판정된다 — 본부(소켓 없음) 목록이 다섯이어도 부서 탭을 완료시키지 않는다", () => {
      const { fns, calls, clock, newTeam } = setup();
      const a = newTeam(1, "/s/1.sock");
      const b = newTeam(2, "/s/2.sock");
      fns.showDeptFormation(a, "booting");
      fns.showDeptFormation(b, "booting");
      clock.now = T0 + 90_000;
      const tick = tickOf([
        ["", seatsOf(5)], // 본부 데몬의 좌석 목록(다섯이 붙어 있다) — 부서 탭의 판정과 무관하다
        ["/s/1.sock", seatsOf(3)],
        ["/s/2.sock", seatsOf(5)],
      ]);
      fns.checkDeptFormationNotices(tick);
      expect(a.formationDone).toBeUndefined();
      expect(b.formationDone).toBe(true);
      expect(calls[calls.length - 1].id).toBe("dept-formation:/s/2.sock");
      expect(calls[calls.length - 1].name).toBe("팀 자리가 모두 붙었습니다");
      expect(calls.filter((c) => c.id === "dept-formation:/s/1.sock").every((c) => c.name === "팀원을 켜는 중")).toBe(true);
    });
    it("★종료한 좌석·변형 역할(worker-2 · cso-fresh-…)은 세지 않는다 — 실제 꼴 목록에서 master 가 종료했으면 다섯이 아니다", () => {
      const { fns, calls, clock, newTeam } = setup();
      const ws = newTeam(1, SOCK);
      fns.showDeptFormation(ws, "booting");
      clock.now = T0 + 120_000;
      const list = seatsOf(5);
      list[0] = row(1, "master", { exited: true });
      list.push(row(6, "worker-2"), row(7, "cso-fresh-1800000123"), row(8, null));
      fns.checkDeptFormationNotices(tickOf([[SOCK, list]]));
      expect(ws.formationDone).toBeUndefined();
      expect(calls.every((c) => c.name === "팀원을 켜는 중")).toBe(true);
      list[0] = row(1, "master"); // 다시 붙었다
      fns.checkDeptFormationNotices(tickOf([[SOCK, list]]));
      expect(ws.formationDone).toBe(true);
      expect(calls[calls.length - 1].name).toBe("팀 자리가 모두 붙었습니다");
    });
    it("★목록을 못 받은 틱(그 소켓의 항목이 없음 · 응답이 배열이 아님 · 인자 없음)은 판정을 건너뛴다 — 다섯이 붙어 있어도 완료로, 상한이 지나도 '확인 필요'로 치지 않는다 · 다음에 받은 틱에 판정한다", () => {
      const { fns, calls, clock, newTeam } = setup();
      const ws = newTeam(1, SOCK);
      fns.showDeptFormation(ws, "booting");
      clock.now = T0 + 100_000;
      for (const tick of [undefined, new Map<string, string[] | null>(), tickOf([["/s/other.sock", seatsOf(5)]]), tickOf([[SOCK, { surfaces: seatsOf(5) }]])]) {
        fns.checkDeptFormationNotices(tick);
        expect(ws.formationDone).toBeUndefined();
        expect(calls.every((c) => c.name === "팀원을 켜는 중")).toBe(true);
      }
      // 상한(15분)을 넘겨도 목록이 없으면 말하지 않는다 — 접지도 않는다(다음 틱에 다시 판정)
      clock.now = T0 + 1_000_000;
      const n = calls.length;
      fns.checkDeptFormationNotices(new Map());
      fns.checkDeptFormationNotices(undefined);
      expect(ws.formationDone).toBeUndefined();
      expect(calls.length).toBe(n);
      // 목록을 받은 틱 — 그제서야 판정한다(여기서는 다섯 → 완료)
      fns.checkDeptFormationNotices(tickOf([[SOCK, seatsOf(5)]]));
      expect(ws.formationDone).toBe(true);
      expect(calls[calls.length - 1].name).toBe("팀 자리가 모두 붙었습니다");
    });
    it("★15분(900초)에 닿았는데 다섯 역할이 다 붙지 않았으면 「팀원 켜기 — 확인 필요」를 **한 번** 낸다 — 본문 '아직 N자리입니다 …'(N = 붙은 의무 역할 수) · watchdog 등급 · 그 뒤 갱신 없음 · 직전(899초)에는 아직", () => {
      const { fns, calls, clock, newTeam } = setup();
      const ws = newTeam(1, SOCK);
      fns.showDeptFormation(ws, "booting");
      ws.tree = treeOf(3);
      clock.now = T0 + 899_000;
      fns.checkDeptFormationNotices(tickOf([[SOCK, seatsOf(3)]]));
      expect(ws.formationDone).toBeUndefined();
      expect(calls.every((c) => c.name === "팀원을 켜는 중")).toBe(true);
      const before = calls.length;
      clock.now = T0 + 900_000;
      fns.checkDeptFormationNotices(tickOf([[SOCK, seatsOf(3)]]));
      expect(calls.length).toBe(before + 1);
      const last = calls[calls.length - 1];
      expect({ id: last.id, category: last.category, name: last.name, detail: last.detail }).toEqual({
        id: "dept-formation:/s/1.sock",
        category: "watchdog",
        name: "팀원 켜기 — 확인 필요",
        detail: "아직 3자리입니다 — Control Center 에서 자리 상태를 확인하세요 · 경과 15분",
      });
      expect(ws.formationDone).toBe(true);
      expect((ws.formationView as { state: string }).state).toBe("check");
      // 그 뒤로는 값이 바뀌어도 호출 0 — 늦게 다섯이 붙어도 이 탭은 이미 접혔다(무한 갱신 금지)
      for (const t of [903, 960, 1200, 3000]) {
        clock.now = T0 + t * 1000;
        ws.tree = treeOf(5);
        fns.checkDeptFormationNotices(tickOf([[SOCK, seatsOf(5)]]));
      }
      expect(calls.length).toBe(before + 1);
    });
    it("상한 틱에 다섯이 모두 붙어 있으면 '확인 필요'가 아니라 '모두 붙었습니다' — 자리 판정이 상한보다 먼저다 · 한 자리도 안 붙었으면 '아직 0자리입니다'", () => {
      const a = setup();
      const wa = a.newTeam(1, SOCK);
      a.fns.showDeptFormation(wa, "booting");
      a.clock.now = T0 + 900_000;
      a.fns.checkDeptFormationNotices(tickOf([[SOCK, seatsOf(5)]]));
      expect(a.calls[a.calls.length - 1].name).toBe("팀 자리가 모두 붙었습니다");
      expect(a.calls[a.calls.length - 1].detail).toBe("자리 5개가 모두 붙었습니다 · 걸린 시간 15분");
      const b = setup();
      const wb = b.newTeam(1, SOCK);
      b.fns.showDeptFormation(wb, "booting");
      b.clock.now = T0 + 905_000;
      b.fns.checkDeptFormationNotices(tickOf([[SOCK, []]]));
      expect(b.calls[b.calls.length - 1].detail).toBe("아직 0자리입니다 — Control Center 에서 자리 상태를 확인하세요 · 경과 15분");
    });
    it("확인 필요·완료 안내는 × 로 닫은 뒤에도 한 번 낸다 — 주기 갱신만 멈추고 결과는 놓치지 않는다(종전: 편성 결과 이벤트가 하던 일)", () => {
      const { fns, calls, clock, stickyToasts, newTeam } = setup();
      const a = newTeam(1, "/s/1.sock");
      const b = newTeam(2, "/s/2.sock");
      fns.showDeptFormation(a, "booting");
      fns.showDeptFormation(b, "booting");
      for (const sock of ["/s/1.sock", "/s/2.sock"]) {
        fns.noteToastClosedByUser(idOf(sock)); // 사용자가 × 를 눌렀다(× 처리기: 표식 → dismissToast)
        stickyToasts.delete(idOf(sock));
      }
      clock.now = T0 + 120_000;
      fns.checkDeptFormationNotices(tickOf([["/s/1.sock", seatsOf(2)], ["/s/2.sock", seatsOf(5)]]));
      expect(calls.length).toBe(3); // 닫힌 팀 1 은 아직 말하지 않는다(주기 갱신 멈춤) · 팀 2 의 완료는 나온다
      expect(calls[2].id).toBe("dept-formation:/s/2.sock");
      expect(calls[2].name).toBe("팀 자리가 모두 붙었습니다");
      clock.now = T0 + 900_000;
      fns.checkDeptFormationNotices(tickOf([["/s/1.sock", seatsOf(2)]]));
      expect(calls.length).toBe(4);
      expect(calls[3].id).toBe("dept-formation:/s/1.sock");
      expect(calls[3].name).toBe("팀원 켜기 — 확인 필요");
      expect(stickyToasts.has("dept-formation:/s/1.sock")).toBe(true);
    });
  });

  // ════ ★S4 m3 — 수명 만료를 '사용자가 닫음'으로 읽어 영구히 침묵했다 ════
  describe("★S4 m3 — 닫음 표식은 × 버튼 경로에서만 선다 · 수명 만료로 사라진 안내는 다음 점검이 다시 낸다", () => {
    const SOCK = "/s/1.sock";
    it("★토스트가 수명(60초)으로 사라졌고(점검이 45초 칸보다 밀렸다) 닫음 표식이 없으면 다음 점검이 안내를 다시 낸다 — 영구 침묵하지 않는다", () => {
      const { fns, calls, clock, stickyToasts, newTeam } = setup();
      const ws = newTeam(1, SOCK);
      fns.showDeptFormation(ws, "booting");
      // 응답 없는 데몬 때문에 틱이 밀렸다 — 수명 타이머(stickyToast 안)가 dismissToast 로 토스트를 걷었다. × 경로(noteToastClosedByUser)는 거치지 않았다.
      stickyToasts.delete(idOf(SOCK));
      clock.now = T0 + 70_000;
      fns.checkDeptFormationNotices();
      expect(calls.length).toBe(2);
      expect(calls[1].name).toBe("팀원을 켜는 중");
      expect(stickyToasts.has(idOf(SOCK))).toBe(true);
      expect((ws.formationView as { muted?: boolean }).muted).toBeUndefined();
      // 다시 선 뒤에는 정상 갱신(같은 열쇠면 호출 0)
      fns.checkDeptFormationNotices();
      expect(calls.length).toBe(2);
      // 몇 번을 사라져도 그때마다 다시 낸다(영구 침묵 없음)
      for (const t of [200, 330, 460]) {
        stickyToasts.delete(idOf(SOCK));
        clock.now = T0 + t * 1000;
        fns.checkDeptFormationNotices();
      }
      expect(calls.length).toBe(5);
    });
    it("★× 로 닫았으면(표식 → dismissToast) 되살리지 않는다 — 값이 바뀌어도 호출 0 · 표식은 그 팀에만 선다(수명 만료로 사라진 다른 팀은 다시 난다)", () => {
      const { fns, calls, clock, stickyToasts, newTeam } = setup();
      const a = newTeam(1, "/s/1.sock");
      const b = newTeam(2, "/s/2.sock");
      fns.showDeptFormation(a, "booting");
      fns.showDeptFormation(b, "booting");
      fns.noteToastClosedByUser(idOf("/s/1.sock")); // 팀 1 의 안내를 사용자가 × 로 닫았다
      stickyToasts.delete(idOf("/s/1.sock"));
      stickyToasts.delete(idOf("/s/2.sock")); // 팀 2 의 안내는 수명으로 사라졌다
      clock.now = T0 + 20_000;
      a.tree = treeOf(2);
      fns.checkDeptFormationNotices();
      expect((a.formationView as { muted?: boolean }).muted).toBe(true);
      expect((b.formationView as { muted?: boolean }).muted).toBeUndefined();
      expect(calls.filter((c) => c.id === idOf("/s/1.sock")).length).toBe(1); // 닫은 뒤 새 호출 없음
      expect(calls.filter((c) => c.id === idOf("/s/2.sock")).length).toBe(2); // 만료된 팀 2 는 다시 났다
      clock.now = T0 + 70_000;
      a.tree = treeOf(3);
      fns.checkDeptFormationNotices();
      expect(calls.filter((c) => c.id === idOf("/s/1.sock")).length).toBe(1);
    });
    it("noteToastClosedByUser — dept-formation: 접두 id 만, 그 소켓의 탭(안내가 있는)에만 표식 · 다른 id·접두만 같은 문자열·없는 탭·안내 없는 탭·이상한 입력은 무동작(던지지 않는다)", () => {
      const { fns, newTeam, workspaces } = setup();
      const a = newTeam(1, "/s/1.sock");
      const b = newTeam(2, "/s/2.sock");
      const c = newTeam(3, "/s/3.sock");
      fns.showDeptFormation(a, "booting");
      fns.showDeptFormation(b, "booting");
      // c 는 안내를 낸 적 없다(formationView 없음)
      for (const id of ["restore", "dept-formation", "dept-formation/s/1.sock", "xdept-formation:/s/1.sock", "", "dept-formation:/s/none.sock", undefined, null, 7, {}] as unknown[]) {
        expect(() => fns.noteToastClosedByUser(id)).not.toThrow();
      }
      expect((a.formationView as { muted?: boolean }).muted).toBeUndefined();
      expect((b.formationView as { muted?: boolean }).muted).toBeUndefined();
      fns.noteToastClosedByUser(idOf("/s/3.sock")); // 안내가 없는 탭 — 무동작(던지지도 view 를 만들지도 않는다)
      expect(c.formationView).toBeUndefined();
      fns.noteToastClosedByUser(idOf("/s/2.sock"));
      expect((b.formationView as { muted?: boolean }).muted).toBe(true);
      expect((a.formationView as { muted?: boolean }).muted).toBeUndefined();
      expect(workspaces.length).toBe(3);
    });
    it("점검은 더 이상 표식을 쓰지 않는다 — 토스트가 없다는 이유만으로 muted 가 서지 않는다(쓰기만 하고 읽지 않던 필드를 '× 로 닫음'의 뜻으로 살렸다)", () => {
      const { fns, clock, stickyToasts, newTeam } = setup();
      const ws = newTeam(1, SOCK);
      fns.showDeptFormation(ws, "booting");
      stickyToasts.delete(idOf(SOCK));
      clock.now = T0 + 10_000;
      fns.checkDeptFormationNotices();
      expect((ws.formationView as { muted?: boolean } | undefined)?.muted).toBeUndefined();
    });
  });

  it("★m3 배선 핀: 닫음 표식은 × 버튼 경로에서만 선다 — addToastCloseButton 의 click 처리기가 noteToastClosedByUser(id) 를 dismissToast 앞에서 부르고 다른 어디서도 부르지 않는다 · muted 쓰기는 그 함수 하나 · 점검은 view.muted 를 읽는다", () => {
    const x = fnText("addToastCloseButton", true);
    expect(x).toContain("noteToastClosedByUser(id);");
    expect(x.indexOf("noteToastClosedByUser(id);")).toBeLessThan(x.indexOf("dismissToast(id);"));
    expect(CODE.split("noteToastClosedByUser(").length - 1).toBe(2); // 정의 1 + × 처리기 1
    expect(CODE.split("muted = true").length - 1).toBe(1);
    expect(fnText("noteToastClosedByUser", true)).toContain("muted = true");
    expect(fnText("checkDeptFormationNotices", true)).toContain("view.muted");
    // 수명 타이머(stickyToast 안)·dismissToast 자체는 × 경로가 아니다
    expect(fnText("stickyToast", true).includes("noteToastClosedByUser")).toBe(false);
    expect(fnText("dismissToast", true).includes("noteToastClosedByUser")).toBe(false);
    // 점검 함수가 표식을 쓰지 않는다
    expect(fnText("checkDeptFormationNotices", true).includes("muted =")).toBe(false);
  });
});

describe("★S4 B1 — 실제 3초 틱(refreshPaneTitles 실제 본문)으로: 데몬이 돌려주는 좌석 목록(surface.list 의 실제 꼴)만으로 팀원 안내가 완료된다", () => {
  const T0 = 30_000_000;
  const SOCK = "/s/1.sock";
  const ID = `dept-formation:${SOCK}`;
  const FIVE = ["master", "cso", "worker", "reviewer-gemini", "reviewer-codex"];
  /** 데몬 surface.list 한 줄의 실제 꼴(handlers.rs surface.list 의 키 전부) — 값은 지어낸 값. */
  const row = (sid: number, role: string | null, over: Record<string, unknown> = {}): Record<string, unknown> => ({
    surface_id: sid, surface_ref: `surface:${sid}`, title: "zsh", role, cmd: "zsh", cwd: "/Users/runner/work", live_cwd: "/Users/runner/work", pid: 41000 + sid, exited: false,
    created_at: 1_800_000_000, pending_input_bytes: 0, pending_input_human_bytes: 0, input_paste_open: false, seat: "occupied", env_injected: true, created_by: null,
    claude_config_dir: null, agent: null, agent_alive: null, awakened_at: null, directive_verified: null, ack_nonce_ok: null, ack_source: null, boot_nonce_generation: null,
    alt_screen: false, line_count: 12, cwd_blocked: null, usage: null, ...over,
  });
  const seatsOf = (n: number): Record<string, unknown>[] => FIVE.slice(0, n).map((r, i) => row(i + 1, r));
  const tree = (n: number): unknown => {
    let t: unknown = null;
    for (let i = 1; i <= n; i++) t = t === null ? { type: "pane", sid: i } : { type: "split", a: t, b: { type: "pane", sid: i } };
    return t;
  };
  function setup() {
    const clock = { now: T0 };
    const toasts: { id: string; category: string; name: string; detail: string }[] = [];
    const stickyToasts = new Map<string, unknown>();
    /** 소켓 → 이번 틱에 데몬이 돌려줄 좌석 목록. "reject" = 응답 없음(시간 초과). 키 = String(socket) — 본부 탭은 소켓이 없다("undefined"). */
    const lists: Record<string, Record<string, unknown>[] | "reject"> = { undefined: seatsOf(1) };
    const invokes: string[] = [];
    const wsBase: Record<string, unknown> = { id: 0, name: "본부", tree: tree(1) };
    const wsDept: Record<string, unknown> = { id: 1, name: "dept-1", socket: SOCK, pending: false, createdAt: T0, tree: tree(3) };
    const workspaces = [wsBase, wsDept];
    const rt = { usageEl: {}, roleEl: {}, titleEl: { isContentEditable: false, textContent: "" } };
    const nothing = (): void => undefined;
    const deps: Record<string, unknown> = {
      // ── refreshPaneTitles 가 읽는 모듈 상태·도우미(실제 순수 함수는 실제 것을 쓴다)
      started: true, refreshing: false, workspaces, wsCounter: 5, activeWs: 0, UNTITLED: "…", focusedSid: null,
      claimFlight: () => true, releaseFlightWhenSettled: nothing,
      invoke: (cmd: string, args: { socket?: string }): Promise<unknown> => {
        invokes.push(`${cmd}:${String(args.socket)}`);
        const v = lists[String(args.socket)];
        return v === undefined || v === "reject" ? Promise.reject(new Error("rpc timeout")) : Promise.resolve({ surfaces: v });
      },
      rpcT: (p: Promise<unknown>): Promise<unknown> => p,
      T_LIST: 8000,
      collectCwdBlocked: () => [],
      annotateRoles, advanceGhostStrikes, seatPriority, tidyHoles,
      ghostStrike: new Map<string, number>(),
      paneKey: (sid: number, sk?: string): string => `${sk ?? ""}#${sid}`,
      holdRolePane: () => false, detachPane: nothing, forgetDaemonEpoch: nothing, reanchorAuto: nothing, destroyPaneRuntime: nothing,
      // 모든 좌석이 이미 pane 으로 입양돼 있는 상태(입양 분기는 이 검체의 대상이 아니다)
      panes: { get: () => rt, has: () => true },
      renderUsage: nothing, setRoleDot: nothing, paneTitle: (t: string): string => String(t),
      makePane: async () => ({ roleEl: {} }), adoptSeat: () => null,
      holeUntil: new Map<string, number>(), pruneHoleUntil: nothing,
      Date: { now: () => clock.now },
      render: nothing, refreshSidebarStatus: () => Promise.resolve(), setFocus: nothing, saveLayout: nothing, updateFtRoot: nothing,
      cwdBlockedNotices: () => ({ seen: new Set<string>(), notices: [] }), cwdBlockedSeen: new Set<string>(), openPrivacySettings: nothing,
      // ── 팀원 안내(실제 함수 — 같은 범위에서 함께 연다)
      stickyToasts,
      stickyToast: (id: string, category: string, name: string, detail: string): void => {
        toasts.push({ id, category, name, detail });
        stickyToasts.set(id, {});
      },
      dismissToast: (id: string): void => void stickyToasts.delete(id),
      deptFormationText, deptFormationNoticeKey, deptFormationCapped, deptFormationToastId, DEPT_FORMATION_TOAST_PREFIX,
      deptFormationVerdict: DP.deptFormationVerdict,
      deptLiveRoles: DP.deptLiveRoles,
    };
    const { fns, scope } = loadWith(["collectSids", "showDeptFormation", "checkDeptFormationNotices", "noteToastClosedByUser", "refreshPaneTitles"], deps);
    // 첫 안내(addDeptWorkspace 성공 직후 한 번)
    fns.showDeptFormation(wsDept, "booting");
    return { clock, toasts, stickyToasts, lists, invokes, wsDept, wsBase, fns, scope };
  }
  const names = (t: { name: string }[]): string[] => t.map((x) => x.name);

  it("★다섯 역할이 붙은 틱 — 실제 refreshPaneTitles 한 번이 새 팀 탭의 안내를 「팀 자리가 모두 붙었습니다」로 바꾼다(본부 feed·slug 는 어디에도 쓰이지 않는다)", async () => {
    const { clock, toasts, lists, wsDept, fns } = setup();
    // 자리가 하나씩 붙는 3초 틱들 — 다섯 전에는 '켜는 중'
    for (const [t, n] of [[3, 1], [30, 2], [90, 3], [150, 4]] as const) {
      clock.now = T0 + t * 1000;
      lists[SOCK] = seatsOf(n);
      await fns.refreshPaneTitles();
      expect({ t, n, 완료: wsDept.formationDone }).toEqual({ t, n, 완료: undefined });
    }
    expect(names(toasts).every((x) => x === "팀원을 켜는 중")).toBe(true);
    clock.now = T0 + 252_000;
    lists[SOCK] = seatsOf(5);
    await fns.refreshPaneTitles();
    const last = toasts[toasts.length - 1];
    expect(last).toEqual({ id: ID, category: "feed", name: "팀 자리가 모두 붙었습니다", detail: "자리 5개가 모두 붙었습니다 · 걸린 시간 4분" });
    expect(wsDept.formationDone).toBe(true);
    // 그 뒤 틱들은 더 갱신하지 않는다
    const n = toasts.length;
    for (const t of [255, 300, 900, 1200]) {
      clock.now = T0 + t * 1000;
      await fns.refreshPaneTitles();
    }
    expect(toasts.length).toBe(n);
  });
  it("★본부 데몬의 좌석 목록이 다섯이어도(편성 feed 가 본부로 가는 그 데몬) 부서 탭은 자기 소켓의 목록(세 자리)으로만 판정된다 — 완료로 바뀌지 않는다", async () => {
    const { clock, toasts, lists, wsDept, fns } = setup();
    lists.undefined = seatsOf(5);
    lists[SOCK] = seatsOf(3);
    clock.now = T0 + 120_000;
    await fns.refreshPaneTitles();
    expect(wsDept.formationDone).toBeUndefined();
    expect(names(toasts).every((x) => x === "팀원을 켜는 중")).toBe(true);
  });
  it("★그 소켓의 좌석 목록을 못 받은 틱(응답 없음)은 판정을 건너뛴다 — 상한을 한참 넘겨도 '확인 필요'로 치지 않고, 다음에 받은 틱에 판정한다 · 다른 소켓(본부)의 목록은 정상으로 처리된다", async () => {
    const { clock, toasts, lists, wsDept, fns } = setup();
    lists[SOCK] = "reject";
    lists.undefined = seatsOf(5);
    clock.now = T0 + 1_000_000; // 15분 상한을 한참 넘었다
    await fns.refreshPaneTitles();
    expect(wsDept.formationDone).toBeUndefined();
    expect(names(toasts)).toEqual(["팀원을 켜는 중"]); // 첫 안내뿐 — 상한 틱의 '확인 필요'도 완료도 없다
    lists[SOCK] = seatsOf(4); // 목록을 받았다 — 이제 판정한다(상한을 넘었고 네 자리뿐)
    await fns.refreshPaneTitles();
    expect(toasts[toasts.length - 1]).toEqual({ id: ID, category: "watchdog", name: "팀원 켜기 — 확인 필요", detail: "아직 4자리입니다 — Control Center 에서 자리 상태를 확인하세요 · 경과 16분" });
    expect(wsDept.formationDone).toBe(true);
  });
  it("★15분 상한에 다섯이 안 붙었으면 확인 필요 한 번 · 종료한 좌석은 세지 않는다(master 가 종료한 다섯 줄 = 네 자리) · 그 뒤 틱은 말이 없다", async () => {
    const { clock, toasts, lists, fns } = setup();
    const list = seatsOf(5);
    list[0] = row(1, "master", { exited: true });
    lists[SOCK] = list;
    clock.now = T0 + 900_000;
    await fns.refreshPaneTitles();
    expect(toasts[toasts.length - 1].name).toBe("팀원 켜기 — 확인 필요");
    expect(toasts[toasts.length - 1].detail).toBe("아직 4자리입니다 — Control Center 에서 자리 상태를 확인하세요 · 경과 15분");
    const n = toasts.length;
    for (const t of [903, 1000, 4000]) {
      clock.now = T0 + t * 1000;
      lists[SOCK] = seatsOf(5);
      await fns.refreshPaneTitles();
    }
    expect(toasts.length).toBe(n);
  });
  it("새 RPC 없음 — 한 틱의 호출은 소켓마다 list_surfaces 한 번뿐(본부 + 새 팀 = 2회)이고, 이전 틱이 진행 중(refreshing)이면 점검도 건너뛴다(종전 계약)", async () => {
    const { clock, lists, invokes, fns, scope, toasts } = setup();
    lists[SOCK] = seatsOf(5);
    clock.now = T0 + 100_000;
    scope.refreshing = true;
    await fns.refreshPaneTitles();
    expect(invokes).toEqual([]);
    expect(names(toasts)).toEqual(["팀원을 켜는 중"]); // 점검도 안 돌았다 — 완료 안내 없음
    scope.refreshing = false;
    await fns.refreshPaneTitles();
    expect(invokes).toEqual(["list_surfaces:undefined", `list_surfaces:${SOCK}`]);
    expect(toasts[toasts.length - 1].name).toBe("팀 자리가 모두 붙었습니다");
  });
});

describe("★S4 m3 — 실제 토스트 기계(stickyToast · addToastCloseButton · dismissToast)로: 수명 만료는 닫음이 아니고 × 버튼만 닫음이다", () => {
  /** 이 시험이 필요로 하는 만큼의 가짜 토스트 엘리먼트 — 클릭 처리기를 모아 두었다가 click() 으로 누른다. */
  class ToastEl {
    className = "";
    type = "";
    title = "";
    textContent = "";
    style: Record<string, string> = {};
    onclick: unknown = null;
    removed = false;
    children: ToastEl[] = [];
    listeners: Record<string, ((e: unknown) => void)[]> = {};
    private spans = new Map<string, ToastEl>();
    set innerHTML(html: string) {
      for (const m of html.matchAll(/class="([^"]+)"/g)) this.spans.set(m[1], new ToastEl());
    }
    querySelector(sel: string): ToastEl | undefined {
      return this.spans.get(sel.replace(/^\./, ""));
    }
    appendChild(k: ToastEl): ToastEl {
      this.children.push(k);
      return k;
    }
    addEventListener(t: string, fn: (e: unknown) => void): void {
      (this.listeners[t] ??= []).push(fn);
    }
    remove(): void {
      this.removed = true;
    }
    click(): void {
      for (const f of this.listeners.click ?? []) f({ stopPropagation: () => undefined });
    }
  }
  const T0 = 20_000_000;
  const SOCK = "/s/1.sock";
  const ID = `dept-formation:${SOCK}`;
  function setup() {
    const clock = { now: T0 };
    const box = new ToastEl();
    const stickyToasts = new Map<string, { el: ToastEl; timer: unknown }>();
    const timers: { fn: () => void; ms: number; cleared: boolean }[] = [];
    const workspaces: Record<string, unknown>[] = [];
    const banners: string[] = [];
    const deps = {
      workspaces,
      stickyToasts,
      document: { getElementById: (): ToastEl => box, createElement: (): ToastEl => new ToastEl() },
      setTimeout: (fn: () => void, ms: number): number => (timers.push({ fn, ms, cleared: false }), timers.length - 1),
      clearTimeout: (h: number): void => {
        if (timers[h]) timers[h].cleared = true;
      },
      recordAlarm: (): void => undefined,
      toastClassName: (category: string): string => `toast ${category}`,
      toastTimerPlan: (kind: "volatile" | "sticky", id?: string, had = false) => ({ ttlMs: toastTtl(kind, id).ttlMs, clearPrevious: kind === "sticky" && had }),
      needsExpiryBanner,
      expiryBannerText,
      osBanner: (t: string): void => void banners.push(t),
      Date: { now: () => clock.now },
      deptFormationText,
      deptFormationNoticeKey,
      deptFormationCapped,
      deptFormationToastId,
      deptFormationVerdict: DP.deptFormationVerdict,
      DEPT_FORMATION_TOAST_PREFIX,
    };
    const fns = load(
      ["collectSids", "showDeptFormation", "checkDeptFormationNotices", "noteToastClosedByUser", "stickyToast", "dismissToast", "addToastCloseButton"],
      deps,
    );
    const ws: Record<string, unknown> = { id: 1, name: "dept-1", tree: null, socket: SOCK, pending: false, createdAt: T0 };
    workspaces.push(ws);
    /** 지금 걸려 있는(걷히지 않은) 수명 타이머 — 가장 최근 것. */
    const liveTimer = () => [...timers].reverse().find((t) => !t.cleared);
    const closeButton = (): ToastEl => (stickyToasts.get(ID) as { el: ToastEl }).el.children.find((c) => c.className === "toast-x") as ToastEl;
    return { clock, fns, ws, stickyToasts, timers, liveTimer, closeButton, banners };
  }

  it("★수명(60초) 타이머가 토스트를 걷으면 닫음 표식이 서지 않고 다음 점검이 다시 낸다 — 영구 침묵하지 않는다(S4 m3 의 재현)", () => {
    const { fns, ws, stickyToasts, liveTimer, clock } = setup();
    fns.showDeptFormation(ws, "booting");
    expect(stickyToasts.has(ID)).toBe(true);
    const t = liveTimer();
    expect(t && t.ms).toBe(STICKY_TTL_MS); // 60초 수명 — 갱신 사이가 이보다 길면 안내가 사라진다
    (t as { fn: () => void }).fn(); // 점검이 45초 칸을 놓쳐 수명이 먼저 끝났다 — 타이머 안에서 dismissToast 가 불린다(× 경로가 아니다)
    expect(stickyToasts.has(ID)).toBe(false);
    expect((ws.formationView as { muted?: boolean }).muted).toBeUndefined();
    clock.now = T0 + 70_000;
    fns.checkDeptFormationNotices();
    expect(stickyToasts.has(ID)).toBe(true); // 다시 났다
    const el = (stickyToasts.get(ID) as { el: ToastEl }).el;
    expect(el.querySelector(".toast-name")?.textContent).toBe("팀원을 켜는 중");
    // 또 만료돼도 그때마다 다시 난다
    (liveTimer() as { fn: () => void }).fn();
    clock.now = T0 + 140_000;
    fns.checkDeptFormationNotices();
    expect(stickyToasts.has(ID)).toBe(true);
  });
  it("★× 버튼(실제 click 처리기)을 누르면 닫음 표식이 서고 토스트가 걷힌다 — 다음 점검은 되살리지 않는다(값이 바뀌어도)", () => {
    const { fns, ws, stickyToasts, closeButton, clock } = setup();
    fns.showDeptFormation(ws, "booting");
    closeButton().click();
    expect(stickyToasts.has(ID)).toBe(false);
    expect((ws.formationView as { muted?: boolean }).muted).toBe(true);
    clock.now = T0 + 70_000;
    ws.tree = { type: "pane", sid: 1 };
    fns.checkDeptFormationNotices();
    expect(stickyToasts.has(ID)).toBe(false); // 되살리지 않았다
    clock.now = T0 + 200_000;
    fns.checkDeptFormationNotices();
    expect(stickyToasts.has(ID)).toBe(false);
  });
  it("× 로 닫은 뒤에도 최종 안내(다섯이 붙음)는 한 번 난다 · 같은 id 의 다시 내기는 같은 엘리먼트를 갱신한다(두 개로 쌓이지 않는다)", () => {
    const { fns, ws, stickyToasts, closeButton, clock } = setup();
    fns.showDeptFormation(ws, "booting");
    closeButton().click();
    clock.now = T0 + 130_000;
    const roles = ["master", "cso", "worker", "reviewer-gemini", "reviewer-codex"];
    fns.checkDeptFormationNotices(new Map([[SOCK, roles]]));
    expect(stickyToasts.has(ID)).toBe(true);
    const el = (stickyToasts.get(ID) as { el: ToastEl }).el;
    expect(el.querySelector(".toast-name")?.textContent).toBe("팀 자리가 모두 붙었습니다");
    expect(el.querySelector(".toast-detail")?.textContent).toBe("자리 5개가 모두 붙었습니다 · 걸린 시간 2분");
    expect(ws.formationDone).toBe(true);
    // 같은 id 로 다시 내면(여기서는 직접) 새 엘리먼트를 만들지 않고 갱신한다
    fns.showDeptFormation(ws, "check", 3);
    expect((stickyToasts.get(ID) as { el: ToastEl }).el === el).toBe(true);
  });
  it("다른 id 의 토스트를 ×로 닫아도 팀원 안내의 표식은 서지 않는다 · 수명 만료 배너 보강 대상이 아니다(안내는 조용히 사라지고 이력에 남는다)", () => {
    const { fns, ws, stickyToasts, banners, liveTimer } = setup();
    fns.showDeptFormation(ws, "booting");
    fns.stickyToast("restore", "feed", "복원 중", "x"); // 무관한 sticky 토스트
    const other = (stickyToasts.get("restore") as { el: ToastEl }).el.children.find((c) => c.className === "toast-x") as ToastEl;
    other.click();
    expect(stickyToasts.has("restore")).toBe(false);
    expect(stickyToasts.has(ID)).toBe(true);
    expect((ws.formationView as { muted?: boolean }).muted).toBeUndefined();
    (liveTimer() as { fn: () => void }).fn();
    expect(banners).toEqual([]); // dept-formation 은 OS 배너 보강 대상이 아니다
  });
});

describe("★S4 n1 — 「팀 직접 만들기」 실패 알림은 지속 알림(sticky · 닫기 버튼 있음)으로 — 문구는 그대로", () => {
  /** cys-dept 가 실제로 내는 데몬 기동 실패 문구(ready_fail_note 꼴 · 로그 경로와 환경변수 이름 포함) — Tauri 가 `dept-create:<코드>:<stderr>` 로 전달한다. 150자 이상이다. */
  const LONG = "dept-create:1:[cys-dept] ERROR: dept-2 데몬 기동 실패 (소켓 대기 12초 · 로그: /Users/runner/.local/state/cys-dept-dept-2/cysd.log · 느린 디스크라면 CYS_DEPT_READY_SECS=60 처럼 대기 예산을 늘릴 수 있다)";
  function setup(failWith: unknown) {
    const toasts: { fn: string; args: unknown[] }[] = [];
    const dismissed: string[] = [];
    const rec = (fn: string) => (...args: unknown[]): void => {
      toasts.push({ fn, args });
    };
    const btn = { disabled: false, textContent: "팀 직접 만들기" };
    const fns = load(
      ["launchDept"],
      {
        daemonActionBlocked: () => false,
        notifyTeamFlowBusy: rec("notifyTeamFlowBusy"),
        deptBtnEl: () => btn,
        addDeptWorkspace: (): Promise<unknown> => (failWith === undefined ? Promise.resolve({}) : Promise.reject(failWith)),
        toast: rec("toast"),
        stickyToast: rec("stickyToast"),
        dismissToast: (id: string): void => void dismissed.push(id),
        buildDeptCreatePlan: () => ({ title: "t", body: "b", yesLabel: "y", noLabel: "n", displayName: "d" }),
        confirmModal: (): Promise<boolean> => Promise.resolve(false),
      },
      "let deptLaunchInFlight = false;",
      "getInFlight: () => deptLaunchInFlight",
    );
    return { fns, toasts, btn, dismissed };
  }
  const ctx = { registry: null, catalog: null };

  it("★일반 실패(코드 1·2·그 밖 · 평문)는 sticky 알림 하나 — id 는 고정(같은 실패는 갱신) · 등급 watchdog · 이름 '팀 만들기 실패' · **문구는 받은 그대로**(앞에 붙는 것도 자르는 것도 없다) · 일반 토스트(8초)는 아니다", async () => {
    // Tauri 의 invoke 는 Rust `Err(String)` 을 **문자열**로 reject 한다(launchDept 가 String(e) 로 읽는다) — 검체도 문자열로 흘려 넣는다.
    for (const err of [LONG, `dept-create:2:팀 소개 기록 실패`, "dept-create:6:시드 실패", "평문 오류(레거시 allocate 실패)", "dept-create:-1:x"]) {
      const { fns, toasts } = setup(err);
      const outcome = await fns.launchDept("sales", ctx, "영업팀");
      expect(outcome).toBe("failed");
      expect({ 오류: String(err), 호출: toasts.map((t) => t.fn) }).toEqual({ 오류: String(err), 호출: ["stickyToast"] });
      expect(toasts[0].args).toEqual(["dept-create-failed", "watchdog", "팀 만들기 실패", String(err)]);
    }
    expect(LONG.length).toBeGreaterThan(150); // 8초 토스트로는 읽고 옮기기 어려운 길이다 — 이 실패 알림이 지속 알림이어야 하는 이유
  });
  it("계정 격리 불가(5)·카탈로그 키(4)는 종전 일반 토스트 그대로 — 짧은 고정 문구라 지속 알림으로 바꾸지 않는다(이 알림만 바꾼다)", async () => {
    const five = setup("dept-create:5:account dir 미존재");
    await five.fns.launchDept("sales", ctx, "영업팀");
    expect(five.toasts).toEqual([{ fn: "toast", args: ["watchdog", "부서 생성 차단(계정 격리 불가)", "account dir 미존재 — 레거시 폴백 금지(보안). 카탈로그 account 경로 점검."] }]);
    const four = setup("dept-create:4:x");
    await four.fns.launchDept("sales", ctx, "영업팀");
    expect(four.toasts).toEqual([{ fn: "toast", args: ["watchdog", "부서 생성 실패(카탈로그 키)", "카탈로그 미정의 부서 — 레거시 폴백 안 함."] }]);
  });
  it("exit 3(카탈로그 부재)는 종전대로 새 대상 재확인 창을 거친다 — 알림을 내지 않고, 거절하면 cancelled", async () => {
    const { fns, toasts } = setup("dept-create:3:카탈로그 없음");
    expect(await fns.launchDept("sales", ctx, "영업팀")).toBe("cancelled");
    expect(toasts).toEqual([]);
  });
  it("★새 시도가 시작되면 지난 실패 알림(지속 알림 · 수명 60초)을 걷는다 — 재시도가 성공했는데 '팀 만들기 실패' 가 남아 있지 않다 · 실패하면 같은 id 로 다시 난다 · 진행 중(busy)·차단 때는 건드리지 않는다", async () => {
    const ok = setup(undefined);
    expect(await ok.fns.launchDept("sales", ctx, "영업팀")).toBe("created");
    expect(ok.dismissed).toEqual(["dept-create-failed"]);
    expect(ok.toasts).toEqual([]);
    const bad = setup(LONG);
    await bad.fns.launchDept("sales", ctx, "영업팀");
    expect(bad.dismissed).toEqual(["dept-create-failed"]); // 시작할 때 한 번 걷고
    expect(bad.toasts.map((t) => t.fn)).toEqual(["stickyToast"]); // 실패하면 같은 id 로 다시 낸다
    expect(bad.toasts[0].args[0]).toBe("dept-create-failed");
    const f = fnText("launchDept", true);
    expect(f.indexOf('dismissToast("dept-create-failed");')).toBeGreaterThan(f.indexOf("deptLaunchInFlight = true;"));
    expect(f.indexOf('dismissToast("dept-create-failed");')).toBeLessThan(f.indexOf("await addDeptWorkspace(catalogKey);"));
  });
  it("성공은 알림 없음 · 실패 뒤에도 진행 중 가드와 버튼이 풀린다(종전 계약)", async () => {
    const ok = setup(undefined);
    expect(await ok.fns.launchDept("sales", ctx, "영업팀")).toBe("created");
    expect(ok.toasts).toEqual([]);
    const bad = setup(LONG);
    await bad.fns.launchDept("sales", ctx, "영업팀");
    expect(bad.fns.getInFlight()).toBe(false);
    expect(bad.btn.disabled).toBe(false);
    expect(bad.btn.textContent).toBe("팀 직접 만들기");
  });
  it("★지속 알림이 하는 일 — 수명 60초(일반 토스트 8초의 7배 넘게)·닫기 버튼(×)이 있다·같은 id 는 갱신된다(이력도 한 건으로 합쳐진다)", () => {
    expect(toastTtl("volatile").ttlMs).toBe(VOLATILE_TTL_MS);
    expect(toastTtl("sticky", "dept-create-failed").ttlMs).toBe(STICKY_TTL_MS);
    expect(STICKY_TTL_MS).toBeGreaterThanOrEqual(7 * VOLATILE_TTL_MS);
    const st = fnText("stickyToast", true);
    expect(st).toContain("addToastCloseButton(el, id);"); // 닫기 버튼
    expect(st).toContain("recordAlarm(category, name, detail, id);"); // 같은 id 는 이력에서 합쳐진다
  });
  it("★배선 핀: launchDept 의 일반 실패 분기가 stickyToast(\"dept-create-failed\", \"watchdog\", \"팀 만들기 실패\", msg) 하나 — 같은 분기에 일반 toast 가 남아 있지 않다 · 다른 호출 경로(팔레트·메뉴의 .catch)는 이번 범위가 아니다", () => {
    const f = fnText("launchDept", true);
    expect(f).toContain('stickyToast("dept-create-failed", "watchdog", "팀 만들기 실패", msg);');
    expect(f.includes('toast("watchdog", "팀 만들기 실패", msg)')).toBe(false);
    expect(f.split("stickyToast(").length - 1).toBe(1);
    // 종전의 두 안내(5·4)는 그대로 일반 토스트
    expect(f).toContain('toast("watchdog", "부서 생성 차단(계정 격리 불가)"');
    expect(f).toContain('toast("watchdog", "부서 생성 실패(카탈로그 키)"');
  });
});
