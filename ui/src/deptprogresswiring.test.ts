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
//   · 팀원 부팅 안내: 성공 직후 1회 + 값(열쇠)이 바뀔 때만 갱신(새 setInterval 0 — refreshPaneTitles 끝에서 점검) · 편성 결과(formation-*)는 socket_slug 가 새로 만든 탭으로 해석될 때만 ·
//     15분 상한 · 닫힌 탭은 거둔다 · 사용자가 닫은 안내는 주기 갱신으로 되살리지 않는다 · 표시 전용(어떤 명령도 보내지 않는다).
//   · Tauri 계약: invoke 인자 progressId ↔ allocate_dept_daemon 의 progress_id · 이벤트 'dept-create-progress' `{id, stage}`.
import { describe, it, expect } from "bun:test";
import { readFileSync } from "node:fs";
import {
  deptPendingText,
  deptFormationText,
  deptFormationNoticeKey,
  deptFormationCapped,
  deptFormationToastId,
  deptFormationStateOfKind,
  deptFormationDetail,
  deptFirstSeatPending,
  deptFirstSeatRemainingMs,
  deptProgressId,
  parseDeptProgressPayload,
  DEPT_FORMATION_TOAST_PREFIX,
  DEPT_FIRST_SEAT_TEXT,
} from "./deptprogress";

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
    const show = f.indexOf('showDeptFormation(ws, "booting", null);');
    const refresh = f.indexOf("await refreshPaneTitles();");
    expect(show).toBeGreaterThan(at);
    expect(show).toBeLessThan(refresh);
    const seg = f.slice(at, refresh);
    expect(/try \{\s*showDeptFormation\(ws, "booting", null\);\s*\} catch \{/.test(seg)).toBe(true);
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
    for (const name of ["onDeptCreateProgress", "showDeptFormation", "checkDeptFormationNotices", "onDeptFormationFeed"]) {
      const g = fnText(name, true);
      expect({ 함수: name, setInterval: g.includes("setInterval(") }).toEqual({ 함수: name, setInterval: false });
    }
    expect(CODE.split("setInterval(refreshPaneTitles, 3000);").length - 1).toBe(1); // 기존 3초 틱은 그대로 하나
    // ★총수 핀: v0.14.42 기준 main.ts 의 `setInterval(` 은 9곳이었고, 이 티켓이 더한 것은 renderDeptPending 의 엘리먼트 수명 타이머 1곳(= 10)뿐이다.
    //   다른 티켓이 합법적으로 타이머를 더하면 이 수를 올리면서 사유를 남긴다 — 팀원 부팅 안내용 새 타이머가 슬쩍 들어오는 길을 막는 핀이다.
    expect(CODE.split("setInterval(").length - 1).toBe(10);
    // 어디에 있든 setInterval 의 첫 인자가 팀원 부팅 안내 함수면 금지
    expect(/setInterval\([^;]{0,120}(checkDeptFormationNotices|showDeptFormation|onDeptFormationFeed|deptFormation)/.test(CODE)).toBe(false);
    // 새 코드가 쓰는 setTimeout 은 renderIdleWorkspace 의 일회성 하나뿐(60초 창이 끝날 때 문구를 한 번 고친다)
    for (const name of ["onDeptCreateProgress", "showDeptFormation", "checkDeptFormationNotices", "onDeptFormationFeed", "renderDeptPending"]) {
      expect({ 함수: name, setTimeout: fnText(name, true).includes("setTimeout(") }).toEqual({ 함수: name, setTimeout: false });
    }
  });
  it("★팀원 부팅 안내 점검은 기존 3초 틱(refreshPaneTitles)의 끝에서 try/catch 로 부른다 — updateFtRoot 앞", () => {
    const f = fnText("refreshPaneTitles", true);
    const call = f.indexOf("checkDeptFormationNotices();");
    expect(call).toBeGreaterThan(0);
    expect(call).toBeGreaterThan(f.indexOf("} finally {")); // 본 루프(try/finally)가 끝난 뒤
    expect(call).toBeLessThan(f.indexOf("updateFtRoot();"));
    expect(/try \{\s*checkDeptFormationNotices\(\);\s*\} catch \{/.test(f)).toBe(true);
  });
  it("onDaemonEvent 의 feed.item.created 분기가 편성 결과를 안내에 반영한다 — 기존 토스트 뒤 · try/catch · 확인 창 0", () => {
    const i = CODE.indexOf('if (name === "feed.item.created") {');
    expect(i).toBeGreaterThan(0);
    const seg = CODE.slice(i, CODE.indexOf("refreshFeed();", i));
    const hook = seg.indexOf("onDeptFormationFeed(event.socket_slug, payload.kind, payload.body, payload.title);");
    expect(hook).toBeGreaterThan(seg.indexOf("feedCreatedToastTitle(payload.kind)")); // 기존 토스트는 그대로, 그 뒤
    expect(/try \{\s*onDeptFormationFeed\(event\.socket_slug, payload\.kind, payload\.body, payload\.title\);\s*\} catch \{/.test(seg)).toBe(true);
    expect(seg.includes("confirmModal")).toBe(false);
    expect(seg.includes("runTeamProposalFlow")).toBe(false);
  });
  it("새 함수들은 입력을 신뢰하지 않는 순수 도우미를 지난다 · HTML 삽입 0 · 명령 전송 0", () => {
    expect(fnText("onDeptCreateProgress", true)).toContain("parseDeptProgressPayload(");
    const feed = fnText("onDeptFormationFeed", true);
    expect(feed).toContain("deptFormationStateOfKind(");
    expect(feed).toContain("deptFormationDetail(");
    for (const name of ["onDeptCreateProgress", "showDeptFormation", "checkDeptFormationNotices", "onDeptFormationFeed", "renderDeptPending"]) {
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
  it("Workspace 에 표시 전용 필드 5개(pendingSince·pendingStage·createdAt·formationDone·formationView)", () => {
    const a = CODE.indexOf("interface Workspace {");
    const iface = CODE.slice(a, CODE.indexOf("\n}\n", a));
    for (const field of ["pendingSince?: number;", "pendingStage?: string;", "createdAt?: number;", "formationDone?: boolean;", "formationView?: {"]) {
      expect({ 필드: field, 있음: iface.includes(field) }).toEqual({ 필드: field, 있음: true });
    }
  });
  it("★저장본에는 실리지 않는다 — saveLayout 이 다섯 필드를 직렬화에서 턴다(기존 첫 map 한 줄은 그대로)", () => {
    const f = fnText("saveLayout", true);
    expect(f).toContain("norm.map(({ deleting: _d, stopFailed: _s, ...w }) => w)");
    for (const strip of ["pendingSince: _ps", "pendingStage: _pg", "createdAt: _ca", "formationDone: _fd", "formationView: _fv"]) {
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
  function setup(over: { invoke?: (cmd: string, args: Record<string, unknown>, ctx: Ctx) => Promise<unknown>; showThrows?: boolean } = {}) {
    const invokes: Inv[] = [];
    const workspaces: Record<string, unknown>[] = [];
    const socketForSlug = new Map<string, string>();
    const events: string[] = [];
    const shown: unknown[] = [];
    const ctx: Ctx = { workspaces, events };
    const deps = {
      workspaces,
      socketForSlug,
      Date: { now: () => T },
      deptProgressId,
      invoke: (cmd: string, args: Record<string, unknown>): Promise<unknown> => {
        invokes.push({ cmd, args });
        return over.invoke ? over.invoke(cmd, args, ctx) : Promise.resolve(INFO);
      },
      render: (): void => void events.push("render"),
      refreshPaneTitles: (): Promise<void> => {
        events.push("refresh");
        return Promise.resolve();
      },
      collectSids: (tree: unknown): number[] => (tree ? [1] : []),
      setFocus: (): void => void events.push("focus"),
      showDeptFormation: (ws: unknown, state: string, detail: unknown): void => {
        events.push("show");
        shown.push({ ws, state, detail });
        if (over.showThrows) throw new Error("표시 실패(검체)");
      },
    };
    const fns = load(["addDeptWorkspace"], deps, "let wsCounter = 41; let activeWs = 0;", "getActive: () => activeWs, getCounter: () => wsCounter");
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
    expect((shown[0] as { state: string; detail: unknown }).state).toBe("booting");
    expect((shown[0] as { state: string; detail: unknown }).detail).toBeNull();
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
});

describe("onDaemonEvent feed.item.created 분기 — 실제 본문 실행(기존 토스트 뒤에 편성 안내를 반영 · 실패해도 나머지는 그대로)", () => {
  const start = CODE.indexOf('    if (name === "feed.item.created") {');
  expect({ 분기: "feed.item.created", 존재: start >= 0 }).toEqual({ 분기: "feed.item.created", 존재: true });
  const branch = CODE.slice(start, CODE.indexOf("\n    }\n", start) + 6);
  function run(payload: Record<string, unknown>, event: Record<string, unknown>, feedHookThrows = false) {
    const calls: { fn: string; args: unknown[] }[] = [];
    const rec = (fn: string) => (...args: unknown[]) => {
      calls.push({ fn, args });
    };
    const js = new Bun.Transpiler({ loader: "ts" }).transformSync(branch);
    const f = new Function("name", "event", "payload", "toast", "feedCreatedToastTitle", "TEAM_CREATE_KIND", "scheduleFeedSwitchIfStillPending", "onDeptFormationFeed", js);
    f(
      "feed.item.created",
      event,
      payload,
      rec("toast"),
      (k: unknown): string => `제목:${String(k)}`,
      "team-create",
      rec("scheduleFeedSwitchIfStillPending"),
      (...a: unknown[]): void => {
        calls.push({ fn: "onDeptFormationFeed", args: a });
        if (feedHookThrows) throw new Error("안내 실패(검체)");
      },
    );
    return calls;
  }
  it("★기존 토스트(feed 알림) 뒤에 (socket_slug, kind, body, title) 를 편성 안내로 넘긴다 — 편성 종류가 아니어도 넘기고 판정은 그쪽이 한다", () => {
    const calls = run({ kind: "formation-complete", title: "팀 편성 완료", body: "5/5", request_id: "r1" }, { socket_slug: "abc" });
    expect(calls.map((c) => c.fn)).toEqual(["toast", "onDeptFormationFeed"]);
    expect(calls[0].args).toEqual(["feed", "제목:formation-complete", "팀 편성 완료"]);
    expect(calls[1].args).toEqual(["abc", "formation-complete", "5/5", "팀 편성 완료"]);
  });
  it("★안내가 던져도 이 분기의 나머지(자동 전환 예약)는 그대로 돈다", () => {
    const calls = run({ kind: "formation-failed", title: "t", body: "b", request_id: "r2", wait: true }, { socket_slug: "abc" }, true);
    expect(calls.map((c) => c.fn)).toEqual(["toast", "onDeptFormationFeed", "scheduleFeedSwitchIfStillPending"]);
    expect(calls[2].args).toEqual(["r2", false]);
  });
  it("팀 제안 종류(team-create)는 종전 토스트 그대로 · 안내 호출은 같은 자리에서 불리되 판정에서 걸러진다(deptFormationStateOfKind)", () => {
    const calls = run({ kind: "team-create", title: "제안", body: "" }, { socket_slug: "abc" });
    expect(calls[0].fn).toBe("toast");
    expect(calls[0].args[0]).toBe("feed");
    expect(deptFormationStateOfKind("team-create")).toBeNull();
  });
});

describe("saveLayout — 실제 본문 실행(저장본에는 런타임 전용·표시 전용 필드가 실리지 않는다)", () => {
  it("★deleting·stopFailed 와 이번에 더한 다섯 필드(pendingSince·pendingStage·createdAt·formationDone·formationView)는 직렬화에서 빠지고 나머지는 그대로 저장된다 · 대기 탭은 저장 제외", () => {
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
        formationView: { state: "booting", detail: null, key: "k" },
        pendingSince: 99,
        pendingStage: "wait",
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
    for (const gone of ["createdAt", "formationDone", "formationView", "pendingSince", "pendingStage", "deleting", "stopFailed", "pending"]) {
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

describe("팀원 부팅 안내 — 실제 본문 실행(showDeptFormation · checkDeptFormationNotices · onDeptFormationFeed)", () => {
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
  function setup() {
    const clock = { now: T0 };
    const calls: Call[] = [];
    const dismissed: string[] = [];
    const stickyToasts = new Map<string, { el: unknown; timer: unknown }>();
    const socketForSlug = new Map<string, string>();
    const workspaces: Record<string, unknown>[] = [];
    const deps = {
      workspaces,
      socketForSlug,
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
      deptFormationStateOfKind,
      deptFormationDetail,
      DEPT_FORMATION_TOAST_PREFIX,
    };
    const fns = load(["collectSids", "showDeptFormation", "checkDeptFormationNotices", "onDeptFormationFeed"], deps);
    const newTeam = (id: number, socket: string, slug: string): Record<string, unknown> => {
      const ws: Record<string, unknown> = { id, name: `dept-${id}`, tree: null, socket, pending: false, createdAt: clock.now };
      workspaces.push(ws);
      socketForSlug.set(slug, socket);
      return ws;
    };
    return { clock, calls, dismissed, stickyToasts, socketForSlug, workspaces, fns, newTeam };
  }

  it("collectSids(실제 본문) — 구멍(음수 sid)은 자리 수에 안 센다", () => {
    const { fns } = setup();
    expect(fns.collectSids(treeOf(3, 2))).toEqual([1, 2, 3]);
    expect(fns.collectSids(null)).toEqual([]);
  });

  it("★성공 직후 첫 안내 — 같은 id(dept-formation:<소켓>)·'feed' 등급·「팀원을 켜는 중」·자리 0·경과 1분 미만 · 탭에 열쇠가 적힌다", () => {
    const { fns, calls, newTeam } = setup();
    const ws = newTeam(1, "/s/1.sock", "slug1");
    fns.showDeptFormation(ws, "booting", null);
    expect(calls.length).toBe(1);
    expect(calls[0].id).toBe("dept-formation:/s/1.sock");
    expect(calls[0].category).toBe("feed");
    expect(calls[0].name).toBe("팀원을 켜는 중");
    expect(calls[0].detail).toBe("부서장·CSO·워커·리뷰어가 차례로 켜집니다(보통 3~5분) · 지금 0자리 · 경과 1분 미만");
    const view = ws.formationView as { state: string; detail: unknown; key: string };
    expect(view.state).toBe("booting");
    expect(view.detail).toBeNull();
    expect(view.key).toBe(deptFormationNoticeKey({ seats: 0, elapsedSec: 0, state: "booting" }));
  });
  it("createdAt 이 없는 탭(복원된 탭)·소켓이 없는 탭은 안내를 내지 않는다", () => {
    const { fns, calls, workspaces } = setup();
    const a: Record<string, unknown> = { id: 1, name: "x", tree: null, socket: "/s/a" };
    const b: Record<string, unknown> = { id: 2, name: "y", tree: null, createdAt: T0 };
    workspaces.push(a, b);
    fns.showDeptFormation(a, "booting", null);
    fns.showDeptFormation(b, "booting", null);
    expect(calls.length).toBe(0);
  });

  it("★값(열쇠)이 바뀔 때만 다시 낸다 — 같은 틱 반복·같은 분 안 · 자리 수가 바뀌면 1회 · 분이 바뀌면 1회 · 45초 칸이 바뀌면 1회", () => {
    const { fns, calls, clock, newTeam } = setup();
    const ws = newTeam(1, "/s/1.sock", "slug1");
    fns.showDeptFormation(ws, "booting", null); // t=0
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
    expect(calls[1].detail).toBe("부서장·CSO·워커·리뷰어가 차례로 켜집니다(보통 3~5분) · 지금 1자리 · 경과 1분 미만");
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
    expect(calls[3].detail).toBe("부서장·CSO·워커·리뷰어가 차례로 켜집니다(보통 3~5분) · 지금 1자리 · 경과 1분");
  });

  it("★5분(3초 틱 100번)을 돌려도 호출은 한 자릿수~십여 회 · 두 호출 사이는 47초 이하(토스트 기본 수명 60초가 갱신 사이에 끝나지 않는다)", () => {
    const { fns, calls, clock, newTeam } = setup();
    const ws = newTeam(1, "/s/1.sock", "slug1");
    fns.showDeptFormation(ws, "booting", null);
    for (let t = 3; t <= 300; t += 3) {
      clock.now = T0 + t * 1000;
      const seats = t >= 250 ? 5 : t >= 190 ? 4 : t >= 130 ? 3 : t >= 70 ? 2 : t >= 10 ? 1 : 0;
      ws.tree = treeOf(seats);
      fns.checkDeptFormationNotices();
    }
    expect(calls.length).toBeLessThan(25);
    expect(calls.length).toBeGreaterThan(6);
    let maxGap = 0;
    for (let i = 1; i < calls.length; i++) maxGap = Math.max(maxGap, (calls[i].at - calls[i - 1].at) / 1000);
    expect(maxGap).toBeLessThan(48); // 47초 이하(틱이 3초 격자라 정수)
    expect(calls[calls.length - 1].detail).toContain("지금 5자리");
    expect(calls[calls.length - 1].detail).toContain("경과 5분");
    // 모든 호출은 같은 id 하나(탭마다 하나 · 갱신)
    expect(new Set(calls.map((c) => c.id)).size).toBe(1);
  });

  it("★15분(900초)이 지나면 formationDone 으로 접는다 — 그 뒤로는 값이 바뀌어도 호출 0", () => {
    const { fns, calls, clock, newTeam } = setup();
    const ws = newTeam(1, "/s/1.sock", "slug1");
    fns.showDeptFormation(ws, "booting", null);
    clock.now = T0 + 899_000;
    fns.checkDeptFormationNotices();
    expect(ws.formationDone).toBeUndefined();
    const before = calls.length;
    clock.now = T0 + 900_000;
    ws.tree = treeOf(3);
    fns.checkDeptFormationNotices();
    expect(ws.formationDone).toBe(true);
    expect(calls.length).toBe(before); // 상한 틱에는 새 안내도 없다
    clock.now = T0 + 1_200_000;
    ws.tree = treeOf(5);
    fns.checkDeptFormationNotices();
    expect(calls.length).toBe(before);
  });

  it("★탭이 닫히면(어떤 경로로든 workspaces 에서 사라지면) 그 안내를 dismissToast 한다 — 다른 탭의 안내·무관한 토스트는 그대로", () => {
    const { fns, calls, dismissed, stickyToasts, workspaces, newTeam } = setup();
    const a = newTeam(1, "/s/1.sock", "slug1");
    const b = newTeam(2, "/s/2.sock", "slug2");
    fns.showDeptFormation(a, "booting", null);
    fns.showDeptFormation(b, "booting", null);
    stickyToasts.set("restore", { el: {}, timer: 0 }); // 무관한 토스트
    stickyToasts.set("dept-formation:/s/ghost.sock", { el: {}, timer: 0 }); // 탭 없는 소켓의 안내 잔재
    workspaces.splice(workspaces.indexOf(a), 1); // 탭 ×
    fns.checkDeptFormationNotices();
    expect(dismissed.sort()).toEqual(["dept-formation:/s/1.sock", "dept-formation:/s/ghost.sock"]);
    expect(stickyToasts.has("dept-formation:/s/2.sock")).toBe(true);
    expect(stickyToasts.has("restore")).toBe(true);
    expect(calls.length).toBe(2); // 닫힌 탭 정리는 새 토스트를 내지 않는다
  });

  it("★사용자가 안내를 ×로 닫았으면 주기 갱신으로 되살리지 않는다(muted) — 값이 바뀌어도 호출 0", () => {
    const { fns, calls, clock, stickyToasts, newTeam } = setup();
    const ws = newTeam(1, "/s/1.sock", "slug1");
    fns.showDeptFormation(ws, "booting", null);
    stickyToasts.delete("dept-formation:/s/1.sock"); // 사용자가 × 를 눌렀다
    clock.now = T0 + 20_000;
    ws.tree = treeOf(2);
    fns.checkDeptFormationNotices();
    expect((ws.formationView as { muted?: boolean }).muted).toBe(true);
    expect(calls.length).toBe(1);
    clock.now = T0 + 70_000;
    ws.tree = treeOf(3);
    fns.checkDeptFormationNotices();
    expect(calls.length).toBe(1);
  });

  it("첫 안내가 빠졌어도(formationView 없음) 점검이 한 번 내 준다(안전망)", () => {
    const { fns, calls, newTeam } = setup();
    const ws = newTeam(1, "/s/1.sock", "slug1");
    fns.checkDeptFormationNotices();
    expect(calls.length).toBe(1);
    expect(calls[0].name).toBe("팀원을 켜는 중");
    expect(ws.formationView === undefined).toBe(false);
  });
  it("pending 탭·createdAt 없는 탭·formationDone 탭은 점검이 건드리지 않는다", () => {
    const { fns, calls, workspaces } = setup();
    workspaces.push(
      { id: 1, name: "…", tree: null, pending: true, socket: "/s/p", createdAt: T0 },
      { id: 2, name: "r", tree: null, socket: "/s/r" },
      { id: 3, name: "d", tree: null, socket: "/s/d", createdAt: T0, formationDone: true },
    );
    fns.checkDeptFormationNotices();
    expect(calls.length).toBe(0);
  });

  describe("편성 결과 이벤트(feed.item.created · formation-*)", () => {
    it("★complete — slug 가 새로 만든 탭의 소켓으로 해석되면 완료 문구로 한 번 갱신하고 formationDone=true · 이후 점검은 갱신하지 않는다", () => {
      const { fns, calls, clock, newTeam } = setup();
      const ws = newTeam(1, "/s/1.sock", "slug1");
      fns.showDeptFormation(ws, "booting", null);
      ws.tree = treeOf(5);
      clock.now = T0 + 252_000;
      fns.onDeptFormationFeed("slug1", "formation-complete", "본문", "제목");
      const last = calls[calls.length - 1];
      expect(last.id).toBe("dept-formation:/s/1.sock");
      expect(last.category).toBe("feed");
      expect(last.name).toBe("팀 준비 완료");
      expect(last.detail).toBe("5자리 · 4분 12초 걸렸습니다");
      expect(ws.formationDone).toBe(true);
      const n = calls.length;
      clock.now = T0 + 400_000;
      ws.tree = treeOf(6);
      fns.checkDeptFormationNotices();
      fns.onDeptFormationFeed("slug1", "formation-complete", "", "");
      expect(calls.length).toBe(n); // 더 갱신하지 않는다(토스트는 수명대로 사라진다)
    });
    it("★partial·pending·failed — feed 본문(없으면 제목)을 사유로 실어 '확인 필요' 문구로 갱신하되 계속 추적한다(formationDone 아님)", () => {
      for (const [kind, state] of [
        ["formation-partial", "partial"],
        ["formation-pending", "pending"],
        ["formation-failed", "failed"],
      ] as const) {
        const { fns, calls, clock, newTeam } = setup();
        const ws = newTeam(1, "/s/1.sock", "slug1");
        fns.showDeptFormation(ws, "booting", null);
        ws.tree = treeOf(2);
        clock.now = T0 + 130_000;
        fns.onDeptFormationFeed("slug1", kind, "worker 자리가 아직 안 떴습니다", "제목은 쓰지 않는다");
        let last = calls[calls.length - 1];
        expect({ kind, name: last.name, category: last.category }).toEqual({ kind, name: "팀원 켜기 — 확인 필요", category: "watchdog" });
        expect(last.detail).toBe("worker 자리가 아직 안 떴습니다 · 지금 2자리 · 경과 2분");
        expect((ws.formationView as { state: string }).state).toBe(state);
        expect(ws.formationDone).toBeUndefined();
        // 계속 추적: 분이 바뀌면 같은 사유로 갱신된다
        clock.now = T0 + 185_000;
        fns.checkDeptFormationNotices();
        last = calls[calls.length - 1];
        expect(last.name).toBe("팀원 켜기 — 확인 필요");
        expect(last.detail).toBe("worker 자리가 아직 안 떴습니다 · 지금 2자리 · 경과 3분");
        // 본문이 없으면 제목 · 둘 다 없으면 일반 안내
        fns.onDeptFormationFeed("slug1", kind, "", "제목만 있음");
        expect(calls[calls.length - 1].detail).toBe("제목만 있음 · 지금 2자리 · 경과 3분");
        fns.onDeptFormationFeed("slug1", kind, undefined, undefined);
        expect(calls[calls.length - 1].detail).toBe("일부 자리가 아직 켜지지 않았습니다 — Control Center 에서 자리 상태를 확인하세요 · 지금 2자리 · 경과 3분");
      }
    });
    it("★slug 가 없거나·해석되지 않거나·다른 부서의 것이면 아무것도 하지 않는다(다른 부서로 오인 금지)", () => {
      const { fns, calls, newTeam, socketForSlug, workspaces } = setup();
      const ws = newTeam(1, "/s/1.sock", "slug1");
      fns.showDeptFormation(ws, "booting", null);
      const n = calls.length;
      for (const slug of [undefined, null, "", 7, {}, [], "unknown-slug"]) {
        fns.onDeptFormationFeed(slug, "formation-complete", "x", "y");
        fns.onDeptFormationFeed(slug, "formation-failed", "x", "y");
      }
      expect(calls.length).toBe(n);
      expect(ws.formationDone).toBeUndefined();
      // 해석은 되지만 그 소켓의 탭이 이 세션에서 새로 만든 탭이 아니다(복원된 탭: createdAt 없음 · 탭 자체가 없음)
      socketForSlug.set("restored", "/s/restored.sock");
      socketForSlug.set("nobody", "/s/nobody.sock");
      const restored: Record<string, unknown> = { id: 9, name: "r", tree: null, socket: "/s/restored.sock", pending: false };
      workspaces.push(restored);
      fns.onDeptFormationFeed("restored", "formation-complete", "", "");
      fns.onDeptFormationFeed("restored", "formation-failed", "사유", "");
      fns.onDeptFormationFeed("nobody", "formation-complete", "", "");
      expect(calls.length).toBe(n);
      expect(restored.formationDone).toBeUndefined();
      expect(restored.formationView).toBeUndefined();
    });
    it("두 팀이 동시에 켜지는 중이면 slug 가 가리키는 팀의 안내만 바뀐다(격리)", () => {
      const { fns, calls, newTeam, clock } = setup();
      const a = newTeam(1, "/s/1.sock", "slug1");
      const b = newTeam(2, "/s/2.sock", "slug2");
      fns.showDeptFormation(a, "booting", null);
      fns.showDeptFormation(b, "booting", null);
      clock.now = T0 + 90_000;
      fns.onDeptFormationFeed("slug2", "formation-complete", "", "");
      expect(a.formationDone).toBeUndefined();
      expect(b.formationDone).toBe(true);
      expect(calls[calls.length - 1].id).toBe("dept-formation:/s/2.sock");
    });
    it("편성 종류가 아닌 feed(approval 등)·pending 탭·이미 접힌 탭은 무시한다", () => {
      const { fns, calls, newTeam } = setup();
      const ws = newTeam(1, "/s/1.sock", "slug1");
      fns.showDeptFormation(ws, "booting", null);
      const n = calls.length;
      for (const kind of ["approval", "formation", "formation-", "team-create", undefined, null, 5]) fns.onDeptFormationFeed("slug1", kind, "x", "y");
      expect(calls.length).toBe(n);
      ws.formationDone = true;
      fns.onDeptFormationFeed("slug1", "formation-complete", "", "");
      expect(calls.length).toBe(n);
      ws.formationDone = undefined;
      ws.pending = true;
      fns.onDeptFormationFeed("slug1", "formation-complete", "", "");
      expect(calls.length).toBe(n);
    });
    it("사용자가 닫은 뒤에도 편성 결과 이벤트는 안내를 다시 낸다(주기 갱신만 멈춘다) — 확인 필요 상태는 놓치지 않는다", () => {
      const { fns, calls, stickyToasts, newTeam } = setup();
      const ws = newTeam(1, "/s/1.sock", "slug1");
      fns.showDeptFormation(ws, "booting", null);
      stickyToasts.delete("dept-formation:/s/1.sock");
      fns.checkDeptFormationNotices();
      expect((ws.formationView as { muted?: boolean }).muted).toBe(true);
      const n = calls.length;
      fns.onDeptFormationFeed("slug1", "formation-partial", "일부 미기동", "");
      expect(calls.length).toBe(n + 1);
      expect((ws.formationView as { muted?: boolean }).muted).toBeUndefined();
      expect(stickyToasts.has("dept-formation:/s/1.sock")).toBe(true);
    });
    it("사유 문자열은 정제되어 전달된다(제어문자·줄바꿈 → 공백) — HTML 은 그대로 글자(렌더는 stickyToast 의 textContent)", () => {
      const { fns, calls, newTeam } = setup();
      const ws = newTeam(1, "/s/1.sock", "slug1");
      fns.showDeptFormation(ws, "booting", null);
      fns.onDeptFormationFeed("slug1", "formation-failed", "a\u0000b\n<img src=x onerror=alert(1)>‮", "");
      expect(calls[calls.length - 1].detail).toBe("a b <img src=x onerror=alert(1)> · 지금 0자리 · 경과 1분 미만");
    });
  });
});
