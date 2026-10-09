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
  surfaceIdOfRef,
  STARVED_HUMAN_CODES,
  STARVED_CALM_CODES,
  STARVED_LEGACY_HUMAN_CODES,
} from "./starvednotice";
import * as SN from "./starvednotice";
// (수정 전 빨강 증거용) 아직 없는 export 를 import 문법 오류로 두면 파일 전체가 한 덩어리 오류가 되어 시험별 빨강이 안 보인다 — 구현 뒤 named import 로 바꾼다.
const starvedShouldPop = (SN as unknown as Record<string, (last: string | undefined, n: unknown) => boolean>).starvedShouldPop;

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
    "이 좌석의 큐가 헬스 조치(pause-queue)로 일시정지됐다 — 정해진 시간이 지나면 스스로 풀린다(반복되면 그 창의 출력을 사람이 확인)",
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
  it("데몬 소스가 기다림 3종 조치 문장 상수에 「막힘」을 넣으면 이 시험이 빨개진다(소스를 읽기만 한다 — 어휘 핀)", () => {
    const gov = readFileSync("/Users/cys/Desktop/CYSjavis/_worktrees/impl-0.14.48-D/src/bin/cysd/governance.rs", "utf-8");
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
  const calm = (title: string) => ({ id: "starved:abc:surface:12", title, detail: "d", humanNeeded: false, level: "idle" as const });
  const human = { id: "starved:abc:surface:12", title: HUMAN_TITLE, detail: "d", humanNeeded: true, level: "health" as const };
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

describe("0.14.48 D — main.ts 배선: 분기 인자 실행 검증(판정 조건 ①~⑥)", () => {
  const src = read("./main.ts");
  const strip = (s: string): string => s.split("\n").map((l) => (l.trimStart().startsWith("//") ? "" : l.replace(/\s\/\/.*$/, ""))).join("\n");
  const code = strip(src);
  const branch = (() => {
    const i = code.indexOf('  if (name === "queue.starved") {');
    expect(i >= 0).toBe(true);
    return code.slice(i, code.indexOf("\n  }\n", i) + 5);
  })();
  const dismissFn = (() => {
    const i = code.indexOf("function dismissStarvedToast(");
    expect(i >= 0).toBe(true);
    return code.slice(i, code.indexOf("\n}\n", i) + 3);
  })();
  type Call = { fn: string; args: unknown[] };
  /** 한 시나리오 = 하나의 맵·호출 기록. fire = queue.starved 분기 실행, delivered = queue.delivered/좌석 종료가 부르는 dismissStarvedToast 실행. */
  function scenario() {
    const calls: Call[] = [];
    const rec = (fn: string) => (...args: unknown[]) => void calls.push({ fn, args });
    const starvedLastTitle = new Map<string, string>();
    const deps = {
      starvedNotice, surfaceIdOfRef, starvedShouldPop, starvedLastTitle, starvedDismissId,
      stickyToast: rec("stickyToast"), osBanner: rec("osBanner"), recordAlarm: rec("recordAlarm"), focusStarvedSeat: rec("focusStarvedSeat"), dismissToast: rec("dismissToast"),
    };
    const fireFn = new Function("deps", "name", "event", "payload", "sid", `with (deps) {\n${branch}\n}\nreturn "fell-through";`) as (...a: unknown[]) => unknown;
    const dismissRun = new Function("deps", "slug", "sid", `with (deps) {\n${dismissFn}\n dismissStarvedToast(slug, sid);\n}`) as (...a: unknown[]) => unknown;
    return {
      calls, starvedLastTitle,
      fire: (p: unknown, slug = "abc") => { expect(fireFn(deps, "queue.starved", { name: "queue.starved", socket_slug: slug, surface_id: 12 }, p, 12)).toBeUndefined(); },
      delivered: (slug = "abc", sid = 12) => dismissRun(deps, slug, sid),
      count: (fn: string) => calls.filter((c) => c.fn === fn).length,
      last: (fn: string) => calls.filter((c) => c.fn === fn).slice(-1)[0],
    };
  }

  it("소스 핀: 분기에 「health」 리터럴 0 · 배너 줄은 원문 그대로 · 마지막 제목 맵 사용 · return 으로 끝남", () => {
    expect(branch).not.toContain('"health"');
    expect(branch).toContain("if (starved.humanNeeded) osBanner(starved.title, starved.detail);");
    expect(branch).toContain("starvedLastTitle");
    expect(branch).toContain("starvedShouldPop(");
    expect(branch).toContain("recordAlarm(");
    expect(/\n    return;\n  \}\n$/.test(branch)).toBe(true);
    expect(dismissFn).toContain("starvedLastTitle.delete(");
  });
  it("③ 같은 기다림 2회: 팝업 1회 + 이력 직접 기록 1회, 구간 끝(delivered) 뒤 다시 1회 팝업", () => {
    const s = scenario();
    s.fire(mk("wait")); s.fire(mk("wait", { waited_secs: 1020 }));
    expect({ 팝업: s.count("stickyToast"), 이력직접: s.count("recordAlarm"), 배너: s.count("osBanner") }).toEqual({ 팝업: 1, 이력직접: 1, 배너: 0 });
    s.delivered();
    s.fire(mk("wait", { waited_secs: 1320 }));
    expect(s.count("stickyToast")).toBe(2);
    expect(s.last("stickyToast")!.args.slice(1, 3)).toEqual(["idle", T_WAIT]);
  });
  it("④ 알람 이력은 발화마다: 기다림 5회 = (팝업 1회 안의 기록) + 직접 기록 4회 — 조용해지는 것은 팝업뿐", () => {
    const s = scenario();
    for (let k = 0; k < 5; k++) s.fire(mk("wait", { waited_secs: 600 + 300 * k }));
    expect({ 팝업: s.count("stickyToast"), 직접기록: s.count("recordAlarm") }).toEqual({ 팝업: 1, 직접기록: 4 });
    const lastRec = s.last("recordAlarm")!.args;
    expect(lastRec[0]).toBe("idle");
    expect(lastRec[1]).toBe(T_WAIT);
    expect(String(lastRec[2])).toContain("분째");
    expect(lastRec[3]).toBe("starved:abc:surface:12");
  });
  it("① 기다림 → 사람 손(stale_screen 승격 포함): 그 즉시 빨강 팝업 + OS 배너 — 억제가 삼키지 않는다", () => {
    for (const humanCode of [...STARVED_HUMAN_CODES]) {
      const s = scenario();
      s.fire(mk("wait")); s.fire(mk("wait", { waited_secs: 900 }));
      const before = s.count("stickyToast");
      s.fire(mk(humanCode, { waited_secs: 1200 }));
      expect({ 코드: humanCode, 팝업증가: s.count("stickyToast") - before, 배너: s.count("osBanner") }).toEqual({ 코드: humanCode, 팝업증가: 1, 배너: 1 });
      expect(s.last("stickyToast")!.args.slice(1, 3)).toEqual(["health", HUMAN_TITLE]);
    }
    const s = scenario(); // stale_screen 승격 모양(busy 정적) — 코드 stale_screen
    s.fire(mk("wait")); s.fire(mk("stale_screen", { blocked_by: "busy(출력 중)" }));
    expect(s.last("stickyToast")!.args.slice(1, 3)).toEqual(["health", HUMAN_TITLE]);
    expect(s.count("osBanner")).toBe(1);
  });
  it("② 사람 손 → 기다림: 같은 id 로 회색 1회(빨강을 대체) · 배너 추가 0", () => {
    const s = scenario();
    s.fire(mk("answer_modal"));
    expect({ 팝업: s.count("stickyToast"), 배너: s.count("osBanner") }).toEqual({ 팝업: 1, 배너: 1 });
    s.fire(mk("wait", { waited_secs: 900 }));
    expect({ 팝업: s.count("stickyToast"), 배너: s.count("osBanner") }).toEqual({ 팝업: 2, 배너: 1 });
    const a = s.last("stickyToast")!.args;
    expect(a[0]).toBe("starved:abc:surface:12");
    expect(a.slice(1, 3)).toEqual(["idle", T_WAIT]);
    s.fire(mk("wait", { waited_secs: 1200 })); // 이어서 같은 기다림은 다시 억제
    expect(s.count("stickyToast")).toBe(2);
  });
  it("기다림 종류가 바뀌면(wait → approval → paused) 제목이 달라 매번 1회 팝업", () => {
    const s = scenario();
    s.fire(mk("wait")); s.fire(mk("approval", { waited_secs: 900 })); s.fire(mk("paused", { waited_secs: 1200 })); s.fire(mk("paused", { waited_secs: 1500 }));
    expect(s.calls.filter((c) => c.fn === "stickyToast").map((c) => c.args[2])).toEqual([T_WAIT, T_APPROVAL, T_PAUSED]);
    expect(s.count("recordAlarm")).toBe(1);
  });
  it("⑤ 사람 손 11종 × 연속 3회: 매번 빨강 팝업 + 배너(5분 재팝업 무변경)", () => {
    for (const humanCode of STARVED_HUMAN_CODES) {
      const s = scenario();
      for (let k = 0; k < 3; k++) s.fire(mk(humanCode, { waited_secs: 600 + 300 * k }));
      expect({ 코드: humanCode, 팝업: s.count("stickyToast"), 배너: s.count("osBanner"), 직접이력: s.count("recordAlarm") }).toEqual({ 코드: humanCode, 팝업: 3, 배너: 3, 직접이력: 0 });
      expect(s.calls.filter((c) => c.fn === "stickyToast").every((c) => c.args[1] === "health" && c.args[2] === HUMAN_TITLE)).toBe(true);
    }
  });
  it("⑥ CEO 실제 표본 — surface:72 · modal_pending · 3,921초(answer_modal): 12회 발화 전부 빨강 팝업 + 배너, 수정 전과 같은 제목", () => {
    const s = scenario();
    const secs = Array.from({ length: 11 }, (_, k) => 600 + 300 * k).concat([3921]); // 600…3600(11회) + 3921 = 12회
    expect(secs.length).toBe(12);
    for (const w of secs) {
      s.fire({ surface_ref: "surface:72", role: "worker", head_entry_id: "q-72", waited_secs: w, depth: 2, blocked_by: "modal_pending(모달·선택기 전경)", remedy_code: "answer_modal", remedy: "질문·선택 창이 떠 있다 — 사람이 답한다" + TAIL });
    }
    expect({ 팝업: s.count("stickyToast"), 배너: s.count("osBanner"), 직접이력: s.count("recordAlarm") }).toEqual({ 팝업: 12, 배너: 12, 직접이력: 0 });
    for (const c of s.calls.filter((c) => c.fn === "stickyToast")) {
      expect(c.args[0]).toBe("starved:abc:surface:72");
      expect(c.args[1]).toBe("health");
      expect(c.args[2]).toBe("⏳ 큐 막힘 — worker surface:72");
    }
    expect(s.last("stickyToast")!.args[3]).toBe("66분째 · 질문·선택 창이 떠 있다 — 사람이 답한다"); // v0.14.47 실출력
  });
  it("두 데몬(slug)의 같은 번호 좌석은 구간이 독립이다 — 한쪽 억제가 다른 쪽 첫 팝업을 막지 않는다", () => {
    const s = scenario();
    s.fire(mk("wait"), "abc"); s.fire(mk("wait"), "xyz");
    expect(s.count("stickyToast")).toBe(2);
    s.fire(mk("wait", { waited_secs: 900 }), "abc"); s.fire(mk("wait", { waited_secs: 900 }), "xyz");
    expect({ 팝업: s.count("stickyToast"), 직접이력: s.count("recordAlarm") }).toEqual({ 팝업: 2, 직접이력: 2 });
    s.delivered("abc");
    s.fire(mk("wait", { waited_secs: 1200 }), "abc"); s.fire(mk("wait", { waited_secs: 1200 }), "xyz");
    expect(s.count("stickyToast")).toBe(3); // abc 만 새 구간
  });
  it("좌석 종료(surface.exited/closed)도 같은 dismissStarvedToast 를 부른다 — 소스 핀", () => {
    const exits = code.match(/dismissStarvedToast\(event\.socket_slug, sid\);/g) ?? [];
    expect(exits.length).toBeGreaterThanOrEqual(2); // queue.delivered 분기 + 좌석 종료 분기
  });
  it("잘못된 payload 는 아무것도 부르지 않지만 분기는 return 한다", () => {
    const s = scenario();
    for (const bad of [null, undefined, "x", 7, [], {}, { surface_ref: "surface:abc" }]) s.fire(bad);
    expect(s.calls.length).toBe(0);
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
