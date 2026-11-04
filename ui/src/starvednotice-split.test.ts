// 0.14.48 D — 「대기/막힘」 알림 표시: 기다리면 풀림 3종(wait·approval·paused)은 상태 제목·회색(idle)·구간당 1회 팝업, 사람 손 11종은 바이트 무변경.
//   근거: _evidence/dept5-impl-0.14.48-20261009/D/DESIGN-D-v3.md (§2 14종 표 · §3 구판 · §4 규칙 B 와 판정 조건 ①~⑥ · §5 시험).
//   · GOLDEN_BEFORE 는 v0.14.47(기준 태그)의 starvedNotice 를 실제로 돌려 얻은 출력을 그대로 박은 값이다(손으로 쓴 기대가 아니다).
//   · 「모르면 경보 유지」: 판정 불능(모르는 코드·문자열이 아닌 값·null·코드 없음+사유 불명)은 사람 손(health · 「큐 막힘」).
//   · 배선 실행 시험은 main.ts 분기 본문을 호출 기록 대역 위에서 실행한다 = 「분기 인자 실행 검증」이다(실제 stickyToast/pushAlarm/DOM/타이머 아님).
import { describe, it, expect } from "bun:test";
import { readFileSync } from "node:fs";
import {
  starvedNotice,
  starvedDismissId,
  starvedShouldPop,
  surfaceIdOfRef,
  STARVED_HUMAN_CODES,
  STARVED_CALM_CODES,
  STARVED_LEGACY_HUMAN_CODES,
} from "./starvednotice";
import * as SN from "./starvednotice";
// F1-R 의 새 export — 아직 없던 시점(빨강)에도 파일이 읽히도록 네임스페이스로 받는다.
const starvedKeyHasHead = (SN as any).starvedKeyHasHead as (key: unknown, entryId: unknown) => boolean;
const STARVED_REPOP_MS = (SN as any).STARVED_REPOP_MS as number;
const starvedPopAgeMs = (SN as any).starvedPopAgeMs as (nowWall: number, popWall: number, nowMono: number, popMono: number) => number;

const read = (rel: string) => readFileSync(new URL(rel, import.meta.url), "utf-8");
const TAIL = " · LLM 에이전트는 자동 조치(강제 배달·드레인·키 주입·동결 해제·항목 삭제) 금지";
const mk = (code: unknown, o: Record<string, unknown> = {}): Record<string, unknown> => ({
  surface_ref: "surface:12",
  role: "worker",
  waited_secs: 720,
  blocked_by: "gate:" + String(code),
  remedy_code: code,
  remedy: "처방-" + String(code) + TAIL,
  ...o,
});
/** v0.14.47 출력(수정 전) — 사람 손 12 코드(11종 + 옛 이름). */
const GOLDEN_BEFORE: Record<string, { id: string; title: string; detail: string; humanNeeded: boolean }> = {
  machine_residue: { id: "starved:abc:surface:12", title: "⏳ 큐 막힘 — worker surface:12", detail: "12분째 · 처방-machine_residue", humanNeeded: true },
  phantom_count: { id: "starved:abc:surface:12", title: "⏳ 큐 막힘 — worker surface:12", detail: "12분째 · 처방-phantom_count", humanNeeded: true },
  after_cursor_text: { id: "starved:abc:surface:12", title: "⏳ 큐 막힘 — worker surface:12", detail: "12분째 · 처방-after_cursor_text", humanNeeded: true },
  human_draft: { id: "starved:abc:surface:12", title: "⏳ 큐 막힘 — worker surface:12", detail: "12분째 · 처방-human_draft", humanNeeded: true },
  input_pending_unknown: { id: "starved:abc:surface:12", title: "⏳ 큐 막힘 — worker surface:12", detail: "12분째 · 처방-input_pending_unknown", humanNeeded: true },
  answer_modal: { id: "starved:abc:surface:12", title: "⏳ 큐 막힘 — worker surface:12", detail: "12분째 · 처방-answer_modal", humanNeeded: true },
  alt_screen: { id: "starved:abc:surface:12", title: "⏳ 큐 막힘 — worker surface:12", detail: "12분째 · 처방-alt_screen", humanNeeded: true },
  empty_seat: { id: "starved:abc:surface:12", title: "⏳ 큐 막힘 — worker surface:12", detail: "12분째 · 처방-empty_seat", humanNeeded: true },
  prompt_unknown: { id: "starved:abc:surface:12", title: "⏳ 큐 막힘 — worker surface:12", detail: "12분째 · 처방-prompt_unknown", humanNeeded: true },
  stale_screen: { id: "starved:abc:surface:12", title: "⏳ 큐 막힘 — worker surface:12", detail: "12분째 · 처방-stale_screen", humanNeeded: true },
  unknown: { id: "starved:abc:surface:12", title: "⏳ 큐 막힘 — worker surface:12", detail: "12분째 · 처방-unknown", humanNeeded: true },
  phantom_count_ctrl_u: { id: "starved:abc:surface:12", title: "⏳ 큐 막힘 — worker surface:12", detail: "12분째 · 처방-phantom_count_ctrl_u", humanNeeded: true },
};
/** v0.14.47 에서 calm 3종의 id·상세 — 상세(조치 문장)는 이번에도 바뀌지 않는다. */
const GOLDEN_CALM_DETAIL: Record<string, { id: string; detail: string }> = {
  wait: { id: "starved:abc:surface:12", detail: "12분째 · 처방-wait" },
  approval: { id: "starved:abc:surface:12", detail: "12분째 · 처방-approval" },
  paused: { id: "starved:abc:surface:12", detail: "12분째 · 처방-paused" },
};
const HUMAN_TITLE = "⏳ 큐 막힘 — worker surface:12";
const T_WAIT = "⏳ 배달 대기 중 — worker surface:12";
const T_APPROVAL = "⏳ 승인 대기 중 — worker surface:12";
const T_PAUSED = "⏳ 일시 정지 중 — worker surface:12";
const CALM_TITLE: Record<string, string> = { wait: T_WAIT, approval: T_APPROVAL, paused: T_PAUSED };
const MAKHIM = /막[힘힌혔혀히]/;

describe("0.14.48 D — 처방 code 14종 전수 표(+옛 이름 1)", () => {
  it("표의 범위 = 데몬 처방 14종: 사람 손 11 + 기다림 3", () => {
    expect([...STARVED_HUMAN_CODES].sort()).toEqual(Object.keys(GOLDEN_BEFORE).filter((k) => k !== "phantom_count_ctrl_u").sort());
    expect([...STARVED_CALM_CODES].sort()).toEqual(["approval", "paused", "wait"]);
    expect(STARVED_HUMAN_CODES.length + STARVED_CALM_CODES.length).toBe(14);
    expect([...STARVED_LEGACY_HUMAN_CODES]).toEqual(["phantom_count_ctrl_u"]);
  });
  for (const code of Object.keys(GOLDEN_BEFORE)) {
    it(`사람 손 ${code}: id·제목·상세·humanNeeded 는 v0.14.47 출력과 한 글자도 같고 등급은 health(빨강)`, () => {
      const n = starvedNotice(mk(code), "abc")!;
      expect({ id: n.id, title: n.title, detail: n.detail, humanNeeded: n.humanNeeded }).toEqual(GOLDEN_BEFORE[code]);
      expect(n.title).toBe(HUMAN_TITLE);
      expect(n.level).toBe("health");
    });
  }
  for (const code of ["wait", "approval", "paused"]) {
    it(`기다림 ${code}: 제목 「${CALM_TITLE[code].slice(0, 10)}…」 · idle(오류색 아님) · 상세·id 그대로 · humanNeeded=false`, () => {
      const n = starvedNotice(mk(code), "abc")!;
      expect(n.title).toBe(CALM_TITLE[code]);
      expect(n.level).toBe("idle");
      expect(n.humanNeeded).toBe(false);
      expect({ id: n.id, detail: n.detail }).toEqual(GOLDEN_CALM_DETAIL[code]);
    });
  }
});

describe("0.14.48 D — 구판 데몬(remedy_code 없음)은 기존 접두 판정 · 모르면 사람 손", () => {
  const legacy = (o: Record<string, unknown>): Record<string, unknown> => ({ surface_ref: "surface:12", role: "worker", waited_secs: 600, ...o });
  it("코드 없음 + 접두 9종: 승인→승인 대기 · paused→일시 정지 · 나머지 6→배달 대기 · queue_paused→공통(배달 대기)", () => {
    const table: [string, string][] = [
      ["approval_pending(승인·관문 대기)", T_APPROVAL],
      ["paused(kill-switch 동결)", T_PAUSED],
      ["busy(출력 중)", T_WAIT],
      ["busy", T_WAIT],
      ["delivery_interval(배달 최소 간격)", T_WAIT],
      ["settle_budget(이 틱의 인계 결판 예산 소진 — 다음 틱이 이 좌석부터 시작한다)", T_WAIT],
      ["quiescing(사이클 진행 중 · /clear~RESUME 창)", T_WAIT],
      ["prompt_not_ready(프롬프트 경계 미도달)", T_WAIT],
      ["human_typing(사람이 입력 중)", T_WAIT],
      ["queue_paused(헬스 조치)", T_WAIT],
    ];
    for (const [b, title] of table) {
      const n = starvedNotice(legacy({ blocked_by: b }), "abc")!;
      expect({ 사유: b, 제목: n.title, 등급: n.level, 사람: n.humanNeeded }).toEqual({ 사유: b, 제목: title, 등급: "idle", 사람: false });
      expect(n.detail).toBe(`10분째 · 대기 사유: ${b}`);
    }
  });
  it("코드 없음 + 사람 조치 사유·모르는 사유·사유 없음·문자열 아님: 「큐 막힘」 · health · 대체 문장은 「막힘 사유」(무변경)", () => {
    for (const b of ["input_pending(입력줄에 미제출 입력)", "modal_pending(모달·선택기 전경)", "empty_seat", "알 수 없음", "", undefined, null, 7, ["busy"], { x: 1 }]) {
      const n = starvedNotice(legacy({ blocked_by: b }), "abc")!;
      expect({ 사유: String(b), 제목: n.title, 등급: n.level, 사람: n.humanNeeded }).toEqual({ 사유: String(b), 제목: HUMAN_TITLE, 등급: "health", 사람: true });
      expect(n.detail.startsWith("10분째 · 막힘 사유: ")).toBe(true);
    }
  });
  it("코드가 있는데 모르는 값·문자열이 아닌 값·null: 사람 손(빨강) — calm 사유(busy)가 같이 와도 코드로만 가른다", () => {
    for (const c of ["wat", "WAIT", "wait ", "", null, 0, 7, false, true, ["wait"], { code: "wait" }]) {
      const n = starvedNotice(mk(c, { blocked_by: "busy" }), "abc")!;
      expect({ 코드: JSON.stringify(c), 제목: n.title, 등급: n.level, 사람: n.humanNeeded }).toEqual({ 코드: JSON.stringify(c), 제목: HUMAN_TITLE, 등급: "health", 사람: true });
    }
  });
  it("불변식: humanNeeded 와 (제목 머리, level) 이 항상 한 쌍 — 사람 손이면 「큐 막힘」·health, 아니면 「막힘」 없는 제목·idle", () => {
    const codes: unknown[] = [undefined, null, "", "x", 5, ...STARVED_HUMAN_CODES, ...STARVED_CALM_CODES, "phantom_count_ctrl_u"];
    const bys: unknown[] = [undefined, null, "", "busy", "paused", "approval_pending", "queue_paused", "input_pending", 9];
    for (const c of codes) for (const b of bys) {
      const p: Record<string, unknown> = { surface_ref: "surface:12", role: "worker", blocked_by: b };
      if (c !== undefined) p.remedy_code = c;
      const n = starvedNotice(p, "abc")!;
      const ok = n.humanNeeded ? n.title === HUMAN_TITLE && n.level === "health" : [T_WAIT, T_APPROVAL, T_PAUSED].includes(n.title) && n.level === "idle" && !MAKHIM.test(n.title);
      expect({ c: String(c), b: String(b), ok }).toEqual({ c: String(c), b: String(b), ok: true });
    }
  });
  it("화면에 올릴 수 없는 payload 는 여전히 null", () => {
    for (const bad of [null, undefined, "x", 7, [], {}, { surface_ref: "surface:abc" }]) expect(starvedNotice(bad)).toBeNull();
  });
});

describe("0.14.48 D — 「막힘」 낱말: 기다림 3종이 만드는 사람용 문자열에 0회(앱이 붙이는 문구 + 데몬 정상 calm 조치 문장)", () => {
  /** 데몬 governance.rs(v0.14.47) 의 기다림 3종 조치 문장 원문 6개 — 8479·8482·8553·8587·8627·8630 (소스를 읽는 핀은 아래 별도). */
  const DAEMON_CALM_SENTENCES = [
    "kill-switch 동결 중 — 정체가 아니라 동결이다. 해제는 오너(사람)가 한다. 동결 시간은 TTL 에서 빠지므로 해제하면 묵은 항목이 차례로 배달된다 — 오너가 해제 전에 `cys queue list` 로 묵은 항목을 확인한다",
    "이 좌석의 큐가 헬스 조치(pause-queue)로 일시정지됐다 — 정해진 시간이 지나면 스스로 풀린다(반복되면 자비스가 그 창의 출력을 읽어 원인을 찾는다)",
    "입력줄 계수는 이미 0 이다 — 다음 틱에 다시 판정된다(스스로 풀린다)",
    "출력 중 표지가 보이지만 75초째 출력이 없다 — 표지가 낡은 화면 사본일 수 있다(1분 넘게 이어지면 cys 가 키 입력 없이 창 크기를 한 칸 흔들어 다시 그리기를 요청한다 · 사람이 그 창 크기를 한 번 바꿔도 다시 그려진다). 그래도 이어지면 그 창 화면을 확인",
    "승인·관문 대기 — 승인 절차(feed)로 처리한다. 큐로 승인을 누르지 않는다",
    "일시 보류 — 스스로 풀린다(오래 지속되면 그 창 화면을 확인)",
  ];
  const KINDS: [string, string][] = [["paused", "paused"], ["paused", "paused"], ["wait", "wait"], ["wait", "wait"], ["approval", "approval"], ["wait", "wait"]];
  it("코드 3종 × 데몬 실제 조치 문장 6개(LLM 꼬리 유/무) — 제목·상세에 막힘 계열 0, 「큐 막힘」 부분 문자열 0", () => {
    DAEMON_CALM_SENTENCES.forEach((s, i) => {
      for (const tail of ["", TAIL]) {
        const n = starvedNotice({ surface_ref: "surface:12", role: "worker", waited_secs: 905, blocked_by: "x", remedy_code: KINDS[i][0], remedy: s + tail }, "abc")!;
        expect({ i, 막힘: MAKHIM.test(n.title + " " + n.detail), 큐막힘: (n.title + n.detail).includes("큐 막힘") }).toEqual({ i, 막힘: false, 큐막힘: false });
      }
    });
  });
  it("remedy 가 없거나 비어 대체 문장 경로로 가는 기다림 3종 — 대체 문장은 「대기 사유」", () => {
    for (const code of ["wait", "approval", "paused"]) for (const remedy of [undefined, "", "   "]) {
      const n = starvedNotice({ surface_ref: "surface:12", role: "worker", waited_secs: 700, blocked_by: "busy(출력 중)", remedy_code: code, remedy }, "abc")!;
      expect(n.detail).toBe("12분째 · 대기 사유: busy(출력 중)");
      expect(MAKHIM.test(n.title + n.detail)).toBe(false);
    }
  });
  it("데몬 소스의 일부 calm 문장 머리(12자)가 그대로 있고 kill-switch 상수 앞 600자에 「막힘」이 없다(소스를 읽기만 한다 — 어휘 핀 · 범위는 이 둘로 한정)", () => {
    const gov = read("../../src/bin/cysd/governance.rs");
    for (const lit of DAEMON_CALM_SENTENCES.slice(0, 3).concat(DAEMON_CALM_SENTENCES.slice(4, 6))) {
      const head = lit.split("(")[0].slice(0, 12);
      expect({ 문장머리: head, 소스에있음: gov.includes(head) }).toEqual({ 문장머리: head, 소스에있음: true });
    }
    expect(MAKHIM.test(gov.slice(gov.indexOf("const REMEDY_BODY_KILL_SWITCH"), gov.indexOf("const REMEDY_BODY_KILL_SWITCH") + 600))).toBe(false);
    expect(MAKHIM.test(gov.slice(gov.indexOf("const REMEDY_BODY_SEAT_PAUSE"), gov.indexOf("const REMEDY_BODY_SEAT_PAUSE") + 300))).toBe(false);
  });
  it("역방향: 사람 손 11종의 제목에는 「큐 막힘」이 그대로 있다", () => {
    for (const code of STARVED_HUMAN_CODES) expect(starvedNotice(mk(code), "abc")!.title).toContain("큐 막힘");
  });
});

describe("0.14.48 D — starvedShouldPop(구간당 1회) 순수 표", () => {
  const calm = (title: string) => ({ id: "starved:abc:surface:12", title, detail: "d", humanNeeded: false, level: "idle" as const, stateKey: title });
  const human = { id: "starved:abc:surface:12", title: HUMAN_TITLE, detail: "d", humanNeeded: true, level: "health" as const, stateKey: HUMAN_TITLE };
  it("사람 손: 직전 제목이 무엇이든(없음·같은 제목·calm 제목) 항상 true", () => {
    for (const last of [undefined, HUMAN_TITLE, T_WAIT, T_APPROVAL, T_PAUSED, "아무거나"]) expect(starvedShouldPop(last, human)).toBe(true);
  });
  it("기다림: 기록 없음·다른 제목 → true, 같은 제목 → false", () => {
    expect(starvedShouldPop(undefined, calm(T_WAIT))).toBe(true);
    expect(starvedShouldPop(T_WAIT, calm(T_WAIT))).toBe(false);
    expect(starvedShouldPop(T_WAIT, calm(T_APPROVAL))).toBe(true);
    expect(starvedShouldPop(T_APPROVAL, calm(T_PAUSED))).toBe(true);
    expect(starvedShouldPop(HUMAN_TITLE, calm(T_WAIT))).toBe(true); // 사람 손 → 기다림 = 회색 1회
  });
});

describe("S5 CHANGE-1 · CHANGE-2 — main.ts 배선 (s5_queue_starved_renders_nothing · s5_no_starved_ui_state_remains)", () => {
  const src = read("./main.ts");
  const strip = (s: string): string => s.split("\n").map((l) => (l.trimStart().startsWith("//") ? "" : l.replace(/\s\/\/.*$/, ""))).join("\n");
  const code = strip(src);
  const branch = (() => {
    const i = code.indexOf('  if (name === "queue.starved") {');
    expect(i >= 0).toBe(true);
    return code.slice(i, code.indexOf("\n  }\n", i) + 5);
  })();

  type Call = { fn: string; args: unknown[] };
  function scenario() {
    const calls: Call[] = [];
    const rec = (fn: string) => (...args: unknown[]) => void calls.push({ fn, args });
    const deps = {
      stickyToast: rec("stickyToast"),
      osBanner: rec("osBanner"),
      recordAlarm: rec("recordAlarm"),
      toast: rec("toast"),
      focusStarvedSeat: rec("focusStarvedSeat"),
      dismissToast: rec("dismissToast"),
    };
    const fireFn = new Function("deps", "name", "event", "payload", "sid", `with (deps) {\n${branch}\n}\nreturn "fell-through";`) as (...a: unknown[]) => unknown;
    return {
      calls,
      fire: (p: unknown, slug = "abc", sid = 12) => fireFn(deps, "queue.starved", { name: "queue.starved", socket_slug: slug, surface_id: sid }, p, sid),
      count: (fn: string) => calls.filter((c) => c.fn === fn).length,
    };
  }

  it("s5_no_starved_ui_state_remains: CHANGE-2 식별자 0건 · module import 부재", () => {
    // CHANGE-2 식별자 목록 (08-removal-ripple.md O1-O3)
    const forbidden = [
      'from "./starvednotice"',
      "starvedLastKey",
      "starvedLastPopAt",
      "dismissStarvedToast",
      "forgetStarvedHead",
      "focusStarvedSeat",
      "starvedNotice",
      "starvedShouldPop",
      "starvedKeyHasHead",
      "starvedPopAgeMs",
      "starvedDismissId",
      "surfaceIdOfRef",
      "locateStarvedSeat",
    ];
    for (const id of forbidden) {
      expect({ id, exists: code.includes(id) }).toEqual({ id, exists: false });
    }
  });

  it("소스 핀: queue.starved 분기에서 stickyToast, osBanner, recordAlarm 0건 · return 으로 끝남", () => {
    expect(branch).not.toContain("stickyToast");
    expect(branch).not.toContain("osBanner");
    expect(branch).not.toContain("recordAlarm");
    expect(branch).not.toContain("toast");
    expect(/\n    return;\n  \}\n$/.test(branch)).toBe(true);
  });

  it("s5_queue_starved_renders_nothing: 사람 손 11종 payload 에서 렌더링 0건 · return", () => {
    for (const code of STARVED_HUMAN_CODES) {
      const s = scenario();
      const ret = s.fire(mk(code, { waited_secs: 1200 }));
      expect({ code, ret, calls: s.calls.length }).toEqual({ code, ret: undefined, calls: 0 });
    }
  });

  it("s5_queue_starved_renders_nothing: calm 3종 payload 에서 렌더링 0건 · return", () => {
    for (const code of STARVED_CALM_CODES) {
      const s = scenario();
      const ret = s.fire(mk(code, { waited_secs: 1800 }));
      expect({ code, ret, calls: s.calls.length }).toEqual({ code, ret: undefined, calls: 0 });
    }
  });

  it("s5_queue_starved_renders_nothing: 반복 발화 5회에서도 렌더링 0건 · return", () => {
    const s = scenario();
    for (let k = 0; k < 5; k++) {
      const ret = s.fire(mk("wait", { waited_secs: 600 + 300 * k }));
      expect(ret).toBeUndefined();
    }
    expect(s.calls.length).toBe(0);
  });

  it("s5_queue_starved_renders_nothing: CEO 실제 표본(12회 발화 modal_pending)에서도 렌더링 0건", () => {
    const s = scenario();
    const secs = Array.from({ length: 11 }, (_, k) => 600 + 300 * k).concat([3921]);
    for (const w of secs) {
      const ret = s.fire({ surface_ref: "surface:72", role: "worker", head_entry_id: "q-72", waited_secs: w, depth: 2, blocked_by: "modal_pending(모달·선택기 전경)", remedy_code: "answer_modal", remedy: "질문·선택 창이 떠 있다 — 사람이 답한다" + TAIL });
      expect(ret).toBeUndefined();
    }
    expect(s.calls.length).toBe(0);
  });

  it("s5_queue_starved_renders_nothing: 잘못된 payload 에서도 조기 return", () => {
    for (const bad of [null, undefined, "x", 7, [], {}, { surface_ref: "surface:abc" }]) {
      const s = scenario();
      const ret = s.fire(bad);
      expect(ret).toBeUndefined();
      expect(s.calls.length).toBe(0);
    }
  });
});

describe("0.14.48 D — 등급은 기존 토스트 등급 중 하나", () => {
  const css = read("./style.css");
  it("idle·health 규칙이 style.css 에 이미 있다 — 새 등급·새 색을 만들지 않았다", () => {
    expect(css).toContain(".toast.idle { border-color: var(--border); }");
    expect(css).toContain(".toast.health { border-color: var(--error); }");
  });
  it("stickyToast 는 낼 때마다 등급 클래스를 다시 못박는다(전이 시 같은 요소의 빨강→회색) — 소스 핀", () => {
    expect(read("./main.ts")).toContain("el.className = toastClassName(category);");
  });
});

// ===================================================================================================
// ★S5 O-3 / m3 — WIRING, THE SPLIT FILE'S HALF. The pair is rewritten TOGETHER on purpose: landing this on
// one file and claiming the pair is the failure shape this lane has ruled against. Same facts as the sibling
// file, asserted through this file's own `mk(code)` builder and its 14-code table, so the two files agree
// about the incident identity instead of each believing its own version of it.
// ===================================================================================================
describe("★S5 m3 — 사건 하나 = 안정된 키 하나(코드 플랩과 무관) · code 가 분류하고 raw 사유는 진단이다", () => {
  const FLIPS = ["human_draft", "machine_residue", "phantom_count", "after_cursor_text", "input_pending_unknown"];

  it("코드가 바뀌어도 id 는 하나다 — 이 파일의 14종 표 전체에서도 좌석 키는 하나다", () => {
    const ids = FLIPS.map((c) => SN.starvedNotice(mk(c), "abc")!.id);
    expect(new Set(ids).size).toBe(1);
    expect(ids[0]).toBe("starved:abc:surface:12");
    // 그리고 14종 전체: 사람 코드든 calm 이든 같은 좌석이면 같은 id 다(없는 알림은 제외)
    const all = SN.STARVED_HUMAN_CODES.concat(SN.STARVED_CALM_CODES)
      .map((c) => SN.starvedNotice(mk(c), "abc"))
      .filter((n): n is NonNullable<typeof n> => n !== null)
      .map((n) => n.id);
    expect(new Set(all).size).toBe(1);
  });

  it("★code 가 사람/calm 을 가른다 — 이 파일의 mk() 는 사유를 'gate:<code>' 로 만들지만 판정은 code 가 한다", () => {
    const human = SN.starvedNotice(mk("answer_modal", { blocked_by: "busy(화면 정적)" }), "abc")!;
    const calm = SN.starvedNotice(mk("wait", { blocked_by: "input_pending(계수 0)" }), "abc")!;
    expect({ human: human.humanNeeded, calm: calm.humanNeeded }).toEqual({ human: true, calm: false });
  });

  it("★calm 3종은 오래 기다려도 사람 알림이 되지 않는다 — 멈춤 판정은 상류의 정렬된 선택자가 code 로 한다", () => {
    for (const c of SN.STARVED_CALM_CODES) {
      const n = SN.starvedNotice(mk(c, { waited_secs: 7200, blocked_by: "busy(멈춘 듯)" }), "abc")!;
      expect({ c, human: n.humanNeeded }).toEqual({ c, human: false });
    }
  });

  it("사람 코드 11종은 전부 사람 손이다 — 상류의 ELIGIBLE 집합과 같은 11종이어야 한다", () => {
    expect(SN.STARVED_HUMAN_CODES.length).toBe(11);
    for (const c of SN.STARVED_HUMAN_CODES) {
      expect({ c, human: SN.starvedNotice(mk(c), "abc")!.humanNeeded }).toEqual({ c, human: true });
    }
  });
});
