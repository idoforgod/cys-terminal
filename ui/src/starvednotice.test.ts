// 0.14.43 UI2 — 큐 기아 경보(queue.starved)의 토스트·배너 회귀 핀(starvednotice.ts 순수 도우미 + main.ts 배선).
//
// 무엇을 지키는가(티켓 UI2):
//   · 이 GUI 는 queue.starved 를 소비하지 않았다 — category "queue" 는 폴백 3종(health·watchdog·feed)에 안 걸려 화면이 0 이었고, 사람이 해야 하는
//     조치(그 창을 클릭해 Ctrl-U · 질문 창에 답하기 …)를 알릴 표면이 없어 좌석 큐가 몇 시간씩 막혀도 아무도 몰랐다.
//   · starvedNotice 는 payload 를 의심한다(타입·꼴) — 올릴 수 없으면 null, 올리면 제목·상세(LLM 꼬리 제거 · 200자 상한)·id·humanNeeded.
//   · remedy_code 표: approval·paused·wait 만 '사람 조치 불필요'(토스트만) — 모르는 값이면 알린다(모르면 알린다).
//   · ★R2F-UI(A3 m1): 코드가 **아예 없는** payload(0.14.42 데몬)는 `blocked_by` 의 접두로 가린다 — 스스로 풀리거나 사람 조치가 아닌 사유(데몬 REMEDY_WAIT_PREFIXES 의 것들 + approval_pending + 일시정지)면 토스트만 ·
//     그 밖은 알린다. 접두 목록은 데몬 소스를 읽는 어휘 핀으로 묶는다.
//   · 토스트를 **눌렀을 때만** 좌석으로 간다 — 다른 데몬의 같은 번호 좌석을 열지 않는다(locateStarvedSeat). 못 찾으면 무동작.
//   · main.ts 배선: name-우선 분기 · 끝의 return(폴백 이중 표시 금지) · 풀림(queue.delivered)·좌석 종료가 토스트를 거둔다.
//   · 순수 모듈 불변식: 최상위 부수효과 0 · 문서/창/저장소 낱말 0 · 구형 WKWebView 비호환 문법 0.
import { describe, it, expect } from "bun:test";
import { readFileSync } from "node:fs";
import {
  starvedNotice,
  starvedHumanNeeded,
  starvedWaitText,
  starvedSlug,
  starvedDismissId,
  surfaceIdOfRef,
  locateStarvedSeat,
  STARVED_DETAIL_MAX,
  STARVED_LLM_TAIL,
  STARVED_HUMAN_CODES,
  STARVED_LEGACY_HUMAN_CODES,
  STARVED_CALM_CODES,
  STARVED_LEGACY_CALM_PREFIXES,
  type StarvedSeatLookup,
} from "./starvednotice";

const read = (rel: string) => readFileSync(new URL(rel, import.meta.url), "utf-8");
const cpLen = (s: string): number => Array.from(s).length;

/** 보이지 않는 문자 — 원문에 raw 로 두지 않고 코드로 만든다(편집기·도구가 이스케이프를 실제 문자로 바꿔 버려도 핀이 깨지지 않게). */
const BIDI = String.fromCharCode(0x202e); // 오른쪽→왼쪽 덮어쓰기
const ZWSP = String.fromCharCode(0x200b); // 제로폭 공백
const NUL = String.fromCharCode(0);
/** 데몬이 모든 remedy 끝에 붙이는 꼬리 — 현재 문면(리뷰 반영: 동결 해제·항목 삭제가 금지 목록에 더해졌다). 앞부분 ` · LLM 에이전트는 자동 조치` 는 그대로다. */
const TAIL = " · LLM 에이전트는 자동 조치(강제 배달·드레인·키 주입·동결 해제·항목 삭제) 금지";
/** 앞선 빌드(0.14.43 C5 첫 커밋)의 꼬리 문면 — 구버전 데몬이 보낼 수 있다. */
const TAIL_V1 = " · LLM 에이전트는 자동 조치(강제 배달·드레인·키 주입) 금지";
const REMEDY_BODY = "입력줄은 비어 보이는데 미제출 계수가 남았다(유령 계수) — 사람이 그 창을 클릭하고 Ctrl-U 한 번(약 30초 뒤 배달 재개)";
/** 0.14.43 데몬의 전형 payload(governance.rs queue_starved_payload) — 기존 7키 + 진단·remedy 가산 키. */
const payload = (o: Record<string, unknown> = {}): Record<string, unknown> => ({
  surface_ref: "surface:12",
  role: "worker",
  head_entry_id: "q-1",
  waited_secs: 720,
  depth: 3,
  blocked_by: "input_pending:12",
  hint: "큐 머리가 장기 대기 중(게이트에 막힘) — 조치는 remedy 참조.",
  pending_input_bytes: 5,
  draft_visible: false,
  remedy_code: "phantom_count",
  remedy: REMEDY_BODY + TAIL,
  ...o,
});
/** 0.14.42 이하 데몬의 payload — remedy_code·remedy 가 없다. */
const legacyPayload = (o: Record<string, unknown> = {}): Record<string, unknown> => ({
  surface_ref: "surface:3",
  role: "worker",
  head_entry_id: "q-9",
  waited_secs: 600,
  depth: 2,
  blocked_by: "busy",
  hint: "큐 머리가 장기 대기 중(게이트에 막힘) — 운영자(사람) 판단 하에 cys queue deliver 로 강제 배달 가능.",
  ...o,
});

describe("remedy_code 별 humanNeeded 표 — approval·paused·wait 만 false, 코드가 없거나 모르면 true(처방 code 13종)", () => {
  // 처방 code 13종(데몬 리뷰 확정) = 사람 조치 10종 + calm 3종. phantom_count_ctrl_u 는 phantom_count 의 옛 이름이다(구버전 데몬이 보낼 수 있다).
  const HUMAN = [
    "machine_residue",
    "phantom_count",
    "after_cursor_text",
    "human_draft",
    "input_pending_unknown",
    "answer_modal",
    "alt_screen",
    "empty_seat",
    "prompt_unknown",
    "unknown",
  ];
  const LEGACY_HUMAN = ["phantom_count_ctrl_u"];
  const CALM = ["approval", "paused", "wait"];
  for (const code of HUMAN)
    it(`${code} → humanNeeded true(사람이 조치한다 — 토스트 + OS 배너)`, () => {
      expect(starvedNotice(payload({ remedy_code: code }))!.humanNeeded).toBe(true);
      expect(starvedHumanNeeded(code)).toBe(true);
    });
  for (const code of LEGACY_HUMAN)
    it(`${code}(옛 이름 — 구버전 데몬) → humanNeeded true · 조치 문장도 그대로 쓴다(아는 code)`, () => {
      const n = starvedNotice(payload({ remedy_code: code }))!;
      expect(n.humanNeeded).toBe(true);
      expect(starvedHumanNeeded(code)).toBe(true);
      expect(n.detail).toBe(`12분째 · ${REMEDY_BODY}`); // 모르는 code 였다면 `막힘 사유: …` 로 떨어졌을 것이다
    });
  for (const code of CALM)
    it(`${code} → humanNeeded false(토스트만 — 승인은 기존 승인 알림 경로 · 동결·일시 보류는 사람 조치 불요)`, () => {
      expect(starvedNotice(payload({ remedy_code: code }))!.humanNeeded).toBe(false);
      expect(starvedHumanNeeded(code)).toBe(false);
    });
  it("remedy_code 가 없거나(구버전 데몬)·null·빈 문자열·모르는 값·대소문자 다른 값·문자열이 아닌 값이면 true — 모르면 알린다", () => {
    const base = payload();
    delete base.remedy_code;
    expect(starvedNotice(base)!.humanNeeded).toBe(true);
    for (const v of [null, undefined, "", "bogus", "WAIT", "Paused", " wait", "wait ", "phantom_count_", "after_cursor", 7, true, {}, []])
      expect({ 값: String(v), humanNeeded: starvedNotice(payload({ remedy_code: v }))!.humanNeeded }).toEqual({ 값: String(v), humanNeeded: true });
    expect(starvedHumanNeeded(undefined)).toBe(true);
  });
  it("표의 목록(소스 상수)은 확정된 13종 + 옛 이름 1개와 같다 · humanNeeded=false 는 정확히 셋 · 서로 겹치지 않는다", () => {
    expect([...STARVED_HUMAN_CODES].sort()).toEqual([...HUMAN].sort());
    expect([...STARVED_LEGACY_HUMAN_CODES].sort()).toEqual([...LEGACY_HUMAN].sort());
    expect([...STARVED_CALM_CODES].sort()).toEqual([...CALM].sort());
    expect(HUMAN.length + CALM.length).toBe(13);
    const all = [...STARVED_HUMAN_CODES, ...STARVED_LEGACY_HUMAN_CODES, ...STARVED_CALM_CODES];
    expect(new Set(all).size).toBe(all.length); // 어느 code 도 두 목록에 걸리지 않는다
    const calmOnes = all.filter((c) => !starvedHumanNeeded(c));
    expect([...calmOnes].sort()).toEqual([...CALM].sort());
  });
});

// ── ★R2F-UI(A3 m1) 구 데몬 + 새 앱: 큐 막힘 OS 배너가 스스로 풀리는 사유에도 나갔다 ─────────────────────────────────────────────
// 0.14.42 데몬의 payload 에는 `remedy_code` 가 없다(7키). 그때 humanNeeded 는 늘 true 라 — 승인 대기·일시정지·`busy`(출력 중) 같은 사유도 OS 배너(고우선 통로)를 냈다. 새 데몬은 같은 사유를 calm 코드로 내 배너가 없다.
// 업데이트 직후 세션을 살려 둔 사용자(= 가장 많이 쓰는 사용자)가 워커가 10분 넘게 일할 때마다 좌석당 5분 간격으로 배너를 받았다. 코드가 **아예 없는** payload 는 `blocked_by` 의 접두로 가려 그런 사유면 토스트만 낸다.
describe("★R2F-UI(A3 m1) 코드가 아예 없는 payload(0.14.42 데몬) — 막힘 사유의 접두로 사람 조치 여부를 가른다", () => {
  const legacy = (blocked_by: unknown): Record<string, unknown> => legacyPayload({ blocked_by });
  const needed = (blocked_by: unknown): boolean => starvedNotice(legacy(blocked_by))!.humanNeeded;
  const CALM_REASONS = [
    "busy(출력 중)",
    "delivery_interval(배달 최소 간격)",
    "settle_budget(이 틱의 인계 결판 예산 소진 — 다음 틱이 이 좌석부터 시작한다)",
    "quiescing(사이클 진행 중 · /clear~RESUME 창)",
    "prompt_not_ready(프롬프트 경계 미도달)",
    "human_typing(사람이 입력 중)",
    "queue_paused(헬스 조치)",
    "approval_pending(승인·관문 대기)",
    "paused(kill-switch 동결)",
  ];
  const HUMAN_REASONS = [
    "input_pending(입력줄에 미제출 입력)",
    "modal_pending(모달·선택기 전경)",
    "prompt_unknown(프롬프트 경계 관측 불능)",
    "alt_screen(전체화면 · 프롬프트 레이아웃 미확인)",
    "empty_seat(좌석에 에이전트가 없다)",
    "schedule_divert(gate:draft · job 7)",
    "future_reason(새 사유)",
  ];
  it("★스스로 풀리거나 사람 조치가 아닌 사유 — 데몬 wait 7종(busy·delivery_interval·settle·quiescing·prompt_not_ready·human_typing·queue_paused) + approval_pending + 일시정지(paused) — 는 토스트만(humanNeeded false)", () => {
    for (const r of CALM_REASONS) expect({ 사유: r, 알림: needed(r) }).toEqual({ 사유: r, 알림: false });
    // 접두만 같으면 된다 — 괄호 설명이 달라도(데몬 판마다 문면이 다르다) · 앞뒤 공백·줄바꿈이 끼어도
    for (const r of ["busy", "busy(다른 설명)", "  busy(출력 중)  ", "busy\n", "queue_paused", "settle", "paused"]) expect({ 사유: r, 알림: needed(r) }).toEqual({ 사유: r, 알림: false });
  });
  it("★사람이 조치해야 하는 사유·모르는 사유는 그대로 알린다(토스트 + OS 배너) — 입력줄·모달·프롬프트 불명·전체화면·빈 좌석·스케줄 우회·새 사유", () => {
    for (const r of HUMAN_REASONS) expect({ 사유: r, 알림: needed(r) }).toEqual({ 사유: r, 알림: true });
  });
  it("사유가 없거나(null·빈 값·공백·키 없음)·문자열이 아니면(숫자·객체·배열·불리언) 알린다 — 모르면 알린다", () => {
    for (const r of [null, undefined, "", "   ", "\n\t", 5, {}, [], ["busy"], true]) expect({ 사유: String(r), 알림: needed(r) }).toEqual({ 사유: String(r), 알림: true });
    const noKey = legacyPayload();
    delete noKey.blocked_by;
    expect(starvedNotice(noKey)!.humanNeeded).toBe(true);
    expect(starvedHumanNeeded(undefined)).toBe(true); // 코드도 사유도 없다
    expect(starvedHumanNeeded(undefined, undefined)).toBe(true);
  });
  it("접두는 **앞**에서만 본다 — 사유 중간에 calm 낱말이 들어 있어도(예: input_pending 의 설명에 'busy') calm 으로 읽지 않는다 · 대소문자 다른 값·접두를 늘린 값도 아니다", () => {
    for (const r of ["input_pending(busy 때문에 막힘)", "x_busy", "Busy", "BUSY(출력 중)", "unbusy", "queue_paused_x_but_not", "not_paused"]) {
      const calm = r === "queue_paused_x_but_not"; // 접두 queue_paused 와 일치 — 데몬도 접두 일치(starts_with)로 읽는다
      expect({ 사유: r, 알림: needed(r) }).toEqual({ 사유: r, 알림: !calm });
    }
  });
  it("★코드가 **있으면** 코드로만 가른다 — calm 사유(busy)라도 사람 조치 코드면 알리고, 코드가 calm 이면 사유와 무관하게 토스트만 · `null` 같은 '있는데 이상한' 값은 코드가 없는 것이 아니다(알린다)", () => {
    expect(starvedNotice(legacyPayload({ remedy_code: "phantom_count" }))!.humanNeeded).toBe(true); // 사유 busy 인데 사람 조치 코드
    expect(starvedNotice(legacyPayload({ blocked_by: "input_pending(…)", remedy_code: "wait" }))!.humanNeeded).toBe(false); // 사유는 입력줄인데 calm 코드
    for (const v of [null, "", "bogus", 7, {}, []]) expect({ 코드: JSON.stringify(v), 알림: starvedNotice(legacyPayload({ remedy_code: v }))!.humanNeeded }).toEqual({ 코드: JSON.stringify(v), 알림: true });
    expect(starvedHumanNeeded("wait", "input_pending")).toBe(false);
    expect(starvedHumanNeeded("phantom_count", "busy")).toBe(true);
  });
  it("접두 목록(STARVED_LEGACY_CALM_PREFIXES)은 정확히 9개 — 데몬 wait 7 + approval_pending + paused · 서로 겹치지 않는다(어느 접두도 다른 접두의 접두가 아니다)", () => {
    expect([...STARVED_LEGACY_CALM_PREFIXES].sort()).toEqual(["approval_pending", "busy", "delivery_interval", "human_typing", "paused", "prompt_not_ready", "queue_paused", "quiescing", "settle"]);
    for (const a of STARVED_LEGACY_CALM_PREFIXES) for (const b of STARVED_LEGACY_CALM_PREFIXES) if (a !== b) expect({ a, b, 겹침: b.startsWith(a) }).toEqual({ a, b, 겹침: false });
  });
  it("조치 문구·제목·id 는 종전 그대로다 — 사유가 calm 이어도 상세는 `막힘 사유: …` 이고 토스트는 뜬다(OS 배너만 없다)", () => {
    const n = starvedNotice(legacy("busy(출력 중)"), "abc")!;
    expect(n).toEqual({ id: "starved:abc:surface:3", title: "⏳ 큐 막힘 — worker surface:3", detail: "10분째 · 막힘 사유: busy(출력 중)", humanNeeded: false });
  });
});

describe("★R2F-UI(A3 m1) 데몬 어휘 핀 — 접두 목록은 데몬 소스를 읽어 묶는다(데몬이 wait 접두를 더하거나 사유 상수를 바꾸면 이 표도 따라가야 한다)", () => {
  const rs = read("../../src/bin/cysd/governance.rs");
  it("STARVED_LEGACY_CALM_PREFIXES = 데몬 REMEDY_WAIT_PREFIXES(소스에서 읽는다) ∪ {approval_pending, paused} — 한쪽만 바뀌면 적색", () => {
    const m = rs.match(/const REMEDY_WAIT_PREFIXES: \[&str; \d+\] = \[([^\]]*)\];/);
    expect(m).not.toBeNull();
    const daemon = Array.from((m as RegExpMatchArray)[1].matchAll(/"([^"]*)"/g), (x) => x[1]);
    expect(daemon.length).toBeGreaterThan(0);
    expect(daemon.length).toBe(Number(((rs.match(/const REMEDY_WAIT_PREFIXES: \[&str; (\d+)\]/) as RegExpMatchArray)[1])));
    expect([...STARVED_LEGACY_CALM_PREFIXES].sort()).toEqual([...daemon, "approval_pending", "paused"].sort());
  });
  it("데몬의 사유 상수(`pub(crate) const BLOCKED_* : &str`)는 전부 이 표의 분류에 있고 — 각 상수의 문면이 구 데몬 payload 로 오면 표대로 읽힌다(calm 이면 토스트만 · 아니면 알린다)", () => {
    // 상수 이름 → 코드 없는 payload 의 humanNeeded. 새 사유 상수가 데몬에 생기면 여기에도 한 줄을 더해야 한다(판단을 강제한다).
    const TABLE: Record<string, boolean> = {
      BLOCKED_APPROVAL: false, // approval(승인 알림 경로가 맡는다)
      BLOCKED_MODAL: true, // answer_modal
      BLOCKED_BUSY: false, // wait
      BLOCKED_INPUT_PENDING: true, // 입력줄 계열
      BLOCKED_PROMPT_UNKNOWN: true, // prompt_unknown
      BLOCKED_PROMPT_NOT_READY: false, // wait
      BLOCKED_ALT_SCREEN: true, // alt_screen
      BLOCKED_INTERVAL: false, // wait
      BLOCKED_QUIESCING: false, // wait
      BLOCKED_SETTLE_BUDGET: false, // wait(settle — 다음 틱에 스스로 풀린다)
      BLOCKED_PAUSED_KILL_SWITCH: false, // paused
      BLOCKED_QUEUE_PAUSED: false, // wait(queue_paused)
    };
    const found = Array.from(rs.matchAll(/pub\(crate\) const (BLOCKED_[A-Z_]+): &str =\s*"([^"]*)";/g), (x) => [x[1], x[2]] as [string, string]);
    expect(found.length).toBeGreaterThanOrEqual(12);
    expect({ 표에_없는_상수: found.map(([n]) => n).filter((n) => !(n in TABLE)) }).toEqual({ 표에_없는_상수: [] });
    expect({ 데몬에_없는_표항목: Object.keys(TABLE).filter((n) => !found.some(([f]) => f === n)) }).toEqual({ 데몬에_없는_표항목: [] });
    for (const [name, text] of found) {
      const n = starvedNotice(legacyPayload({ blocked_by: text }))!;
      expect({ 상수: name, 문면: text, 알림: n.humanNeeded }).toEqual({ 상수: name, 문면: text, 알림: TABLE[name] });
    }
  });
});

describe("데몬 어휘와 대조 — 데몬이 내는 remedy_code 는 전부 이 표가 분류한다(어휘 이탈 방지)", () => {
  const rs = read("../../src/bin/cysd/governance.rs");
  // ★R2F-UI(A3 n17): 종전에는 이 대조가 데몬 소스에 `fn queue_remedy(` 가 있을 때만 돌았다('그 티켓이 아직 안 들어온 중간 커밋'을 위한 탈출구). 처방 표는 이제 트리에 있으니 탈출구는 쓸모가 없고,
  //   남아 있으면 `queue_remedy` 의 이름을 바꾸거나 다른 파일로 옮기는 순간 데몬↔화면 어휘를 묶는 **유일한** 대조가 초록인 채 꺼진다. 무조건 돌리고, 생산자가 없으면 적색이다.
  const hasProducer = rs.includes("fn queue_remedy(");
  it("★R2F-UI(A3 n17): 처방 표 생산자(`fn queue_remedy(`)가 데몬 소스에 있다 — 없으면 아래 어휘 대조가 말없이 꺼지던 탈출구가 닫혔다", () => {
    expect(hasProducer).toBe(true);
  });
  it("QUEUE_REMEDY_CODES ⊆ (사람 조치 필요 ∪ 옛 이름 ∪ calm) · 꼬리 상수 REMEDY_LLM_SUFFIX 가 STARVED_LLM_TAIL 로 시작한다", () => {
    const m = rs.match(/const QUEUE_REMEDY_CODES: \[&str; \d+\] = \[([^\]]*)\];/);
    expect(m).not.toBeNull();
    const daemon = Array.from((m as RegExpMatchArray)[1].matchAll(/"([^"]*)"/g), (x) => x[1]);
    expect(daemon.length).toBeGreaterThan(0);
    const known = [...STARVED_HUMAN_CODES, ...STARVED_LEGACY_HUMAN_CODES, ...STARVED_CALM_CODES];
    expect({ 분류_안_된_코드: daemon.filter((c) => known.indexOf(c) < 0) }).toEqual({ 분류_안_된_코드: [] });
    const t = rs.match(/const REMEDY_LLM_SUFFIX: &str = "([^"]*)";/);
    expect(t).not.toBeNull();
    expect((t as RegExpMatchArray)[1].startsWith(STARVED_LLM_TAIL)).toBe(true);
  });
});

describe("starvedNotice — 제목·상세·id", () => {
  it("전형 payload: 제목 · 상세(분 + 조치 문장, LLM 꼬리 없음) · id · humanNeeded", () => {
    expect(starvedNotice(payload(), "abc123")).toEqual({
      id: "starved:abc123:surface:12",
      title: "⏳ 큐 막힘 — worker surface:12",
      detail: `12분째 · ${REMEDY_BODY}`,
      humanNeeded: true,
    });
  });
  it("돌려주는 객체의 키는 정확히 4개(id·title·detail·humanNeeded)", () => {
    expect(Object.keys(starvedNotice(payload(), "x")!).sort()).toEqual(["detail", "humanNeeded", "id", "title"]);
  });
  it("역할이 없거나(null·undefined·빈 값·공백·문자열이 아님) 이상하면 제목에서 빠지고 공백이 겹치지 않는다", () => {
    for (const r of [null, undefined, "", "   ", "\n\t", 7, {}, [], true])
      expect({ 역할: String(r), 제목: starvedNotice(payload({ role: r }))!.title }).toEqual({ 역할: String(r), 제목: "⏳ 큐 막힘 — surface:12" });
    const noKey = payload();
    delete noKey.role;
    expect(starvedNotice(noKey)!.title).toBe("⏳ 큐 막힘 — surface:12");
  });
  it("역할의 줄바꿈·제어문자·양방향 제어·제로폭 문자는 공백으로 접힌다 · 40자를 넘으면 39자 + …", () => {
    expect(starvedNotice(payload({ role: "  wor\nker\t" + BIDI + ZWSP + " x " }))!.title).toBe("⏳ 큐 막힘 — wor ker x surface:12");
    const long = starvedNotice(payload({ role: "역".repeat(100) }))!.title;
    expect(long).toBe(`⏳ 큐 막힘 — ${"역".repeat(39)}… surface:12`);
  });
  it("분 단위 표기는 올림이다 — 0 이하·비수치는 분 표기만 생략한다(지어내지 않는다)", () => {
    const table: [unknown, string][] = [
      [1, "1분째"],
      [0.4, "1분째"],
      [59, "1분째"],
      [60, "1분째"],
      [61, "2분째"],
      [119, "2분째"],
      [120, "2분째"],
      [121, "3분째"],
      [599, "10분째"],
      [600, "10분째"],
      [601, "11분째"],
      [720, "12분째"],
      [3599, "60분째"],
      [3600, "60분째"],
      [3601, "61분째"],
      [1e12, "99999분째"],
    ];
    for (const [secs, text] of table) {
      expect({ 초: String(secs), 표기: starvedWaitText(secs) }).toEqual({ 초: String(secs), 표기: text });
      expect(starvedNotice(payload({ waited_secs: secs }))!.detail.startsWith(`${text} · `)).toBe(true);
    }
    for (const bad of [0, -5, NaN, Infinity, -Infinity, "600", "12분", null, undefined, {}, [], true]) {
      expect({ 값: String(bad), 표기: starvedWaitText(bad) }).toEqual({ 값: String(bad), 표기: "" });
      expect(starvedNotice(payload({ waited_secs: bad }))!.detail).toBe(REMEDY_BODY);
    }
    const noKey = payload();
    delete noKey.waited_secs;
    expect(starvedNotice(noKey)!.detail).toBe(REMEDY_BODY);
  });
});

describe("LLM 꼬리 제거 — 사람에게는 불필요한 ' · LLM 에이전트는 자동 조치…' 를 뗀다", () => {
  it("데몬 문면의 꼬리(현재·앞선 빌드 두 문면)가 상세에 없다", () => {
    for (const tail of [TAIL, TAIL_V1]) {
      const d = starvedNotice(payload({ remedy: REMEDY_BODY + tail }))!.detail;
      expect({ 꼬리: tail, LLM: d.includes("LLM") }).toEqual({ 꼬리: tail, LLM: false });
      for (const part of ["자동 조치", "강제 배달·드레인·키 주입", "동결 해제", "항목 삭제", "금지"])
        expect({ 꼬리: tail, 조각: part, 있음: d.includes(part) }).toEqual({ 꼬리: tail, 조각: part, 있음: false });
      expect(d).toBe(`12분째 · ${REMEDY_BODY}`);
      expect(d.endsWith("(약 30초 뒤 배달 재개)")).toBe(true);
    }
  });
  it("꼬리는 접두(` · LLM 에이전트는 자동 조치`)로 찾는다 — 괄호 안 금지 목록이 어떻게 늘어도·없어도 떨어진다", () => {
    for (const tail of [
      " · LLM 에이전트는 자동 조치",
      " · LLM 에이전트는 자동 조치(가) 금지",
      " · LLM 에이전트는 자동 조치(강제 배달·드레인·키 주입·동결 해제·항목 삭제·새로 생긴 항목) 금지",
      " · LLM 에이전트는 자동 조치 금지 — 뒤에 문장이 더 붙어도",
    ])
      expect({ 꼬리: tail, 상세: starvedNotice(payload({ remedy: REMEDY_BODY + tail }))!.detail }).toEqual({ 꼬리: tail, 상세: `12분째 · ${REMEDY_BODY}` });
  });
  it("꼬리가 없으면 원문 그대로(앞뒤 공백만 다듬는다)", () => {
    expect(starvedNotice(payload({ remedy: "질문·선택 창이 떠 있다 — 사람이 답한다", remedy_code: "answer_modal" }))!.detail).toBe(
      "12분째 · 질문·선택 창이 떠 있다 — 사람이 답한다",
    );
    expect(starvedNotice(payload({ remedy: "  질문 창  ", remedy_code: "answer_modal" }))!.detail).toBe("12분째 · 질문 창");
  });
  it("꼬리는 그 자리부터 **끝까지** 뗀다(꼬리 뒤에 붙은 글도 함께)", () => {
    const r = starvedNotice(payload({ remedy: `앞 문장${STARVED_LLM_TAIL}(강제 배달) 금지 — 뒤에 붙은 군더더기`, remedy_code: "wait" }))!;
    expect(r.detail).toBe("12분째 · 앞 문장");
  });
  it("파서 패닉 주석 같은 문장 중간 가산 문구는 남는다(꼬리 앞)", () => {
    const rem = `${REMEDY_BODY} (이 좌석 화면 파서 패닉 2회 — 화면 판독이 순간 비었을 수 있다)${TAIL}`;
    expect(starvedNotice(payload({ remedy: rem }))!.detail).toBe(`12분째 · ${REMEDY_BODY} (이 좌석 화면 파서 패닉 2회 — 화면 판독이 순간 비었을 수 있다)`);
  });
  it("문장이 꼬리뿐이면(맨 앞에 붙어 앞 공백이 없어도) 조치 문장이 비어 막힘 사유로 대체된다", () => {
    expect(starvedNotice(payload({ remedy: TAIL }))!.detail).toBe("12분째 · 막힘 사유: input_pending:12");
    expect(starvedNotice(payload({ remedy: TAIL.trim() }))!.detail).toBe("12분째 · 막힘 사유: input_pending:12");
  });
  it("꼬리 상수는 데몬 문면(현재·앞선 빌드)의 머리와 같다", () => {
    expect(TAIL.startsWith(STARVED_LLM_TAIL)).toBe(true);
    expect(TAIL_V1.startsWith(STARVED_LLM_TAIL)).toBe(true);
    expect(STARVED_LLM_TAIL).toBe(" · LLM 에이전트는 자동 조치");
  });
});

describe("200자 절단 — 넘으면 앞 199자 + … (총 200자, 코드 포인트 단위)", () => {
  // "12분째 · " 가 7자라 조치 문장이 193자면 총 200자(그대로) · 194자면 201자(절단).
  const withRemedy = (n: number, ch = "가") => starvedNotice(payload({ remedy: ch.repeat(n), remedy_code: "wait" }))!.detail;
  it("경계: 200자는 그대로 · 201자는 절단", () => {
    const keep = withRemedy(193);
    expect(cpLen(keep)).toBe(STARVED_DETAIL_MAX);
    expect(keep.endsWith("…")).toBe(false);
    const cut = withRemedy(194);
    expect(cpLen(cut)).toBe(STARVED_DETAIL_MAX);
    expect(cut.endsWith("…")).toBe(true);
    expect(cut.startsWith("12분째 · 가")).toBe(true);
  });
  it("아주 긴 조치 문장(5000자)도 총 200자 이내 · … 로 끝난다", () => {
    const d = withRemedy(5000);
    expect(cpLen(d)).toBe(200);
    expect(d.endsWith("…")).toBe(true);
    expect(d.slice(0, 7)).toBe("12분째 · ");
  });
  it("이모지(서로게이트 쌍)를 절단 경계에서 반으로 자르지 않는다", () => {
    const d = starvedNotice(payload({ remedy: "가".repeat(180) + "😀".repeat(40), remedy_code: "wait" }))!.detail;
    expect(cpLen(d)).toBe(200);
    for (const ch of Array.from(d)) {
      const cp = ch.codePointAt(0) as number;
      expect(cp >= 0xd800 && cp <= 0xdfff).toBe(false); // 외톨이 서로게이트 없음
    }
    expect(d.endsWith("…")).toBe(true);
  });
  it("막힘 사유 대체 문장(구버전)도 같은 상한을 받는다 — blocked_by 는 80자에서 먼저 잘린다", () => {
    const d = starvedNotice(legacyPayload({ blocked_by: "b".repeat(1000) }))!.detail;
    expect(d).toBe(`10분째 · 막힘 사유: ${"b".repeat(79)}…`);
    expect(cpLen(d) <= STARVED_DETAIL_MAX).toBe(true);
  });
});

describe("구버전 데몬 payload(remedy_code·remedy 없음) · 조치 문장을 믿을 수 없는 경우", () => {
  // ★R2F-UI(A3 m1): 이름·기대를 고쳤다 — 종전 「remedy 없음 → humanNeeded true」 는 코드가 없으면 **모든** 사유를 '모르면 알린다'로 읽었다. 이제 코드가 아예 없는 payload 는 막힘 사유의 접두로 가린다:
  //   기본 legacyPayload(blocked_by=`busy`)는 스스로 풀리는 사유라 토스트만(humanNeeded false), 사람 조치 사유(`input_pending…`)는 그대로 true. 상세 문구(`막힘 사유: <blocked_by>`)·분 표기·id·제목은 그대로다.
  it("remedy 없음 → 상세는 `막힘 사유: <blocked_by>` 만(분 표기는 유지) · humanNeeded 는 사유로 가른다 — 사람 조치 사유(입력줄)면 true, 스스로 풀리는 사유(busy)면 false", () => {
    expect(starvedNotice(legacyPayload({ blocked_by: "input_pending(입력줄에 미제출 입력)" }), "base")).toEqual({
      id: "starved:base:surface:3",
      title: "⏳ 큐 막힘 — worker surface:3",
      detail: "10분째 · 막힘 사유: input_pending(입력줄에 미제출 입력)",
      humanNeeded: true,
    });
    expect(starvedNotice(legacyPayload(), "base")).toEqual({
      id: "starved:base:surface:3",
      title: "⏳ 큐 막힘 — worker surface:3",
      detail: "10분째 · 막힘 사유: busy",
      humanNeeded: false,
    });
  });
  it("remedy_code 가 없거나 모르는 값이면 remedy 문장이 있어도 쓰지 않는다 — blocked_by 만(티켓 문면)", () => {
    expect(starvedNotice(legacyPayload({ remedy: `조치 문장${TAIL}` }))!.detail).toBe("10분째 · 막힘 사유: busy");
    const futureCode = starvedNotice(legacyPayload({ remedy_code: "future_code", remedy: `새 조치${TAIL}` }))!;
    expect(futureCode.detail).toBe("10분째 · 막힘 사유: busy");
    expect(futureCode.humanNeeded).toBe(true); // ★코드가 **있는데** 모르는 값이면 사유가 busy 여도 알린다 — 사유 접두 규칙은 코드가 아예 없는 payload 에만 쓴다(A3 m1)
  });
  it("아는 코드인데 remedy 가 없거나(빈 값·공백·문자열 아님) 비면 `막힘 사유: …` 로 대체된다", () => {
    for (const r of [undefined, null, "", "   ", 12, {}, [], TAIL])
      expect({ remedy: String(r), 상세: starvedNotice(payload({ remedy: r }))!.detail }).toEqual({
        remedy: String(r),
        상세: "12분째 · 막힘 사유: input_pending:12",
      });
  });
  it("blocked_by 도 없으면 `막힘 사유: 알 수 없음`", () => {
    for (const b of [undefined, null, "", "  ", 5, {}]) {
      const d = starvedNotice(legacyPayload({ blocked_by: b }))!.detail;
      expect({ 값: String(b), 상세: d }).toEqual({ 값: String(b), 상세: "10분째 · 막힘 사유: 알 수 없음" });
    }
  });
  it("대기 시간도 없는 극단 payload — 상세는 막힘 사유 한 줄", () => {
    const n = starvedNotice({ surface_ref: "surface:1" })!;
    expect(n.detail).toBe("막힘 사유: 알 수 없음");
    expect(n.title).toBe("⏳ 큐 막힘 — surface:1");
    expect(n.humanNeeded).toBe(true);
    expect(n.id).toBe("starved:base:surface:1");
  });
});

describe("잘못된 payload → null (화면에 올리지 않는다)", () => {
  it("객체가 아닌 값", () => {
    for (const v of [null, undefined, "", "surface:12", 0, 12, true, false, [], [payload()], () => 1, Symbol("s")])
      expect({ 값: typeof v + ":" + String(typeof v === "symbol" ? "sym" : v), 결과: starvedNotice(v) }).toEqual({
        값: typeof v + ":" + String(typeof v === "symbol" ? "sym" : v),
        결과: null,
      });
  });
  it("surface_ref 가 없거나 `surface:<숫자>` 꼴이 아니면 null", () => {
    const bad: unknown[] = [
      undefined,
      null,
      "",
      12,
      "12",
      "surface:",
      "surface:abc",
      "surface:12x",
      "surface:1 2",
      "surface: 12",
      " surface:12",
      "surface:12 ",
      "surface:12\n",
      "surface:-1",
      "surface:1.5",
      "surface:+3",
      "Surface:12",
      "surfaces:12",
      "xsurface:12",
      "surface:12:13",
      "surface:" + "9".repeat(21),
      "surface:١٢",
      {},
      ["surface:12"],
    ];
    for (const ref of bad)
      expect({ 참조: String(ref), 결과: starvedNotice(payload({ surface_ref: ref })) }).toEqual({ 참조: String(ref), 결과: null });
    expect(starvedNotice({})).toBeNull();
    expect(starvedNotice({ role: "worker", waited_secs: 700 })).toBeNull();
  });
  it("올바른 꼴은 받는다(0 · 20자리까지)", () => {
    for (const ref of ["surface:0", "surface:7", "surface:00012", "surface:" + "9".repeat(20)])
      expect({ 참조: ref, 존재: starvedNotice(payload({ surface_ref: ref })) !== null }).toEqual({ 참조: ref, 존재: true });
  });
});

describe("id 형식 — starved:<소켓 slug>:<surface_ref> · 같은 좌석은 같은 id", () => {
  it("socketSlug 를 그대로 싣는다", () => {
    expect(starvedNotice(payload({ surface_ref: "surface:5" }), "1a2b3c4d5e6f7a8b")!.id).toBe("starved:1a2b3c4d5e6f7a8b:surface:5");
    expect(starvedNotice(payload({ surface_ref: "surface:5" }), "dept-2")!.id).toBe("starved:dept-2:surface:5");
  });
  it("구분 필드가 없거나(undefined·null·빈 값·공백·문자열 아님) 쓸 수 없으면 base", () => {
    for (const s of [undefined, null, "", "  ", "\n", 7, {}, [], true])
      expect({ slug: String(s), id: starvedNotice(payload(), s)!.id }).toEqual({ slug: String(s), id: "starved:base:surface:12" });
    expect(starvedNotice(payload())!.id).toBe("starved:base:surface:12");
    expect(starvedSlug(undefined)).toBe("base");
  });
  it("slug 의 제어문자는 공백으로 접히고 64자에서 잘린다", () => {
    expect(starvedSlug("a\nb")).toBe("a b");
    expect(starvedSlug("x".repeat(200)).length).toBe(64);
  });
  it("본부·부서 데몬의 같은 surface 번호는 id 가 겹치지 않는다 · 같은 데몬의 같은 좌석은 내용이 달라도 같은 id(갱신)", () => {
    const a = starvedNotice(payload(), "base")!.id;
    const b = starvedNotice(payload(), "dept-2")!.id;
    expect(a === b).toBe(false);
    expect(starvedNotice(payload({ waited_secs: 9999, remedy_code: "wait", remedy: "x" }), "dept-2")!.id).toBe(b);
    expect(starvedNotice(payload({ surface_ref: "surface:13" }), "dept-2")!.id === b).toBe(false);
  });
});

describe("해제용 id — starvedDismissId 는 starvedNotice().id 와 같은 id 를 만든다", () => {
  it("좌석 번호(정수)로 같은 id", () => {
    for (const slug of ["base", "dept-2", "1a2b3c4d5e6f7a8b", undefined, null, ""])
      for (const sid of [0, 1, 12, 4242])
        expect({ slug: String(slug), sid, 같음: starvedDismissId(slug, sid) === starvedNotice(payload({ surface_ref: `surface:${sid}` }), slug)!.id }).toEqual({
          slug: String(slug),
          sid,
          같음: true,
        });
  });
  it("번호가 0 이상의 안전한 정수가 아니면 null(아무것도 거두지 않는다)", () => {
    for (const bad of [-1, 1.5, NaN, Infinity, "12", "surface:12", null, undefined, {}, [], true, Number.MAX_SAFE_INTEGER + 2])
      expect({ 값: String(bad), 결과: starvedDismissId("base", bad) }).toEqual({ 값: String(bad), 결과: null });
  });
  it("surfaceIdOfRef — `surface:<숫자>` 에서 번호(안전한 정수만)", () => {
    expect(surfaceIdOfRef("surface:12")).toBe(12);
    expect(surfaceIdOfRef("surface:0")).toBe(0);
    expect(surfaceIdOfRef("surface:" + "9".repeat(20))).toBeNull(); // 안전한 정수 밖 — 누를 좌석이 없다
    for (const bad of [undefined, null, 12, "", "surface:", "surface:x", "12", {}])
      expect({ 값: String(bad), 결과: surfaceIdOfRef(bad) }).toEqual({ 값: String(bad), 결과: null });
  });
});

describe("신뢰할 수 없는 payload — 결정론 난수 600판(던지지 않고, 나오는 문자열은 한 줄·200자 이내)", () => {
  // mulberry32 — 시드 고정이라 재현된다.
  const rng = (seed: number): (() => number) => {
    let a = seed >>> 0;
    return () => {
      a = (a + 0x6d2b79f5) >>> 0;
      let t = a;
      t = Math.imul(t ^ (t >>> 15), t | 1);
      t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
  };
  const JUNK: unknown[] = [
    null, undefined, "", " ", "x", "surface:7", "surface:", 0, -1, 1.5, 59, 600, NaN, Infinity, true, false, [], [1], {}, { a: 1 },
    "\n\t", "⏳", "a".repeat(500), "가".repeat(300), "😀".repeat(150), BIDI + ZWSP + NUL, "<img src=x onerror=alert(1)>",
    STARVED_LLM_TAIL, TAIL, TAIL_V1, "wait", "paused", "approval", "unknown", "human_draft", "phantom_count", "phantom_count_ctrl_u", "after_cursor_text", REMEDY_BODY + TAIL,
  ];
  const CONTROL = /[\u{0}-\u{1f}\u{7f}-\u{9f}\u{200b}-\u{200f}\u{2028}\u{2029}\u{202a}-\u{202e}\u{2066}-\u{2069}\u{feff}]/u;
  it("어떤 조합이든 null 이거나 정해진 모양 — 제목·상세에 제어문자 0 · 상세 ≤ 200자 · id 는 surface_ref 로 끝난다", () => {
    const r = rng(20261003);
    const pick = (): unknown => JUNK[Math.floor(r() * JUNK.length)];
    const keys = ["surface_ref", "role", "waited_secs", "blocked_by", "remedy_code", "remedy", "depth", "hint"];
    let nonNull = 0;
    for (let i = 0; i < 600; i++) {
      const p: Record<string, unknown> = {};
      for (const k of keys) if (r() < 0.8) p[k] = r() < 0.35 ? (payload() as Record<string, unknown>)[k] : pick();
      if (r() < 0.5) p.surface_ref = `surface:${Math.floor(r() * 50)}`;
      const slug = r() < 0.3 ? pick() : "dept-" + Math.floor(r() * 4);
      const n = starvedNotice(p, slug);
      if (n === null) continue;
      nonNull++;
      expect(Object.keys(n).sort()).toEqual(["detail", "humanNeeded", "id", "title"]);
      expect(n.title.startsWith("⏳ 큐 막힘")).toBe(true);
      expect(n.id.startsWith("starved:")).toBe(true);
      expect(n.id.endsWith(":" + String(p.surface_ref))).toBe(true);
      expect(CONTROL.test(n.title) || CONTROL.test(n.detail) || CONTROL.test(n.id)).toBe(false);
      expect(cpLen(n.detail) <= STARVED_DETAIL_MAX).toBe(true);
      expect(n.detail.length > 0).toBe(true);
      expect(typeof n.humanNeeded).toBe("boolean");
    }
    expect(nonNull).toBeGreaterThan(100); // 시험이 공허하지 않다(올바른 surface_ref 가 충분히 섞인다)
  });
  it("HTML 은 해석하지 않고 글자 그대로 둔다 — 화면은 textContent 로만 그린다(이스케이프는 렌더 층의 일이 아니다)", () => {
    const evil = "<img src=x onerror=alert(1)>";
    const n = starvedNotice(payload({ role: evil, remedy: evil, remedy_code: "wait" }))!;
    expect(n.title).toBe(`⏳ 큐 막힘 — ${evil} surface:12`);
    expect(n.detail).toBe(`12분째 · ${evil}`);
  });
});

describe("locateStarvedSeat — 눌렀을 때 갈 좌석 확정(다른 데몬의 같은 번호 좌석을 열지 않는다)", () => {
  type Opts = {
    dept?: Record<string, string>; // slug → 부서 소켓
    baseSlug?: string | null; // 본부의 slug(백엔드가 알려 주는 값)
    seats?: [string | undefined, number][]; // (소켓·undefined=본부, 번호)
    throwIn?: "dept" | "isBase" | "hasSeat";
  };
  const make = (o: Opts) => {
    const calls: string[] = [];
    const look: StarvedSeatLookup = {
      deptSocket: (slug) => {
        calls.push(`dept:${slug}`);
        if (o.throwIn === "dept") throw new Error("boom");
        return o.dept?.[slug];
      },
      isBaseSlug: async (slug, sid) => {
        calls.push(`isBase:${slug}:${sid}`);
        if (o.throwIn === "isBase") throw new Error("ipc down");
        return o.baseSlug === slug;
      },
      hasSeat: (socket, sid) => {
        calls.push(`has:${String(socket)}:${sid}`);
        if (o.throwIn === "hasSeat") throw new Error("boom");
        return (o.seats ?? []).some(([s, n]) => s === socket && n === sid);
      },
    };
    return { look, calls };
  };
  it("부서 slug 가 풀리면 그 부서 소켓 — 본부 확인(IPC)은 하지 않는다", async () => {
    const { look, calls } = make({ dept: { d2: "/s/dept-2.sock" }, seats: [["/s/dept-2.sock", 4]] });
    expect(await locateStarvedSeat("d2", 4, look)).toEqual({ socket: "/s/dept-2.sock" });
    expect(calls.some((c) => c.startsWith("isBase:"))).toBe(false);
  });
  it("★부서 slug 가 풀리는데 그 부서 탭에 그 번호가 없고 **본부에 같은 번호**가 있으면 null — 본부 좌석을 열지 않는다", async () => {
    const { look } = make({ dept: { d2: "/s/dept-2.sock" }, seats: [[undefined, 4]] });
    expect(await locateStarvedSeat("d2", 4, look)).toBeNull();
  });
  it("slug 가 비면(구분 필드 없음) 본부 — IPC 없이 본부 좌석", async () => {
    const { look, calls } = make({ seats: [[undefined, 7]] });
    const r = await locateStarvedSeat("", 7, look);
    expect(r).not.toBeNull();
    expect((r as { socket: string | undefined }).socket).toBeUndefined();
    expect(calls.some((c) => c.startsWith("isBase:") || c.startsWith("dept:"))).toBe(false);
  });
  it("slug 가 안 풀리는데 본부 slug 와 같으면(백엔드 확인) 본부 좌석", async () => {
    const { look } = make({ baseSlug: "basehash", seats: [[undefined, 3]] });
    const r = await locateStarvedSeat("basehash", 3, look);
    expect(r).not.toBeNull();
    expect((r as { socket: string | undefined }).socket).toBeUndefined();
  });
  it("★slug 가 안 풀리고 본부도 아니면(탭 없는 부서의 경보) null — 같은 번호의 본부 좌석이 있어도 열지 않는다", async () => {
    const { look } = make({ baseSlug: "basehash", seats: [[undefined, 3]] });
    expect(await locateStarvedSeat("someotherdept", 3, look)).toBeNull();
  });
  it("어느 탭에도 그 좌석이 없으면 null(무동작)", async () => {
    const { look } = make({ baseSlug: "basehash", dept: { d2: "/s/dept-2.sock" }, seats: [[undefined, 1], ["/s/dept-2.sock", 2]] });
    expect(await locateStarvedSeat("basehash", 9, look)).toBeNull();
    expect(await locateStarvedSeat("d2", 9, look)).toBeNull();
    expect(await locateStarvedSeat("", 9, look)).toBeNull();
  });
  it("확인 중 오류(조회 실패·던짐)는 던지지 않고 null — 눌러도 아무 일 없는 쪽이 엉뚱한 좌석을 여는 쪽보다 안전하다", async () => {
    for (const throwIn of ["dept", "isBase", "hasSeat"] as const) {
      const { look } = make({ throwIn, baseSlug: "basehash", dept: { d2: "/s/dept-2.sock" }, seats: [[undefined, 3], ["/s/dept-2.sock", 3]] });
      const slug = throwIn === "isBase" ? "unknown-slug" : "d2";
      expect({ 던진곳: throwIn, 결과: await locateStarvedSeat(slug, 3, look) }).toEqual({ 던진곳: throwIn, 결과: null });
    }
  });
});

describe("main.ts 배선 — queue.starved 분기(name-우선 · 끝의 return · 누를 때만 이동)", () => {
  const src = read("./main.ts");
  // 줄 머리 주석·꼬리 주석을 걷은 본문(usagewiring.test.ts 와 같은 규칙) — 설명문이 낱말을 언급해도 핀이 깨지지 않는다.
  const stripComments = (s: string): string =>
    s
      .split("\n")
      .map((l) => (l.trimStart().startsWith("//") ? "" : l.replace(/\s\/\/.*$/, "")))
      .join("\n");
  const code = stripComments(src);
  /** 열 0 의 `function name(` 부터 첫 열 0 닫는 중괄호까지(`async function` 도 같은 문자열로 잡힌다). */
  const fnBody = (name: string): string => {
    const i = code.indexOf(`function ${name}(`);
    expect({ 함수: name, 존재: i >= 0 }).toEqual({ 함수: name, 존재: true });
    const end = code.indexOf("\n}\n", i);
    return code.slice(i, end > i ? end + 3 : undefined);
  };
  /** onDaemonEvent 안 두 칸 들여쓴 name-우선 분기 하나(`if (name === "…") {` ~ 그 분기의 닫는 두 칸 중괄호). */
  const branch = (evName: string): string => {
    const head = `  if (name === "${evName}") {`;
    const i = code.indexOf(head);
    expect({ 분기: evName, 존재: i >= 0 }).toEqual({ 분기: evName, 존재: true });
    const end = code.indexOf("\n  }\n", i);
    expect({ 분기: evName, 닫힘: end > i }).toEqual({ 분기: evName, 닫힘: true });
    return code.slice(i, end + 5);
  };
  const handler = fnBody("onDaemonEvent");

  it("★이 이벤트의 name-우선 분기는 정확히 하나이고, category 폴백 사슬(health → watchdog → feed → surface.*)보다 앞에 있다", () => {
    expect(code.split('name === "queue.starved"').length - 1).toBe(1);
    const at = handler.indexOf('if (name === "queue.starved") {');
    const chain = handler.indexOf('if (category === "health") {');
    expect({ 분기: at >= 0, 폴백_사슬: chain >= 0, 분기가_앞: at >= 0 && chain > at }).toEqual({ 분기: true, 폴백_사슬: true, 분기가_앞: true });
  });
  it("starvedNotice 로 문구를 만들고 stickyToast(id, \"health\", 제목, 상세, …) 로만 올린다 — 같은 id 라 갱신된다", () => {
    const b = branch("queue.starved");
    expect(b).toContain("const starved = starvedNotice(payload, event.socket_slug);");
    expect(b).toContain('stickyToast(starved.id, "health", starved.title, starved.detail, ');
    expect(b.split("stickyToast(").length - 1).toBe(1);
    expect(/(^|[^A-Za-z])toast\(/.test(b)).toBe(false); // 일회성 toast( 를 따로 부르지 않는다(같은 좌석의 토스트가 쌓이지 않는다)
  });
  it("OS 배너는 humanNeeded 일 때만 · 같은 제목·상세로 — 데몬 쿨다운이 빈도를 묶는다(GUI 쪽 타이머 0)", () => {
    const b = branch("queue.starved");
    expect(b).toContain("if (starved.humanNeeded) osBanner(starved.title, starved.detail);");
    expect(b.split("osBanner(").length - 1).toBe(1);
    for (const timer of ["setTimeout", "setInterval", "Date.now("]) expect({ 금지: timer, 있음: b.includes(timer) }).toEqual({ 금지: timer, 있음: false });
  });
  it("★분기는 return 으로 끝난다 — 폴백 레인을 타지 않는다(이중 표시 금지)", () => {
    const b = branch("queue.starved");
    expect(/\n    return;\n  \}\n$/.test(b)).toBe(true);
    // 새 payload 가 null 이어도(잘못된 payload) return 은 if (starved) 밖에 있다 — 폴백으로 새지 않는다
    expect(b.indexOf("return;") > b.indexOf("if (starved) {")).toBe(true);
    expect(/if \(starved\) \{[\s\S]*?\n    \}\n    return;/.test(b)).toBe(true);
  });
  it("★포커스는 클릭 처리기 안에서만 — 이벤트가 올 때 좌석·탭·패널을 건드리는 코드가 분기에 없다(포커스 강탈 금지)", () => {
    const b = branch("queue.starved");
    expect(b.split("focusStarvedSeat(").length - 1).toBe(1);
    expect(b).toContain("() => void focusStarvedSeat(slug, seat)");
    for (const direct of ["jumpToSurface(", "setFocus(", "render(", "renderWsTabs(", "openFeed(", "setCcOpen(", "switchToWorkspaceBySocket(", "activeWs", "scrollIntoView", ".focus("])
      expect({ 금지: direct, 있음: b.includes(direct) }).toEqual({ 금지: direct, 있음: false });
  });
  it("누를 좌석을 못 정하면(번호 해석 불가) 클릭 처리기를 달지 않는다 · slug 가 문자열이 아니면 빈 값(= 본부)", () => {
    const b = branch("queue.starved");
    expect(b).toContain("seat === null ? undefined : ");
    expect(b).toContain("const seat = surfaceIdOfRef(payload.surface_ref);");
    expect(b).toContain('const slug = typeof event.socket_slug === "string" ? event.socket_slug : "";');
  });
  it("화면 문자열은 textContent 경로(stickyToast)로만 — 분기·도우미 함수에 innerHTML 0", () => {
    for (const part of [branch("queue.starved"), branch("queue.delivered"), fnBody("dismissStarvedToast"), fnBody("focusStarvedSeat")])
      expect(part.includes("innerHTML")).toBe(false);
  });
  it("starvednotice 의 도우미를 import 한다", () => {
    expect(code).toContain('import { starvedNotice, starvedDismissId, surfaceIdOfRef, locateStarvedSeat } from "./starvednotice";');
  });
});

describe("main.ts 배선 — 풀림(queue.delivered)·좌석 종료가 토스트를 거둔다", () => {
  const src = read("./main.ts");
  const stripComments = (s: string): string =>
    s
      .split("\n")
      .map((l) => (l.trimStart().startsWith("//") ? "" : l.replace(/\s\/\/.*$/, "")))
      .join("\n");
  const code = stripComments(src);
  const fnBody = (name: string): string => {
    const i = code.indexOf(`function ${name}(`);
    expect({ 함수: name, 존재: i >= 0 }).toEqual({ 함수: name, 존재: true });
    const end = code.indexOf("\n}\n", i);
    return code.slice(i, end > i ? end + 3 : undefined);
  };
  const handler = fnBody("onDaemonEvent");

  it("queue.delivered: 같은 좌석의 토스트를 거두고 return(다른 처리 없음)", () => {
    expect(code.split('name === "queue.delivered"').length - 1).toBe(1);
    const i = handler.indexOf('if (name === "queue.delivered") {');
    expect(i).toBeGreaterThan(0);
    const b = handler.slice(i, handler.indexOf("\n  }\n", i) + 5);
    expect(b).toContain("dismissStarvedToast(event.socket_slug, sid);");
    expect(/\n    return;\n  \}\n$/.test(b)).toBe(true);
    for (const other of ["toast(", "stickyToast(", "osBanner(", "refreshFeed(", "refreshSidebarStatus("])
      expect({ 금지: other, 있음: b.includes(other) }).toEqual({ 금지: other, 있음: false });
    expect(handler.indexOf('if (name === "queue.delivered") {')).toBeLessThan(handler.indexOf('if (category === "health") {'));
  });
  it("surface.exited·closed·reaped: 기존 분기의 맨 앞(slug 미해석 조기 return 보다 앞)에서 거둔다 — 기존 줄은 그대로", () => {
    const iExit = handler.indexOf('} else if (name === "surface.exited" || name === "surface.closed" || name === "surface.reaped") {');
    expect(iExit).toBeGreaterThan(0);
    const iDismiss = handler.indexOf("dismissStarvedToast(event.socket_slug, sid);", iExit);
    const iGuard = handler.indexOf("if (event.socket_slug && !sock) return;", iExit);
    const iRemove = handler.indexOf('removeDeadPane(Number(sid), sock, name !== "surface.closed" && !isClosingSid(Number(sid), sock));', iExit);
    expect({ 거둠: iDismiss > iExit, 조기_return_앞: iDismiss < iGuard, 제거_앞: iDismiss < iRemove }).toEqual({ 거둠: true, 조기_return_앞: true, 제거_앞: true });
  });
  it("dismissStarvedToast 는 starvednotice 가 정한 id 로 기존 dismissToast 를 부른다(id 모양의 진실원은 한 곳)", () => {
    const b = fnBody("dismissStarvedToast");
    expect(b).toContain("starvedDismissId(socketSlug, surfaceId)");
    expect(b).toContain("if (id) dismissToast(id);");
    expect(code.includes("`starved:")).toBe(false); // main.ts 가 id 모양을 따로 만들지 않는다
    expect(code.includes('"starved:')).toBe(false);
  });
  it("백엔드 계약 — isBaseSlug 가 기대는 사실: attach_surface 의 이벤트 이름에 소켓 slug 가 들어 있고, 모든 이벤트의 socket_slug 는 같은 sock_slug 가 만든다(본부 포워더도 같다)", () => {
    const rs = read("../../src-tauri/src/main.rs");
    const a = rs.indexOf("async fn attach_surface(");
    expect(a).toBeGreaterThan(0);
    const attach = rs.slice(a, rs.indexOf("\n}\n", a));
    expect(attach).toContain("let slug = sock_slug(&sock);");
    expect(attach).toContain('"output_event": format!("surface-output-{slug}-{surface_id}"),');
    const f = rs.indexOf("fn spawn_event_forwarder(");
    expect(f).toBeGreaterThan(0);
    const fwd = rs.slice(f, rs.indexOf("\n}\n", f));
    expect(fwd).toContain("let slug = sock_slug(&socket);");
    expect(fwd).toContain('obj.insert("socket_slug".into(), json!(slug));');
    // 본부(기본) 소켓도 같은 포워더를 쓴다 — 그래서 본부 이벤트에도 slug 가 실리지만 socketForSlug(부서만 등록)에는 없다
    expect(rs).toContain("spawn_event_forwarder(handle.clone(), default_socket());");
  });
  it("focusStarvedSeat: 좌석 확정(locateStarvedSeat)이 없으면 return · 확정되면 jumpToSurface(그 소켓) — 호출 지점은 클릭 처리기 하나", () => {
    const b = fnBody("focusStarvedSeat");
    expect(b).toContain("await locateStarvedSeat(socketSlug, sid, {");
    expect(b).toContain("deptSocket: (slug) => socketForSlug.get(slug),");
    expect(b).toContain('invoke("attach_surface", { socket: null, surfaceId: id })');
    expect(b).toContain("ev.output_event.includes(slug)");
    expect(b).toContain("!w.pending && (w.socket ?? undefined) === socket && collectSids(w.tree).includes(id)");
    const iNone = b.indexOf("if (!seat) return;");
    const iCc = b.indexOf("if (ccOpen) setCcOpen(false);");
    const iJump = b.indexOf("jumpToSurface(sid, seat.socket);");
    expect({ 없으면_return: iNone > 0, 패널_닫기: iCc > iNone, 이동: iJump > iCc }).toEqual({ 없으면_return: true, 패널_닫기: true, 이동: true });
    expect(b.split("jumpToSurface(").length - 1).toBe(1); // 번호만으로 고르는 다른 이동 경로 없음
    expect(code.split("focusStarvedSeat(").length - 1).toBe(2); // 정의 1 + 클릭 처리기 1
  });
});

describe("main.ts 배선 — 이벤트 분기의 실제 본문을 대역 위에서 실행(문자열 핀을 우회하는 변형 차단)", () => {
  const src = read("./main.ts");
  const stripComments = (s: string): string =>
    s
      .split("\n")
      .map((l) => (l.trimStart().startsWith("//") ? "" : l.replace(/\s\/\/.*$/, "")))
      .join("\n");
  const code = stripComments(src);
  const branchText = (evName: string): string => {
    const i = code.indexOf(`  if (name === "${evName}") {`);
    expect({ 분기: evName, 존재: i >= 0 }).toEqual({ 분기: evName, 존재: true });
    return code.slice(i, code.indexOf("\n  }\n", i) + 5);
  };
  type Call = { fn: string; args: unknown[] };
  /** 분기 본문을 그대로 실행한다 — 분기가 return 하면 undefined, 안 하고 지나가면 "fell-through". */
  function run(evName: string, event: Record<string, unknown>, p: unknown): { ret: unknown; calls: Call[] } {
    const calls: Call[] = [];
    const rec = (fn: string) => (...args: unknown[]) => {
      calls.push({ fn, args });
    };
    const deps = {
      starvedNotice,
      surfaceIdOfRef,
      stickyToast: rec("stickyToast"),
      osBanner: rec("osBanner"),
      focusStarvedSeat: rec("focusStarvedSeat"),
      dismissStarvedToast: rec("dismissStarvedToast"),
      toast: rec("toast"),
    };
    const fn = new Function("deps", "name", "event", "payload", "sid", `with (deps) {\n${branchText(evName)}\n}\nreturn "fell-through";`) as (
      ...a: unknown[]
    ) => unknown;
    const ret = fn(deps, String(event.name), event, p, event.surface_id);
    return { ret, calls };
  }

  it("queue.starved(사람 조치 필요): stickyToast 1회(health·제목·상세) + OS 배너 1회(같은 제목·상세) · return · 이벤트 시점에 포커스 이동 0", () => {
    const ev = { name: "queue.starved", socket_slug: "abc", surface_id: 12 };
    const { ret, calls } = run("queue.starved", ev, payload());
    expect(ret).toBeUndefined();
    const n = starvedNotice(payload(), "abc")!;
    expect(calls.map((c) => c.fn)).toEqual(["stickyToast", "osBanner"]);
    expect(calls[0].args.slice(0, 4)).toEqual([n.id, "health", n.title, n.detail]);
    expect(calls[1].args).toEqual([n.title, n.detail]);
    expect(calls.filter((c) => c.fn === "focusStarvedSeat")).toEqual([]); // 자동 전환·포커스 강탈 없음
  });
  it("누르면 그 좌석으로 — 클릭 처리기가 (slug, 번호) 로 focusStarvedSeat 를 부른다(클릭 전에는 0회)", () => {
    const { calls } = run("queue.starved", { name: "queue.starved", socket_slug: "abc", surface_id: 12 }, payload());
    const onClick = calls[0].args[4] as (() => void) | undefined;
    expect(typeof onClick).toBe("function");
    expect(calls.filter((c) => c.fn === "focusStarvedSeat").length).toBe(0);
    (onClick as () => void)();
    expect(calls.filter((c) => c.fn === "focusStarvedSeat").map((c) => c.args)).toEqual([["abc", 12]]);
  });
  it("calm 사유(approval·paused·wait): 토스트만 — OS 배너 0", () => {
    for (const code of ["approval", "paused", "wait"]) {
      const { ret, calls } = run("queue.starved", { name: "queue.starved", socket_slug: "abc", surface_id: 12 }, payload({ remedy_code: code }));
      expect({ 코드: code, 호출: calls.map((c) => c.fn), ret }).toEqual({ 코드: code, 호출: ["stickyToast"], ret: undefined });
    }
  });
  // ★R2F-UI(A3 m1): 이름·기대를 고쳤다 — 종전 「구버전 payload(remedy_code 없음): 토스트 + OS 배너(모르면 알린다)」 는 기본 legacyPayload(blocked_by=`busy`)로 배너를 기대했다. 코드가 없는 payload 는 이제
  //   막힘 사유로 가른다 — 스스로 풀리는 사유(busy)는 토스트만 · 사람 조치 사유(입력줄)는 종전처럼 토스트 + OS 배너.
  it("구버전 payload(remedy_code 없음) · 사람 조치 사유(입력줄): 토스트 + OS 배너(모르면 알린다)", () => {
    const { calls } = run("queue.starved", { name: "queue.starved", surface_id: 3 }, legacyPayload({ blocked_by: "input_pending(입력줄에 미제출 입력)" }));
    expect(calls.map((c) => c.fn)).toEqual(["stickyToast", "osBanner"]);
    expect(calls[0].args[0]).toBe("starved:base:surface:3"); // socket_slug 없음 → base
    expect(calls[0].args[3]).toBe("10분째 · 막힘 사유: input_pending(입력줄에 미제출 입력)");
  });
  it("★R2F-UI(A3 m1) 구버전 payload(remedy_code 없음) · 스스로 풀리는 사유(busy)·승인 대기·일시정지: 토스트만 — OS 배너 0(좌석당 5분 간격 배너가 고우선 통로를 묽히던 것)", () => {
    for (const reason of ["busy", "busy(출력 중)", "approval_pending(승인·관문 대기)", "queue_paused(헬스 조치)", "paused(kill-switch 동결)", "human_typing(사람이 입력 중)"]) {
      const { ret, calls } = run("queue.starved", { name: "queue.starved", surface_id: 3 }, legacyPayload({ blocked_by: reason }));
      expect({ 사유: reason, 호출: calls.map((c) => c.fn), ret }).toEqual({ 사유: reason, 호출: ["stickyToast"], ret: undefined });
      expect(calls[0].args[3]).toBe(`10분째 · 막힘 사유: ${reason}`); // 토스트는 그대로 뜬다
    }
  });
  it("socket_slug 가 문자열이 아니면 클릭 처리기는 빈 slug(= 본부)로 부른다", () => {
    const { calls } = run("queue.starved", { name: "queue.starved", socket_slug: 77, surface_id: 3 }, legacyPayload());
    (calls[0].args[4] as () => void)();
    expect(calls.filter((c) => c.fn === "focusStarvedSeat").map((c) => c.args)).toEqual([["", 3]]);
  });
  it("잘못된 payload: 아무것도 띄우지 않지만 분기는 return 한다(폴백으로 새지 않는다)", () => {
    for (const bad of [null, undefined, "x", 7, [], {}, { surface_ref: "surface:abc" }, { surface_ref: 12 }]) {
      const { ret, calls } = run("queue.starved", { name: "queue.starved", socket_slug: "abc", surface_id: 12 }, bad);
      expect({ payload: JSON.stringify(bad) ?? "undefined", 호출: calls.length, ret }).toEqual({ payload: JSON.stringify(bad) ?? "undefined", 호출: 0, ret: undefined });
    }
  });
  it("번호를 정수로 못 읽는 surface_ref(20자리)는 토스트는 띄우되 클릭 처리기를 달지 않는다", () => {
    const { calls } = run("queue.starved", { name: "queue.starved", socket_slug: "abc" }, payload({ surface_ref: "surface:" + "9".repeat(20) }));
    expect(calls[0].fn).toBe("stickyToast");
    expect(calls[0].args[4]).toBeUndefined();
  });
  it("queue.delivered: (slug, 좌석 번호)로 거두고 return — 토스트·배너를 새로 띄우지 않는다", () => {
    const { ret, calls } = run("queue.delivered", { name: "queue.delivered", socket_slug: "abc", surface_id: 12 }, { surface_ref: "surface:12" });
    expect(ret).toBeUndefined();
    expect(calls).toEqual([{ fn: "dismissStarvedToast", args: ["abc", 12] }]);
  });
});

describe("새 순수 모듈 starvednotice.ts — 불변식(최상위 부수효과 0 · 문서/창/저장소 0 · 구형 WKWebView 비호환 문법 0)", () => {
  const raw = read("./starvednotice.ts");
  const stripLine = (s: string): string =>
    s
      .split("\n")
      .map((l) => (l.trimStart().startsWith("//") ? "" : l.replace(/\s\/\/.*$/, "")))
      .join("\n");
  const code = stripLine(raw.replace(/\/\*[\s\S]*?\*\//g, ""));
  it("원문(주석 포함)에 document·window·localStorage 문자열이 없다(티켓 문면)", () => {
    for (const w of ["document", "window", "localStorage"]) expect({ 낱말: w, 있음: raw.includes(w) }).toEqual({ 낱말: w, 있음: false });
  });
  it("전역 부수효과 표면(저장소·문서·창·타이머·IPC) 0", () => {
    for (const bad of ["localStorage", "sessionStorage", "indexedDB", "document.", "window.", "navigator", "setInterval", "setTimeout", "__TAURI__", "invoke(", "fetch("])
      expect({ 표면: bad, 있음: code.includes(bad) }).toEqual({ 표면: bad, 있음: false });
  });
  it("구형 WKWebView 비호환 문법 0", () => {
    for (const bad of ["(?<=", "(?<!", ".at(", "findLast", "structuredClone", "Object.hasOwn", "replaceAll("])
      expect({ 문법: bad, 있음: code.includes(bad) }).toEqual({ 문법: bad, 있음: false });
  });
  it("최상위 문장은 선언뿐", () => {
    const bad = code
      .split("\n")
      .filter((l) => l.length > 0 && !/^\s/.test(l))
      .filter((l) => !/^(import |export |const |function |interface |type |\}|\)|\]|;)/.test(l));
    expect({ 최상위_비선언: bad }).toEqual({ 최상위_비선언: [] });
  });
  it("화면·저장소·조치를 일으키는 코드가 없다 — 순수 함수와 주입받은 조회만(키 주입·강제 배달·화면 전환 낱말 0)", () => {
    for (const bad of ["send_input", "send_key", "queue.deliver", "force", "jumpToSurface", "setFocus", "innerHTML", "textContent"])
      expect({ 낱말: bad, 있음: code.includes(bad) }).toEqual({ 낱말: bad, 있음: false });
  });
});
