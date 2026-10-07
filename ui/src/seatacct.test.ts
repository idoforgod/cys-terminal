// ★0.14.45 부서 카드 노드별 계정 줄 · 사용량 패널 역매핑 — 순수 판정(seatacct.ts) + 배선 핀(main.ts·style.css 를 데이터로 읽는다).
import { describe, it, expect } from "bun:test";
import { readFileSync } from "node:fs";
import {
  accountUsers,
  buildAcctIndex,
  buildWsAccountGroups,
  parseSeatAccount,
  roleShort,
  wsAccountLineText,
  SEAT_ACCT_MISMATCH_LABEL,
  SEAT_ACCT_UNKNOWN_LABEL,
  type SeatAcctSig,
} from "./seatacct";
import { accountCardLabels, accountDisplayLabels, buildUsageBarModel, type AcctRow } from "./usagebar";

const NOW = 1_800_000_000_000; // ms
const STALE = 30_000;
const plain = (s: string) => s;
const redact = (s: string) => `#${s.length}`;

const claude = (uuid: string): unknown => ({ provider: "claude", agent: "claude", account_id: uuid, profile: null, state: "known" });
const sig = (role: string | null, account: unknown, extra: Partial<SeatAcctSig> = {}): SeatAcctSig => ({
  present: true,
  raw: account,
  role,
  exited: false,
  at: NOW - 1000,
  ...extra,
});
const rows: AcctRow[] = [
  { provider: "claude", account_id: "u-4", label: "four@example.test", profiles: [".claude-4"], current_profiles: [".claude-4"], in_use: true, updated_at: 1 },
  { provider: "claude", account_id: "u-1", label: "one@example.test", profiles: [".claude-1"], current_profiles: [".claude-1"], in_use: true, updated_at: 1 },
  { provider: "claude", account_id: "u-0", label: "zero@example.test", profiles: [".claude"], current_profiles: [".claude"], alias: "개인", in_use: true },
  { provider: "codex", account_id: "default", label: "OpenAI Codex", profiles: [".codex"], in_use: true, updated_at: 1 },
  { provider: "antigravity", account_id: "default", label: "Antigravity (agy)", profiles: [".gemini/antigravity-cli"], in_use: true },
];
const idx = buildAcctIndex(rows, accountDisplayLabels(rows, plain));
/** 해석 결과의 판정 칸만(known · key 또는 사유) — toMatchObject 대신(bun-env.d.ts 최소 matcher 계약). */
const verdict = (sa: { known: boolean; key: string; state: string }) => (sa.known ? { known: true, key: sa.key } : { known: false, state: sa.state });
const seats = (...xs: (SeatAcctSig | undefined)[]) => xs.map((s, i) => ({ sid: i + 1, sig: s }));

describe("parseSeatAccount — 모르는 것은 모른다", () => {
  it("known 만 계정 키를 낸다 · 그 밖은 사유 코드", () => {
    expect(verdict(parseSeatAccount(sig("master", claude("u-4")), NOW, STALE))).toEqual({ known: true, key: "claude:u-4" });
    expect(verdict(parseSeatAccount(sig("master", undefined, { present: false }), NOW, STALE))).toEqual({ known: false, state: "no_key" });
    expect(verdict(parseSeatAccount(sig("master", null), NOW, STALE))).toEqual({ known: false, state: "none" });
    expect(verdict(parseSeatAccount(sig("master", "u-4"), NOW, STALE))).toEqual({ known: false, state: "bad" });
    for (const st of ["folder_unknown", "no_login", "unread", "unsupported"])
      expect(verdict(parseSeatAccount(sig("w", { provider: "claude", account_id: null, profile: ".claude-2", state: st }), NOW, STALE))).toEqual({ known: false, state: st });
    // state 가 known 이라도 account_id 가 없으면(결측) 계정이 아니다
    expect(parseSeatAccount(sig("w", { provider: "claude", account_id: null, state: "known" }), NOW, STALE).known).toBe(false);
    expect(parseSeatAccount(sig("w", { provider: "claude", account_id: "", state: "known" }), NOW, STALE).known).toBe(false);
    // 낡은 신호(데몬 무응답)는 옛 계정을 말하지 않는다
    expect(verdict(parseSeatAccount(sig("w", claude("u-4"), { at: NOW - STALE - 1 }), NOW, STALE))).toEqual({ known: false, state: "stale" });
  });
  it("역할 약칭", () => {
    expect([roleShort("master", 1), roleShort("reviewer-codex", 2), roleShort("reviewer-gemini", 3), roleShort(null, 7)]).toEqual(["master", "rv-codex", "rv-gemini", "#7"]);
  });
});

describe("부서 카드 — 노드별 계정 묶음", () => {
  it("같은 계정을 쓰는 노드는 한 묶음 · 패널과 같은 이름 · 제공자 순 · 이메일은 본문에 없다", () => {
    const g = buildWsAccountGroups(
      seats(
        sig("worker", claude("u-4")),
        sig("master", claude("u-4")),
        sig("cso", claude("u-1")),
        sig("reviewer-codex", { provider: "codex", agent: "codex", account_id: "default", profile: null, state: "known" }),
        sig("reviewer-gemini", { provider: "antigravity", agent: "gemini", account_id: "default", profile: null, state: "known" }),
      ),
      idx,
      plain,
      false,
      NOW,
      STALE,
    );
    expect(wsAccountLineText(g)).toBe("claude-1 cso · claude-4 master·worker · Codex rv-codex · Antigravity rv-gemini");
    const body = g.map((x) => `${x.label} ${x.roles.join(" ")}`).join(" ");
    expect(body.includes("@")).toBe(false);
    // 이메일은 툴팁에만
    expect(g[1].title).toContain("four@example.test");
    expect(g[1].title).toContain("노드: master, worker");
    expect(g[2].title).toContain("이메일은 cys 가 읽지 않습니다");
  });
  it("별명이 있으면 별명(패널과 같은 이름)", () => {
    const g = buildWsAccountGroups(seats(sig("master", claude("u-0"))), idx, plain, false, NOW, STALE);
    expect(wsAccountLineText(g)).toBe("개인 master");
  });
  it("🔒 가림이면 툴팁 이메일도 가림 함수를 거친다", () => {
    const g = buildWsAccountGroups(seats(sig("master", claude("u-4"))), idx, redact, true, NOW, STALE);
    expect(g[0].title).toContain("#17");
    expect(g[0].title.includes("four@example.test")).toBe(false);
  });
  it("모르는 계정은 '계정 미확인' 한 묶음(맨 끝) — 짐작한 이름을 쓰지 않는다 · 사유는 툴팁", () => {
    const g = buildWsAccountGroups(
      seats(
        sig("master", claude("u-4")),
        sig("worker", { provider: "claude", agent: "claude", account_id: null, profile: ".claude-2", state: "no_login" }),
        sig("cso", undefined, { present: false }),
        sig("worker-2", claude("u-1"), { at: NOW - STALE - 5 }),
      ),
      idx,
      plain,
      false,
      NOW,
      STALE,
    );
    expect(g.map((x) => x.label)).toEqual(["claude-4", SEAT_ACCT_UNKNOWN_LABEL]);
    const u = g[1];
    expect(u.unknown).toBe(true);
    expect(u.roles).toEqual(["cso", "worker", "worker-2"]);
    expect(u.title).toContain("로그아웃");
    expect(u.title).toContain("구버전 데몬");
    expect(u.title).toContain("데몬 응답이 끊겨");
    // 폴더 이름(.claude-2 → claude-2)으로 짐작한 라벨이 나오지 않는다
    expect(wsAccountLineText(g).includes("claude-2")).toBe(false);
  });
  it("★(M5) 계정 불일치(검증되지 않은 기록 폴더)는 '계정 불일치·확인 필요' 묶음 — 확인된 계정으로 보이지 않고 · 미확인 묶음 앞 · 기록 폴더는 툴팁에만(🔒 가림)", () => {
    const mm = { provider: "claude", agent: "claude", account_id: null, profile: ".cys/claude", state: "mismatch" };
    expect(verdict(parseSeatAccount(sig("worker", mm), NOW, STALE))).toEqual({ known: false, state: "mismatch" });
    const g = buildWsAccountGroups(
      seats(sig("master", claude("u-4")), sig("worker", mm), sig("cso", { provider: "claude", agent: "claude", account_id: null, profile: ".claude-2", state: "no_login" })),
      idx,
      plain,
      false,
      NOW,
      STALE,
    );
    expect(g.map((x) => x.label)).toEqual(["claude-4", SEAT_ACCT_MISMATCH_LABEL, SEAT_ACCT_UNKNOWN_LABEL]);
    expect(g[1].unknown).toBe(true);
    expect(g[1].roles).toEqual(["worker"]);
    expect(g[1].title).toContain("확인된 것으로 표시하지 않습니다");
    expect(g[1].title).toContain("기록된 폴더: .cys/claude");
    expect(g[1].title).toContain("/config");
    expect(g[2].roles).toEqual(["cso"]);
    // 기록 폴더 이름으로 짐작한 라벨이 본문에 없다 · 역매핑에도 붙지 않는다
    expect(wsAccountLineText(g)).toBe(`claude-4 master · ${SEAT_ACCT_MISMATCH_LABEL} worker · ${SEAT_ACCT_UNKNOWN_LABEL} cso`);
    expect(accountUsers([{ name: "d", seats: seats(sig("worker", mm)) }], NOW, STALE).size).toBe(0);
    // 🔒 가림이면 툴팁의 기록 폴더도 꼬리표만
    const h = buildWsAccountGroups(seats(sig("worker", { ...mm, profile: "/Users/x/.cys/claude" })), idx, redact, true, NOW, STALE);
    expect(h[0].title.includes("/Users/x")).toBe(false);
  });
  it("셸 pane·종료 좌석·아직 조회 안 된 좌석은 뺀다", () => {
    const g = buildWsAccountGroups(
      seats(sig(null, null), sig("worker", claude("u-4"), { exited: true }), undefined, sig(null, undefined, { present: false })),
      idx,
      plain,
      false,
      NOW,
      STALE,
    );
    expect(g).toEqual([]);
  });
  it("계정 행이 아직 없으면(사용량 조회 전) claude 는 좌석 설정 폴더 이름 · 그 밖은 제공자", () => {
    const empty = buildAcctIndex([], new Map());
    const g = buildWsAccountGroups(
      seats(sig("master", { provider: "claude", agent: "claude", account_id: "u-9", profile: ".claude-9", state: "known" }), sig("rv", { provider: "codex", account_id: "default", state: "known" })),
      empty,
      plain,
      false,
      NOW,
      STALE,
    );
    expect(wsAccountLineText(g)).toBe("claude-9 master · Codex rv");
    expect(g[0].title).toContain("(이메일 미확인)");
  });
});

describe("역매핑 — 계정 → 부서·노드", () => {
  const depts = [
    { name: "CEO", seats: seats(sig("worker", claude("u-4")), sig("master", claude("u-1"))) },
    { name: "일반저술출판부", seats: seats(sig("master", claude("u-4")), sig("worker", claude("u-4")), sig("cso", { provider: "claude", account_id: null, state: "unread" })) },
    { name: "심층리서치부", seats: seats(sig("master", claude("u-4"), { exited: true })) },
  ];
  it("여러 부서가 한 계정을 쓰면 부서별로 · 미확인·종료 좌석은 어느 계정에도 붙지 않는다", () => {
    const m = accountUsers(depts, NOW, STALE);
    expect(m.get("claude:u-4")).toEqual(["CEO worker", "일반저술출판부 master·worker"]);
    expect(m.get("claude:u-1")).toEqual(["CEO master"]);
    expect([...m.keys()].sort()).toEqual(["claude:u-1", "claude:u-4"]);
  });
  it("사용량 모델이 계정마다 사용처를 싣는다 — 쓰는 곳이 없는 계정은 키 자체가 없다", () => {
    const users = accountUsers(depts, NOW, STALE);
    const model = buildUsageBarModel(rows, 2, { everOk: true, failStreak: 0, okAtSec: 2 }, plain, false, undefined, { mode: "one", users });
    const all = [model.primary, ...model.others].filter(Boolean) as { label: string; users?: string[] }[];
    const by = new Map(all.map((x) => [x.label, x.users]));
    expect(by.get("claude-4")).toEqual(["CEO worker", "일반저술출판부 master·worker"]);
    expect(by.get("claude-1")).toEqual(["CEO master"]);
    expect(by.has("Codex") && by.get("Codex")).toBe(undefined);
    for (const x of all) if (x.label !== "claude-4" && x.label !== "claude-1") expect("users" in x).toBe(false);
    // 인자를 생략하면 종전 모델과 같다(키 없음)
    const old = buildUsageBarModel(rows, 2, { everOk: true, failStreak: 0, okAtSec: 2 }, plain, false, undefined, { mode: "one" });
    expect(JSON.stringify(old).includes('"users"')).toBe(false);
  });
});

describe("배선 핀(main.ts·style.css)", () => {
  const read = (rel: string) => readFileSync(new URL(rel, import.meta.url), "utf-8");
  const src = read("./main.ts");
  const css = read("./style.css");
  const mod = read("./seatacct.ts");
  it("좌석 계정은 org.status 10초 틱이 적는다(새 RPC·타이머 0) · 키 부재를 present=false 로", () => {
    const a = src.indexOf("async function refreshSidebarStatus");
    const b = src.indexOf("\n}\n", a);
    const body = src.slice(a, b);
    expect(body).toContain("seatAccts.set(`${sock}#${n.surface_id}`");
    expect(body).toContain('present: Object.prototype.hasOwnProperty.call(n, "account")');
    expect(src.includes('invoke("seat_accounts')).toBe(false);
  });
  it("카드는 buildTab 의 탭 머리 뒤에 붙고, 실패는 삼킨다 · 렌더는 textContent 로만", () => {
    expect(src).toContain("const acctLine = ws.pending ? null : buildWsAcctLine(ws);");
    const a = src.indexOf("function buildWsAcctLine(");
    const b = src.indexOf("\n}\n", a);
    const body = src.slice(a, b);
    expect(body).toContain("catch");
    expect(body.includes("innerHTML")).toBe(false);
    expect(body).toContain("ccAcctLabel"); // 🔒 가림 함수
  });
  it("★(C1) 카드 라벨은 🔒 상태와 무관하게 가린 꼬리표 — 본문에 이메일 원문 0 · (C2) 계정 표는 렌더당 한 번", () => {
    // 순수: 같은 이름 둘(별명 없음 · 폴더 라벨 같음)에 이메일이 있어도 카드 라벨에는 '@' 가 없고 패널의 🔒 켬 라벨과 같다.
    const twins: AcctRow[] = [
      { provider: "claude", account_id: "u-a", label: "alpha@example.test", profiles: [".claude-9"], current_profiles: [".claude-9"], in_use: true },
      { provider: "claude", account_id: "u-b", label: "beta@example.test", profiles: [".claude-9"], current_profiles: [".claude-9"], in_use: true },
    ];
    const hash6 = (x: string) => `h${x.length}`;
    const card = accountCardLabels(twins, hash6);
    const panelLocked = accountDisplayLabels(twins, (x) => `#${hash6(x)}`);
    const panelOpen = accountDisplayLabels(twins, (x) => x);
    for (const a of twins) {
      expect(card.get(a)!.includes("@")).toBe(false);
      expect(card.get(a)).toBe(panelLocked.get(a)!);
      expect(panelOpen.get(a)!.includes("@")).toBe(true); // 패널(🔒 끔)만 이메일 꼬리표 — 카드는 아니다
    }
    expect(card.get(twins[0])).not.toBe(card.get(twins[1]));
    // 배선: 카드 표는 accountCardLabels(ccHash6) 로 만들고(ccAcctLabel 아님), buildWsAcctLine 은 렌더 캐시를 쓴다.
    const a = src.indexOf("function wsAcctIndex(");
    const idxBody = src.slice(a, src.indexOf("\n}\n", a));
    expect(idxBody).toContain("accountCardLabels(visible, ccHash6)");
    expect(idxBody.includes("accountDisplayLabels(")).toBe(false);
    const r = src.indexOf("function renderWsTabs(");
    const renderBody = src.slice(r, src.indexOf("\n}\n", r));
    expect(renderBody).toContain("wsAcctIdxForRender = wsAcctIndex();");
    expect(renderBody).toContain("wsAcctIdxForRender = null;");
    const b = src.indexOf("function buildWsAcctLine(");
    const lineBody = src.slice(b, src.indexOf("\n}\n", b));
    expect(lineBody).toContain("wsAcctIndexCached()");
    expect(lineBody.includes("wsAcctIndex()")).toBe(false);
  });
  it("사용량 패널은 역매핑을 모델 인자로만 받는다(renderUsageBar 안에 nodeSig 0)", () => {
    expect(src).toContain("users: accountUsersNow()");
  });
  it("좌석 제거 시 계정 캐시도 지운다", () => {
    expect(src.split("seatAccts.delete(").length - 1).toBe(2);
  });
  it("CSS 와 모듈 불변식(최상위 부수효과·구형 WebKit 비호환 문법 0)", () => {
    expect(css).toContain(".ws-tab .ws-accts");
    expect(css).toContain("#wsbar-usage .usage-users");
    for (const bad of ["localStorage", "sessionStorage", "document.", "window.", "setTimeout", "setInterval", ".at(", "findLast", "structuredClone", "Object.hasOwn", "replaceAll", "(?<"])
      expect({ bad, 있음: mod.includes(bad) }).toEqual({ bad, 있음: false });
  });
});
