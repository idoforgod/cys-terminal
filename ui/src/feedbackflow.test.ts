// U6(0.14.41 · 리뷰1 수정 라운드) 피드백 작성 창 **수명 동작**의 행동 검체 — 가짜 객체로 실제 함수를 돌린다
// (DOM·Tauri 불요). 창(feedbackmodal.ts)이 이 함수들을 실제로 쓰는지는 feedbackwiring.test.ts ④ 가 소스로 대조한다.
//
// 리뷰1 변이 M5(focusin 되찾기 끄기)·M7(버리기 때 초안 삭제 빼기)·M11(늦게 온 구독 미해제)·
// M13(폴더↔메일 순서 뒤집기)·M14(IME 조합 중 Esc 가드 빼기)가 저장소 안 테스트로는 GREEN 이었다.
// 여기 실패는 "창의 행동이 약속과 다르다"는 뜻이다.
import { describe, it, expect } from "bun:test";
import {
  discardDraftAfter,
  makeEscHandler,
  makeFocusReclaimer,
  openBundleForMail,
  scopedListener,
} from "./feedbackflow";
import { MODAL_LAYER_SELECTOR } from "./modalguard";

const flush = () => new Promise((r) => setTimeout(r, 0));

/** preventDefault·stopPropagation 호출을 기록하는 가짜 키 이벤트. */
function key(k: string, extra: { isComposing?: boolean; keyCode?: number } = {}) {
  const ev = {
    key: k,
    isComposing: extra.isComposing ?? false,
    keyCode: extra.keyCode ?? 0,
    prevented: 0,
    stopped: 0,
    preventDefault() {
      ev.prevented++;
    },
    stopPropagation() {
      ev.stopped++;
    },
  };
  return ev;
}

describe("Esc 처리 (M14 IME 가드 · 리뷰 minor #4 위에 뜬 모달)", () => {
  it("맨 위 층이면 Esc = 닫기 요청 + 전파 차단(pane 의 vim·claude 로 가지 않게)", () => {
    let closes = 0;
    const h = makeEscHandler(() => true, () => closes++);
    const e = key("Escape");
    h(e);
    expect(closes).toBe(1);
    expect(e.prevented).toBe(1);
    expect(e.stopped).toBe(1);
  });
  it("IME 조합 중 Esc(isComposing)는 조합 취소다 — 창을 닫지도, 이벤트를 삼키지도 않는다", () => {
    let closes = 0;
    const h = makeEscHandler(() => true, () => closes++);
    const e = key("Escape", { isComposing: true });
    h(e);
    expect(closes).toBe(0);
    expect(e.prevented + e.stopped).toBe(0);
  });
  it("IME 처리 중 keyCode 229 도 같은 취급(구형 WebKit 은 isComposing 을 안 줄 때가 있다)", () => {
    let closes = 0;
    const h = makeEscHandler(() => true, () => closes++);
    const e = key("Escape", { keyCode: 229 });
    h(e);
    expect(closes).toBe(0);
    expect(e.prevented + e.stopped).toBe(0);
  });
  it("다른 키는 건드리지 않는다", () => {
    let closes = 0;
    const h = makeEscHandler(() => true, () => closes++);
    for (const k of ["Enter", "a", "Tab", "ArrowDown"]) {
      const e = key(k);
      h(e);
      expect(e.prevented + e.stopped).toBe(0);
    }
    expect(closes).toBe(0);
  });
  it("이 창 위에 다른 모달이 떠 있으면 Esc 는 그 창의 몫 — 가로채지 않는다(전파도 막지 않는다)", () => {
    let closes = 0;
    const h = makeEscHandler(() => false, () => closes++);
    const e = key("Escape");
    h(e);
    expect(closes).toBe(0);
    expect(e.prevented + e.stopped).toBe(0);
  });
});

describe("focusin 되찾기 (M5 · 반박 D2 이중 방어)", () => {
  const outside = { closest: (_: string): unknown => null }; // xterm 숨은 textarea 등
  const inside = { closest: (sel: string): unknown => (sel === MODAL_LAYER_SELECTOR ? {} : null) };
  const spot = () => {
    const s = { n: 0, focus: () => s.n++ };
    return s;
  };
  it("포커스가 모달 층 밖으로 나가면 창의 기본 칸으로 즉시 되찾는다", () => {
    const desc = spot();
    const h = makeFocusReclaimer({ closed: () => false, home: () => desc });
    h({ target: outside });
    expect(desc.n).toBe(1);
  });
  it("묶음을 만든 뒤에는 기본 칸이 [닫기] 단추다(home 은 매번 다시 묻는다)", () => {
    const desc = spot();
    const close = spot();
    let done = false;
    const h = makeFocusReclaimer({ closed: () => false, home: () => (done ? close : desc) });
    h({ target: outside });
    done = true;
    h({ target: outside });
    expect([desc.n, close.n]).toEqual([1, 1]);
  });
  it("모달 층 안(창 자신·위에 뜬 확인 창)의 포커스 이동은 건드리지 않는다", () => {
    const desc = spot();
    const h = makeFocusReclaimer({ closed: () => false, home: () => desc });
    h({ target: inside });
    h({ target: null });
    expect(desc.n).toBe(0);
  });
  it("창을 닫은 뒤에는 되찾지 않는다(pane 으로 돌아가는 포커스와 다투지 않게)", () => {
    const desc = spot();
    const h = makeFocusReclaimer({ closed: () => true, home: () => desc });
    h({ target: outside });
    expect(desc.n).toBe(0);
  });
});

describe("창 수명 구독 (M11 · 반박 D13)", () => {
  type Un = () => void;
  /** listen 을 손으로 풀어 주는 가짜 — 늦게 풀리는 구독을 재현한다. */
  function fakeListen() {
    const pending: Array<{ name: string; resolve: (u: Un) => void; reject: (e: unknown) => void }> = [];
    let unCalls = 0;
    const listen = (name: string, _h: (e: { payload: unknown }) => void): Promise<Un> =>
      new Promise<Un>((resolve, reject) => pending.push({ name, resolve, reject }));
    const un = (): Un => () => {
      unCalls++;
    };
    return { listen, pending, un, unCalls: () => unCalls };
  }
  it("정상: 풀린 구독은 dispose 때 전부 해제된다", async () => {
    const f = fakeListen();
    const s = scopedListener(f.listen);
    s.sub("tauri://drag-enter", () => {});
    s.sub("tauri://drag-drop", () => {});
    for (const p of f.pending) p.resolve(f.un());
    await flush();
    expect(f.unCalls()).toBe(0);
    s.dispose();
    expect(f.unCalls()).toBe(2);
    s.dispose(); // 두 번 불러도 중복 해제 없음
    expect(f.unCalls()).toBe(2);
  });
  it("창이 닫힌 뒤에 늦게 풀린 구독은 즉시 해제된다(남으면 닫힌 창의 드롭 핸들러가 계속 산다)", async () => {
    const f = fakeListen();
    const s = scopedListener(f.listen);
    s.sub("tauri://drag-drop", () => {});
    s.dispose();
    f.pending[0].resolve(f.un());
    await flush();
    expect(f.unCalls()).toBe(1);
  });
  it("해제 함수가 던져도 나머지 해제와 창 닫기는 계속된다", async () => {
    const f = fakeListen();
    const s = scopedListener(f.listen);
    s.sub("a", () => {});
    s.sub("b", () => {});
    f.pending[0].resolve(() => {
      throw new Error("already");
    });
    f.pending[1].resolve(f.un());
    await flush();
    expect(() => s.dispose()).not.toThrow();
    expect(f.unCalls()).toBe(1);
  });
  it("구독 실패는 조용히 두지 않는다 — 실패한 이름으로 알린다", async () => {
    const f = fakeListen();
    const failed: string[] = [];
    const s = scopedListener(f.listen);
    s.sub("tauri://drag-drop", () => {}, (n) => failed.push(n));
    f.pending[0].reject(new Error("denied"));
    await flush();
    expect(failed).toEqual(["tauri://drag-drop"]);
  });
  it("listen 이 동기로 던져도 창을 죽이지 않고 실패로 알린다", () => {
    const failed: string[] = [];
    const s = scopedListener(() => {
      throw new Error("sync");
    });
    expect(() => s.sub("x", () => {}, (n) => failed.push(n))).not.toThrow();
    expect(failed).toEqual(["x"]);
  });
});

describe("버리기 = 초안 삭제 (M7)", () => {
  function rec() {
    const calls: Array<[string, unknown]> = [];
    const invoke = async (cmd: string, args?: Record<string, unknown>): Promise<unknown> => {
      calls.push([cmd, args]);
      return null;
    };
    return { calls, invoke };
  }
  it("묶음을 만들기 전에 닫으면 초안 폴더를 지운다 — 줄 선 첨부 작업이 **끝난 뒤에**", async () => {
    const r = rec();
    const order: string[] = [];
    let release: () => void = () => {};
    const job = new Promise<void>((res) => {
      release = res;
    }).then(() => {
      order.push("attach-done");
    });
    const chain = discardDraftAfter(job, { draft: Promise.resolve("fb-20260923-113456-ab12"), bundled: false }, async (c, a) => {
      order.push(c);
      return r.invoke(c, a);
    });
    await flush();
    expect(r.calls.length).toBe(0); // 첨부가 아직 진행 중
    release();
    await chain;
    expect(order).toEqual(["attach-done", "feedback_discard"]);
    expect(r.calls).toEqual([["feedback_discard", { id: "fb-20260923-113456-ab12" }]]);
  });
  it("묶음을 만든 뒤(완성본)에는 지우지 않는다 — 사용자가 아직 메일에 첨부하지 않았을 수 있다", async () => {
    const r = rec();
    await discardDraftAfter(Promise.resolve(), { draft: Promise.resolve("fb-20260923-113456-ab12"), bundled: true }, r.invoke);
    expect(r.calls.length).toBe(0);
  });
  it("초안을 만든 적이 없으면 아무것도 부르지 않는다", async () => {
    const r = rec();
    await discardDraftAfter(Promise.resolve(), { draft: null, bundled: false }, r.invoke);
    expect(r.calls.length).toBe(0);
  });
  it("초안 만들기가 실패했거나 삭제가 실패해도 줄은 깨지지 않는다(다음 작업이 이어진다)", async () => {
    const r = rec();
    const failedDraft = Promise.reject(new Error("no draft"));
    failedDraft.catch(() => {});
    await expect(discardDraftAfter(Promise.resolve(), { draft: failedDraft, bundled: false }, r.invoke)).resolves.toBeUndefined();
    const boom = async (): Promise<unknown> => {
      throw new Error("busy");
    };
    await expect(
      discardDraftAfter(Promise.resolve(), { draft: Promise.resolve("fb-20260923-113456-ab12"), bundled: false }, boom),
    ).resolves.toBeUndefined();
  });
});

describe("보내기 뒤 여는 순서 (M13)", () => {
  function rec(fail: Record<string, string> = {}) {
    const calls: Array<[string, unknown]> = [];
    const tryInvoke = async (cmd: string, args: Record<string, unknown>): Promise<string | null> => {
      calls.push([cmd, args]);
      return fail[cmd] ?? null;
    };
    return { calls, tryInvoke };
  }
  const rep = (attachments: number, include_diag: boolean) => ({
    id: "fb-20260923-113456-ab12",
    attachments: Array.from({ length: attachments }, () => ({})),
    include_diag,
  });
  it("첨부가 있으면 폴더를 먼저, 메일 창을 나중에 연다 — 나중에 뜬 메일 창이 앞에 온다", async () => {
    const r = rec();
    const out = await openBundleForMail(rep(2, false), r.tryInvoke);
    expect(r.calls.map((c) => c[0])).toEqual(["feedback_reveal", "feedback_open_mail"]);
    expect(r.calls[0][1]).toEqual({ id: "fb-20260923-113456-ab12" });
    expect(out).toEqual({ folderErr: null, mailErr: null });
  });
  it("진단 파일만 있어도 폴더를 연다(끌어다 넣을 파일이 있다)", async () => {
    const r = rec();
    await openBundleForMail(rep(0, true), r.tryInvoke);
    expect(r.calls.map((c) => c[0])).toEqual(["feedback_reveal", "feedback_open_mail"]);
  });
  it("넣을 파일이 없으면 폴더는 열지 않고 메일만 연다", async () => {
    const r = rec();
    const out = await openBundleForMail(rep(0, false), r.tryInvoke);
    expect(r.calls.map((c) => c[0])).toEqual(["feedback_open_mail"]);
    expect(out.folderErr).toBeNull();
  });
  it("폴더 열기가 실패해도 메일은 연다 — 실패 사유는 각각 돌려준다", async () => {
    const r = rec({ feedback_reveal: "no finder", feedback_open_mail: "no mail app" });
    const out = await openBundleForMail(rep(1, false), r.tryInvoke);
    expect(r.calls.map((c) => c[0])).toEqual(["feedback_reveal", "feedback_open_mail"]);
    expect(out).toEqual({ folderErr: "no finder", mailErr: "no mail app" });
  });
});
