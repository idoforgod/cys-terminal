// 0.14.44 (WC · D1 · D2) 사용량 칸 보기 방식 — 순수 판정 핀(usagebar.ts · DOM·Tauri 불요).
//
// 지키는 것(설계 WP-D):
//   · buildUsageBarModel 의 선택 인자를 생략하면 "한 계정만"(= 0.14.43 동작) — 기존 시험 1,601줄(usagebar.test.ts)이 무수정으로 통과하는 근거.
//   · 자동(기본): 관측 계정 3개 이하 → 전부 막대 상자 · 4개 이상 → 제공자 묶음(대표 상자 + 나머지 줄 · 묶음당 8줄 · 넘으면 "외 N개" · 접을 수 있음) · 배율 1.6 이상이면 한 줄형.
//   · 모두: 전 관측 계정이 막대 · 배율과 무관 · 한 계정만: 주 계정 하나(boxes = [primary]).
//   · D2: 상자마다 리셋 시각과 소진 예상(값이 있고 · 지나지 않았고 · 관측 30분 안일 때만) · Antigravity 도 같은 상자 구조.
import { describe, it, expect } from "bun:test";
import {
  buildUsageBarModel,
  sanitizeUsageMode,
  nextUsageMode,
  sanitizeFoldKeys,
  USAGE_MODES,
  USAGE_MODE_LABEL,
  USAGE_AUTO_BOXES_MAX,
  USAGE_COMPACT_SCALE,
  USAGE_FOLD_KEYS_MAX,
  USAGE_OTHERS_MAX,
  type AcctRow,
  type UsageViewMode,
} from "./usagebar";

const NOW = 1_800_000_000;
let seq = 0;
const acct = (o: Partial<AcctRow>): AcctRow => ({
  provider: "claude",
  account_id: `id-${++seq}`,
  label: `owner${seq}@example.com`,
  plan: null,
  profiles: [],
  rate: [
    { label: "5h", used_pct: 10 + (seq % 50), resets_at: NOW + 3600 },
    { label: "7d", used_pct: 20, resets_at: NOW + 86400 },
  ],
  updated_at: NOW - 30,
  stale_secs: 30,
  source: "statusline",
  adapter: true,
  ...o,
});
const fetchOk = { everOk: true, failStreak: 0, okAtSec: NOW };
const noRedact = (s: string) => s;
const build = (accts: AcctRow[], mode?: UsageViewMode | unknown, fontScale?: number, folded?: Set<string>) =>
  buildUsageBarModel(accts, NOW, fetchOk, noRedact, false, undefined, mode === undefined && fontScale === undefined && !folded ? undefined : { mode, fontScale, folded });
const many = (n: number, provider = "claude"): AcctRow[] =>
  Array.from({ length: n }, (_, i) => acct({ provider, profiles: [`/Users/x/.claude-${i + 1}`], current_profiles: [`/Users/x/.claude-${i + 1}`], updated_at: NOW - 30 - i }));

describe("D1 — 선택 인자를 생략하면 한 계정만(종전 동작)", () => {
  it("opts 없음 → mode one · boxes = [primary] · groups 없음 · 한 줄형 아님 · others/moreCount 는 종전 규칙", () => {
    const m = build(many(12));
    expect(m.mode).toBe("one");
    expect(m.compact).toBe(false);
    expect(m.groups).toEqual([]);
    expect(m.boxes.length).toBe(1);
    expect(m.boxes[0]).toBe(m.primary as typeof m.boxes[0]);
    expect(m.others.length).toBe(USAGE_OTHERS_MAX);
    expect(m.moreCount).toBe(12 - 1 - USAGE_OTHERS_MAX);
  });
  it("opts 는 있는데 mode 가 이상한 값이면 한 계정만으로 떨어진다(저장소·IPC 값 의심)", () => {
    for (const bad of [undefined, null, "", "ALL", 3, {}, "auto "]) expect(build(many(5), bad as unknown, 1).mode).toBe("one");
  });
  it("sanitizeUsageMode: 저장소 기본은 auto · 세 값만 통과 · 전환 순서 자동 → 모두 → 하나 → 자동", () => {
    expect(sanitizeUsageMode(null)).toBe("auto");
    expect(sanitizeUsageMode("x")).toBe("auto");
    expect(USAGE_MODES.map((m) => sanitizeUsageMode(m))).toEqual(["auto", "all", "one"]);
    expect(nextUsageMode("auto")).toBe("all");
    expect(nextUsageMode("all")).toBe("one");
    expect(nextUsageMode("one")).toBe("auto");
    expect(USAGE_MODES.map((m) => USAGE_MODE_LABEL[m])).toEqual(["자동", "모두", "하나"]);
  });
  it("sanitizeFoldKeys: 배열 · 문자열 원소 · 최대 개수", () => {
    expect([...sanitizeFoldKeys(["claude", 3, "", "codex"])]).toEqual(["claude", "codex"]);
    expect(sanitizeFoldKeys("claude").size).toBe(0);
    expect(sanitizeFoldKeys(Array.from({ length: 99 }, (_, i) => `k${i}`)).size).toBe(USAGE_FOLD_KEYS_MAX);
  });
});

describe("D1 — 계정 1~12개 × 방식 셋 × 배율 1 · 1.6 · 2.2 의 상자·줄 수", () => {
  for (let n = 1; n <= 12; n++) {
    for (const scale of [1, 1.6, 2.2]) {
      it(`계정 ${n}개 · 배율 ${scale}`, () => {
        const accts = many(n);
        const one = build(accts, "one", scale);
        expect({ 상자: one.boxes.length, 묶음: one.groups.length, 한줄형: one.compact }).toEqual({ 상자: 1, 묶음: 0, 한줄형: false });
        const all = build(accts, "all", scale);
        expect({ 상자: all.boxes.length, 묶음: all.groups.length, 한줄형: all.compact, 줄: all.others.length }).toEqual({ 상자: n, 묶음: 0, 한줄형: false, 줄: 0 }); // 모두 = 배율과 무관하게 막대
        const auto = build(accts, "auto", scale);
        expect(auto.compact).toBe(scale >= USAGE_COMPACT_SCALE); // 자동 ∧ 배율 ≥ 1.6 → 한 줄형
        if (n <= USAGE_AUTO_BOXES_MAX) {
          expect({ 상자: auto.boxes.length, 묶음: auto.groups.length }).toEqual({ 상자: n, 묶음: 0 });
        } else {
          // 한 제공자뿐이면 묶음 하나 — 대표 1개 + 나머지 줄(최대 8) + 넘으면 외 N개
          expect(auto.groups.length).toBe(1);
          expect(auto.boxes.length).toBe(1);
          const g = auto.groups[0];
          expect(g.count).toBe(n - 1);
          expect(g.lines.length).toBe(Math.min(n - 1, USAGE_OTHERS_MAX));
          expect(g.moreCount).toBe(Math.max(0, n - 1 - USAGE_OTHERS_MAX));
          expect(g.box.key).toBe(auto.boxes[0].key);
        }
      });
    }
  }
  it("한 제공자에 계정 9개 이상: 묶음 8줄 + '외 N개'(툴팁에 잘린 라벨)", () => {
    const g = build(many(12), "auto", 1).groups[0];
    expect(g.lines.length).toBe(8);
    expect(g.moreCount).toBe(3);
    expect(g.moreTooltip.split(" · ").length).toBe(3);
  });
});

describe("D1 — 제공자 순서 · 대표 선정 · 접기 상태", () => {
  const mixed = () => [
    ...many(2, "codex").map((a, i) => ({ ...a, rate: [{ label: "7d", used_pct: 40 + i, resets_at: NOW + 86400 }] })),
    ...many(3, "claude"),
    acct({ provider: "antigravity", profiles: [], rate: [{ label: "5h", used_pct: 5, resets_at: NOW + 3600 }, { label: "7d", used_pct: 9, resets_at: NOW + 86400 }] }),
  ];
  it("묶음은 claude → codex → antigravity 순 · 각 묶음의 대표는 그 제공자 안의 주 계정 규칙(사용 중 우선)", () => {
    const accts = mixed();
    const inUse = accts.filter((a) => a.provider === "claude")[2];
    inUse.in_use = true;
    inUse.updated_at = NOW - 500; // 가장 덜 신선해도 사용 중이면 대표
    const m = build(accts, "auto", 1);
    expect(m.groups.map((g) => g.key)).toEqual(["claude", "codex", "antigravity"]);
    expect(m.groups[0].box.key).toBe(`claude:${inUse.account_id}`);
    expect(m.groups[0].count).toBe(2);
    expect(m.groups[1].count).toBe(1);
    expect(m.groups[2].count).toBe(0);
    expect(m.boxes.map((b) => b.key)).toEqual(m.groups.map((g) => g.box.key));
  });
  it("종전 primary 는 뜻이 그대로다 — 전체에서 고른 주 계정(접힌 머리줄 요약이 쓴다)", () => {
    const accts = mixed();
    const one = build(accts, "one", 1);
    const auto = build(accts, "auto", 1);
    expect(auto.primary).toEqual(one.primary);
    expect(auto.headline).toBe(one.headline);
  });
  it("접힌 묶음: lines 가 비고 count 는 그대로 · 모델이 달라진다(다시 그리기 판정에 반영)", () => {
    const accts = mixed();
    const open = build(accts, "auto", 1);
    const fold = build(accts, "auto", 1, new Set(["claude"]));
    expect(fold.groups[0].folded).toBe(true);
    expect(fold.groups[0].lines).toEqual([]);
    expect(fold.groups[0].count).toBe(open.groups[0].count);
    expect(fold.groups[1].folded).toBe(false);
    expect(JSON.stringify(fold)).not.toBe(JSON.stringify(open));
  });
  it("보기 방식이 바뀌어도 모델이 달라진다(본문 시그니처 = 모델 전체)", () => {
    const accts = many(3);
    const sigs = USAGE_MODES.map((m) => JSON.stringify(build(accts, m, 1)));
    expect(new Set(sigs).size).toBe(3);
  });
  it("관측 전 계정은 어느 방식에서도 줄로 남는다(사라지지 않는다) — 한 계정만이 아니면 others 에는 관측 전 줄만", () => {
    const accts = [...many(2), acct({ provider: "codex", updated_at: null, rate: [] })];
    const all = build(accts, "all", 1);
    expect(all.boxes.length).toBe(2);
    expect(all.others.map((l) => l.unobserved)).toEqual([true]);
    expect(all.unobservedCount).toBe(1);
    const one = build(accts, "one", 1);
    expect(one.others.length).toBe(2); // 관측 줄 1 + 관측 전 줄 1
  });
  it("계정이 없거나 관측이 없으면 상자·묶음은 비어 있다(안내 문구는 종전 그대로)", () => {
    const none = build([], "auto", 1);
    expect({ b: none.boxes.length, g: none.groups.length, hl: none.headline }).toEqual({ b: 0, g: 0, hl: "관측 없음" });
    const unobs = build([acct({ updated_at: null, rate: [] })], "all", 1);
    expect({ b: unobs.boxes.length, hl: unobs.headline }).toEqual({ b: 0, hl: "관측 전 1계정" });
  });
  it("숨긴 계정은 상자·묶음에서 빠진다", () => {
    const accts = many(5);
    const hidden = new Set([`claude:${accts[0].account_id}`, `claude:${accts[1].account_id}`]);
    const m = buildUsageBarModel(accts, NOW, fetchOk, noRedact, false, hidden, { mode: "all", fontScale: 1 });
    expect(m.boxes.length).toBe(3);
    expect(m.hiddenCount).toBe(2);
  });
});

describe("D2 — 상자마다 리셋 시각 · 소진 예상 · Antigravity 도 같은 상자", () => {
  const base = (o: Partial<AcctRow>) => acct({ rate: [{ label: "5h", used_pct: 50, resets_at: NOW + 3600 }, { label: "7d", used_pct: 20, resets_at: NOW + 86400 }], ...o });
  it("Antigravity 행이 Claude 행과 같은 상자 구조(이름 줄 · 5h/7d 막대 · 리셋 시각)로 나온다", () => {
    const c = base({ provider: "claude", profiles: ["/Users/x/.claude-1"], current_profiles: ["/Users/x/.claude-1"] });
    const a = base({ provider: "antigravity", account_id: "agy-1", label: "Antigravity (agy)", source: "agy-statusline" });
    const m = build([c, a], "all", 1);
    expect(m.boxes.length).toBe(2);
    const shape = (b: (typeof m.boxes)[number]) => ({ 창: b.windows.map((w) => w.label), 리셋: b.windows.map((w) => w.resetText !== ""), 막대: b.windows.map((w) => w.pct !== null) });
    expect(shape(m.boxes[1])).toEqual(shape(m.boxes[0]));
    expect(m.boxes[1].label).toBe("Antigravity");
  });
  it("소진 예상은 상자마다 — 값 있음 · 지난 시각 · 오래된 관측 · 값 없음 4갈래", () => {
    const ok = base({ account_id: "ok", exhaust_at: NOW + 1800 });
    const past = base({ account_id: "past", exhaust_at: NOW - 10 });
    const old = base({ account_id: "old", exhaust_at: NOW + 1800, updated_at: NOW - 40 * 60 });
    const none = base({ account_id: "none", exhaust_at: null });
    const m = build([ok, past, old, none], "all", 1);
    const by = (id: string) => m.boxes.find((b) => b.key === `claude:${id}`)!;
    expect(/^이 속도면 \d\d:\d\d 소진$/.test(by("ok").exhaust)).toBe(true);
    expect(by("past").exhaust).toBe("");
    expect(by("old").exhaust).toBe("");
    expect(by("none").exhaust).toBe("");
  });
  it("주 계정이 아닌 상자에도 소진 예상이 붙는다(종전엔 주 계정에만)", () => {
    const a = base({ account_id: "a", exhaust_at: NOW + 600, updated_at: NOW - 5 });
    const b = base({ account_id: "b", exhaust_at: NOW + 1200, updated_at: NOW - 90 });
    const m = build([a, b], "all", 1);
    expect(m.boxes.filter((x) => x.exhaust !== "").length).toBe(2);
  });
  it("세 제공자 상자의 구조가 같다 — 어느 상자든 창 두 개(5h · 7d)", () => {
    const m = build(
      [base({ provider: "claude" }), base({ provider: "codex", rate: [{ label: "7d", used_pct: 30, resets_at: NOW + 86400 }] }), base({ provider: "antigravity" })],
      "auto",
      1,
    );
    expect(m.groups.length).toBe(0);
    expect(m.boxes.map((b) => b.windows.length)).toEqual([2, 2, 2]);
  });
});
