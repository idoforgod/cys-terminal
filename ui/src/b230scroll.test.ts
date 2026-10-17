// 0.14.52 U — B-230 「저절로 첫 대화로 점프」 수리의 main.ts 회귀 핀 — 본문을 그대로 떼어 실행한다(deptprogresswiring.test.ts ② 관례).
//
// ★무엇을 지키는가(B230-REPRO-REPORT · SCOPE-NOTE-H1 · 부서장 승인 2026-10-16):
//   ① render() 는 보이는 pane 호스트를 떼었다 다시 붙인다 — WebKit 이 다시 붙인 `.xterm-viewport` 의 스크롤 위치를 0 으로 되돌려도 xterm 은 제 캐시가
//      바뀔 때만 DOM 을 맞추므로, 다시 붙인 **뒤에** 각 pane 에 resyncViewport 가 불려야 한다(같은 태스크 · rAF 전).
//   ② resyncViewport 는 xterm 공개 API 로 한 행 갔다 온다 — xterm 이 **자기** 갱신을 돌리게 하는 길이고, 위치는 지금 모델에서 다시 나온다.
//      DOM 오프셋을 직접 쓰거나 저장해 둔 옛 값을 되살리는 변형은 떠나 있던 동안 쌓인 출력·낡은 스크롤 영역에서 틀린다(네이티브 재현 FT/s7 셀이 판정).
//   ③ 폭이 바뀌는 리플로우: 위를 읽는 중(follow=false)이면 화면 맨 위 **논리 줄**의 첫 행 마커로 돌아간다(옛 절대 행 번호가 아니다) · follow=true 는
//      종전대로 snapToBottom 몫(여기서는 손대지 않는다) · 대체 화면은 건드리지 않는다.
// ★한계: 이 검체는 가짜 터미널 위의 배선·순서·산술이다. 실제 WebKit/xterm 에서 점프가 사라졌는지는 b230 네이티브 재현(FROZEN-TRACE-1..7 · s7 대조군)이 판정한다.
// ★한계 2(U 게이트 R1 minor 10): render() 를 떼어 실행하므로 변형이 새 자유 이름을 쓰면 단언이 아니라 ReferenceError 로 떨어진다 — '저장해 둔 DOM 오프셋'(M2)이
//   그렇다(구조적 사망 · 판별력 없음). M2 의 판별 검체는 네이티브 s7-away-output-return 이다(B230-REPAIR-REPORT · M2 RED, 첫 휠 59줄).
import { describe, it, expect } from "bun:test";
import { readFileSync } from "node:fs";

declare const Bun: { Transpiler: new (o: { loader: "ts" }) => { transformSync(code: string): string } };

const SRC = readFileSync(new URL("./main.ts", import.meta.url), "utf-8");
const tr = new Bun.Transpiler({ loader: "ts" });

/** 최상위 함수 하나(머리 `function name(` 부터 첫 `\n}\n` 까지). */
function fnText(name: string): string {
  const m = new RegExp(`^function ${name}\\(`, "m").exec(SRC);
  expect({ 함수: name, 존재: m !== null }).toEqual({ 함수: name, 존재: true });
  const a = (m as RegExpExecArray).index;
  const b = SRC.indexOf("\n}\n", a);
  expect({ 함수: name, 끝: b > a }).toEqual({ 함수: name, 끝: true });
  return SRC.slice(a, b + 2);
}
/** makePane 안의 `const keepReadingLine = ...;` 한 덩어리(두 칸 들여쓴 `  };` 까지). */
function closureText(name: string): string {
  const head = `  const ${name} = (`;
  const a = SRC.indexOf(head);
  expect({ 클로저: name, 존재: a >= 0, 유일: a >= 0 && SRC.indexOf(head, a + 1) < 0 }).toEqual({ 클로저: name, 존재: true, 유일: true });
  const b = SRC.indexOf("\n  };\n", a);
  expect({ 클로저: name, 끝: b > a }).toEqual({ 클로저: name, 끝: true });
  return SRC.slice(a, b + 5);
}
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type AnyFn = (...a: any[]) => any;
function loadWith(code: string, ret: string, scope: Record<string, unknown>): AnyFn {
  const js = tr.transformSync(code);
  return new Function("__scope", `with (__scope) {\n${js}\nreturn ${ret};\n}`)(Object.assign(Object.create(null), scope)) as AnyFn;
}

// ── 대역: xterm 공개 API 의 필요한 만큼 ──
interface FakeLine { isWrapped: boolean }
function fakeTerm(o: { baseY: number; viewportY: number; cursorY?: number; type?: string; cols?: number; wrapped?: number[] }) {
  const calls: string[] = [];
  const markers: { line: number; isDisposed: boolean; dispose: () => void }[] = [];
  const buf = {
    type: o.type ?? "normal",
    baseY: o.baseY,
    viewportY: o.viewportY,
    cursorY: o.cursorY ?? 10,
    getLine: (i: number): FakeLine | undefined => (i >= 0 ? { isWrapped: (o.wrapped ?? []).includes(i) } : undefined),
  };
  const term = {
    cols: o.cols ?? 149,
    buffer: { active: buf },
    scrollLines(n: number) {
      calls.push(`scrollLines(${n})`);
      buf.viewportY = Math.max(0, Math.min(buf.baseY, buf.viewportY + n));
    },
    scrollToLine(n: number) {
      calls.push(`scrollToLine(${n})`);
      buf.viewportY = Math.max(0, Math.min(buf.baseY, n));
    },
    registerMarker(off: number) {
      const mk = { line: buf.baseY + buf.cursorY + off, isDisposed: false, dispose() { mk.isDisposed = true; calls.push(`dispose(${mk.line})`); } };
      markers.push(mk);
      calls.push(`registerMarker(${off})`);
      return mk;
    },
    get element() { calls.push("element"); return undefined; },
  };
  return { term, buf, calls, markers };
}

describe("B-230 ② resyncViewport — xterm 이 자기 갱신을 돌리게 한다(위치는 지금 모델)", () => {
  const resync = loadWith(fnText("resyncViewport"), "resyncViewport", {});
  it("바닥에서: 한 행 위로 갔다가 돌아온다 — 위치는 그대로 바닥", () => {
    const t = fakeTerm({ baseY: 945, viewportY: 945 });
    resync(t.term);
    expect({ calls: t.calls, viewportY: t.buf.viewportY }).toEqual({ calls: ["scrollLines(-1)", "scrollLines(1)"], viewportY: 945 });
  });
  it("위를 읽는 중: 같은 줄로 돌아온다", () => {
    const t = fakeTerm({ baseY: 945, viewportY: 645 });
    resync(t.term);
    expect({ calls: t.calls, viewportY: t.buf.viewportY }).toEqual({ calls: ["scrollLines(-1)", "scrollLines(1)"], viewportY: 645 });
  });
  it("맨 위(0)를 읽는 중: 아래로 갔다가 돌아온다(위로는 갈 수 없다)", () => {
    const t = fakeTerm({ baseY: 945, viewportY: 0 });
    resync(t.term);
    expect({ calls: t.calls, viewportY: t.buf.viewportY }).toEqual({ calls: ["scrollLines(1)", "scrollLines(-1)"], viewportY: 0 });
  });
  it("스크롤백이 없으면(대체 화면 포함) 아무것도 하지 않는다", () => {
    const t = fakeTerm({ baseY: 0, viewportY: 0, type: "alternate" });
    resync(t.term);
    expect(t.calls).toEqual([]);
  });
  it("DOM 오프셋을 직접 만지지 않는다(term.element 를 읽지 않는다)", () => {
    const t = fakeTerm({ baseY: 945, viewportY: 500 });
    resync(t.term);
    expect(t.calls.includes("element")).toBe(false);
  });
});

describe("B-230 ① render() — 다시 붙인 뒤 보이는 pane 마다 resyncViewport(같은 태스크 · rAF 전)", () => {
  function world(sids: number[]) {
    const log: string[] = [];
    // termHost 가 있어야 '저장해 둔 DOM 오프셋' 같은 변형이 형식 오류가 아니라 아래 단언에서 떨어진다(뷰포트 없음 = null).
    const rts = new Map<string, { el: { isConnected: boolean; remove: () => void }; termHost: { querySelector: () => null }; term: { id: number } }>();
    for (const s of [...sids, 99]) {
      const el = { isConnected: true, remove() { el.isConnected = false; log.push(`remove(${s})`); } };
      rts.set(`k${s}`, { el, termHost: { querySelector: () => null }, term: { id: s } });
    }
    const tree = { type: "fake", sids };
    const scope: Record<string, unknown> = {
      panes: rts,
      root: {
        set innerHTML(_v: string) { log.push("root.clear"); },
        appendChild(node: { sids: number[] }) { for (const s of node.sids) rts.get(`k${s}`)!.el.isConnected = true; log.push(`root.append(${node.sids})`); },
      },
      current: () => ({ tree, socket: "sock" }),
      nodeShown: () => true,
      holeShownFor: () => new Set(),
      renderNode: (t: typeof tree) => ({ sids: t.sids }),
      renderDeptPending: () => { throw new Error("not expected"); },
      renderIdleWorkspace: () => { throw new Error("not expected"); },
      renderWsTabs: () => log.push("renderWsTabs"),
      collectSids: (t: typeof tree | null) => (t ? t.sids : []),
      paneKey: (sid: number) => `k${sid}`,
      resyncViewport: (term: { id: number }) => log.push(`resync(${term.id},connected=${rts.get(`k${term.id}`)!.el.isConnected})`),
      requestAnimationFrame: () => log.push("rAF-scheduled"),
      fitPane: () => log.push("fitPane"),
      saveLayout: () => log.push("saveLayout"),
    };
    return { log, scope };
  }
  it("보이는 pane 둘: 둘 다 다시 붙은 뒤 한 번씩, rAF 예약 전에", () => {
    const w = world([1, 3]);
    loadWith(fnText("render"), "render", w.scope)();
    const resyncs = w.log.filter((l) => l.startsWith("resync("));
    expect(resyncs).toEqual(["resync(1,connected=true)", "resync(3,connected=true)"]);
    const iAppend = w.log.findIndex((l) => l.startsWith("root.append"));
    const iFirst = w.log.indexOf(resyncs[0]);
    const iRaf = w.log.indexOf("rAF-scheduled");
    expect({ afterReinsert: iFirst > iAppend, beforeRaf: iFirst < iRaf }).toEqual({ afterReinsert: true, beforeRaf: true });
  });
  it("보이지 않는 pane(다른 탭 · 떼어진 채)은 건드리지 않는다", () => {
    const w = world([1]);
    loadWith(fnText("render"), "render", w.scope)();
    expect(w.log.some((l) => l.startsWith("resync(99"))).toBe(false);
  });
});

describe("B-230 ③ keepReadingLine — 폭 리플로우 뒤 읽던 논리 줄로(옛 절대 행 번호가 아니다)", () => {
  const mk = (follow: boolean, t: ReturnType<typeof fakeTerm>) => loadWith(closureText("keepReadingLine"), "keepReadingLine", { term: t.term, follow });
  it("follow=true(바닥 고정): 리플로우만 하고 마커·스크롤 없음 — 바닥은 snapToBottom 몫", () => {
    const t = fakeTerm({ baseY: 945, viewportY: 945 });
    let ran = 0;
    mk(true, t)(() => { ran++; t.term.cols = 73; });
    expect({ ran, calls: t.calls }).toEqual({ ran: 1, calls: [] });
  });
  it("follow=false · 폭 변경: 화면 맨 위 논리 줄의 첫 행에 마커 → 리플로우 뒤 그 마커 줄로", () => {
    // 맨 위 행 645 는 접힌 줄의 이어진 행(644·645 isWrapped) — 논리 줄은 643 에서 시작한다.
    const t = fakeTerm({ baseY: 945, viewportY: 645, cursorY: 10, wrapped: [644, 645] });
    mk(false, t)(() => {
      t.term.cols = 73;
      t.buf.baseY = 1945; // 줄이 두 행으로 접힌다
      t.markers[0].line = 1286; // xterm 이 마커를 옮긴다(리플로우 삽입 이벤트)
      // viewportY 는 xterm 처럼 옛 절대 행 645 에 남는다
    });
    expect(t.calls).toEqual([`registerMarker(${643 - (945 + 10)})`, "scrollToLine(1286)", "dispose(1286)"]);
    expect(t.buf.viewportY).toBe(1286);
  });
  it("follow=false · 폭 그대로(높이만): 스크롤하지 않고 마커만 거둔다", () => {
    const t = fakeTerm({ baseY: 945, viewportY: 645 });
    mk(false, t)(() => {});
    expect(t.calls.filter((c) => c.startsWith("scrollToLine"))).toEqual([]);
    expect(t.markers[0].isDisposed).toBe(true);
  });
  it("마커가 리플로우 중 잘려 폐기됐으면 손대지 않는다", () => {
    const t = fakeTerm({ baseY: 5000, viewportY: 10 });
    mk(false, t)(() => { t.term.cols = 73; t.markers[0].isDisposed = true; });
    expect(t.calls.filter((c) => c.startsWith("scrollToLine"))).toEqual([]);
  });
  it("리플로우가 던져도 마커는 거둔다", () => {
    const t = fakeTerm({ baseY: 945, viewportY: 645 });
    expect(() => mk(false, t)(() => { throw new Error("boom"); })).toThrow("boom");
    expect(t.markers[0].isDisposed).toBe(true);
  });
  it("대체 화면은 앱 몫 — 마커 없음", () => {
    const t = fakeTerm({ baseY: 0, viewportY: 0, type: "alternate" });
    let ran = 0;
    mk(false, t)(() => { ran++; t.term.cols = 73; });
    expect({ ran, calls: t.calls }).toEqual({ ran: 1, calls: [] });
  });
});

describe("B-230 ③ fitPane — fit 은 keepReadingLine 안에서만 돈다", () => {
  it("fit.fit() 이 keepReadingLine 에 넘긴 함수 안에서 불린다(그 밖의 맨 fit 호출 없음)", () => {
    const log: string[] = [];
    const rt = {
      termHost: { offsetWidth: 800, offsetHeight: 600 },
      keepReadingLine: (change: () => void) => { log.push("keep:start"); change(); log.push("keep:end"); },
      fit: { fit: () => log.push("fit") },
      snapToBottom: () => log.push("snap"),
      term: { rows: 56, cols: 149 },
      socket: "sock",
      sid: 1,
    };
    const fitPane = loadWith(fnText("fitPane"), "fitPane", { invoke: () => ({ catch: () => {} }) });
    fitPane(rt);
    expect(log).toEqual(["keep:start", "fit", "keep:end", "snap"]);
  });
});
