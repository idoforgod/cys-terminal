// 성찰 1회차 R1F-UB (S4 B1) — 팀원 부팅 안내의 **완료 판정을 좌석 목록의 역할**로 한다(deptprogress.ts 순수 도우미 검체).
//
// 왜 바꿨나: 종전 판정은 편성 결과 feed(`formation-*`)의 `socket_slug` 가 새로 만든 부서 탭의 소켓으로 풀릴 때만 반응했다. 그런데 편성 도구(javis_formation.py `_feed`)는
// 소켓 지정 없이 `cys feed push` 를 불러 **본부 데몬**으로 간다 — 화면의 `socketForSlug` 에는 부서 소켓만 들어 있어 본부 slug 는 풀리지 않는다. 그래서 화면으로 만든 팀에서는
// 「팀 준비 완료」가 나오지 않고 「팀원을 켜는 중」이 15분 상한까지 남았다(S4 B1 · 합성 이벤트로만 통과하던 배선 검체가 이것을 못 잡았다).
// 새 판정은 3초 틱이 **이미 받는** 그 팀 소켓의 좌석 목록(`list_surfaces` → `surfaces[].role`)에서 의무 역할 다섯이 모두 붙었는가로 한다(새 RPC·새 타이머 0).
//
// ★이 파일의 입력은 데몬 `surface.list` 한 줄의 **실제 꼴**(handlers.rs surface.list 의 키 전부)이다 — 손으로 만든 이벤트가 아니다.
// ★'준비 완료'라고 단정하지 않는다: 자리가 붙은 것과 에이전트가 실제로 떴는지는 다르고, 화면은 뒤를 모른다.
import { describe, it, expect } from "bun:test";
import { readFileSync } from "node:fs";
import {
  DEPT_FORMATION_CAP_SECS,
  deptLiveRoles,
  deptSeatedCount,
  deptFormationVerdict,
  deptFormationText,
  formatDeptMinutes,
} from "./deptprogress";
import { DEPT_SEAT_ROLES } from "./deptcreate";

const read = (rel: string): string => readFileSync(new URL(rel, import.meta.url), "utf-8");

/** 데몬 surface.list 한 줄의 실제 꼴 — 키 전부(필요한 것만이 아니다). 값은 지어낸 값. */
function row(sid: number, role: string | null, over: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    surface_id: sid,
    surface_ref: `surface:${sid}`,
    title: "zsh",
    role,
    cmd: "zsh",
    cwd: "/Users/tester/work",
    live_cwd: "/Users/tester/work",
    pid: 41000 + sid,
    exited: false,
    created_at: 1_800_000_000,
    pending_input_bytes: 0,
    pending_input_human_bytes: 0,
    input_paste_open: false,
    seat: "occupied",
    env_injected: true,
    created_by: null,
    claude_config_dir: null,
    agent: null,
    agent_alive: null,
    awakened_at: null,
    directive_verified: null,
    ack_nonce_ok: null,
    ack_source: null,
    boot_nonce_generation: null,
    alt_screen: false,
    line_count: 12,
    cwd_blocked: null,
    usage: null,
    ...over,
  };
}
const FIVE = ["master", "cso", "worker", "reviewer-gemini", "reviewer-codex"];
const five = (): Record<string, unknown>[] => FIVE.map((r, i) => row(i + 1, r));

describe("의무 역할의 출처 — DEPT_SEAT_ROLES(deptcreate.ts) 하나", () => {
  it("다섯 역할 · 순서까지 편성 로스터와 같다 — 이름은 0.14.42 와 같다(javis_formation.py REQUIRED_ROLES · 드리프트 핀은 deptcreate.test.ts)", () => {
    expect([...DEPT_SEAT_ROLES]).toEqual(FIVE);
    const py = read("../../cysjavis-pack/bin/javis_formation.py");
    const m = /^REQUIRED_ROLES\s*=\s*\(([^)]*)\)/m.exec(py);
    expect(m).not.toBeNull();
    expect([...(m as RegExpExecArray)[1].matchAll(/"([^"]+)"/g)].map((x) => x[1])).toEqual(FIVE);
  });
  it("★deptprogress.ts 는 역할 이름을 따로 적지 않고 그 상수를 가져다 쓴다(두 번째 목록이 생기면 어긋날 수 있다)", () => {
    const code = read("./deptprogress.ts")
      .split("\n")
      .map((l) => (l.trimStart().startsWith("//") || l.trimStart().startsWith("*") || l.trimStart().startsWith("/**") ? "" : l))
      .join("\n");
    expect(code).toContain('import { DEPT_SEAT_ROLES } from "./deptcreate";');
    for (const lit of ['"reviewer-gemini"', '"reviewer-codex"', '"cso"', '"worker"', '"master"']) expect({ 리터럴: lit, 있음: code.includes(lit) }).toEqual({ 리터럴: lit, 있음: false });
  });
});

describe("deptLiveRoles — list_surfaces 의 surfaces → 종료하지 않은 좌석의 역할(IPC 데이터라 전부 의심한다)", () => {
  it("★실제 꼴 다섯 줄 → 다섯 역할(순서 보존)", () => {
    expect(deptLiveRoles(five())).toEqual(FIVE);
  });
  it("종료한 좌석(exited: true)은 붙은 자리가 아니다 — 역할이 있어도 뺀다", () => {
    const list = five();
    list[0] = row(1, "master", { exited: true });
    expect(deptLiveRoles(list)).toEqual(["cso", "worker", "reviewer-gemini", "reviewer-codex"]);
  });
  it("역할 없는 셸(role: null)·문자열이 아닌 역할·빈 문자열은 건너뛴다", () => {
    const list = [row(1, null), row(2, "cso"), row(3, 7 as unknown as string), row(4, ""), row(5, "worker")];
    expect(deptLiveRoles(list)).toEqual(["cso", "worker"]);
  });
  it("빈 목록은 빈 배열(데몬이 성공적으로 0개를 돌려준 것) — '목록을 못 받음'(null)과 다르다", () => {
    expect(deptLiveRoles([])).toEqual([]);
  });
  it("★배열이 아니면 null — 목록을 받지 못한 것과 같다(판정을 건너뛴다)", () => {
    for (const bad of [undefined, null, {}, "surfaces", 7, true, { surfaces: [] }]) expect({ 입력: bad, 값: deptLiveRoles(bad) }).toEqual({ 입력: bad, 값: null });
  });
  it("원소가 객체가 아니어도(null·숫자·문자열) 던지지 않고 건너뛴다", () => {
    expect(deptLiveRoles([null, 3, "x", undefined, row(1, "master")])).toEqual(["master"]);
  });
  it("exited 키가 없는 줄은 살아 있는 것으로 본다(같은 데이터를 읽는 refreshPaneTitles 의 규칙 — `s.exited ? 종료 : 살아 있음`)", () => {
    const r = row(1, "master") as Record<string, unknown>;
    delete r.exited;
    expect(deptLiveRoles([r])).toEqual(["master"]);
  });
});

describe("deptSeatedCount — 의무 역할 가운데 붙어 있는 서로 다른 역할의 수(0~5)", () => {
  it("다섯이면 5 · 하나씩 빠질 때마다 4 · 비면 0", () => {
    expect(deptSeatedCount(FIVE)).toBe(5);
    for (const gone of FIVE) expect({ 빠진: gone, 수: deptSeatedCount(FIVE.filter((r) => r !== gone)) }).toEqual({ 빠진: gone, 수: 4 });
    expect(deptSeatedCount([])).toBe(0);
  });
  it("같은 역할이 여럿이어도 한 번만 센다(워커 둘 ≠ 두 자리)", () => {
    expect(deptSeatedCount(["master", "worker", "worker", "worker"])).toBe(2);
  });
  it("★이름이 정확히 같은 것만 센다 — 변형(worker-2 · cso-1)·일회용(cso-fresh-<epoch>)·대소문자·공백·접두만 같은 이름은 의무 자리가 아니다", () => {
    for (const bad of ["worker-2", "cso-1", "cso-fresh-1800000000", "Master", " master", "master ", "reviewer", "reviewer-gemini-x", "reviewer-codex2", "ceo", "dept-master"])
      expect({ 역할: bad, 수: deptSeatedCount([bad]) }).toEqual({ 역할: bad, 수: 0 });
  });
  it("배열이 아니거나 문자열이 아닌 원소가 섞여도 던지지 않는다", () => {
    for (const bad of [undefined, null, {}, "master", 5]) expect({ 입력: bad, 수: deptSeatedCount(bad) }).toEqual({ 입력: bad, 수: 0 });
    expect(deptSeatedCount([null, 1, {}, "master", ["cso"]])).toBe(1);
  });
});

describe("deptFormationVerdict — 한 틱의 판정(skip · wait · seated · check)", () => {
  it("★목록을 못 받은 틱(null · undefined · 배열 아님)은 skip — 완료로도 확인 필요로도 치지 않는다(상한을 한참 넘겨도)", () => {
    for (const roles of [null, undefined, {}, "x"]) for (const sec of [0, 100, DEPT_FORMATION_CAP_SECS, DEPT_FORMATION_CAP_SECS * 10])
      expect({ roles, sec, 판정: deptFormationVerdict(roles, sec) }).toEqual({ roles, sec, 판정: { verdict: "skip", seated: 0 } });
  });
  it("다섯 역할이 모두 붙었으면 seated — 경과와 무관하다(0초든 상한 뒤든)", () => {
    for (const sec of [0, 30, 252, DEPT_FORMATION_CAP_SECS - 1, DEPT_FORMATION_CAP_SECS, DEPT_FORMATION_CAP_SECS + 600])
      expect({ sec, 판정: deptFormationVerdict(FIVE, sec) }).toEqual({ sec, 판정: { verdict: "seated", seated: 5 } });
  });
  it("★다섯이 안 됐고 상한 전이면 wait · 자리 수를 함께 돌려준다", () => {
    expect(deptFormationVerdict(["master"], 0)).toEqual({ verdict: "wait", seated: 1 });
    expect(deptFormationVerdict(["master", "cso", "worker", "reviewer-gemini"], DEPT_FORMATION_CAP_SECS - 1)).toEqual({ verdict: "wait", seated: 4 });
    expect(deptFormationVerdict([], 10)).toEqual({ verdict: "wait", seated: 0 });
  });
  it("★다섯이 안 됐는데 상한(900초)에 닿았으면 check — 경계 899/900", () => {
    const four = ["master", "cso", "worker", "reviewer-codex"];
    expect(deptFormationVerdict(four, DEPT_FORMATION_CAP_SECS - 1).verdict).toBe("wait");
    expect(deptFormationVerdict(four, DEPT_FORMATION_CAP_SECS)).toEqual({ verdict: "check", seated: 4 });
    expect(deptFormationVerdict([], DEPT_FORMATION_CAP_SECS + 1)).toEqual({ verdict: "check", seated: 0 });
    expect(deptFormationVerdict(["worker-2", "cso-fresh-1"], DEPT_FORMATION_CAP_SECS)).toEqual({ verdict: "check", seated: 0 });
  });
  it("★실제 꼴 입력: 좌석 목록을 deptLiveRoles 로 읽어 그대로 판정한다 — 네 자리 + 종료한 마스터는 다섯이 아니다", () => {
    const list = five();
    expect(deptFormationVerdict(deptLiveRoles(list), 120)).toEqual({ verdict: "seated", seated: 5 });
    list[0] = row(1, "master", { exited: true });
    expect(deptFormationVerdict(deptLiveRoles(list), 120)).toEqual({ verdict: "wait", seated: 4 });
    expect(deptFormationVerdict(deptLiveRoles(list), DEPT_FORMATION_CAP_SECS)).toEqual({ verdict: "check", seated: 4 });
    expect(deptFormationVerdict(deptLiveRoles({ surfaces: list }), DEPT_FORMATION_CAP_SECS)).toEqual({ verdict: "skip", seated: 0 }); // 응답 껍데기가 그대로 들어와도 '못 받음'
  });
});

describe("최종 화면 문구 — 자리 판정의 두 결과(WORKLOG 에 전문을 붙이는 문구와 같은 줄)", () => {
  it("seated — 제목 「팀 자리가 모두 붙었습니다」 · 본문 「자리 5개가 모두 붙었습니다 · 걸린 시간 <분>」(formatDeptMinutes)", () => {
    expect(deptFormationText({ seats: 5, elapsedSec: 252, state: "seated" })).toEqual({ title: "팀 자리가 모두 붙었습니다", body: "자리 5개가 모두 붙었습니다 · 걸린 시간 4분" });
    expect(deptFormationText({ seats: 5, elapsedSec: 30, state: "seated" }).body).toBe("자리 5개가 모두 붙었습니다 · 걸린 시간 1분 미만");
    expect(deptFormationText({ seats: 5, elapsedSec: 60, state: "seated" }).body).toBe("자리 5개가 모두 붙었습니다 · 걸린 시간 1분");
    // 자리 수 인자와 무관하게 '5개'(의무 역할 수) — 판정이 이미 다섯을 확인했다
    expect(deptFormationText({ seats: 0, elapsedSec: 252, state: "seated" }).body).toBe(`자리 ${DEPT_SEAT_ROLES.length}개가 모두 붙었습니다 · 걸린 시간 ${formatDeptMinutes(252)}`);
  });
  it("check — 제목 「팀원 켜기 — 확인 필요」 · 본문 「아직 <N>자리입니다 — Control Center 에서 자리 상태를 확인하세요 · 경과 <분>」", () => {
    expect(deptFormationText({ seats: 3, elapsedSec: DEPT_FORMATION_CAP_SECS, state: "check" })).toEqual({
      title: "팀원 켜기 — 확인 필요",
      body: "아직 3자리입니다 — Control Center 에서 자리 상태를 확인하세요 · 경과 15분",
    });
    expect(deptFormationText({ seats: 0, elapsedSec: 905, state: "check" }).body).toBe("아직 0자리입니다 — Control Center 에서 자리 상태를 확인하세요 · 경과 15분");
  });
  it("★'준비 완료'·'정상'을 단정하지 않는다 — 에이전트가 실제로 떴는지는 화면이 모른다(두 문구 어디에도 완료·준비됐·정상·성공·켜졌습니다가 없다)", () => {
    for (const state of ["seated", "check"] as const) {
      const t = deptFormationText({ seats: 4, elapsedSec: 300, state });
      for (const lie of ["완료", "준비됐", "준비 완료", "정상", "성공", "켜졌습니다", "작동"]) expect({ 상태: state, 낱말: lie, 있음: (t.title + t.body).includes(lie) }).toEqual({ 상태: state, 낱말: lie, 있음: false });
    }
  });
});
