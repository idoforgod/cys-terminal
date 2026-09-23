// U16(0.14.41) 말로 팀 만들기 1차 — 팀 제안 카드·확인 창의 순수 판정 + main.ts 배선 핀.
//
// 설계 정본: _reports/수정설계-0.14.41-오너18항목-20260923.md §3 U16
//   · 카드에 [확인 창 열기] → 확인 창(원문 그대로) → [만들기] = 기존 생성 경로(addDeptWorkspace)
//     + 이름·하는 일 전달 → **성공 뒤에만** feed_reply allow.
//   · 창 닫기·바깥 클릭 = 나중에(pending 유지) · deny 는 카드의 [만들지 않기] 버튼만 · 자동 팝업 없음.
// ★통합 메모: WP-A2(U17)의 openTeamCreateFlow/confirmModal 공유는 병합 뒤다 — 이 파일의 배선 핀은
//   함수 경계(runTeamProposalFlow)만 고정하므로 내부를 공유 흐름으로 바꿔도 핀 의미는 유지된다.
import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import {
  TEAM_CREATE_KIND,
  TEAM_DISPLAY_MAX,
  TEAM_PURPOSE_MAX,
  parseTeamProposal,
  buildTeamConfirm,
  teamCardText,
  teamCreateErrorText,
} from "./teamproposal";

const good = { v: 1, id: "tp-1726000000-00ab", display: "영상편집팀", purpose: "유튜브 영상을 편집한다.\n자막도 단다." };
const item = (body: unknown, extra: Record<string, unknown> = {}) => ({
  kind: TEAM_CREATE_KIND,
  request_id: good.id,
  body: typeof body === "string" ? body : JSON.stringify(body),
  ...extra,
});

describe("parseTeamProposal — 데몬 본문 스키마(v1)의 UI 쪽 재검증", () => {
  test("정상 → 원문 그대로", () => {
    const r = parseTeamProposal(item(good));
    expect(r.ok).toBe(true);
    if (r.ok) expect(r.spec).toEqual({ id: good.id, display: good.display, purpose: good.purpose });
  });

  test("형식 위반은 전부 거부(생성 버튼을 열지 않는다)", () => {
    const bad: unknown[] = [
      "not json",
      { ...good, v: 2 },
      { ...good, id: "tp-other-0001" }, // request_id 와 불일치
      { ...good, display: "" },
      { ...good, display: "가".repeat(TEAM_DISPLAY_MAX + 1) },
      { ...good, display: "팀\n둘" },
      { ...good, display: "팀\u202e둘" }, // REVIEW1 m1(c) invisible: RLO reorders on screen
      { ...good, display: "팀\u200b둘" }, // REVIEW1 m1(c) invisible: zero width space
      { ...good, display: "팀\u2028둘" }, // REVIEW1 m3: U+2028 line separator (Zl, bypassed old CONTROL-only check)
      { ...good, display: "팀\u2029둘" }, // REVIEW1 m3: U+2029 paragraph separator (Zp)
      { ...good, display: " 앞공백" },
      { ...good, purpose: "" },
      { ...good, purpose: "가".repeat(TEAM_PURPOSE_MAX + 1) },
      { ...good, purpose: "일\u0000" },
      { ...good, purpose: "일\r\n둘" },
      { ...good, extra: "숨은 필드" },
      { v: 1, id: good.id, display: "팀" },
    ];
    for (const b of bad) expect(parseTeamProposal(item(b)).ok).toBe(false);
    expect(parseTeamProposal({ ...item(good), kind: "permission" }).ok).toBe(false);
  });

  test("길이는 코드포인트 기준(한글·이모지 1자 = 1자) — Rust chars().count() 와 같은 단위", () => {
    expect(parseTeamProposal(item({ ...good, display: "가".repeat(TEAM_DISPLAY_MAX) })).ok).toBe(true);
    expect(parseTeamProposal(item({ ...good, display: "😀".repeat(TEAM_DISPLAY_MAX) })).ok).toBe(true);
  });
});

describe("buildTeamConfirm — 확인 창 문구(오너가 본 내용 = 만들어지는 내용)", () => {
  const spec = { id: good.id, display: good.display, purpose: good.purpose };
  test("버튼: [만들기] / [나중에] — 취소가 거부가 아니다", () => {
    const c = buildTeamConfirm(spec, { firstTeam: false });
    expect(c.yesLabel).toBe("만들기");
    expect(c.noLabel).toBe("나중에");
  });
  test("본문: 이름·하는 일 원문·자리 최대 5·로그인·나중에의 뜻", () => {
    const c = buildTeamConfirm(spec, { firstTeam: false });
    expect(c.body).toContain(good.display);
    expect(c.body).toContain(good.purpose); // 원문 그대로(요약·가공 금지)
    expect(c.body).toContain("최대 5");
    expect(c.body).toContain("로그인");
    expect(c.body).toContain("만들지 않기");
    expect(c.body).not.toContain("CEO");
  });
  test("첫 팀: CEO 전환 + 대표 창 지침 재주입 고지(반박 M5) / 모름: 조건형", () => {
    expect(buildTeamConfirm(spec, { firstTeam: true }).body).toContain("CEO");
    expect(buildTeamConfirm(spec, { firstTeam: true }).body).toContain("입력");
    expect(buildTeamConfirm(spec, { firstTeam: null }).body).toContain("첫 팀이면");
  });
  test("카드 요약도 원문을 그대로 보인다", () => {
    const t = teamCardText(spec);
    expect(t).toContain(good.display);
    expect(t).toContain(good.purpose);
  });
});

describe("teamCreateErrorText — 생성 실패 사유(무음 실패 금지)", () => {
  test("상한(8)·형식(2)·계정(5)·그 외", () => {
    expect(teamCreateErrorText("dept-create:8:[cys-dept] allocate 거부")).toContain("8");
    expect(teamCreateErrorText("dept-create:2:bad")).toContain("형식");
    expect(teamCreateErrorText("dept-create:5:acct")).toContain("로그인 폴더");
    expect(teamCreateErrorText("some other error")).toContain("some other error");
  });
});

describe("Rust SOT 대조(사본 드리프트 차단)", () => {
  test("kind·한도가 src/team_spec.rs 와 같다", () => {
    const rs = readFileSync(new URL("../../src/team_spec.rs", import.meta.url), "utf-8");
    expect(rs).toContain(`pub const KIND: &str = "${TEAM_CREATE_KIND}";`);
    expect(rs).toContain(`pub const DISPLAY_MAX_CHARS: usize = ${TEAM_DISPLAY_MAX};`);
    expect(rs).toContain(`pub const PURPOSE_MAX_CHARS: usize = ${TEAM_PURPOSE_MAX};`);
  });
});

describe("main.ts 배선 핀", () => {
  const src = readFileSync(new URL("./main.ts", import.meta.url), "utf-8");
  const strip = (s: string) =>
    s
      .split("\n")
      .map((l) => (l.trimStart().startsWith("//") ? "" : l.replace(/\s\/\/.*$/, "")))
      .join("\n");
  const code = strip(src);
  const fnBody = (name: string): string => {
    const a = code.indexOf(`async function ${name}(`);
    expect(a).toBeGreaterThan(0);
    const b = code.indexOf("\n}\n", a);
    expect(b).toBeGreaterThan(a);
    return code.slice(a, b);
  };

  test("재진입 가드가 첫 await 앞에서 세워지고 finally 에서 풀린다(WP-A2 U17 공유 가드 teamFlowBusy)", () => {
    const f = fnBody("runTeamProposalFlow");
    const g = f.indexOf("teamFlowBusy = true");
    expect(g).toBeGreaterThan(0);
    expect(g).toBeLessThan(f.indexOf("await "));
    expect(f).toContain("finally");
    expect(f.indexOf("teamFlowBusy = false")).toBeGreaterThan(f.indexOf("finally"));
  });

  test("진행 중 가드는 openTeamCreateFlow/confirmAndCreateTeam 과 같은 모듈 변수를 공유한다(중복 확인 창 방지)", () => {
    // teamFlowBusy·notifyTeamFlowBusy·deptBtnEl 이 main.ts 안에 정확히 한 번만 선언돼 있어야
    // "공유"다(각 함수가 자기 사본을 따로 선언하면 재진입 가드가 갈라져 두 확인 창이 동시에 뜰 수 있다).
    const decl = (pat: RegExp) => (code.match(pat) ?? []).length;
    expect(decl(/\blet teamFlowBusy\b/g)).toBe(1);
    expect(decl(/\bfunction notifyTeamFlowBusy\b/g)).toBe(1);
    expect(decl(/\bconst deptBtnEl\b/g)).toBe(1);
    // runTeamProposalFlow 도 그 하나뿐인 변수·함수를 그대로 쓴다(자기 사본 재선언 없음).
    const f = fnBody("runTeamProposalFlow");
    expect(f).toContain("teamFlowBusy");
    expect(f).toContain("deptBtnEl()");
  });

  test("순서: 확인 창 → 기존 생성 경로(addDeptWorkspace) → 성공 뒤에만 allow", () => {
    const f = fnBody("runTeamProposalFlow");
    const c = f.indexOf("confirmModal(");
    const a = f.indexOf("addDeptWorkspace(");
    const y = f.indexOf('decision: "allow"');
    expect(c).toBeGreaterThan(0);
    expect(a).toBeGreaterThan(c);
    expect(y).toBeGreaterThan(a);
    expect(f).not.toContain('decision: "deny"'); // 확인 창의 '나중에' 는 거부가 아니다
  });

  test("생성 RPC 호출처는 addDeptWorkspace 하나뿐(새 생성 경로 0)", () => {
    expect(code.split('invoke("allocate_dept_daemon"').length - 1).toBe(1);
    const a = fnBody("addDeptWorkspace");
    expect(a).toContain('invoke("allocate_dept_daemon"');
    expect(a).toContain("teamSpec");
  });

  test("카드: team-create 는 Allow/Deny 대신 [확인 창 열기]·[만들지 않기] · deny 는 그 버튼에서만", () => {
    const i = code.indexOf('classifyPendingFeed(item) === "team-create"');
    expect(i).toBeGreaterThan(0);
    const seg = code.slice(i, code.indexOf("} else if", i + 10));
    expect(seg).toContain("확인 창 열기");
    expect(seg).toContain("만들지 않기");
    expect(seg).toContain('decision: "deny"');
    expect(seg).toContain("runTeamProposalFlow(");
    expect(seg).not.toContain('"Allow"');
  });

  test("REVIEW1 m5: 생성 도중 탭을 닫아 회수됐으면(addDeptWorkspace→null) allow 를 보내지 않는다", () => {
    const a = fnBody("addDeptWorkspace");
    expect(a).toContain("return dup ?? null");
    const f = fnBody("runTeamProposalFlow");
    const chk = f.indexOf("if (!created)");
    const y = f.indexOf('decision: "allow"');
    expect(chk).toBeGreaterThan(0);
    expect(y).toBeGreaterThan(chk);
    const guardSeg = f.slice(chk, f.indexOf("\n    }", chk));
    expect(guardSeg).toContain("return");
  });

  test("자동 팝업 없음: feed.item.created 경로는 확인 창을 열지 않는다", () => {
    const i = code.indexOf('if (name === "feed.item.created")');
    expect(i).toBeGreaterThan(0);
    const seg = code.slice(i, code.indexOf("refreshFeed();", i));
    expect(seg).not.toContain("runTeamProposalFlow");
    expect(seg).not.toContain("confirmModal");
  });
});
