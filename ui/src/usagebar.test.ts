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
  profileTail,
  hasSeatProfile,
  acctAlias,
  acctKey,
  isOldObservation,
  isPreviousLogin,
  kpiCandidates,
  aggSeatRates,
  sanitizeHiddenKeys,
  USAGE_OTHERS_MAX,
  USAGE_ALIAS_MAX,
  USAGE_HIDDEN_MAX,
  USAGE_FOLD_TEXT_MAX,
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
      expect(accountShortLabel(acct({ profiles: p, label: "leak@example.com" })).includes("@")).toBe(false);
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
    expect({ pct: v.pct, text: v.text, state: v.state, sev: v.sev }).toEqual({ pct: null, text: "—", state: "missing", sev: "" });
  });
  it("리셋 시각이 지났으면 옛 % 를 보여 주지 않는다(리셋 뒤 빨간 78% 오경보 차단)", () => {
    const v = windowView(acct({ rate: [win("5h", 78, NOW - 1)] }), "5h", NOW);
    expect({ pct: v.pct, text: v.text, state: v.state, sev: v.sev }).toEqual({ pct: null, text: "리셋됨", state: "rolled", sev: "" });
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
  it("★좌석이 아닌 라이브 고사용 계정이 좌석 저사용 계정을 이긴다(좌석 우선으로 되돌리면 빨강 — 리뷰1 V2)", () => {
    // 반박 D1 이 기각한 '좌석 경로 우선'으로 비교 순서를 바꿔도 종전 사례들은 초록이었다 — 정면 대결 사례.
    const cat = acct({ account_id: "cat-hi", profiles: [".claude-2"], rate: [win("5h", 60)] });
    const seat = acct({ account_id: "seat-lo", profiles: [".cys/claude"], rate: [win("5h", 10)] });
    expect(pickPrimaryAccount([seat, cat], NOW)?.account_id).toBe("cat-hi");
    expect(pickPrimaryAccount([cat, seat], NOW)?.account_id).toBe("cat-hi");
    // 좌석이 더 최근에 관측됐어도(최신 관측도 5h 뒤의 동률 해소용) 5h 가 먼저다.
    seat.updated_at = NOW - 1;
    cat.updated_at = NOW - 600;
    expect(pickPrimaryAccount([seat, cat], NOW)?.account_id).toBe("cat-hi");
    // 윈도우 좌석 경로(역슬래시·절대경로)여도 같다.
    const wseat = acct({ account_id: "wseat-lo", profiles: ["C:\\Users\\x\\.cys\\claude"], rate: [win("5h", 10)] });
    expect(pickPrimaryAccount([wseat, cat], NOW)?.account_id).toBe("cat-hi");
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
  it("대기 문구는 기한 없이 떠 있어도 참이다 — '화면 복원 뒤 표시' 같은 약속 금지(리뷰1 M8 · 반박 D5)", () => {
    // start() 실패 경로에는 10초 틱이 없어 이 문구가 무기한 남을 수 있다. Control Center Live 는 force 조회라 그 경로에서도 된다.
    const m = buildUsageBarModel([], NOW, { everOk: false, failStreak: 0, okAtSec: null }, noRedact);
    expect(m.message.includes("복원 뒤")).toBe(false);
    expect(m.message.includes("표시됩니다")).toBe(false);
    expect(m.message).toContain("Control Center");
  });
  it("조회 성공·계정 0 → '아직 관측된 사용량 없음'", () => {
    const m = buildUsageBarModel([], NOW, ok, noRedact);
    expect(m.primary).toBeNull();
    expect(m.message).toContain("아직 관측된 사용량 없음");
  });
  it("전부 관측 전이어도 계정마다 한 줄씩 이름이 보인다(개수·툴팁으로 숨기지 않는다 — 0.14.42 RC4)", () => {
    const m = buildUsageBarModel(
      [acct({ updated_at: null, profiles: [".claude-1"] }), acct({ updated_at: null, profiles: [".claude-2"] })],
      NOW, ok, noRedact,
    );
    expect(m.primary).toBeNull();
    expect(m.unobservedCount).toBe(2);
    expect(m.others.map((o) => o.label).sort()).toEqual(["claude-1", "claude-2"]);
    expect(m.others.every((o) => o.unobserved && o.dim && o.text.startsWith("관측 전"))).toBe(true);
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
  it("🔒 가림이면 툴팁의 설정 폴더도 끝 이름만 — 윈도우 절대경로의 OS 사용자명 비노출(리뷰1 M9)", () => {
    const a = acct({
      account_id: "acc-uuid-77",
      label: "",
      profiles: ["C:\\Users\\runner\\.cys\\claude", "/Users/runner/.claude-2", "C:/Users/runner/.cys/claude"],
      rate: [win("5h", 5)],
    });
    const red = buildUsageBarModel([a], NOW, ok, (s) => `#${s.length}`, true).primary!.tooltip;
    expect(red.includes("runner")).toBe(false);
    expect(red.includes("Users")).toBe(false);
    const folders = red.split("\n").find((l) => l.startsWith("설정 폴더:"))!;
    expect(folders).toBe("설정 폴더: .claude-2, .cys/claude"); // 가린 뒤에도 중복은 접는다
    // 라벨이 비면 account_id 가 신원 줄에 나온다 — 그것도 가림 함수를 거친다(CC 계정 섹션과 같은 규칙).
    expect(red.includes("acc-uuid-77")).toBe(false);
    const plain = buildUsageBarModel([a], NOW, ok, noRedact).primary!.tooltip;
    expect(plain).toContain("C:/Users/runner/.cys/claude"); // 가림을 끄면 정규화한 원래 경로
    expect(plain).toContain("acc-uuid-77");
    // 다른 계정 줄의 툴팁도 같은 규칙.
    const other = acct({ account_id: "o2", profiles: ["/home/alice/.claude-9"], rate: [win("5h", 1)] });
    const m = buildUsageBarModel([a, other], NOW, ok, (s) => `#${s.length}`, true);
    const tips = [m.primary!.tooltip, ...m.others.map((o) => o.tooltip)].join("\n");
    expect(tips.includes("alice")).toBe(false);
    expect(tips).toContain(".claude-9");
  });
  it("profileTail — 좌석·부서 폴더는 `.cys/<이름>`, 그 밖은 끝 이름", () => {
    expect(profileTail("C:\\Users\\runner\\.cys\\claude")).toBe(".cys/claude");
    expect(profileTail("/Users/runner/.cys/claude-sales/")).toBe(".cys/claude-sales");
    expect(profileTail("/Users/runner/.claude-4")).toBe(".claude-4");
    expect(profileTail(".claude-1")).toBe(".claude-1");
    expect(profileTail("")).toBe("");
  });
  it("툴팁은 집계 범위를 정직하게 적는다(0.14.42 RC4-b: 외부 터미널 Claude 세션도 모인다 · 표시용 · 한계)", () => {
    const a = acct({ rate: [win("5h", 5)] });
    const t = buildUsageBarModel([a], NOW, ok, noRedact).primary!.tooltip;
    expect(t).toContain("외부 터미널");
    expect(t).not.toContain("빠집니다"); // 수정 전 문구 — 이제 거짓이다
    expect(t).toContain("표시용"); // 창 밖 값은 경보 근거가 아니다
  });
  it("창 밖 세션 값(source statusline-outside)은 라이브 관측이고 툴팁에 출처가 사람 말로 드러난다", () => {
    const a = acct({ source: "statusline-outside", rate: [win("5h", 40)] });
    expect(isLiveAccount(a)).toBe(true);
    const t = buildUsageBarModel([a], NOW, ok, noRedact).primary!.tooltip;
    expect(t).toContain("관측: cys 창 밖 상태줄"); // 출처 줄 자체가 사람 말(고지 문구에 기대지 않는다)
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

describe("0.14.42 — 발견된 계정은 전부 한 줄씩(오너 제보: 클로드 일부·antigravity 가 안 보임)", () => {
  const ok = { everOk: true, failStreak: 0, okAtSec: NOW };
  // 합성 픽스처 — 계정 id·사용률·폴더 배치 모두 지어낸 값이다(실제 계정 식별자·실사용 값·실제 폴더 대응을 싣지 않는다).
  // 모양만 재현: 관측된 Claude 2계정(여러 폴더·좌석·부서 폴더 공유) + 관측된 Codex + 관측 전 Claude 2계정
  // (그중 하나는 기본 프로필 ~/.claude 포함) + RC1 수리로 생기는 관측 전 antigravity 행.
  const live = () => [
    acct({ account_id: "acct-a", profiles: [".claude-2", ".cys/claude-default-dept-1"], rate: [win("5h", 30), win("7d", 20)] }),
    acct({ account_id: "acct-b", profiles: [".claude-1", ".cys/claude", ".cys/claude-default-dept-2"], rate: [win("5h", 10), win("7d", 60)] }),
    acct({ provider: "codex", account_id: "default", label: "OpenAI Codex", profiles: [".codex"], source: "rollout", updated_at: NOW - 600, rate: [win("7d", 15)] }),
    acct({ account_id: "acct-c", profiles: [".claude-4"], updated_at: null, stale_secs: null, source: "", rate: [] }),
    acct({ account_id: "acct-d", profiles: [".claude", ".claude-3"], updated_at: null, stale_secs: null, source: "", rate: [] }),
    acct({ provider: "antigravity", account_id: "default", label: "Antigravity (agy)", profiles: [".gemini/antigravity-cli"], updated_at: null, stale_secs: null, source: "", rate: [] }),
  ];
  it("★재현: 관측 없는 Claude 2계정·Antigravity 도 이름이 행으로 나온다(수정 전: '관측 없음 2개' 한 줄 + 툴팁)", () => {
    const m = buildUsageBarModel(live(), NOW, ok, noRedact);
    const shown = [m.primary!.label, ...m.others.map((o) => o.label)];
    for (const want of ["claude-2", "claude-1", "Codex", "claude-4", "claude", "Antigravity"])
      expect({ 계정: want, 보임: shown.includes(want) }).toEqual({ 계정: want, 보임: true });
    expect(m.moreCount).toBe(0); // 6계정은 상한 안에 다 들어간다
    expect(shown.length).toBe(6);
  });
  it("관측된 계정이 관측 전 계정보다 먼저 — 상한이 관측값을 밀어내지 않는다", () => {
    const m = buildUsageBarModel(live(), NOW, ok, noRedact);
    const firstUnobs = m.others.findIndex((o) => o.unobserved);
    const lastObs = m.others.map((o) => !o.unobserved).lastIndexOf(true);
    expect(firstUnobs).toBeGreaterThan(lastObs);
  });
  it("관측 전 행: 흐리게 · '관측 전 · <짧은 사유>' · 값(%)을 지어내지 않는다", () => {
    const m = buildUsageBarModel(live(), NOW, ok, noRedact);
    const c4 = m.others.find((o) => o.label === "claude-4")!;
    expect(c4.unobserved).toBe(true);
    expect(c4.dim).toBe(true);
    expect(c4.text.startsWith("관측 전")).toBe(true);
    expect(c4.text.includes("%")).toBe(false);
    expect(c4.tooltip).toContain("외부 터미널"); // 왜 비었는지 — 창 밖 세션도 상태줄이 cys 로 연결돼 있어야 모인다
    expect(c4.tooltip).not.toContain("집계되지 않습니다"); // 수정 전 문구(0.14.42 RC4-b 로 거짓이 됨)
    expect(c4.text).not.toContain("cys 창에서"); // 창 안에서만 들어온다는 사유는 더 이상 참이 아니다
  });
  it("agy CSRF 거절(agy_csrf_required)은 '무엇을 하면 값이 들어오나'(상태줄 연결)를 말한다", () => {
    const rows = live();
    rows[5].source_error = "agy_csrf_required";
    const agy = buildUsageBarModel(rows, NOW, ok, noRedact).others.find((o) => o.label === "Antigravity")!;
    expect(agy.text.startsWith("관측 실패")).toBe(true);
    expect(agy.text).toContain("상태줄");
    expect(agy.tooltip).toContain("CSRF");
    expect(agy.tooltip).toContain("agy_csrf_required");
    rows[5].source_error = null; // 결측형 — 관측 전 사유도 상태줄 경로를 가리킨다
    const pre = buildUsageBarModel(rows, NOW, ok, noRedact).others.find((o) => o.label === "Antigravity")!;
    expect(pre.text).toContain("상태줄");
  });
  it("fatal-fix W5: RPC 경로가 없는 플랫폼(agy_statusline_required)도 '상태줄 연결'을 가리킨다 — 포트 못 찾음이 아니다", () => {
    const rows = live();
    rows[5].source_error = "agy_statusline_required";
    const agy = buildUsageBarModel(rows, NOW, ok, noRedact).others.find((o) => o.label === "Antigravity")!;
    expect(agy.text.startsWith("관측 실패")).toBe(true);
    expect(agy.text).toContain("상태줄");
    expect(agy.text).not.toContain("포트");
    expect(agy.tooltip).toContain("agy_statusline_required");
    expect(agy.tooltip).toContain("Windows");
  });
  it("관측 경로 고장(source_error)은 '관측 전'과 구별된다 — agy 거부 코드 보존", () => {
    const rows = live();
    rows[5].source_error = "agy_http_403";
    const m = buildUsageBarModel(rows, NOW, ok, noRedact);
    const agy = m.others.find((o) => o.label === "Antigravity")!;
    expect(agy.text.startsWith("관측 실패")).toBe(true);
    expect(agy.text).toContain("403");
    expect(agy.tooltip).toContain("agy_http_403");
    for (const code of ["agy_unreachable", "agy_no_quota", "agy_no_port", "agy_no_process", "agy_csrf_required", "agy_statusline_required", "brand_new_code"]) {
      rows[5].source_error = code;
      const t = buildUsageBarModel(rows, NOW, ok, noRedact).others.find((o) => o.label === "Antigravity")!;
      expect({ code, 실패표기: t.text.startsWith("관측 실패") }).toEqual({ code, 실패표기: true });
    }
    rows[5].source_error = null; // 결측형 — 오류 없음이면 '관측 전'
    expect(buildUsageBarModel(rows, NOW, ok, noRedact).others.find((o) => o.label === "Antigravity")!.text.startsWith("관측 전")).toBe(true);
  });
  it("관측 어댑터 없는 선언 계정(adapter:false)은 그 사실을 적는다", () => {
    const m = buildUsageBarModel([acct({ provider: "grok", account_id: "default", label: "grok", updated_at: null, source: "", adapter: false })], NOW, ok, noRedact);
    expect(m.others[0].text).toContain("어댑터 없음");
  });
  it("🔒 가림은 관측 전 행 툴팁에도 그대로(이메일·절대경로 비노출) · 라벨에 이메일 없음", () => {
    const rows = [acct({ account_id: "zz", label: "hidden@corp.example", profiles: ["/Users/runner/.claude-7"], updated_at: null, source: "" })];
    const m = buildUsageBarModel(rows, NOW, ok, (s) => `#${s.length}`, true);
    const row = m.others[0];
    expect(row.label).toBe("claude-7");
    expect(row.tooltip.includes("hidden@corp.example")).toBe(false);
    expect(row.tooltip.includes("runner")).toBe(false);
    expect(row.tooltip).toContain("#19");
  });
  it("Antigravity 제공자 라벨 — 프로필(.gemini/antigravity-cli)은 Claude 규칙에 걸리지 않는다", () => {
    expect(accountShortLabel(acct({ provider: "antigravity", profiles: [".gemini/antigravity-cli"], label: "Antigravity (agy)" }))).toBe("Antigravity");
  });
  it("상한은 관측·관측 전 구분 없이 같은 줄 수 규칙 — 잘린 관측 전 계정은 개수가 아니라 접힘 줄로 알린다(0.14.43 A4)", () => {
    const many = Array.from({ length: USAGE_OTHERS_MAX + 4 }, (_, i) =>
      acct({ account_id: `u${i}`, profiles: [`.claude-${i}`], updated_at: i === 0 ? NOW - 5 : null, source: i === 0 ? "statusline" : "", rate: i === 0 ? [win("5h", 5)] : [] }),
    );
    const m = buildUsageBarModel(many, NOW, ok, noRedact);
    expect(m.others.length).toBe(USAGE_OTHERS_MAX);
    // ※0.14.43: moreCount 는 잘린 '관측 줄'만 센다(여기선 잘린 3줄이 전부 관측 전 → 0). 그 3계정은 접힘 줄이 라벨로 남긴다.
    expect(m.moreCount).toBe(0);
    expect(m.unobservedFold!.text.startsWith("관측 전 3계정 — ")).toBe(true);
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

// ═════════ 0.14.43 (티켓 UI1) — 별명 · '● 사용 중' · 주 계정 선정 · 오래됨 · 숨기기 · 관측 전 · 제공자 요약 ═════════
// 데몬 가산 키(alias · current_profiles · in_use · rate_observed_at · rate[].alert_eligible)는 B1·B3 가 넣는다 — 아래 검체는 그 계약대로 만든
// **합성 행**이다(계정 id·이메일·사용률 모두 지어낸 값). 키가 없는 행(구버전 데몬)은 종전 규칙으로 폴백해야 한다(IPC 데이터 — 전부 의심).
const okFetch = { everOk: true, failStreak: 0, okAtSec: NOW };
// main.ts ccHash6 와 같은 알고리즘(djb2 → hex 8자 → 앞 6자). 사이드바 겹침 꼬리표와 Control Center 계정 표의 `#hash6` 가 같은 값이어야
// 두 화면을 대조할 수 있다(main.ts 쪽 알고리즘 동일성은 usagewiring.test.ts 가 핀).
const ccHash6 = (s: string): string => {
  let h = 5381;
  for (let i = 0; i < s.length; i++) h = ((h << 5) + h + s.charCodeAt(i)) >>> 0;
  return h.toString(16).padStart(8, "0").slice(0, 6);
};
const ccRedact = (s: string): string => `#${ccHash6(s)}`;
/** 합성 행 — 나이(초)·창 값·사용 중 표식을 한 줄로. 키를 안 주면 그 키는 행에 없다(구버전 모양). */
type RowOpt = { age?: number; p5?: number; p7?: number; in_use?: boolean | null; src?: string; prov?: string; profiles?: string[]; current?: string[] };
const R = (id: string, o: RowOpt = {}): AcctRow =>
  acct({
    account_id: id,
    provider: o.prov ?? "claude",
    source: o.src ?? "statusline",
    updated_at: NOW - (o.age ?? 30),
    profiles: o.profiles ?? [],
    rate: [...(o.p5 === undefined ? [] : [win("5h", o.p5)]), ...(o.p7 === undefined ? [] : [win("7d", o.p7)])],
    ...(o.in_use === undefined ? {} : { in_use: o.in_use }),
    ...(o.current === undefined ? {} : { current_profiles: o.current }),
  });
const ids = (xs: AcctRow[]): string[] => xs.map((a) => String(a.account_id));

describe("0.14.43 라벨 규칙 표 — 별명 > 현재 로그인 폴더 > (구버전) profiles > 이전 로그인", () => {
  const rows: Array<[string, Partial<AcctRow>, string]> = [
    ["별명이 있으면 별명(폴더·이메일보다 먼저)", { alias: "업무용", current_profiles: [".claude-2"], profiles: [".claude-9"], label: "a@corp.example" }, "업무용"],
    ["별명 앞뒤 공백은 걷는다", { alias: "  개인  " }, "개인"],
    ["공백뿐인 별명은 없는 것 → 폴더", { alias: "   ", current_profiles: [".claude-2"] }, "claude-2"],
    ["문자열이 아닌 별명(IPC 오염)은 없는 것 → 폴더", { alias: 5 as unknown as string, current_profiles: [".claude-2"] }, "claude-2"],
    ["별명 null → 폴더", { alias: null, current_profiles: [".cys/claude"] }, "좌석"],
    ["별명이 24자를 넘으면 24자로 자른다", { alias: "가".repeat(30) }, "가".repeat(24)],
    ["current_profiles 가 배열이면 그것만 쓴다(추가 전용 profiles 에 남은 옛 로그인 폴더는 무시)", { current_profiles: [".cys/claude"], profiles: [".claude-4", ".cys/claude"] }, "좌석"],
    ["current 가 여럿이면 기존 rank(claude-N > 좌석 > 부서)", { current_profiles: [".cys/claude", ".claude-3", ".cys/claude-default-dept-1"] }, "claude-3"],
    ["current 의 부서 포크 폴더 → '부서 x'", { current_profiles: [".cys/claude-default-dept-1"] }, "부서 dept-1"],
    ["current_profiles 키가 없으면(구버전 데몬) 종전처럼 profiles 로", { profiles: [".cys/claude", ".claude-4"] }, "claude-4"],
    ["claude · current 빈 배열 → 'Claude (이전 로그인)'(profiles 에 폴더가 남아 있어도)", { current_profiles: [], profiles: [".claude-2"] }, "Claude (이전 로그인)"],
    ["codex · current 빈 배열 → 제공자 라벨", { provider: "codex", current_profiles: [] }, "Codex"],
    ["antigravity · current 폴더가 claude 규칙에 안 걸리면 제공자 라벨", { provider: "antigravity", current_profiles: [".gemini/antigravity-cli"] }, "Antigravity"],
    ["current_profiles 가 배열이 아니면(null) 없는 키로 본다 → profiles 폴백", { current_profiles: null as unknown as string[], profiles: [".claude-5"] }, "claude-5"],
    ["current_profiles 원소가 전부 쓸 수 없는 값이면 빈 배열과 같다(이전 로그인)", { current_profiles: [1, null, ""] as unknown as string[] }, "Claude (이전 로그인)"],
    ["current 에 폴더 라벨을 못 만드는 경로뿐이어도 claude 는 '이전 로그인'(티켓 문면: 폴더 라벨을 하나도 못 만들면)", { current_profiles: ["/weird/place"] }, "Claude (이전 로그인)"],
    ["별명은 '이전 로그인' 라벨보다 먼저", { alias: "예비", current_profiles: [] }, "예비"],
  ];
  for (const [name, over, want] of rows)
    it(name, () => {
      expect(accountShortLabel(acct({ account_id: "lbl", ...over }))).toBe(want);
    });
  it("어떤 행에서도 이메일(label 필드)은 라벨이 되지 않는다(오너 정책 — 화면 공유 노출 방지)", () => {
    for (const [, over] of rows) expect(accountShortLabel(acct({ ...over, label: "leak@example.com" })).includes("@")).toBe(false);
  });
  it("별명 상한은 코드 포인트 24자 — 이모지(서로게이트 쌍)를 반으로 자르지 않는다 · 정확히 24자는 그대로", () => {
    expect(USAGE_ALIAS_MAX).toBe(24);
    const cut = acctAlias(acct({ alias: "😀".repeat(30) }));
    expect(cut).toBe("😀".repeat(24));
    expect(acctAlias(acct({ alias: "가".repeat(24) }))).toBe("가".repeat(24));
  });
  it("acctAlias — 비문자열·빈 값은 빈 문자열(던지지 않는다)", () => {
    for (const v of [undefined, null, "", "  ", 0, 7, true, {}, []]) expect(acctAlias(acct({ alias: v as unknown as string }))).toBe("");
    expect(() => acctAlias(null as unknown as AcctRow)).not.toThrow();
  });
  it("isPreviousLogin — claude 이고 current_profiles 가 (쓸 수 있는 원소 없는) 배열일 때만 · 키 없음(구버전)은 판정하지 않는다", () => {
    expect(isPreviousLogin(acct({ current_profiles: [] }))).toBe(true);
    expect(isPreviousLogin(acct({ current_profiles: [".claude-1"] }))).toBe(false);
    expect(isPreviousLogin(acct({ profiles: [".claude-1"] }))).toBe(false); // 구버전 — 키 없음
    expect(isPreviousLogin(acct({ provider: "codex", current_profiles: [] }))).toBe(false);
    expect(isPreviousLogin(acct({ current_profiles: null as unknown as string[] }))).toBe(false);
  });
});

describe("0.14.43 겹침 구분 — 이메일(🔒 가림 거침) · 이메일이 없으면 tag4", () => {
  const twoSeat = (la: string, lb: string): AcctRow[] => [
    acct({ account_id: "ov-a", label: la, current_profiles: [".cys/claude"], rate: [win("5h", 30)] }),
    acct({ account_id: "ov-b", label: lb, current_profiles: [".cys/claude"], rate: [win("5h", 10)] }),
  ];
  it("🔒 끔 — 꼬리표는 이메일 원문", () => {
    const m = buildUsageBarModel(twoSeat("a@corp.example", "b@corp.example"), NOW, okFetch, noRedact);
    expect(m.primary!.label).toBe("좌석 ·a@corp.example");
    expect(m.others[0].label).toBe("좌석 ·b@corp.example");
  });
  it("🔒 켬 — 꼬리표는 Control Center 계정 표와 같은 `#hash6`(이메일이 본문에 안 나온다)", () => {
    const m = buildUsageBarModel(twoSeat("a@corp.example", "b@corp.example"), NOW, okFetch, ccRedact, true);
    expect(m.primary!.label).toBe(`좌석 ·#${ccHash6("a@corp.example")}`);
    expect(m.others[0].label).toBe(`좌석 ·#${ccHash6("b@corp.example")}`);
    expect([m.primary!.label, m.others[0].label, m.headline].join("\n").includes("@")).toBe(false);
    expect(/^좌석 ·#[0-9a-f]{6}$/.test(m.primary!.label)).toBe(true);
  });
  it("이메일이 없으면(label 빈 값) 종전 tag4 꼬리표", () => {
    const m = buildUsageBarModel(twoSeat("", ""), NOW, okFetch, noRedact);
    expect(/^좌석 ·[0-9a-f]{4}$/.test(m.primary!.label)).toBe(true);
    expect(/^좌석 ·[0-9a-f]{4}$/.test(m.others[0].label)).toBe(true);
    expect(m.primary!.label).not.toBe(m.others[0].label);
  });
  it("같은 이메일이라 이메일로도 못 가르면 tag4 로 가른다(둘 다 같은 줄로 보이지 않게)", () => {
    const m = buildUsageBarModel(twoSeat("same@corp.example", "same@corp.example"), NOW, okFetch, noRedact);
    expect(/^좌석 ·[0-9a-f]{4}$/.test(m.primary!.label)).toBe(true);
    expect(m.primary!.label).not.toBe(m.others[0].label);
  });
  it("겹치는 쪽에만 꼬리표 — 겹치지 않는 계정 라벨은 그대로", () => {
    const rows = [...twoSeat("a@corp.example", "b@corp.example"), acct({ account_id: "solo", label: "c@corp.example", current_profiles: [".claude-2"], rate: [win("5h", 5)] })];
    const labels = (() => {
      const m = buildUsageBarModel(rows, NOW, okFetch, noRedact);
      return [m.primary!.label, ...m.others.map((o) => o.label)].sort();
    })();
    expect(labels).toEqual(["claude-2", "좌석 ·a@corp.example", "좌석 ·b@corp.example"]);
  });
  it("같은 별명도 같은 규칙으로 구분한다", () => {
    const rows = [
      acct({ account_id: "al-a", label: "a@corp.example", alias: "업무", rate: [win("5h", 30)] }),
      acct({ account_id: "al-b", label: "b@corp.example", alias: "업무", rate: [win("5h", 10)] }),
    ];
    const m = buildUsageBarModel(rows, NOW, okFetch, noRedact);
    expect([m.primary!.label, m.others[0].label]).toEqual(["업무 ·a@corp.example", "업무 ·b@corp.example"]);
  });
  it("★겹침이 없으면 이메일은 본문 어디에도 없다(라벨·줄 글·요약·약식 풀이·접힘 줄) — 툴팁에만", () => {
    const rows = [
      acct({ account_id: "n1", label: "x1@corp.example", current_profiles: [".claude-1"], rate: [win("5h", 40), win("7d", 20)] }),
      acct({ account_id: "n2", label: "x2@corp.example", alias: "업무", current_profiles: [".claude-2"], rate: [win("5h", 10)] }),
      acct({ account_id: "n3", label: "x3@corp.example", current_profiles: [], updated_at: null, source: "", rate: [] }),
      acct({ provider: "codex", account_id: "cx", label: "OpenAI Codex", source: "rollout", rate: [win("7d", 50)] }),
    ];
    const m = buildUsageBarModel(rows, NOW, okFetch, noRedact);
    const body = [
      m.headline, m.headlineTitle, m.message, m.footer, m.moreTooltip, m.unobservedFold ? m.unobservedFold.text : "",
      m.primary!.label, m.primary!.fresh.note, m.primary!.exhaust, ...m.primary!.windows.map((w) => `${w.text}${w.resetText}`),
      ...m.others.map((o) => `${o.label}\n${o.text}`),
    ].join("\n");
    expect(body.includes("@")).toBe(false);
    expect(m.primary!.tooltip).toContain("x1@corp.example"); // 이메일은 툴팁에만(🔒 끔)
  });
});

describe("0.14.43 주 계정 선정 — 사용 중 → 신선도 등급 → (등급별 순서) → 좌석 경로 → 키", () => {
  it("★재로그인 시나리오: 옛 계정(100% · 3시간 전 · 사용 중 아님) 대신 새 계정(12% · 20초 전 · 사용 중)", () => {
    const old = R("old", { age: 3 * 3600, p5: 100, p7: 60, in_use: false, current: [] });
    const cur = R("new", { age: 20, p5: 12, p7: 20, in_use: true, current: [".cys/claude"] });
    expect(pickPrimaryAccount([old, cur], NOW)?.account_id).toBe("new");
    expect(pickPrimaryAccount([cur, old], NOW)?.account_id).toBe("new");
    const m = buildUsageBarModel([old, cur], NOW, okFetch, noRedact);
    expect(m.primary!.label).toBe("좌석");
    expect(m.primary!.inUse).toBe(true);
    expect(m.primary!.windows.map((w) => w.sev)).toEqual(["", ""]);
    expect(m.others[0].label).toBe("Claude (이전 로그인)");
    expect(m.others[0].inUse).toBe(false);
    expect(m.headline).toBe("5h 12% · 7d 20%"); // 옛 100% 가 요약줄을 빨갛게 만들지 않는다
    expect(m.headlineSev).toBe("");
  });
  it("같은 시나리오를 구버전 데몬(키 없음)이 보내도 신선도 등급으로 새 계정 — 옛 100% 가 이기지 않는다", () => {
    const old = R("old", { age: 3 * 3600, p5: 100, p7: 60 });
    const cur = R("new", { age: 20, p5: 12, p7: 20 });
    expect(pickPrimaryAccount([old, cur], NOW)?.account_id).toBe("new");
    expect(pickPrimaryAccount([cur, old], NOW)?.account_id).toBe("new");
  });
  it("사용 중이 신선도·사용률보다 먼저 — 사용 중인데 오래된 계정이 신선한 고사용 계정을 이긴다(① 키)", () => {
    const inUseStale = R("use-stale", { age: 3 * 3600, p5: 5, in_use: true });
    const hotFresh = R("hot-fresh", { age: 10, p5: 99, in_use: false });
    expect(pickPrimaryAccount([hotFresh, inUseStale], NOW)?.account_id).toBe("use-stale");
    expect(pickPrimaryAccount([inUseStale, hotFresh], NOW)?.account_id).toBe("use-stale");
    // 사용 중인 스냅샷도 사용 중 아닌 신선한 계정보다 위(① 이 ② 보다 먼저)
    const snap = R("use-snap", { age: 600, p5: 1, in_use: true, src: "snapshot" });
    expect(pickPrimaryAccount([hotFresh, snap], NOW)?.account_id).toBe("use-snap");
  });
  it("in_use 혼합 — 사용 중(true)이 하나라도 있으면 그 계정이 먼저 · 둘 이상이면 그 안에서 신선도 → 5h", () => {
    const t = R("t", { in_use: true, p5: 1, age: 3 * 3600 });
    const n = R("n", { in_use: null, p5: 10 });
    const f = R("f", { in_use: false, p5: 90 });
    expect(pickPrimaryAccount([f, n, t], NOW)?.account_id).toBe("t");
    expect(pickPrimaryAccount([t, n, f], NOW)?.account_id).toBe("t");
    // 사용 중을 숨기면 이 키는 꺼진다(남은 건 null·false) → 같은 등급 안에서 5h 가 높은 f
    expect(pickPrimaryAccount([f, n, t], NOW, new Set([acctKey(t)]))?.account_id).toBe("f");
    // 사용 중이 둘(좌석 둘이 서로 다른 계정) — 그 안에서는 신선도 등급 → 5h
    expect(pickPrimaryAccount([R("t-old", { in_use: true, p5: 99, age: 3 * 3600 }), R("t-new", { in_use: true, p5: 5, age: 10 })], NOW)?.account_id).toBe("t-new");
    expect(pickPrimaryAccount([R("t-lo", { in_use: true, p5: 20 }), R("t-hi", { in_use: true, p5: 60 })], NOW)?.account_id).toBe("t-hi");
  });
  it("사용 중이 하나도 없으면(전부 구버전·null·false) 사용 중 키를 건너뛴다 — 신선도 등급이 그대로 이긴다", () => {
    expect(pickPrimaryAccount([R("n", { in_use: null, age: 3 * 3600, p5: 99 }), R("f", { in_use: false, age: 10, p5: 1 })], NOW)?.account_id).toBe("f");
    expect(pickPrimaryAccount([R("n", { in_use: null, p5: 80 }), R("f", { in_use: false, p5: 90 })], NOW)?.account_id).toBe("f"); // 같은 등급 → 5h
    expect(pickPrimaryAccount([R("a", { in_use: null, p5: 20 }), R("b", { p5: 30 })], NOW)?.account_id).toBe("b"); // 키 없음 = null 과 같은 취급
  });
  it("in_use 가 boolean 이 아니면(문자열 'true' 등 IPC 오염) 사용 중으로 치지 않는다", () => {
    const polluted = R("poll", { age: 3 * 3600, p5: 99 });
    (polluted as unknown as Record<string, unknown>).in_use = "true";
    expect(pickPrimaryAccount([polluted, R("fresh", { age: 10, p5: 1 })], NOW)?.account_id).toBe("fresh");
    const m = buildUsageBarModel([polluted, R("fresh", { age: 10, p5: 1 })], NOW, okFetch, noRedact);
    expect([m.primary!.inUse, ...m.others.map((o) => o.inUse)]).toEqual([false, false]);
    // 오염된 값이 주 계정(신선한 쪽)이어도 배지는 켜지지 않는다
    const polluted2 = R("poll2", { age: 10, p5: 50 });
    (polluted2 as unknown as Record<string, unknown>).in_use = "true";
    const m2 = buildUsageBarModel([polluted2, R("other", { age: 3 * 3600, p5: 1 })], NOW, okFetch, noRedact);
    expect(m2.primary!.fresh.level).toBe("fresh"); // 주 계정은 신선한 poll2
    expect(m2.primary!.inUse).toBe(false);
  });
  it("★신선도 등급이 5h 보다 먼저 — 라이브(30분 안) > 오래됨(30분 초과) > 스냅샷", () => {
    // 라이브 10% 가 오래된 95% 를 이긴다(라이브가 fresh 든 recent 든)
    for (const liveAge of [5, 600]) {
      const stale = R("stale-hot", { age: 3 * 3600, p5: 95 });
      const live = R("live-cold", { age: liveAge, p5: 10 });
      expect({ liveAge, 주: pickPrimaryAccount([stale, live], NOW)?.account_id }).toEqual({ liveAge, 주: "live-cold" });
      expect({ liveAge, 주: pickPrimaryAccount([live, stale], NOW)?.account_id }).toEqual({ liveAge, 주: "live-cold" });
    }
    // 오래된 95% 가 스냅샷 100% 를 이긴다 — 스냅샷은 나이가 더 최근이어도 오래된 라이브보다 아래(종전 '라이브 무리 우선'이 등급에 흡수된다)
    const snap = R("snap-hot", { age: 300, p5: 100, src: "snapshot" });
    const stale95 = R("stale-95", { age: 3 * 3600, p5: 95 });
    expect(pickPrimaryAccount([snap, stale95], NOW)?.account_id).toBe("stale-95");
    expect(pickPrimaryAccount([stale95, snap], NOW)?.account_id).toBe("stale-95");
    // 라이브가 스냅샷 100% 도 이긴다
    expect(pickPrimaryAccount([snap, R("live-1", { age: 600, p5: 1 })], NOW)?.account_id).toBe("live-1");
  });
  it("★깜빡임 방지 — 라이브(fresh·recent) 안에서는 2분 경계가 순위를 바꾸지 않는다(둘 다 사용 중 · 둘 다 키 없음)", () => {
    // 100초 전 10% 와 600초 전 60%: 둘 다 라이브라 5h 가 높은 60% 가 주 계정. 600초 쪽 관측 시각을 2분 경계 양쪽(119·121)과
    // 30분 직전(1799)으로 옮겨도 결과가 같다 — 리뷰어 좌석처럼 가끔만 보고하는 계정이 fresh↔recent 를 오가도 주 계정이 안 바뀐다.
    for (const inUse of [true, undefined] as const) {
      for (const age of [119, 121, 600, 1799]) {
        const opt = inUse === undefined ? {} : { in_use: inUse };
        const lo = R("fresh-lo", { age: 100, p5: 10, ...opt });
        const hi = R("older-hi", { age, p5: 60, ...opt });
        const why = { inUse, age };
        expect({ ...why, 주: pickPrimaryAccount([lo, hi], NOW)?.account_id }).toEqual({ ...why, 주: "older-hi" });
        expect({ ...why, 주: pickPrimaryAccount([hi, lo], NOW)?.account_id }).toEqual({ ...why, 주: "older-hi" });
        // 요약 줄도 같다 — 60% 가 보이고 '(오래됨)' 접미사는 30분 안에서 붙지 않는다
        expect({ ...why, 요약: buildUsageBarModel([lo, hi], NOW, okFetch, noRedact).headline }).toEqual({ ...why, 요약: "5h 60% · 7d —" });
      }
    }
  });
  it("30분 경계는 여전히 등급을 가른다 — 1799초는 라이브(5h 우선) · 1800초는 오래됨(신선한 쪽이 이긴다)", () => {
    const fresh = R("fresh-lo", { age: 5, p5: 10 });
    expect(pickPrimaryAccount([fresh, R("hi", { age: 1799, p5: 60 })], NOW)?.account_id).toBe("hi");
    expect(pickPrimaryAccount([fresh, R("hi", { age: 1800, p5: 60 })], NOW)?.account_id).toBe("fresh-lo");
  });
  it("같은 등급 안의 순서 — 라이브(fresh·recent)는 5h → 7d → 최신 관측 · 오래됨·스냅샷은 최신 관측 → 5h → 7d", () => {
    // fresh 둘: 관측이 더 오래돼도(100초) 5h 가 높은 쪽
    expect(pickPrimaryAccount([R("a", { age: 100, p5: 80 }), R("b", { age: 5, p5: 10 })], NOW)?.account_id).toBe("a");
    // fresh 둘: 5h 같으면 7d
    expect(pickPrimaryAccount([R("a", { age: 5, p5: 30, p7: 10 }), R("b", { age: 5, p5: 30, p7: 70 })], NOW)?.account_id).toBe("b");
    // recent 둘도 같은 규칙 · fresh 와 recent 가 섞여도 같다(2분 경계 양쪽)
    expect(pickPrimaryAccount([R("a", { age: 900, p5: 80 }), R("b", { age: 300, p5: 10 })], NOW)?.account_id).toBe("a");
    expect(pickPrimaryAccount([R("a", { age: 300, p5: 30, p7: 10 }), R("b", { age: 5, p5: 30, p7: 70 })], NOW)?.account_id).toBe("b"); // 5h 같으면 7d
    expect(pickPrimaryAccount([R("a", { age: 300, p5: 30, p7: 40 }), R("b", { age: 5, p5: 30, p7: 40 })], NOW)?.account_id).toBe("b"); // 5h·7d 같으면 최신 관측
    // stale 둘(스냅샷 아님): 5h 가 낮아도 최신 관측이 위
    expect(pickPrimaryAccount([R("a", { age: 3600, p5: 90 }), R("b", { age: 2400, p5: 10 })], NOW)?.account_id).toBe("b");
    // 스냅샷 둘: 5h 가 낮아도 updated_at 큰 쪽
    expect(pickPrimaryAccount([R("s1", { age: 900, p5: 95, src: "snapshot" }), R("s2", { age: 300, p5: 10, src: "snapshot" })], NOW)?.account_id).toBe("s2");
    // 오래된 등급에서 관측 시각이 같으면 5h → 7d
    expect(pickPrimaryAccount([R("a", { age: 3600, p5: 10, p7: 5 }), R("b", { age: 3600, p5: 40, p7: 1 })], NOW)?.account_id).toBe("b");
  });
  it("전부 같으면 좌석 경로 → 키 사전순(결정론) · 입력 순서와 무관", () => {
    const plain = R("b-plain", { p5: 30, profiles: [".claude-1"] });
    const seat = R("c-seat", { p5: 30, profiles: [".cys/claude"] });
    plain.updated_at = seat.updated_at = NOW - 10;
    expect(pickPrimaryAccount([plain, seat], NOW)?.account_id).toBe("c-seat");
    expect(pickPrimaryAccount([seat, plain], NOW)?.account_id).toBe("c-seat");
    const k1 = R("k1", { p5: 30 }), k2 = R("k2", { p5: 30 });
    k1.updated_at = k2.updated_at = NOW - 10;
    expect(pickPrimaryAccount([k2, k1], NOW)?.account_id).toBe("k1");
  });
  it("숨긴 계정은 후보가 아니다 · 사용 중 키 계산에서도 빠진다", () => {
    const best = R("best", { p5: 90, in_use: false });
    const next = R("next", { p5: 40, in_use: false });
    expect(pickPrimaryAccount([best, next], NOW)?.account_id).toBe("best");
    expect(pickPrimaryAccount([best, next], NOW, new Set([acctKey(best)]))?.account_id).toBe("next");
    expect(pickPrimaryAccount([best], NOW, new Set([acctKey(best)]))).toBeNull();
    // 사용 중인 계정을 숨기면 그 키는 꺼진다(남은 건 전부 사용 중 아님) → 신선도·5h 로
    const using = R("using", { p5: 1, age: 3 * 3600, in_use: true });
    expect(pickPrimaryAccount([best, using], NOW)?.account_id).toBe("using");
    expect(pickPrimaryAccount([best, using], NOW, new Set([acctKey(using)]))?.account_id).toBe("best");
  });
  it("관측 전 계정은 후보가 아니다(사용 중 true 여도)", () => {
    const unobs = acct({ account_id: "unobs", updated_at: null, source: "", in_use: true, rate: [] });
    expect(pickPrimaryAccount([unobs], NOW)).toBeNull();
    expect(pickPrimaryAccount([unobs, R("seen", { age: 3 * 3600, p5: 5, in_use: false })], NOW)?.account_id).toBe("seen");
  });
  it("★구버전(키 없음) 동치 — 라이브(30분 안) 계정끼리는 fresh·recent 가 2분 경계 양쪽으로 섞여도 종전(0.14.42) 규칙과 같은 주 계정", () => {
    // 종전 pickPrimaryAccount 원문(라이브 무리 → 5h → 7d → 최신 관측 → 좌석 경로 → 키).
    const legacy = (accounts: AcctRow[], now: number): AcctRow | null => {
      const rank = (a: AcctRow, l: string): number => windowView(a, l, now).pct ?? -1;
      const list = accounts.filter((a) => a.updated_at != null && Number(a.updated_at) > 0);
      const live = list.filter(isLiveAccount);
      const pool = live.length ? live : list;
      if (!pool.length) return null;
      return [...pool].sort((x, y) => {
        const d5 = rank(y, "5h") - rank(x, "5h");
        if (d5) return d5;
        const d7 = rank(y, "7d") - rank(x, "7d");
        if (d7) return d7;
        const du = Number(y.updated_at) - Number(x.updated_at);
        if (du) return du;
        const ds = Number(hasSeatProfile(y)) - Number(hasSeatProfile(x));
        if (ds) return ds;
        return acctKey(x) < acctKey(y) ? -1 : acctKey(x) > acctKey(y) ? 1 : 0;
      })[0];
    };
    let seed = 20261003; // 고정 시드 — 같은 입력에 같은 판정(재현 가능)
    const rnd = (): number => {
      seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0;
      return seed / 4294967296;
    };
    const pickOne = <T,>(xs: T[]): T => xs[Math.floor(rnd() * xs.length)];
    for (let it = 0; it < 400; it++) {
      // 짝수 판 = 모두 fresh · 홀수 판 = fresh·recent 가 2분 경계(119·121) 양쪽으로 섞임 — 관측 시각 동률을 일부러 만든다
      const ages = it % 2 === 0 ? [5, 5, 10, 60, 100] : [5, 60, 100, 119, 121, 300, 300, 600, 900, 1500, 1799];
      const n = 2 + Math.floor(rnd() * 5);
      const rows: AcctRow[] = [];
      for (let i = 0; i < n; i++) {
        const rate: Array<{ label: string; used_pct: number; resets_at: number | null }> = [];
        if (rnd() < 0.85) rate.push(win("5h", Math.floor(rnd() * 8) * 12, rnd() < 0.1 ? NOW - 1 : NOW + 3600)); // 값 겹침·리셋 지남 포함
        if (rnd() < 0.85) rate.push(win("7d", Math.floor(rnd() * 8) * 12));
        rows.push(
          acct({
            account_id: `p${i}`,
            provider: pickOne(["claude", "claude", "codex"]),
            source: pickOne(["statusline", "rollout", "statusline-outside"]),
            updated_at: NOW - pickOne(ages),
            profiles: pickOne([[], [".claude-1"], [".cys/claude"], ["C:\\Users\\x\\.cys\\claude"]]),
            rate,
          }),
        );
      }
      expect({ 판: it, 오래됨_포함: rows.some((a) => freshness(a, NOW).level === "stale") }).toEqual({ 판: it, 오래됨_포함: false }); // 전부 라이브(30분 안) 보장
      expect({ 판: it, 새: pickPrimaryAccount(rows, NOW)?.account_id }).toEqual({ 판: it, 새: legacy(rows, NOW)?.account_id });
    }
  });
});

describe("0.14.43 오래된 값 — 경고색 없이 '오래됨'(값·게이지 폭은 그대로)", () => {
  it("★3시간 전 관측의 95%·75%: sev 는 빈 문자열 · 값(text)·게이지 폭(pct)은 그대로", () => {
    const a = R("st", { age: 3 * 3600, p5: 95, p7: 75 });
    const m = buildUsageBarModel([a], NOW, okFetch, noRedact);
    expect(m.primary!.windows.map((w) => [w.text, w.pct, w.sev])).toEqual([["95%", 95, ""], ["75%", 75, ""]]);
    expect(m.headlineSev).toBe("");
  });
  it("대조: 같은 값이라도 신선하면 종전대로 crit/warn(경고색 제거는 오래된 값에만)", () => {
    const m = buildUsageBarModel([R("fr", { age: 10, p5: 95, p7: 75 })], NOW, okFetch, noRedact);
    expect(m.primary!.windows.map((w) => w.sev)).toEqual(["crit", "warn"]);
    expect(m.headlineSev).toBe("crit");
    const w = buildUsageBarModel([R("fr", { age: 10, p5: 75, p7: 20 })], NOW, okFetch, noRedact);
    expect(w.headlineSev).toBe("warn");
    const rc = buildUsageBarModel([R("rc", { age: 300, p5: 95 })], NOW, okFetch, noRedact); // recent(5분) 는 아직 오래된 값이 아니다
    expect(rc.primary!.windows[0].sev).toBe("crit");
    expect(rc.headlineSev).toBe("crit");
  });
  it("스냅샷 주 계정도 경고색 없음 · 문구는 종전 '지난 기록 · …' 그대로", () => {
    const m = buildUsageBarModel([R("sn", { age: 600, p5: 99, src: "snapshot" })], NOW, okFetch, noRedact);
    expect(m.primary!.windows[0].sev).toBe("");
    expect(m.primary!.windows[0].text).toBe("99%");
    expect(m.primary!.fresh.note).toBe("지난 기록 · 10분 전");
    expect(m.headlineSev).toBe("");
  });
  it("주 계정 fresh.note — stale 은 '오래됨 · 3시간 전 관측' · recent 는 '5분 전 관측'(머리말 없음) · fresh 는 빈 문자열", () => {
    expect(buildUsageBarModel([R("a", { age: 3 * 3600, p5: 10 })], NOW, okFetch, noRedact).primary!.fresh).toEqual({ level: "stale", note: "오래됨 · 3시간 전 관측" });
    expect(buildUsageBarModel([R("a", { age: 300, p5: 10 })], NOW, okFetch, noRedact).primary!.fresh).toEqual({ level: "recent", note: "5분 전 관측" });
    expect(buildUsageBarModel([R("a", { age: 10, p5: 10 })], NOW, okFetch, noRedact).primary!.fresh).toEqual({ level: "fresh", note: "" });
    // freshness() 자체는 종전 그대로(머리말은 화면 모델의 몫)
    expect(freshness(R("a", { age: 3 * 3600 }), NOW).note).toBe("3시간 전 관측");
  });
  it("다른 줄의 괄호 표기도 (오래됨 · 3시간 전 관측) · 스냅샷 줄은 종전 (지난 기록 · …)", () => {
    const rows = [R("main", { p5: 50, in_use: true, profiles: [".claude-1"] }), R("old", { age: 3 * 3600, p5: 100, profiles: [".claude-2"] }), R("snap", { age: 900, p5: 60, src: "snapshot", profiles: [".claude-3"] })];
    const m = buildUsageBarModel(rows, NOW, okFetch, noRedact);
    const byLabel = (l: string) => m.others.find((o) => o.label === l)!;
    expect(byLabel("claude-2").text).toBe("5h 100% · 7d — (오래됨 · 3시간 전 관측)");
    expect(byLabel("claude-2").dim).toBe(true);
    expect(byLabel("claude-3").text).toBe("5h 60% · 7d — (지난 기록 · 15분 전)");
  });
  it("접힘 요약 — 주 계정이 오래됐으면 끝에 ' (오래됨)' · 신선하면 접미사 없음(종전 형식)", () => {
    expect(buildUsageBarModel([R("a", { age: 3 * 3600, p5: 12, p7: 30 })], NOW, okFetch, noRedact).headline).toBe("5h 12% · 7d 30% (오래됨)");
    expect(buildUsageBarModel([R("a", { age: 10, p5: 12, p7: 30 })], NOW, okFetch, noRedact).headline).toBe("5h 12% · 7d 30%");
    expect(buildUsageBarModel([R("a", { age: 600, p5: 12, p7: 30, src: "snapshot" })], NOW, okFetch, noRedact).headline).toBe("5h 12% · 7d 30% (오래됨)");
  });
  it("headlineSev = 주 계정 창들의 최고 심각도(crit > warn > 없음)", () => {
    const sev = (p5: number, p7: number) => buildUsageBarModel([R("a", { age: 10, p5, p7 })], NOW, okFetch, noRedact).headlineSev;
    expect([sev(10, 20), sev(75, 20), sev(10, 75), sev(95, 75), sev(75, 95), sev(89, 89), sev(90, 0)]).toEqual(["", "warn", "warn", "crit", "crit", "warn", "crit"]);
  });
  it("리셋 지난 창은 오래됨과 무관하게 '리셋됨'(경고색 없음) · 창이 없으면 '—'", () => {
    const a = R("a", { age: 10, p7: 30 });
    a.rate = [win("5h", 99, NOW - 1), win("7d", 30)];
    const m = buildUsageBarModel([a], NOW, okFetch, noRedact);
    expect(m.primary!.windows.map((w) => [w.text, w.sev])).toEqual([["리셋됨", ""], ["30%", ""]]);
    expect(m.headline).toBe("5h 리셋됨 · 7d 30%");
  });
});

describe("0.14.43 '● 사용 중' 표식 — in_use === true 일 때만(null·false·키 없음·문자열은 아님)", () => {
  it("주 계정·다른 줄·관측 전 줄 모두 같은 규칙", () => {
    const rows = [
      R("p", { in_use: true, p5: 50, profiles: [".claude-1"] }),
      R("o-true", { in_use: true, p5: 10, age: 100, profiles: [".claude-2"] }),
      R("o-false", { in_use: false, p5: 10, age: 101, profiles: [".claude-3"] }),
      R("o-null", { in_use: null, p5: 10, age: 102, profiles: [".claude-4"] }),
      R("o-none", { p5: 10, age: 103, profiles: [".claude-5"] }),
      acct({ account_id: "u-true", updated_at: null, source: "", in_use: true, profiles: [".claude-6"], rate: [] }),
      acct({ account_id: "u-false", updated_at: null, source: "", in_use: false, profiles: [".claude-7"], rate: [] }),
    ];
    (rows[3] as unknown as Record<string, unknown>).in_use = "true"; // 오염 — 문자열은 사용 중이 아니다
    const m = buildUsageBarModel(rows, NOW, okFetch, noRedact);
    expect(m.primary!.inUse).toBe(true);
    const mark = Object.fromEntries(m.others.map((o) => [o.label, o.inUse]));
    expect(mark).toEqual({ "claude-2": true, "claude-3": false, "claude-4": false, "claude-5": false, "claude-6": true, "claude-7": false });
  });
});

describe("0.14.43 숨기기(뷰어별) — 후보·줄·요약에서 빠지고 hiddenCount 로 알린다", () => {
  const rows = (): AcctRow[] => [
    R("h-best", { p5: 90, profiles: [".claude-1"] }),
    R("h-next", { p5: 40, age: 20, profiles: [".claude-2"] }),
    R("h-low", { p5: 5, age: 25, profiles: [".claude-3"] }),
    acct({ account_id: "h-unobs", updated_at: null, source: "", profiles: [".claude-4"], rate: [] }),
  ];
  it("주 계정을 숨기면 다음 계정이 주 계정 · 숨긴 계정은 줄에도 없다 · hiddenCount", () => {
    const hidden = new Set(["claude:h-best"]);
    const m = buildUsageBarModel(rows(), NOW, okFetch, noRedact, false, hidden);
    expect(m.primary!.label).toBe("claude-2");
    expect(m.others.map((o) => o.label)).toEqual(["claude-3", "claude-4"]);
    expect(m.hiddenCount).toBe(1);
    expect(m.headline).toBe("5h 40% · 7d —"); // 숨긴 계정의 90% 는 요약에도 없다
  });
  it("관측 전 계정도 숨길 수 있다 — 줄·unobservedCount 에서 빠진다", () => {
    const m = buildUsageBarModel(rows(), NOW, okFetch, noRedact, false, new Set(["claude:h-unobs"]));
    expect(m.hiddenCount).toBe(1);
    expect(m.unobservedCount).toBe(0);
    expect(m.others.some((o) => o.unobserved)).toBe(false);
  });
  it("hidden 을 안 넘기면 숨김 0 · 목록에 없는 키는 세지 않는다 · Set 이 아닌 값에도 던지지 않는다", () => {
    expect(buildUsageBarModel(rows(), NOW, okFetch, noRedact).hiddenCount).toBe(0);
    expect(buildUsageBarModel(rows(), NOW, okFetch, noRedact, false, new Set(["claude:없는-계정", "codex:default"])).hiddenCount).toBe(0);
    expect(() => buildUsageBarModel(rows(), NOW, okFetch, noRedact, false, [] as unknown as ReadonlySet<string>)).not.toThrow();
    expect(buildUsageBarModel(rows(), NOW, okFetch, noRedact, false, {} as unknown as ReadonlySet<string>).hiddenCount).toBe(0);
  });
  it("전부 숨기면 주 계정 없음 · '숨김 N계정'(거짓 '관측 없음' 아님) · 거짓 안내 문구 없음", () => {
    const all = new Set(rows().map(acctKey));
    const m = buildUsageBarModel(rows(), NOW, okFetch, noRedact, false, all);
    expect(m.primary).toBeNull();
    expect(m.hiddenCount).toBe(4);
    expect(m.others).toEqual([]);
    expect(m.headline).toBe("숨김 4계정");
    expect(m.message).toBe("");
  });
  it("숨긴 계정은 라벨 겹침 계산에서도 빠진다 — 겹치던 라벨이 꼬리표 없이 돌아온다", () => {
    const pair = [
      acct({ account_id: "s1", label: "a@corp.example", current_profiles: [".cys/claude"], rate: [win("5h", 30)] }),
      acct({ account_id: "s2", label: "b@corp.example", current_profiles: [".cys/claude"], rate: [win("5h", 10)] }),
    ];
    expect(buildUsageBarModel(pair, NOW, okFetch, noRedact).primary!.label).toBe("좌석 ·a@corp.example");
    expect(buildUsageBarModel(pair, NOW, okFetch, noRedact, false, new Set(["claude:s2"])).primary!.label).toBe("좌석");
  });
  it("숨긴 계정은 제공자 약식 요약의 제공자 수에도 들어가지 않는다", () => {
    const mixed = [R("c1", { p5: 12, p7: 30 }), R("x1", { prov: "codex", src: "rollout", p7: 50 })];
    expect(buildUsageBarModel(mixed, NOW, okFetch, noRedact).headline).toBe("C 5h12%·7d30% │ X 7d50%");
    expect(buildUsageBarModel(mixed, NOW, okFetch, noRedact, false, new Set(["codex:x1"])).headline).toBe("5h 12% · 7d 30%");
  });
  it("모델 서명(JSON) — 숨김·사용 중이 바뀌면 달라진다(본문 재구성 트리거 · 서명 로직은 그대로)", () => {
    const sig = (hidden?: ReadonlySet<string>, inUse?: boolean | null) =>
      JSON.stringify(buildUsageBarModel([R("s", { p5: 10, ...(inUse === undefined ? {} : { in_use: inUse }) }), R("t", { p5: 5, age: 20 })], NOW, okFetch, noRedact, false, hidden));
    expect(sig()).toBe(sig());
    expect(sig()).not.toBe(sig(new Set(["claude:t"])));
    expect(sig(undefined, true)).not.toBe(sig(undefined, false));
    expect(sig(undefined, undefined)).toBe(sig(undefined, null)); // null·키 없음은 둘 다 '사용 중 아님' 표기라 같은 모델
  });
});

describe("sanitizeHiddenKeys — 저장소 값 검증(배열 · 문자열 · 최대 64개)", () => {
  it("배열의 문자열 원소만 · 빈 문자열·비문자열·중복은 버린다", () => {
    expect([...sanitizeHiddenKeys(["claude:a", "claude:a", "codex:default", "", 7, null, {}, ["x"]])]).toEqual(["claude:a", "codex:default"]);
  });
  it("배열이 아니면 빈 집합(던지지 않는다)", () => {
    for (const v of [undefined, null, "claude:a", 5, {}, { 0: "claude:a", length: 1 }]) expect(sanitizeHiddenKeys(v).size).toBe(0);
  });
  it("최대 64개 — 넘는 원소는 버린다", () => {
    expect(USAGE_HIDDEN_MAX).toBe(64);
    const many = Array.from({ length: 100 }, (_, i) => `claude:k${i}`);
    const out = sanitizeHiddenKeys(many);
    expect(out.size).toBe(64);
    expect(out.has("claude:k63")).toBe(true);
    expect(out.has("claude:k64")).toBe(false);
  });
});

describe("0.14.43 A4 — 관측 없음 잔여: '관측 전 N계정' · 접힘 줄 · 외 N개 툴팁", () => {
  const U = (id: string, label: string, o: Partial<AcctRow> = {}): AcctRow =>
    acct({ account_id: id, alias: label, updated_at: null, source: "", rate: [], ...o });
  it("관측된 계정 0 · 관측 전 N → headline '관측 전 N계정'(주 계정 없음 · 줄은 한 줄씩)", () => {
    const m = buildUsageBarModel([U("u1", "가"), U("u2", "나"), U("u3", "다")], NOW, okFetch, noRedact);
    expect(m.primary).toBeNull();
    expect(m.headline).toBe("관측 전 3계정");
    expect(m.unobservedCount).toBe(3);
    expect(m.others.map((o) => o.label)).toEqual(["가", "나", "다"]);
    expect(m.message).toContain("아직 관측된 사용량 없음");
  });
  it("계정이 아예 0 이면 종전 '관측 없음'", () => {
    const m = buildUsageBarModel([], NOW, okFetch, noRedact);
    expect(m.headline).toBe("관측 없음");
    expect(m.unobservedFold).toBeNull();
    expect(m.hiddenCount).toBe(0);
  });
  it("관측 전 계정이 줄 상한을 넘으면 개수로만 사라지지 않는다 — 접힘 줄이 잘린 계정의 라벨을 나열(moreCount 는 0)", () => {
    const many = Array.from({ length: USAGE_OTHERS_MAX + 3 }, (_, i) => U(`u${String(i).padStart(2, "0")}`, `미관측${String(i).padStart(2, "0")}`));
    const m = buildUsageBarModel(many, NOW, okFetch, noRedact);
    expect(m.others.length).toBe(USAGE_OTHERS_MAX);
    expect(m.moreCount).toBe(0);
    expect(m.moreTooltip).toBe("");
    expect(m.unobservedFold).toEqual({ text: "관측 전 3계정 — 미관측08 · 미관측09 · 미관측10", tooltip: "미관측08 · 미관측09 · 미관측10" });
    expect(m.headline).toBe(`관측 전 ${USAGE_OTHERS_MAX + 3}계정`);
  });
  it("관측 줄이 먼저 자리를 차지한다 — 주 계정 + 관측 2줄 + 관측 전 9 → 관측 전 6줄 + 접힘 줄(잘린 3계정)", () => {
    const rows = [
      R("o0", { p5: 50, profiles: [".claude-0"] }), R("o1", { p5: 10, age: 20, profiles: [".claude-1"] }), R("o2", { p5: 5, age: 25, profiles: [".claude-2"] }),
      ...Array.from({ length: 9 }, (_, i) => U(`u${i}`, `미${i}`)),
    ];
    const m = buildUsageBarModel(rows, NOW, okFetch, noRedact);
    expect(m.others.map((o) => o.unobserved)).toEqual([false, false, true, true, true, true, true, true]);
    expect(m.moreCount).toBe(0);
    expect(m.unobservedFold!.text).toBe("관측 전 3계정 — 미6 · 미7 · 미8");
    expect(m.unobservedCount).toBe(9);
  });
  it("관측 줄이 상한을 넘으면 moreCount(잘린 관측 줄 수)·moreTooltip(그 라벨) · 관측 전은 전부 접힘 줄로", () => {
    const rows = [
      ...Array.from({ length: 11 }, (_, i) => R(`o${i}`, { p5: i, age: 10 + i, profiles: [`.claude-${i}`] })),
      U("ux", "미가"), U("uy", "미나"), U("uz", "미다"),
    ];
    const m = buildUsageBarModel(rows, NOW, okFetch, noRedact);
    expect(m.primary!.label).toBe("claude-10"); // 5h 최고
    // 나머지 관측 10줄은 최신 관측 순(claude-0 가 가장 최근) → 상한 8 이후 claude-8·claude-9 가 잘린다
    expect(m.others.length).toBe(USAGE_OTHERS_MAX);
    expect(m.others.every((o) => !o.unobserved)).toBe(true);
    expect(m.moreCount).toBe(2);
    expect(m.moreTooltip).toBe("claude-8 · claude-9");
    expect(m.unobservedFold!.text).toBe("관측 전 3계정 — 미가 · 미나 · 미다");
  });
  it("접힘 줄이 60자를 넘으면 말줄임(…) · 툴팁은 전체 나열", () => {
    const names = Array.from({ length: 5 }, (_, i) => `${"힣".repeat(19)}${i}`); // 20자 × 5 — 라벨 순 정렬에서 "앞N" 보다 뒤(잘리는 쪽)
    const many = [...Array.from({ length: USAGE_OTHERS_MAX }, (_, i) => U(`a${i}`, `앞${i}`)), ...names.map((n, i) => U(`z${i}`, n))];
    const m = buildUsageBarModel(many, NOW, okFetch, noRedact);
    const fold = m.unobservedFold!;
    expect(USAGE_FOLD_TEXT_MAX).toBe(60);
    expect(Array.from(fold.text).length).toBe(60);
    expect(fold.text.endsWith("…")).toBe(true);
    expect(fold.text.startsWith("관측 전 5계정 — ")).toBe(true);
    expect(fold.tooltip).toBe(names.join(" · "));
  });
  it("상한 안이면 접힘 줄·툴팁 없음(null · 빈 문자열)", () => {
    const m = buildUsageBarModel([R("a", { p5: 10 }), U("u1", "가")], NOW, okFetch, noRedact);
    expect(m.unobservedFold).toBeNull();
    expect(m.moreTooltip).toBe("");
    expect(m.moreCount).toBe(0);
  });
});

describe("0.14.43 A3 — 제공자별 접힘 요약", () => {
  const C = (id: string, o: RowOpt = {}) => R(id, o);
  const X = (id: string, o: RowOpt = {}) => R(id, { prov: "codex", src: "rollout", ...o });
  const A = (id: string, o: RowOpt = {}) => R(id, { prov: "antigravity", src: "agy-rpc", ...o });
  it("★두 제공자 이상이면 약식 — `C 5h12%·7d30% │ X 7d50%` · 풀이는 headlineTitle", () => {
    const m = buildUsageBarModel([C("c1", { p5: 12, p7: 30 }), X("x1", { p7: 50 })], NOW, okFetch, noRedact);
    expect(m.headline).toBe("C 5h12%·7d30% │ X 7d50%");
    expect(m.headlineTitle).toBe("Claude 5h 12% · 7d 30% / Codex 7d 50%");
  });
  it("한 제공자뿐이면 종전 형식 그대로(바이트 동일) · headlineTitle 은 빈 문자열", () => {
    const m = buildUsageBarModel([C("c1", { p5: 78, p7: 31 }), C("c2", { p5: 10, p7: 5, age: 20, profiles: [".claude-2"] })], NOW, okFetch, noRedact);
    expect(m.headline).toBe("5h 78% · 7d 31%");
    expect(m.headlineTitle).toBe("");
    // codex 하나만 — 종전의 '5h — · 7d 9%'
    expect(buildUsageBarModel([X("x1", { p7: 9 })], NOW, okFetch, noRedact).headline).toBe("5h — · 7d 9%");
  });
  it("창별 값은 그 제공자의 오래되지 않은(fresh·recent) 계정들의 최댓값 — 오래된 계정의 99% 는 끼지 않는다", () => {
    const rows = [
      C("c-fresh", { p5: 10, p7: 5, age: 10 }),
      C("c-recent", { p5: 60, p7: 20, age: 300, profiles: [".claude-2"] }),
      C("c-stale", { p5: 99, p7: 99, age: 7200, profiles: [".claude-3"] }),
      X("x1", { p7: 50 }),
    ];
    const m = buildUsageBarModel(rows, NOW, okFetch, noRedact);
    expect(m.headline).toBe("C 5h60%·7d20% │ X 7d50%");
  });
  it("그 제공자에 오래되지 않은 계정이 없으면 오래된 값에 `?` · 풀이는 (오래됨)", () => {
    const m = buildUsageBarModel([C("c1", { p5: 12, p7: 30 }), X("x1", { p7: 50, age: 7200 })], NOW, okFetch, noRedact);
    expect(m.headline).toBe("C 5h12%·7d30% │ X 7d50%?");
    expect(m.headlineTitle).toBe("Claude 5h 12% · 7d 30% / Codex 7d 50% (오래됨)");
    // 스냅샷도 오래된 값
    const s = buildUsageBarModel([C("c1", { p5: 12, p7: 30 }), A("a1", { p5: 40, src: "snapshot", age: 600 })], NOW, okFetch, noRedact);
    expect(s.headline).toBe("C 5h12%·7d30% │ A 5h40%?");
  });
  it("리셋 지난 창은 '리셋됨'(? 없음) · 값 없는 창은 생략", () => {
    const c = C("c1", { p7: 30 });
    c.rate = [win("5h", 78, NOW - 1), win("7d", 30)];
    const m = buildUsageBarModel([c, X("x1", { p7: 50 })], NOW, okFetch, noRedact);
    expect(m.headline).toBe("C 5h리셋됨·7d30% │ X 7d50%");
    expect(buildUsageBarModel([C("c1", {}), X("x1", { p7: 50 })], NOW, okFetch, noRedact).headline).toBe("C — │ X 7d50%");
  });
  it("제공자 순서(claude · codex · antigravity · 그 밖) · 머리글자(C·X·A · 그 밖은 표시명 첫 글자 대문자)", () => {
    const rows = [R("g1", { prov: "grok", src: "adapter:grok", p5: 7 }), A("a1", { p5: 40 }), X("x1", { p7: 50 }), C("c1", { p5: 12 })];
    const m = buildUsageBarModel(rows, NOW, okFetch, noRedact);
    expect(m.headline).toBe("C 5h12% │ X 7d50% │ A 5h40% │ G 5h7%");
    expect(m.headlineTitle).toBe("Claude 5h 12% / Codex 7d 50% / Antigravity 5h 40% / grok 5h 7%");
  });
  it("옛 표기 gemini 는 antigravity 와 한 제공자(머리글자 A) — 둘이 섞여도 제공자 하나", () => {
    const mixed = [R("g1", { prov: "gemini", src: "agy-rpc", p5: 20 }), A("a1", { p5: 40 })];
    expect(buildUsageBarModel(mixed, NOW, okFetch, noRedact).headline).toBe("5h 40% · 7d —"); // 한 제공자 → 종전 형식
    const withClaude = [...mixed, C("c1", { p5: 12 })];
    expect(buildUsageBarModel(withClaude, NOW, okFetch, noRedact).headline).toBe("C 5h12% │ A 5h40%");
  });
  it("관측 전 계정은 제공자 수에 들지 않는다 — 관측 전 codex 가 있어도 Claude 하나면 종전 형식", () => {
    const rows = [C("c1", { p5: 12, p7: 30 }), acct({ provider: "codex", account_id: "x0", updated_at: null, source: "", rate: [] })];
    const m = buildUsageBarModel(rows, NOW, okFetch, noRedact);
    expect(m.headline).toBe("5h 12% · 7d 30%");
    expect(m.headlineTitle).toBe("");
  });
  it("주 계정이 오래된 값이면 약식 요약의 끝에도 ' (오래됨)'", () => {
    const m = buildUsageBarModel([C("c1", { p5: 50, age: 7200 }), X("x1", { p7: 20, age: 7200 })], NOW, okFetch, noRedact);
    expect(m.headline).toBe("C 5h50%? │ X 7d20%? (오래됨)");
  });
  it("headlineSev 는 약식 요약에서도 주 계정 창들의 최고 심각도", () => {
    const m = buildUsageBarModel([C("c1", { p5: 95, p7: 10 }), X("x1", { p7: 50 })], NOW, okFetch, noRedact);
    expect(m.headline).toBe("C 5h95%·7d10% │ X 7d50%");
    expect(m.headlineSev).toBe("crit");
  });
});

describe("0.14.43 KPI 후보(kpiCandidates) — 숨김·리셋·사용 중·관측 나이·스냅샷", () => {
  const cand = (accts: AcctRow[], label = "5h", now = NOW, hidden?: ReadonlySet<string>): string[] => ids(kpiCandidates(accts, label, now, hidden));
  it("★옛 계정(100% · 리셋 전 · 사용 중 아님 · 3시간 전)은 후보가 아니다 — KPI 가 지난 100% 로 빨개지지 않는다", () => {
    const old = R("old", { age: 3 * 3600, p5: 100, in_use: false });
    const cur = R("cur", { age: 20, p5: 12, in_use: true });
    expect(cand([old])).toEqual([]);
    expect(cand([old, cur])).toEqual(["cur"]);
  });
  it("사용 중이면 관측 나이와 무관하게 후보(3시간 전이어도 · 스냅샷이어도)", () => {
    expect(cand([R("u1", { age: 3 * 3600, p5: 40, in_use: true })])).toEqual(["u1"]);
    expect(cand([R("u2", { age: 600, p5: 40, in_use: true, src: "snapshot" })])).toEqual(["u2"]);
  });
  it("판정 불가(null)·구버전(키 없음)은 관측 나이로 — 1800초 이하 후보 · 초과 제외(경계 포함)", () => {
    for (const inUse of [null, undefined] as const) {
      const row = (age: number, id: string) => R(id, { age, p5: 30, ...(inUse === undefined ? {} : { in_use: inUse }) });
      expect(cand([row(1799, "a"), row(1800, "b"), row(1801, "c")])).toEqual(["a", "b"]);
    }
    expect(cand([R("f", { age: 5, p5: 30, in_use: false })])).toEqual(["f"]); // 사용 중 아님이어도 신선하면 후보(조회 직후 세션 전환 직전의 값)
  });
  it("리셋 지난 창은 제외(사용 중이어도) · 다른 창 라벨은 따로 판정", () => {
    const a = R("a", { age: 5, in_use: true });
    a.rate = [win("5h", 99, NOW - 1), win("7d", 40, NOW + 3600)];
    expect(cand([a], "5h")).toEqual([]);
    expect(cand([a], "7d")).toEqual(["a"]);
    expect(cand([a], "5h", NOW - 10)).toEqual(["a"]); // 시계가 리셋 전이면 후보
  });
  it("스냅샷은 제외 · 숨김은 제외(사용 중이어도)", () => {
    expect(cand([R("s", { age: 10, p5: 30, src: "snapshot" })])).toEqual([]);
    const hid = R("hid", { age: 5, p5: 50, in_use: true });
    expect(cand([hid, R("shown", { age: 5, p5: 10 })], "5h", NOW, new Set([acctKey(hid)]))).toEqual(["shown"]);
  });
  it("창이 없는 계정(codex 5h)·값이 유한하지 않은 창은 제외", () => {
    expect(cand([R("cx", { prov: "codex", src: "rollout", p7: 20 })], "5h")).toEqual([]);
    expect(cand([R("cx", { prov: "codex", src: "rollout", p7: 20 })], "7d")).toEqual(["cx"]);
    const nan = R("nan", { age: 5 });
    nan.rate = [{ label: "5h", used_pct: Number.NaN, resets_at: null }];
    expect(cand([nan])).toEqual([]);
  });
  it("후보는 입력 행 그대로(같은 객체) · 입력 순서 유지 · 리셋 시각이 없는(null) 창도 후보", () => {
    const a = R("a", { age: 5 });
    a.rate = [win("5h", 101, NOW + 600)]; // 100 초과 값도 후보(값 판정은 호출측 몫)
    const b = R("b", { age: 5 });
    b.rate = [win("5h", 20, null)];
    const out = kpiCandidates([a, b], "5h", NOW);
    expect(out.length).toBe(2);
    expect(out[0]).toBe(a);
    expect(out[1]).toBe(b);
  });
  it("비정상 입력(배열 아님·객체 아닌 원소·rate 없음)에도 던지지 않는다", () => {
    expect(kpiCandidates(null as unknown as AcctRow[], "5h", NOW)).toEqual([]);
    expect(kpiCandidates([null, 3, "x", acct({ rate: null as unknown as [] })] as unknown as AcctRow[], "5h", NOW)).toEqual([]);
  });
});

describe("isOldObservation — Control Center 행의 '오래됨' 판정(30분 초과 또는 스냅샷)", () => {
  it("관측 나이 > 1800초 또는 snapshot · 관측 전은 아니다(그쪽은 '관측 전' 배지)", () => {
    expect(isOldObservation(R("a", { age: 1800 }), NOW)).toBe(false);
    expect(isOldObservation(R("a", { age: 1801 }), NOW)).toBe(true);
    expect(isOldObservation(R("a", { age: 5, src: "snapshot" }), NOW)).toBe(true);
    expect(isOldObservation(acct({ updated_at: null, source: "" }), NOW)).toBe(false);
  });
});

// ═════════ 0.14.43 (티켓 UI2 · 추가 과제) — Control Center Live KPI 전 좌석 폴백(aggSeatRates) ═════════
// 계정 병합 값이 없을 때 KPI '세션(5h)'·'주간(7d)' 는 전 좌석 usage.rate 의 최댓값으로 폴백한다. 좌석의 usage.rate 는 로그인을 바꾼 뒤에도 옛 계정의
// 마지막 값(예 99%)을 리셋 전까지 들고 있어, 그 값이 대표값으로 되살아났다(UI1 잔여 위험 R2). 창 단위로 뺀다: alert_eligible === false · 리셋 지난 창 ·
// 유한한 숫자가 아닌 사용률. 나머지(최댓값·가장 이른 리셋)는 종전과 같다.
describe("aggSeatRates — 전 좌석 폴백 집계(옛 좌석 값을 되살리지 않는다)", () => {
  type AggRow = { label?: unknown; used_pct?: unknown; resets_at?: unknown; alert_eligible?: unknown };
  const seat = (rate: unknown, extra: Record<string, unknown> = {}) => ({ role: "worker", state: "working", usage: { ctx_pct: 10, rate, ...extra } });
  const w = (label: string, used_pct: number, resets_at: number | null = NOW + 3600, alert_eligible?: boolean): AggRow => ({
    label,
    used_pct,
    resets_at,
    ...(alert_eligible === undefined ? {} : { alert_eligible }),
  });
  /** 종전 main.ts ccAggRate 본문 그대로(0.14.43 UI2 이전) — 동치 속성 검체의 기준. */
  function legacyAggRate(fleet: any[]): Record<string, { used: number; reset: number | null }> {
    const agg: Record<string, { used: number; reset: number | null }> = {};
    for (const f of fleet) {
      for (const x of f.usage?.rate ?? []) {
        const cur = agg[x.label] ?? { used: 0, reset: null };
        if (x.used_pct > cur.used) cur.used = x.used_pct;
        if (x.resets_at != null && (cur.reset == null || x.resets_at < cur.reset)) cur.reset = x.resets_at;
        agg[x.label] = cur;
      }
    }
    return agg;
  }

  it("★alert_eligible:false 창은 뺀다 — 로그인을 바꾼 옛 좌석의 99% 가 대표값이 되지 않는다", () => {
    const fleet = [seat([w("5h", 99, NOW + 600, false)]), seat([w("5h", 12, NOW + 600, true)])];
    expect(aggSeatRates(fleet, NOW)).toEqual({ "5h": { used: 12, reset: NOW + 600 } });
    // 종전에는 99 가 이겼다 — 그 차이가 이 과제의 전부다
    expect(legacyAggRate(fleet)["5h"].used).toBe(99);
  });
  it("부적격 창만 있으면 그 라벨은 아예 없다(0% 로 위장하지 않는다 — 호출측이 '값 없음' 경로를 탄다)", () => {
    expect(aggSeatRates([seat([w("5h", 99, NOW + 600, false), w("7d", 80, NOW + 600, false)])], NOW)).toEqual({});
    // 같은 좌석에서 한 창만 부적격이면 다른 창은 남는다
    expect(aggSeatRates([seat([w("5h", 99, NOW + 600, false), w("7d", 40, NOW + 600, true)])], NOW)).toEqual({ "7d": { used: 40, reset: NOW + 600 } });
  });
  it("alert_eligible 가 true 이거나 키가 없으면 넣는다(구버전 데몬은 키가 없다) · 부적격(false)이 아닌 값(null·0·문자열)도 '키 없음'처럼 넣는다", () => {
    expect(aggSeatRates([seat([w("5h", 30, NOW + 60, true)])], NOW)).toEqual({ "5h": { used: 30, reset: NOW + 60 } });
    expect(aggSeatRates([seat([w("5h", 30, NOW + 60)])], NOW)).toEqual({ "5h": { used: 30, reset: NOW + 60 } });
    for (const v of [null, 0, "false", "", undefined])
      expect({ 값: String(v), 결과: aggSeatRates([seat([{ label: "5h", used_pct: 30, resets_at: NOW + 60, alert_eligible: v }])], NOW) }).toEqual({
        값: String(v),
        결과: { "5h": { used: 30, reset: NOW + 60 } },
      });
  });
  it("★리셋 지난 창은 뺀다 — 키가 없는 구버전도 · alert_eligible:true 여도(데몬 판정 뒤에 리셋이 지났을 수 있다)", () => {
    for (const ae of [undefined, true] as const) {
      expect({ ae: String(ae), 결과: aggSeatRates([seat([w("5h", 99, NOW - 1, ae), w("7d", 88, NOW - 3600, ae)])], NOW) }).toEqual({ ae: String(ae), 결과: {} });
    }
    // 같은 라벨의 리셋 전 값이 있으면 그것만 남는다
    expect(aggSeatRates([seat([w("5h", 99, NOW - 1)]), seat([w("5h", 20, NOW + 100)])], NOW)).toEqual({ "5h": { used: 20, reset: NOW + 100 } });
  });
  it("리셋 경계 — windowView 의 '리셋됨' 규칙과 같다: nowSec ≥ resets_at 이면 뺀다 · 1초 전이면 넣는다", () => {
    expect(aggSeatRates([seat([w("5h", 50, NOW)])], NOW)).toEqual({}); // 같은 초 = 리셋됨
    expect(aggSeatRates([seat([w("5h", 50, NOW + 1)])], NOW)).toEqual({ "5h": { used: 50, reset: NOW + 1 } });
    expect(windowView(acct({ rate: [win("5h", 50, NOW)] }), "5h", NOW).state).toBe("rolled"); // 같은 경계를 사이드바가 이미 쓴다
    // 시계가 리셋 전이면(되돌린 시계) 넣는다
    expect(aggSeatRates([seat([w("5h", 50, NOW)])], NOW - 10)).toEqual({ "5h": { used: 50, reset: NOW } });
  });
  it("리셋 시각이 없거나(null·키 없음) 쓸 수 없는 값(문자열·NaN)이면 리셋 판정 없이 넣는다 — reset 은 null", () => {
    for (const r of [null, undefined, "soon", Number.NaN, Infinity]) {
      expect({ 리셋: String(r), 결과: aggSeatRates([seat([{ label: "5h", used_pct: 61, resets_at: r }])], NOW) }).toEqual({
        리셋: String(r),
        결과: { "5h": { used: 61, reset: null } },
      });
    }
    // 0·음수는 '유효한 리셋 시각'이 아니라 리셋 지남 판정에서 빠진다(windowView 와 같다) — 값은 넣는다
    for (const r of [0, -5]) expect(aggSeatRates([seat([{ label: "5h", used_pct: 61, resets_at: r }])], NOW)["5h"].used).toBe(61);
  });
  it("used_pct 가 유한한 숫자가 아니면 뺀다(문자열·null·undefined·NaN·±Infinity·불리언·객체·배열)", () => {
    for (const v of ["50", "99%", "", null, undefined, Number.NaN, Infinity, -Infinity, true, false, {}, [50], [], () => 1])
      expect({ 값: String(v), 결과: aggSeatRates([seat([{ label: "5h", used_pct: v, resets_at: NOW + 60 }])], NOW) }).toEqual({ 값: String(v), 결과: {} });
    // 오염된 창이 정상 창의 집계를 오염시키지 않는다
    expect(aggSeatRates([seat([{ label: "5h", used_pct: "99", resets_at: NOW + 60 }, w("5h", 7, NOW + 60)])], NOW)).toEqual({ "5h": { used: 7, reset: NOW + 60 } });
  });
  it("라벨이 비어 있지 않은 문자열이 아니면 뺀다(숫자·null·빈 문자열·객체)", () => {
    for (const l of [7, null, undefined, "", {}, [], true])
      expect({ 라벨: String(l), 결과: aggSeatRates([seat([{ label: l, used_pct: 50, resets_at: NOW + 60 }])], NOW) }).toEqual({ 라벨: String(l), 결과: {} });
    // 알 수 없는 라벨도 라벨대로 집계한다(종전과 같다)
    expect(aggSeatRates([seat([w("30d", 11, NOW + 60)])], NOW)).toEqual({ "30d": { used: 11, reset: NOW + 60 } });
  });
  it("집계 — 라벨별 사용률 최댓값 · 가장 이른 리셋(좌석이 여러 개) · 0 미만은 0 으로 접힌다(종전과 같다)", () => {
    const fleet = [
      seat([w("5h", 40, NOW + 900), w("7d", 10, NOW + 90000)]),
      seat([w("5h", 75, NOW + 300), w("7d", 55, NOW + 80000)]),
      seat([w("5h", 20, NOW + 100)]),
      seat([w("5h", -3, NOW + 5000)]),
    ];
    expect(aggSeatRates(fleet, NOW)).toEqual({ "5h": { used: 75, reset: NOW + 100 }, "7d": { used: 55, reset: NOW + 80000 } });
    expect(aggSeatRates([seat([w("5h", -3, NOW + 5000)])], NOW)).toEqual({ "5h": { used: 0, reset: NOW + 5000 } });
    // 리셋이 없는 창은 가장 이른 리셋 계산에 끼지 않는다
    expect(aggSeatRates([seat([w("5h", 10, null)]), seat([w("5h", 5, NOW + 77)])], NOW)).toEqual({ "5h": { used: 10, reset: NOW + 77 } });
  });
  it("★오염값 무해 — fleet 이 배열이 아니거나 좌석·usage·rate·창이 이상해도 던지지 않고 빈 집계", () => {
    for (const f of [null, undefined, {}, "x", 7, true, { length: 3 }, () => 1])
      expect({ fleet: String(f), 결과: aggSeatRates(f, NOW) }).toEqual({ fleet: String(f), 결과: {} });
    const junk = [
      null,
      7,
      "x",
      [],
      {},
      { usage: null },
      { usage: 7 },
      { usage: "x" },
      { usage: {} },
      { usage: { rate: null } },
      { usage: { rate: "x" } },
      { usage: { rate: {} } },
      { usage: { rate: 7 } },
      { usage: { rate: [null, 3, "x", [], {}, { label: "5h" }, { used_pct: 5 }] } },
    ];
    expect(aggSeatRates(junk, NOW)).toEqual({});
    // 정상 좌석이 섞여 있으면 그것만 남는다
    expect(aggSeatRates([...junk, seat([w("5h", 33, NOW + 60)])], NOW)).toEqual({ "5h": { used: 33, reset: NOW + 60 } });
    // 시계가 이상해도(NaN) 던지지 않는다 — 리셋 지남 판정이 안 걸릴 뿐이다
    expect(aggSeatRates([seat([w("5h", 33, NOW - 1000)])], Number.NaN)).toEqual({ "5h": { used: 33, reset: NOW - 1000 } });
  });
  it("프로토타입 없는 결과 — __proto__·constructor 같은 이상한 라벨이 와도 전역 객체를 건드리지 않고 그 라벨대로 집계된다", () => {
    const evil = ["__proto__", "constructor", "toString", "hasOwnProperty", "prototype"];
    const out = aggSeatRates([seat(evil.map((l, i) => w(l, 10 + i, NOW + 100)))], NOW);
    expect(Object.keys(out).sort()).toEqual([...evil].sort());
    for (const l of evil) expect(out[l].used).toBe(10 + evil.indexOf(l));
    expect(({} as Record<string, unknown>)["used"]).toBeUndefined(); // Object.prototype 이 오염되지 않았다
    expect(({} as Record<string, unknown>)["reset"]).toBeUndefined();
    expect(typeof Object.prototype.toString).toBe("function");
  });
  it("입력을 바꾸지 않는다 · 결과는 입력의 창 객체를 공유하지 않는다", () => {
    const fleet = [seat([w("5h", 40, NOW + 900, true), w("5h", 99, NOW - 5, false)]), seat([w("7d", 12, null)])];
    const before = JSON.stringify(fleet);
    const out = aggSeatRates(fleet, NOW);
    expect(JSON.stringify(fleet)).toBe(before);
    out["5h"].used = 1234;
    expect(JSON.stringify(fleet)).toBe(before);
  });
  it("★종전 동치 — 가산 키가 없고 리셋 전 · 유한한 값만 있는 fleet 은 종전 ccAggRate 와 같은 결과(결정론 난수 100판 · 가산 키 true 판 100판 추가)", () => {
    let s = 20261003 >>> 0;
    const rnd = (): number => {
      s = (Math.imul(s, 1664525) + 1013904223) >>> 0;
      return s / 4294967296;
    };
    const pickN = (n: number): number => Math.floor(rnd() * n);
    const LABELS = ["5h", "7d", "30d"];
    const makeFleet = (eligibleKey: boolean): unknown[] => {
      const fleet: unknown[] = [];
      const seats = pickN(7);
      for (let i = 0; i < seats; i++) {
        const t = rnd();
        if (t < 0.12) {
          fleet.push({ role: "r" + i, state: "idle" }); // usage 없음
          continue;
        }
        if (t < 0.2) {
          fleet.push({ role: "r" + i, usage: null });
          continue;
        }
        if (t < 0.28) {
          fleet.push({ role: "r" + i, usage: { ctx_pct: 5 } }); // rate 없음
          continue;
        }
        const rate: Record<string, unknown>[] = [];
        const n = pickN(5);
        for (let j = 0; j < n; j++) {
          const used = rnd() < 0.15 ? -pickN(4) : rnd() < 0.5 ? pickN(131) : Math.round(rnd() * 13000) / 100;
          const win: Record<string, unknown> = { label: LABELS[pickN(LABELS.length)], used_pct: used };
          const r = rnd();
          if (r < 0.25) win.resets_at = null;
          else if (r >= 0.35) win.resets_at = NOW + 1 + pickN(1_000_000); // 리셋 전
          // 0.25 ≤ r < 0.35: 키 없음
          if (eligibleKey) win.alert_eligible = true;
          rate.push(win);
        }
        fleet.push({ role: "r" + i, usage: { ctx_pct: pickN(100), rate } });
      }
      return fleet;
    };
    let nonEmpty = 0;
    for (const eligibleKey of [false, true])
      for (let k = 0; k < 100; k++) {
        const fleet = makeFleet(eligibleKey);
        const got = aggSeatRates(fleet, NOW);
        const want = legacyAggRate(fleet);
        expect({ 판: `${eligibleKey ? "true키" : "키없음"}#${k}`, 결과: got }).toEqual({ 판: `${eligibleKey ? "true키" : "키없음"}#${k}`, 결과: want });
        if (Object.keys(want).length) nonEmpty++;
      }
    expect(nonEmpty).toBeGreaterThan(120); // 시험이 공허하지 않다(빈 집계끼리만 맞은 것이 아니다)
  });
  it("종전과 달라지는 곳은 정확히 셋 — 부적격 창 · 리셋 지난 창 · 유한하지 않은 사용률(그 밖의 입력은 같다)", () => {
    const base = [w("5h", 30, NOW + 600)];
    const cases: [string, AggRow][] = [
      ["부적격", w("5h", 99, NOW + 600, false)],
      ["리셋 지남", w("5h", 99, NOW - 1)],
      ["문자열 사용률", { label: "5h", used_pct: "99", resets_at: NOW + 600 }],
    ];
    for (const [name, extra] of cases) {
      const fleet = [seat(base), seat([extra])];
      expect({ 사례: name, 새: aggSeatRates(fleet, NOW)["5h"].used }).toEqual({ 사례: name, 새: 30 });
      expect({ 사례: name, 종전이_다르다: legacyAggRate(fleet)["5h"].used !== 30 }).toEqual({ 사례: name, 종전이_다르다: true });
    }
  });
});
