// U1 사이드바 사용량 패널 — 순수 판정 회귀 핀(usagebar.ts · DOM·Tauri 불요).
//
// 무엇을 지키는가(설계 §3 U1 + 반박 D1·D2·D7 반영):
//   · 주 계정은 **관측 출처**(라이브 source · updated_at≠null)로 고른다 — 경로 정규식은 동률 해소용일 뿐이다.
//     카탈로그로 계정 폴더가 `.claude-2` 처럼 좌석 경로가 아니어도 라이브 관측이면 주 계정이 된다(반박 D1).
//   · 라벨은 프로필명(claude-N / 좌석 / 부서 x / 제공자)만 — 이메일은 **툴팁에만**, 🔒 가림을 따른다(반박 D2).
//   · 100% 초과·창 누락('—')·리셋 지남(값 숨김)·오래됨(흐림)·응답 없음을 정직하게 표기한다.
//   · 조회 스로틀(부팅 유예·최소 간격·force)은 순수 함수가 정한다 — 새 타이머 없이 기존 10초 틱에 얹기 위해.
import { describe, it, expect } from "bun:test";
import {
  normalizeProfiles,
  accountShortLabel,
  isLiveAccount,
  windowView,
  freshness,
  pickPrimaryAccount,
  buildUsageBarModel,
  shouldFetchAccounts,
  USAGE_OTHERS_MAX,
  type AcctRow,
} from "./usagebar";

const NOW = 1_800_000_000; // 고정 시계(epoch 초)
const acct = (o: Partial<AcctRow>): AcctRow => ({
  provider: "claude",
  account_id: "id-" + Math.random().toString(16).slice(2, 8),
  label: "owner@example.com",
  plan: null,
  profiles: [],
  rate: [],
  updated_at: NOW - 30,
  stale_secs: 30,
  source: "statusline",
  adapter: true,
  ...o,
});
const win = (label: string, used_pct: number, resets_at: number | null = NOW + 3600) => ({ label, used_pct, resets_at });
const noRedact = (s: string) => s;

describe("프로필 표기 — 윈도우 구분자 정규화 후 중복 제거", () => {
  it("`\\` 와 `/` 가 섞인 같은 프로필은 한 줄로 접힌다(usage_accounts_all 의 문자열 dedup 이 못 접는 것)", () => {
    expect(normalizeProfiles([".cys\\claude", ".cys/claude", ".claude-4/"])).toEqual([".claude-4", ".cys/claude"]);
  });
  it("배열이 아니거나 문자열이 아닌 원소는 버린다(IPC 데이터 방어)", () => {
    expect(normalizeProfiles(undefined)).toEqual([]);
    expect(normalizeProfiles([1, null, ".claude-1"] as unknown)).toEqual([".claude-1"]);
  });
});

describe("계정 라벨 — 프로필명만, 이메일은 절대 라벨이 되지 않는다(반박 D2)", () => {
  it("홈 프로필 .claude-N 이 가장 먼저", () => {
    expect(accountShortLabel(acct({ profiles: [".cys/claude", ".claude-4"] }))).toBe("claude-4");
  });
  it("좌석 폴더(.cys/claude) → '좌석' · 윈도우 절대경로·역슬래시도 같다(비앵커)", () => {
    expect(accountShortLabel(acct({ profiles: [".cys/claude"] }))).toBe("좌석");
    expect(accountShortLabel(acct({ profiles: ["C:\\Users\\x\\.cys\\claude"] }))).toBe("좌석");
  });
  it("부서 포크 폴더(.cys/claude-<x>) → '부서 <x>' (default- 접두는 걷는다)", () => {
    expect(accountShortLabel(acct({ profiles: [".cys\\claude-default-dept-1"] }))).toBe("부서 dept-1");
    expect(accountShortLabel(acct({ profiles: [".cys/claude-sales"] }))).toBe("부서 sales");
  });
  it("프로필이 없으면 제공자 이름 — 이메일(label 필드)은 쓰지 않는다", () => {
    const a = acct({ profiles: [], label: "someone@corp.example" });
    expect(accountShortLabel(a)).toBe("Claude");
    expect(accountShortLabel(acct({ provider: "codex", label: "OpenAI Codex" }))).toBe("Codex");
    expect(accountShortLabel(acct({ provider: "gemini", label: "Antigravity (agy)" }))).toBe("agy");
    for (const p of [[], [".cys/claude"], ["/weird/place"]])
      expect(accountShortLabel(acct({ profiles: p, label: "leak@x.io" })).includes("@")).toBe(false);
  });
});

describe("라이브 관측 판정", () => {
  it("statusline·rollout·agy-rpc 관측 = 라이브", () => {
    for (const source of ["statusline", "rollout", "agy-rpc", "adapter:grok"])
      expect(isLiveAccount(acct({ source }))).toBe(true);
  });
  it("스냅샷 예열·관측 전·출처 빈값은 라이브가 아니다", () => {
    expect(isLiveAccount(acct({ source: "snapshot" }))).toBe(false);
    expect(isLiveAccount(acct({ updated_at: null }))).toBe(false);
    expect(isLiveAccount(acct({ source: "" }))).toBe(false);
  });
});

describe("창 표기 — 임계 70/90 · 클램프 · 누락 · 리셋 지남", () => {
  const view = (pct: number) => windowView(acct({ rate: [win("5h", pct)] }), "5h", NOW);
  it("임계 경계 69/70/89/90", () => {
    expect(view(69).sev).toBe("");
    expect(view(70).sev).toBe("warn");
    expect(view(89).sev).toBe("warn");
    expect(view(90).sev).toBe("crit");
  });
  it("100 초과는 '100%+'(게이지는 100에서 멈춤) — 실데이터에 101% 가 실재한다", () => {
    const v = view(101);
    expect(v.text).toBe("100%+");
    expect(v.pct).toBe(100);
    expect(v.sev).toBe("crit");
  });
  it("음수는 0 으로, NaN 은 누락('—')으로", () => {
    expect(view(-5).pct).toBe(0);
    expect(view(-5).text).toBe("0%");
    const nan = windowView(acct({ rate: [{ label: "5h", used_pct: Number.NaN, resets_at: null }] }), "5h", NOW);
    expect(nan.text).toBe("—");
    expect(nan.state).toBe("missing");
  });
  it("창이 없으면 0% 가 아니라 '—' (codex 는 5h 창이 없다)", () => {
    const v = windowView(acct({ provider: "codex", rate: [win("7d", 9)] }), "5h", NOW);
    expect(v).toMatchObject({ pct: null, text: "—", state: "missing", sev: "" });
  });
  it("리셋 시각이 지났으면 옛 % 를 보여 주지 않는다(리셋 뒤 빨간 78% 오경보 차단)", () => {
    const v = windowView(acct({ rate: [win("5h", 78, NOW - 1)] }), "5h", NOW);
    expect(v).toMatchObject({ pct: null, text: "리셋됨", state: "rolled", sev: "" });
    expect(v.resetText).toBe("재관측 대기");
  });
  it("리셋 표기 — 5h 는 시:분, 7d 는 월/일", () => {
    const r = NOW + 7200;
    const d = new Date(r * 1000);
    const p = (x: number) => String(x).padStart(2, "0");
    expect(windowView(acct({ rate: [win("5h", 10, r)] }), "5h", NOW).resetText).toBe(`리셋 ${p(d.getHours())}:${p(d.getMinutes())}`);
    expect(windowView(acct({ rate: [win("7d", 10, r)] }), "7d", NOW).resetText).toBe(`리셋 ${p(d.getMonth() + 1)}/${p(d.getDate())}`);
    expect(windowView(acct({ rate: [win("5h", 10, null)] }), "5h", NOW).resetText).toBe("");
  });
});

describe("신선도 — 관측 나이는 updated_at 과 로컬 시계로", () => {
  it("관측 전 → never", () => {
    expect(freshness(acct({ updated_at: null }), NOW).level).toBe("never");
  });
  it("스냅샷 → stale + '지난 기록'", () => {
    const f = freshness(acct({ source: "snapshot", updated_at: NOW - 600 }), NOW);
    expect(f.level).toBe("stale");
    expect(f.note).toContain("지난 기록");
  });
  it("2분 미만 fresh · 2분~30분 recent('N분 전 관측') · 30분 이상 stale", () => {
    expect(freshness(acct({ updated_at: NOW - 60 }), NOW)).toEqual({ level: "fresh", note: "" });
    expect(freshness(acct({ updated_at: NOW - 300 }), NOW)).toEqual({ level: "recent", note: "5분 전 관측" });
    expect(freshness(acct({ updated_at: NOW - 7200 }), NOW)).toEqual({ level: "stale", note: "2시간 전 관측" });
  });
});

describe("주 계정 — 관측 출처 기준(반박 D1) · 경로는 동률 해소용", () => {
  it("라이브 관측이 스냅샷을 이긴다(스냅샷 % 가 더 높아도)", () => {
    const live = acct({ account_id: "live", rate: [win("5h", 20)] });
    const snap = acct({ account_id: "snap", source: "snapshot", rate: [win("5h", 95)] });
    expect(pickPrimaryAccount([snap, live], NOW)?.account_id).toBe("live");
  });
  it("좌석 경로가 아닌 계정 폴더(.claude-2 — 카탈로그 부서)도 라이브면 주 계정이 된다", () => {
    const a = acct({ account_id: "cat", profiles: [".claude-2"], rate: [win("5h", 40)] });
    const never = acct({ account_id: "seat-never", profiles: [".cys/claude"], updated_at: null });
    expect(pickPrimaryAccount([never, a], NOW)?.account_id).toBe("cat");
  });
  it("라이브끼리는 5h 사용률이 높은 쪽(한도 임박 경보 목적 · CC KPI '최고 사용 계정'과 같은 축)", () => {
    const lo = acct({ account_id: "lo", rate: [win("5h", 10), win("7d", 90)] });
    const hi = acct({ account_id: "hi", rate: [win("5h", 60), win("7d", 5)] });
    expect(pickPrimaryAccount([lo, hi], NOW)?.account_id).toBe("hi");
  });
  it("5h 창이 없는 라이브(codex)는 5h 가 있는 라이브에 밀린다", () => {
    const codex = acct({ provider: "codex", account_id: "cx", rate: [win("7d", 50)] });
    const cl = acct({ account_id: "cl", rate: [win("5h", 1)] });
    expect(pickPrimaryAccount([codex, cl], NOW)?.account_id).toBe("cl");
  });
  it("동률이면 최신 관측 → 그래도 동률이면 좌석 경로(비앵커·두 구분자)", () => {
    const older = acct({ account_id: "older", updated_at: NOW - 100, rate: [win("5h", 30)] });
    const newer = acct({ account_id: "newer", updated_at: NOW - 10, rate: [win("5h", 30)] });
    expect(pickPrimaryAccount([older, newer], NOW)?.account_id).toBe("newer");
    const plain = acct({ account_id: "plain", profiles: [".claude-1"], rate: [win("5h", 30)] });
    const seat = acct({ account_id: "seat", profiles: ["C:\\Users\\x\\.cys\\claude"], rate: [win("5h", 30)] });
    plain.updated_at = seat.updated_at = NOW - 10;
    expect(pickPrimaryAccount([plain, seat], NOW)?.account_id).toBe("seat");
  });
  it("라이브가 없으면 스냅샷 중 최신 · 관측이 하나도 없으면 null", () => {
    const s1 = acct({ account_id: "s1", source: "snapshot", updated_at: NOW - 900 });
    const s2 = acct({ account_id: "s2", source: "snapshot", updated_at: NOW - 300 });
    expect(pickPrimaryAccount([s1, s2], NOW)?.account_id).toBe("s2");
    expect(pickPrimaryAccount([acct({ updated_at: null })], NOW)).toBeNull();
    expect(pickPrimaryAccount([], NOW)).toBeNull();
  });
});

describe("패널 모델 — 정직한 공백", () => {
  const ok = { everOk: true, failStreak: 0, okAtSec: NOW };
  it("첫 조회 전 → '사용량 확인 대기 중'", () => {
    const m = buildUsageBarModel([], NOW, { everOk: false, failStreak: 0, okAtSec: null }, noRedact);
    expect(m.primary).toBeNull();
    expect(m.message).toContain("대기");
    expect(m.footer).toBe("");
  });
  it("조회 성공·계정 0 → '아직 관측된 사용량 없음'", () => {
    const m = buildUsageBarModel([], NOW, ok, noRedact);
    expect(m.primary).toBeNull();
    expect(m.message).toContain("아직 관측된 사용량 없음");
  });
  it("전부 관측 전이면 개수만(목록은 툴팁)", () => {
    const m = buildUsageBarModel(
      [acct({ updated_at: null, profiles: [".claude-1"] }), acct({ updated_at: null, profiles: [".claude-2"] })],
      NOW, ok, noRedact,
    );
    expect(m.primary).toBeNull();
    expect(m.unobservedCount).toBe(2);
    expect(m.unobservedTooltip).toContain("claude-1");
    expect(m.message).toContain("아직 관측된 사용량 없음");
  });
  it("3회 연속 실패면 값은 유지하고 '데몬 응답 없음 — HH:MM 기준 값'", () => {
    const a = acct({ rate: [win("5h", 40)] });
    const m = buildUsageBarModel([a], NOW, { everOk: true, failStreak: 3, okAtSec: NOW - 120 }, noRedact);
    expect(m.primary).not.toBeNull();
    const d = new Date((NOW - 120) * 1000);
    const hhmm = `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
    expect(m.footer).toContain("데몬 응답 없음");
    expect(m.footer).toContain(hhmm);
    expect(buildUsageBarModel([a], NOW, { everOk: true, failStreak: 2, okAtSec: NOW }, noRedact).footer).toBe("");
  });
  it("한 번도 성공 못 하고 3회 실패 → 값 없이 응답 없음", () => {
    const m = buildUsageBarModel([], NOW, { everOk: false, failStreak: 3, okAtSec: null }, noRedact);
    expect(m.primary).toBeNull();
    expect(m.footer).toContain("데몬 응답 없음");
  });
  it("헤드라인 = 주 계정 5h·7d 요약 · codex 5h 누락은 '—'", () => {
    const a = acct({ rate: [win("5h", 78), win("7d", 31)] });
    expect(buildUsageBarModel([a], NOW, ok, noRedact).headline).toBe("5h 78% · 7d 31%");
    const cx = acct({ provider: "codex", rate: [win("7d", 9)] });
    expect(buildUsageBarModel([cx], NOW, ok, noRedact).headline).toBe("5h — · 7d 9%");
  });
  it("이메일은 라벨에 없고 툴팁에만 · 🔒 가림 함수를 거친다", () => {
    const a = acct({ label: "boss@corp.example", profiles: [".cys/claude"], rate: [win("5h", 5)] });
    const m = buildUsageBarModel([a], NOW, ok, (s) => `#${s.length}`);
    expect(m.primary!.label).toBe("좌석");
    expect(m.primary!.tooltip.includes("boss@corp.example")).toBe(false);
    expect(m.primary!.tooltip).toContain("#17");
    const plain = buildUsageBarModel([a], NOW, ok, noRedact);
    expect(plain.primary!.tooltip).toContain("boss@corp.example");
    expect(plain.primary!.label.includes("@")).toBe(false);
  });
  it("툴팁은 집계 범위를 정직하게 적는다(기본 claude·외부 터미널은 빠진다 — 반박 §5-8)", () => {
    const a = acct({ rate: [win("5h", 5)] });
    const t = buildUsageBarModel([a], NOW, ok, noRedact).primary!.tooltip;
    expect(t).toContain("외부 터미널");
  });
  it("나머지 관측 계정은 한 줄씩 · 주 계정 제외 · 상한 초과분은 개수", () => {
    const many = Array.from({ length: USAGE_OTHERS_MAX + 3 }, (_, i) =>
      acct({ account_id: `a${i}`, profiles: [`.claude-${i}`], rate: [win("5h", i)] }),
    );
    const m = buildUsageBarModel(many, NOW, ok, noRedact);
    expect(m.others.length).toBe(USAGE_OTHERS_MAX);
    expect(m.moreCount).toBe(2);
    expect(m.others.some((o) => o.label === m.primary!.label)).toBe(false);
  });
  it("오래된(스냅샷) 다른 계정은 흐리게 + 표기", () => {
    const live = acct({ account_id: "l", rate: [win("5h", 5)] });
    const snap = acct({ account_id: "s", source: "snapshot", profiles: [".claude-3"], updated_at: NOW - 900, rate: [win("5h", 60)] });
    const m = buildUsageBarModel([live, snap], NOW, ok, noRedact);
    expect(m.others[0].dim).toBe(true);
    expect(m.others[0].text).toContain("지난 기록");
  });
  it("소진 예측은 신선한 주 계정에서만", () => {
    const fresh = acct({ rate: [win("5h", 60)], exhaust_at: NOW + 1800 });
    expect(buildUsageBarModel([fresh], NOW, ok, noRedact).primary!.exhaust).toContain("소진");
    const old = acct({ rate: [win("5h", 60)], exhaust_at: NOW + 1800, updated_at: NOW - 7200 });
    expect(buildUsageBarModel([old], NOW, ok, noRedact).primary!.exhaust).toBe("");
  });
  it("라벨이 겹치는 두 계정은 구분 꼬리표가 붙는다(둘 다 'Claude' 로 보이지 않게)", () => {
    const a = acct({ account_id: "x1", rate: [win("5h", 50)] });
    const b = acct({ account_id: "x2", rate: [win("5h", 10)] });
    const m = buildUsageBarModel([a, b], NOW, ok, noRedact);
    expect(m.primary!.label).not.toBe(m.others[0].label);
  });
  it("비정상 입력(배열 아님·객체 아닌 원소)에도 던지지 않는다", () => {
    expect(() => buildUsageBarModel(null as unknown as AcctRow[], NOW, ok, noRedact)).not.toThrow();
    expect(() => buildUsageBarModel([null, 3, "x"] as unknown as AcctRow[], NOW, ok, noRedact)).not.toThrow();
    expect(() => buildUsageBarModel([acct({ rate: null as unknown as [] , profiles: null as unknown as [] })], NOW, ok, noRedact)).not.toThrow();
  });
});

describe("조회 스로틀 — 새 타이머 없이 10초 틱에 얹는 정책", () => {
  const base = { started: true, startedAtMs: 0, lastAttemptAtMs: null as number | null, force: false, graceMs: 15_000, minIntervalMs: 30_000 };
  it("복원 완료 전이면 조회하지 않는다(부트 체인 비개입)", () => {
    expect(shouldFetchAccounts(100_000, { ...base, started: false })).toBe(false);
    expect(shouldFetchAccounts(100_000, { ...base, startedAtMs: null })).toBe(false);
  });
  it("부팅 유예 안이면 조회하지 않는다(윈도우 기동 파이프 fan-out 회피)", () => {
    expect(shouldFetchAccounts(14_999, base)).toBe(false);
    expect(shouldFetchAccounts(15_000, base)).toBe(true);
  });
  it("최소 간격 30초", () => {
    expect(shouldFetchAccounts(50_000, { ...base, lastAttemptAtMs: 30_001 })).toBe(false);
    expect(shouldFetchAccounts(60_001, { ...base, lastAttemptAtMs: 30_001 })).toBe(true);
  });
  it("force(Control Center Live)는 유예·간격을 무시한다 — 종전 CC 동작 보존", () => {
    expect(shouldFetchAccounts(1, { ...base, started: false, force: true })).toBe(true);
    expect(shouldFetchAccounts(30_002, { ...base, lastAttemptAtMs: 30_001, force: true })).toBe(true);
  });
  it("시계가 뒤로 가도 영구 정지하지 않는다", () => {
    expect(shouldFetchAccounts(20_000, { ...base, lastAttemptAtMs: 999_999 })).toBe(true);
  });
});
