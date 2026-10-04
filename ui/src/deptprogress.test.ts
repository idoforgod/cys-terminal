// 0.14.43 GU — 「팀 직접 만들기」 대기 문구(경과·단계)와 팀원 부팅 안내(deptprogress.ts 순수 도우미) 회귀 핀.
//
// 무엇을 지키는가(티켓 GU):
//   · 대기 문구는 거짓 약속을 하지 않는다 — 옛 "최대 십여 초" 는 실측(25~30초 · 느린 PC 1분+)과 달랐다. 90초 미만은 실측 범위, 90초부터 '평소보다 오래'.
//   · 단계 표지(`[cys-dept] @stage <키>`)는 키 7종만 한글 줄이 되고 모르는 키·빈 값은 단계 줄 없이 경과만 보인다(구 팩에도 견딘다).
//   · 표지 파서는 Rust 쪽 같은 이름의 파서와 **같은 벡터표**를 잰다(이 파일이 src-tauri/src/main.rs 의 표를 소스에서 읽어 자기 표와 대조한다).
//   · 팀원 부팅 안내(R1F-UB 개정 · 성찰 2회차에서 5상태로): 문구 5상태(켜는 중 · 자리가 모두 붙음 · 15분 경과 · 팀 데몬 무응답 · 자리 정체) · 갱신 열쇠(자리 수·경과 분·45초 칸) · 15분 상한 · 첫 자리 창(60초).
//     완료 판정(좌석 목록의 역할 다섯)은 deptprogressseats.test.ts 가 지킨다 — 편성 결과 feed 를 읽던 판정·사유 정제(deptFormationStateOfKind·deptFormationDetail)는 걷었다(S4 B1).
//   · 화면 문구의 수치는 **잰 만큼만**(S4 M1): 사전 검사는 「10~30초」(실측: 개발 맥 12.8초 · 윈도우 11 러너 28.4초 — '최대 12초'도 '약 12초'도 아니다) · 팀원이 켜지는 데는 「보통 5분 안팎」(실측 1회 약 5분 — 3분 쪽은 잰 적이 없다).
//   · 신뢰할 수 없는 입력(이벤트 payload)은 걸러 낸다 — 단계 키 정규식.
//   · 순수 모듈 불변식: 최상위 부수효과 0 · 문서/창/저장소 낱말 0 · 구형 WKWebView 비호환 문법 0.
//   · ★성찰 2회차 R2F-UI(A2 B-1 · A3 M1): 팀원 안내는 **설치 여부에 기대지 않는다** — 편성 도구는 설치된 프로그램(claude·agy·codex)의 역할만 띄우므로(미설치 역할은 건너뛰어 정상 종결 partial·pending-cli)
//     「부서장·CSO·워커·리뷰어가 차례로 켜집니다」 를 조건 없이 말하면 거짓이다. 켜는 중 문구는 '설치된 프로그램의 자리' 로 조건을 달고 · 모든 문구의 자리 수는 붙은 의무 역할 수이며(「지금 N자리」 — 탭의 칸 수 — 는 없다) ·
//     15분 상한(「15분 경과」)은 경고가 아닌 일반 알림이고 · 좌석 목록을 못 받는 팀은 「팀 데몬이 응답하지 않습니다」(경고)로 따로 말한다.
//   · ★후속(정체 판정 · A2 B-1 최소 수정안 2): 부서장 자리가 붙어 있고(M ≥ 1) M < 5 인데 붙은 수가 3분(DEPT_FORMATION_STALL_SECS = 180) 동안 늘지 않으면 「팀원 켜기 — 자리가 더 붙지 않습니다」(일반 알림) —
//     순수 판정(deptFormationStalled)은 경계(179.999초 아직 · 180초부터)·자리 수 조건(0 과 5 는 아님)·모르는 입력(false)을, 문구는 「3분」 이 상수에서 파생되는 것을 이 파일이 지킨다(점검 배선은 deptprogresswiring.test.ts).
//   · ★후속(정체 판정 — A2 B-1 최소 수정안 2): claude 만 깐 PC 는 3자리에서 더 늘지 않는데 「켜는 중」 이 15분까지 갱신됐다. 부서장 자리가 붙어 있고(M ≥ 1) 다섯이 안 됐고(M < 5) 붙은 수가 180초 이상 늘지 않으면
//     「팀원 켜기 — 자리가 더 붙지 않습니다」(일반 알림)를 한 번 알리고 갱신을 접는다 — 순수 판정(deptFormationStalled)·문구·등급·상수(180초 · 문구의 「3분」 은 상수에서 파생)를 이 파일이 지킨다(배선은 deptprogresswiring.test.ts).
import { describe, it, expect } from "bun:test";
import { readFileSync } from "node:fs";
import {
  DEPT_TYPICAL_SECS,
  DEPT_SLOW_FACTOR,
  DEPT_FORMATION_CAP_SECS,
  DEPT_FORMATION_TOAST_PREFIX,
  DEPT_FORMATION_REFRESH_SECS,
  DEPT_FORMATION_LIST_SILENT_SECS,
  DEPT_FORMATION_STALL_SECS,
  DEPT_FIRST_SEAT_WINDOW_SECS,
  DEPT_FIRST_SEAT_TEXT,
  deptStageLabel,
  deptPendingText,
  formatDeptElapsed,
  formatDeptMinutes,
  parseDeptStageLine,
  sanitizeDeptStageKey,
  deptProgressId,
  parseDeptProgressPayload,
  deptFormationToastId,
  deptFormationText,
  deptFormationNoticeKey,
  deptFormationNoticeKind,
  deptFormationListSilent,
  deptFormationStalled,
  deptFormationCapped,
  deptFirstSeatPending,
  deptFirstSeatRemainingMs,
  type DeptFormationState,
} from "./deptprogress";

const read = (rel: string) => readFileSync(new URL(rel, import.meta.url), "utf-8");

// ── 문안 전문(티켓 문구 그대로 — 모듈 상수로 다시 짓지 않고 리터럴로 적는다: 문구가 바뀌면 여기서 적색) ──
const NORMAL = (elapsed: string): string => `팀을 만드는 중입니다 — 보통 30초 안팎, 컴퓨터에 따라 1분 넘게 걸릴 수 있어요 · 경과 ${elapsed}`;
const SLOW = (elapsed: string): string => `평소보다 오래 걸리고 있습니다 — 그대로 기다려 주세요(중간에 닫으면 만들던 팀이 정리됩니다) · 경과 ${elapsed}`;
const STAGES: [string, string][] = [
  ["reserve", "팀 번호를 잡는 중"],
  ["probe", "이미 켜진 데몬이 있는지 확인하는 중(10~30초)"],
  ["spawn", "데몬을 켜는 중"],
  ["wait", "데몬이 팩을 설치하는 중(파일 수백 개)"],
  ["up", "데몬이 켜졌습니다 — 설정을 심는 중"],
  ["seat", "부서장 자리를 여는 중"],
  ["done", "마무리하는 중"],
];

describe("상수 — 90초 경계의 근거", () => {
  it("DEPT_TYPICAL_SECS=30 · DEPT_SLOW_FACTOR=3 · 상한 900초(15분) · 첫 자리 창 60초", () => {
    expect(DEPT_TYPICAL_SECS).toBe(30);
    expect(DEPT_SLOW_FACTOR).toBe(3);
    expect(DEPT_TYPICAL_SECS * DEPT_SLOW_FACTOR).toBe(90);
    expect(DEPT_FORMATION_CAP_SECS).toBe(900);
    expect(DEPT_FIRST_SEAT_WINDOW_SECS).toBe(60);
    expect(DEPT_FORMATION_TOAST_PREFIX).toBe("dept-formation:");
  });
  it("★후속(정체 판정): 자리 정체 상한은 180초(3분) — 15분 상한보다 짧고(그 앞에서 알려야 한다) 토스트 기본 수명(60초)·갱신 칸(45초)보다 길다 · 정수 분이라 문구의 「N분」 이 딱 떨어진다", () => {
    expect(DEPT_FORMATION_STALL_SECS).toBe(180);
    expect(DEPT_FORMATION_STALL_SECS).toBeLessThan(DEPT_FORMATION_CAP_SECS);
    expect(DEPT_FORMATION_STALL_SECS).toBeGreaterThan(DEPT_FORMATION_REFRESH_SECS);
    expect(DEPT_FORMATION_STALL_SECS % 60).toBe(0);
  });
  it("★R2F-UI: 좌석 목록 공백 상한은 60초 — 토스트 기본 수명(60초)과 같은 선(그 안에 목록이 한 번도 안 오면 안내가 이미 사라졌을 때다)", () => {
    expect(DEPT_FORMATION_LIST_SILENT_SECS).toBe(60);
    const tt = read("./toastttl.ts");
    expect(tt).toContain("export const STICKY_TTL_MS = 60_000;");
    expect(DEPT_FORMATION_LIST_SILENT_SECS * 1000).toBe(60_000);
  });
  it("수명 갱신 칸(45초)은 sticky 토스트 기본 수명(60초)보다 짧다 — 갱신 사이에 토스트가 사라지지 않는다", () => {
    expect(DEPT_FORMATION_REFRESH_SECS).toBe(45);
    const tt = read("./toastttl.ts");
    expect(tt).toContain("export const STICKY_TTL_MS = 60_000;");
    expect(DEPT_FORMATION_REFRESH_SECS * 1000).toBeLessThan(60_000);
  });
});

describe("deptStageLabel — 키 7종 + 모르는 키", () => {
  for (const [key, label] of STAGES) {
    it(`${key} → 「${label}」`, () => {
      expect(deptStageLabel(key)).toBe(label);
    });
  }
  it("모르는 키·빈 값·null·undefined → null(단계 줄을 그리지 않는다)", () => {
    for (const bad of ["", " ", "Reserve", "RESERVE", "reserve ", " reserve", "예약", "finish", "stage", "r"]) {
      expect({ 키: bad, 라벨: deptStageLabel(bad) }).toEqual({ 키: bad, 라벨: null });
    }
    expect(deptStageLabel(null)).toBeNull();
    expect(deptStageLabel(undefined)).toBeNull();
  });
  it("★S4 M1 · W11 5차: 사전 검사 문구는 「10~30초」 — 실측이 맥 12.8초·윈도우 11 러너 28.4초라 한 숫자('최대 12초'·'약 12초')로 약속하지 않는다 · 어느 단계 문구에도 '최대'가 없다", () => {
    expect(deptStageLabel("probe")).toBe("이미 켜진 데몬이 있는지 확인하는 중(10~30초)");
    expect(deptStageLabel("probe")).not.toContain("12초");
    for (const [key] of STAGES) expect({ 키: key, 최대: (deptStageLabel(key) ?? "").includes("최대") }).toEqual({ 키: key, 최대: false });
    for (const rel of ["./deptprogress.ts", "./main.ts"]) expect({ 파일: rel, 최대12초: read(rel).includes("최대 12초") }).toEqual({ 파일: rel, 최대12초: false });
  });
  it("프로토타입 이름·문자열이 아닌 값에도 속지 않는다", () => {
    for (const bad of ["constructor", "__proto__", "toString", "hasOwnProperty", "valueOf", "prototype"]) {
      expect({ 키: bad, 라벨: deptStageLabel(bad) }).toEqual({ 키: bad, 라벨: null });
    }
    for (const bad of [0, 1, true, {}, [], ["reserve"], () => "reserve"]) {
      expect(deptStageLabel(bad as unknown as string)).toBeNull();
    }
  });
});

describe("deptPendingText — 경과 × 단계 유/무", () => {
  // [경과(초), 어느 문구('n'=보통 · 's'=평소보다 오래), 경과 표기]
  const CASES: [number, "n" | "s", string][] = [
    [0, "n", "0초"],
    [12, "n", "12초"],
    [59, "n", "59초"],
    [60, "n", "1분 0초"],
    [65, "n", "1분 5초"],
    [89, "n", "1분 29초"],
    [90, "s", "1분 30초"],
    [95, "s", "1분 35초"],
    [3600, "s", "60분 0초"],
    [-3, "n", "0초"],
    [Number.NaN, "n", "0초"],
  ];
  for (const [sec, kind, elapsed] of CASES) {
    it(`경과 ${String(sec)}초 → ${kind === "n" ? "보통 문구" : "평소보다 오래"} · 「경과 ${elapsed}」`, () => {
      const want = kind === "n" ? NORMAL(elapsed) : SLOW(elapsed);
      // 단계 없음(undefined·null·모르는 키) → sub 는 null
      for (const stage of [undefined, null, "", "zzz"]) {
        expect({ 단계: stage, 값: deptPendingText(sec, stage) }).toEqual({ 단계: stage, 값: { main: want, sub: null } });
      }
      // 단계 있음 → main 은 같고 sub 는 `지금: <라벨>`
      for (const [key, label] of STAGES) {
        expect(deptPendingText(sec, key)).toEqual({ main: want, sub: `지금: ${label}` });
      }
      // stage 인자를 생략한 호출도 같다
      expect(deptPendingText(sec)).toEqual({ main: want, sub: null });
    });
  }
  it("★90초 경계 — 89.999초는 보통 문구, 90초부터 '평소보다 오래'(돌연변이 M3: 경계 뒤집기)", () => {
    expect(deptPendingText(89, null).main).toBe(NORMAL("1분 29초"));
    expect(deptPendingText(89.999, null).main).toBe(NORMAL("1분 29초"));
    expect(deptPendingText(90, null).main).toBe(SLOW("1분 30초"));
    expect(deptPendingText(90.0001, null).main).toBe(SLOW("1분 30초"));
    // 두 문구는 서로 다르고, 90초 미만 전 구간이 보통·90초 이상 전 구간이 오래다(연속 구간 전수).
    let flips = 0;
    let prevSlow = false;
    for (let s = 0; s <= 600; s++) {
      const slow = deptPendingText(s, null).main.startsWith("평소보다 오래");
      expect({ 초: s, 오래: slow }).toEqual({ 초: s, 오래: s >= 90 });
      if (slow !== prevSlow) flips++;
      prevSlow = slow;
    }
    expect(flips).toBe(1);
  });
  it("소수·무한대·숫자 아님은 0 으로 접거나 내린다 — 이상한 값이 화면을 어지럽히지 않는다", () => {
    expect(deptPendingText(12.9, null).main).toBe(NORMAL("12초"));
    expect(deptPendingText(Number.POSITIVE_INFINITY, null).main).toBe(NORMAL("0초"));
    expect(deptPendingText(Number.NEGATIVE_INFINITY, null).main).toBe(NORMAL("0초"));
    expect(deptPendingText("abc" as unknown as number, null).main).toBe(NORMAL("0초"));
    expect(deptPendingText(undefined as unknown as number, null).main).toBe(NORMAL("0초"));
    expect(deptPendingText(1e12, null).main).toBe(SLOW("5999분 59초")); // 상한 359999초
    expect(deptPendingText(1e12, null).main).not.toContain("e+");
  });
  it("★옛 거짓 약속 「십여 초」 가 어떤 입력에서도 다시 나오지 않는다(재유입 차단)", () => {
    for (const sec of [0, 1, 12, 30, 59, 60, 89, 90, 95, 600, 3600, -1, Number.NaN]) {
      for (const [key] of [...STAGES, ["zzz", ""]]) {
        const t = deptPendingText(sec, key);
        expect(t.main.includes("십여")).toBe(false);
        expect((t.sub ?? "").includes("십여")).toBe(false);
      }
    }
    for (const rel of ["./deptprogress.ts", "./main.ts"]) {
      expect({ 파일: rel, 옛문구: read(rel).includes("최대 십여 초") }).toEqual({ 파일: rel, 옛문구: false });
    }
  });
  it("경과 표기 도우미 — 60초 미만 `N초` · 이상 `M분 S초`", () => {
    expect(formatDeptElapsed(0)).toBe("0초");
    expect(formatDeptElapsed(59)).toBe("59초");
    expect(formatDeptElapsed(60)).toBe("1분 0초");
    expect(formatDeptElapsed(125)).toBe("2분 5초");
    expect(formatDeptElapsed(-1)).toBe("0초");
    expect(formatDeptMinutes(0)).toBe("1분 미만");
    expect(formatDeptMinutes(59)).toBe("1분 미만");
    expect(formatDeptMinutes(60)).toBe("1분");
    expect(formatDeptMinutes(119)).toBe("1분");
    expect(formatDeptMinutes(120)).toBe("2분");
    expect(formatDeptMinutes(Number.NaN)).toBe("1분 미만");
  });
});

// ── 단계 표지 파서 — Rust 와 같은 벡터표 ──
/** [줄, 기대 키 | null] — src-tauri/src/main.rs 의 `GU_STAGE_VECTORS`(JSON)와 **같은 표**다(아래 대조 검체가 둘을 맞춘다). */
const STAGE_VECTORS: [string, string | null][] = [
  ["[cys-dept] @stage reserve", "reserve"],
  ["[cys-dept] @stage probe", "probe"],
  ["[cys-dept] @stage spawn", "spawn"],
  ["[cys-dept] @stage wait", "wait"],
  ["[cys-dept] @stage up", "up"],
  ["[cys-dept] @stage seat", "seat"],
  ["[cys-dept] @stage done", "done"],
  ["[cys-dept] @stage reserve\n", "reserve"],
  ["[cys-dept] @stage reserve\r\n", "reserve"],
  ["[cys-dept] @stage reserve\r", "reserve"],
  ["[cys-dept] @stage a1_b-2", "a1_b-2"],
  ["[cys-dept] @stage -", "-"],
  ["[cys-dept] @stage _", "_"],
  ["[cys-dept] @stage 7", "7"],
  ["[cys-dept] @stage abcdefghijklmnopqrstuvwxyz012345", "abcdefghijklmnopqrstuvwxyz012345"],
  ["[cys-dept] @stage abcdefghijklmnopqrstuvwxyz0123456", null],
  ["[cys-dept] @stage ", null],
  ["[cys-dept] @stage", null],
  ["[cys-dept] @stage reserve ", null],
  ["[cys-dept] @stage  reserve", null],
  ["[cys-dept] @stage re serve", null],
  ["[cys-dept] @stage Reserve", null],
  ["[cys-dept] @stage RESERVE", null],
  ["[cys-dept] @stage 예약", null],
  ["[cys-dept] @stage rés", null],
  ["[cys-dept] @stage a.b", null],
  ["[cys-dept] @stage a/b", null],
  ["[cys-dept] @stage a\u0000b", null],
  ["[cys-dept] @stage reserve\n\n", null],
  ["[cys-dept] @stage reserve\r\r\n", null],
  ["[cys-dept] @stage reserve extra", null],
  ["[cys-dept] @stage=reserve", null],
  ["[cys-dept] stage reserve", null],
  ["cys-dept @stage reserve", null],
  ["[cys-dept]  @stage reserve", null],
  [" [cys-dept] @stage reserve", null],
  ["x [cys-dept] @stage reserve", null],
  ["[CYS-DEPT] @stage reserve", null],
  ["[cys-dept] @STAGE reserve", null],
  ["[cys-dept] allocate 완료", null],
  ["", null],
  ["\n", null],
];

describe("parseDeptStageLine — 정상·접두 다름·키에 공백/한글/대문자/33자·빈 줄", () => {
  it("벡터표 전수", () => {
    for (const [line, want] of STAGE_VECTORS) {
      expect({ 입력: line, 키: parseDeptStageLine(line) }).toEqual({ 입력: line, 키: want });
    }
  });
  it("정상 7키 · 32자는 통과 · 33자는 거부 · 키 문자 집합", () => {
    for (const [key] of STAGES) expect(parseDeptStageLine(`[cys-dept] @stage ${key}`)).toBe(key);
    expect(parseDeptStageLine("[cys-dept] @stage " + "a".repeat(32))).toBe("a".repeat(32));
    expect(parseDeptStageLine("[cys-dept] @stage " + "a".repeat(33))).toBeNull();
    expect(parseDeptStageLine("[cys-dept] @stage Abc")).toBeNull(); // 대문자
    expect(parseDeptStageLine("[cys-dept] @stage a b")).toBeNull(); // 공백
    expect(parseDeptStageLine("[cys-dept] @stage 한글")).toBeNull(); // 한글
  });
  it("문자열이 아닌 값은 null", () => {
    for (const bad of [null, undefined, 7, {}, [], true]) expect(parseDeptStageLine(bad as unknown as string)).toBeNull();
  });
  it("★Rust 검체의 벡터표(src-tauri/src/main.rs GU_STAGE_VECTORS)와 이 표가 같다 — 두 언어가 같은 입력에서 같은 답을 낸다", () => {
    const rs = read("../../src-tauri/src/main.rs");
    const open = 'const GU_STAGE_VECTORS: &str = r##"';
    const a = rs.indexOf(open);
    expect(a).toBeGreaterThan(0);
    const b = rs.indexOf('"##;', a);
    expect(b).toBeGreaterThan(a);
    const table = JSON.parse(rs.slice(a + open.length, b)) as [string, string | null][];
    expect(table).toEqual(STAGE_VECTORS);
    for (const [line, want] of table) expect({ 입력: line, 키: parseDeptStageLine(line) }).toEqual({ 입력: line, 키: want });
  });
  it("sanitizeDeptStageKey — 정규식(영소문자·숫자·_·- · 1~32자)만 통과", () => {
    expect(sanitizeDeptStageKey("seat")).toBe("seat");
    expect(sanitizeDeptStageKey("a-b_c9")).toBe("a-b_c9");
    for (const bad of ["", "Seat", "se at", "seat\n", "예약", "a".repeat(33), 5, null, undefined, {}, []]) {
      expect(sanitizeDeptStageKey(bad)).toBeNull();
    }
  });
});

describe("진행 id·이벤트 payload", () => {
  it("deptProgressId — `dp-<탭 번호>` · 탭마다(호출마다) 다르다", () => {
    expect(deptProgressId(1)).toBe("dp-1");
    expect(deptProgressId(42)).toBe("dp-42");
    expect(deptProgressId(1)).not.toBe(deptProgressId(2));
  });
  it("parseDeptProgressPayload — {id, stage} 만 받는다(이벤트는 신뢰하지 않는다)", () => {
    expect(parseDeptProgressPayload({ id: "dp-3", stage: "wait" })).toEqual({ id: "dp-3", stage: "wait" });
    expect(parseDeptProgressPayload({ id: "dp-3", stage: "wait", extra: 1 })).toEqual({ id: "dp-3", stage: "wait" });
    for (const bad of [
      null,
      undefined,
      "x",
      7,
      [],
      [{ id: "dp-3", stage: "wait" }],
      {},
      { id: "dp-3" },
      { stage: "wait" },
      { id: 3, stage: "wait" },
      { id: "", stage: "wait" },
      { id: "x".repeat(65), stage: "wait" },
      { id: "dp-3", stage: "Wait" },
      { id: "dp-3", stage: "wa it" },
      { id: "dp-3", stage: 5 },
      { id: "dp-3", stage: "<img src=x onerror=1>" },
    ]) {
      expect({ payload: JSON.stringify(bad) ?? "undefined", 값: parseDeptProgressPayload(bad) }).toEqual({ payload: JSON.stringify(bad) ?? "undefined", 값: null });
    }
    expect(parseDeptProgressPayload({ id: "x".repeat(64), stage: "up" })).toEqual({ id: "x".repeat(64), stage: "up" });
  });
  it("deptFormationToastId — `dept-formation:<소켓>`", () => {
    expect(deptFormationToastId("/tmp/a/cys.sock")).toBe("dept-formation:/tmp/a/cys.sock");
    expect(deptFormationToastId("S1").startsWith(DEPT_FORMATION_TOAST_PREFIX)).toBe(true);
  });
});

// ★R2F-UI(A2 B-1 · A3 M1 · n13): 이 describe 는 종전 「3상태(booting · seated · check)」 였다 — 종전 문구의 단정(「부서장·CSO·워커·리뷰어가 차례로 켜집니다」 · 「지금 N자리」 — 탭의 칸 수 ·
//   경고색 「팀원 켜기 — 확인 필요 · 아직 N자리입니다」)이 편성의 실제(설치된 프로그램의 역할만 붙는다)와 어긋나 새 문구로 고쳤다. 아래 각 검체의 이름·기대값은 새 문안 그대로이고,
//   종전 검체의 취지(보통 시간은 잰 만큼만 · 완료 낱말 금지 · 이상한 입력 방어 · 모르는 상태는 켜는 중)는 새 상태 silent 까지 넓혀 유지했다.
// ★후속(정체 판정): 이 describe 는 「4상태」 였다 — 자리 정체(stall)를 더해 5상태가 됐다. 기존 검체의 취지(보통 시간은 잰 만큼만 · 완료 낱말 금지 · 옛 문구 부재 · 이상한 입력 방어 · 십여 초 부재)는 stall 까지 넓혔다.
describe("deptFormationText — 5상태(booting · seated · check · silent · stall) — 설치 여부를 모르는 채 말한다", () => {
  const BOOT = (seats: number, elapsed: string): string =>
    `설치된 프로그램(claude·agy·codex)의 자리가 차례로 붙습니다(최대 5자리 · 보통 5분 안팎) · 붙은 자리 ${seats} · 경과 ${elapsed}`;
  const CHECK = (seats: number): string => `붙은 자리 ${seats}개 — 설치하지 않은 프로그램(claude·agy·codex)의 자리는 생기지 않습니다. 그 밖이면 Control Center 에서 자리 상태를 확인하세요`;
  const SILENT = (elapsed: string): string => `좌석 목록을 받지 못했습니다 — Control Center 에서 팀 상태를 확인하세요 · 경과 ${elapsed}`;
  const STALL = (seats: number, elapsed: string): string =>
    `3분 동안 자리가 더 붙지 않았습니다 — 붙은 자리 ${seats}개 · 경과 ${elapsed}. 설치하지 않은 프로그램(claude·agy·codex)의 자리는 생기지 않습니다. 더 붙어야 한다면 Control Center 에서 자리 상태를 확인하세요`;
  const STATES: DeptFormationState[] = ["booting", "seated", "check", "silent", "stall"];

  it("booting(기본) — 제목 「팀원을 켜는 중」 · 본문 「설치된 프로그램(claude·agy·codex)의 자리가 차례로 붙습니다(최대 5자리 · 보통 5분 안팎) · 붙은 자리 M · 경과 T」 — M = 붙은 의무 역할 수", () => {
    expect(deptFormationText({ seats: 0, elapsedSec: 0 })).toEqual({ title: "팀원을 켜는 중", body: BOOT(0, "1분 미만") });
    expect(deptFormationText({ seats: 3, elapsedSec: 125, state: "booting" })).toEqual({ title: "팀원을 켜는 중", body: BOOT(3, "2분") });
    expect(deptFormationText({ seats: 5, elapsedSec: 60 }).body).toBe(BOOT(5, "1분"));
    // state 를 생략한 것과 booting 은 같다
    expect(deptFormationText({ seats: 2, elapsedSec: 10 })).toEqual(deptFormationText({ seats: 2, elapsedSec: 10, state: "booting" }));
  });
  it("★S4 M1: 보통 시간은 잰 만큼만 — 「보통 5분 안팎」(실측 1회 약 5분) · 재 본 적 없는 하한 「3~5분」 은 어떤 상태·입력에서도 나오지 않는다", () => {
    expect(deptFormationText({ seats: 1, elapsedSec: 10 }).body).toContain("보통 5분 안팎)");
    for (const state of STATES)
      for (const sec of [0, 59, 60, 300, 900, 5000]) {
        const t = deptFormationText({ seats: 3, elapsedSec: sec, state });
        expect({ state, sec, 하한: (t.title + t.body).includes("3~5분") }).toEqual({ state, sec, 하한: false });
      }
    for (const rel of ["./deptprogress.ts", "./main.ts"]) expect({ 파일: rel, 하한: read(rel).includes("보통 3~5분") }).toEqual({ 파일: rel, 하한: false });
  });
  it("seated — 제목 「팀 자리가 모두 붙었습니다」 · 본문 「자리 5개가 모두 붙었습니다 · 걸린 시간 <분>」(formatDeptMinutes) — '준비 완료'라고 단정하지 않는다", () => {
    expect(deptFormationText({ seats: 5, elapsedSec: 252, state: "seated" })).toEqual({ title: "팀 자리가 모두 붙었습니다", body: "자리 5개가 모두 붙었습니다 · 걸린 시간 4분" });
    expect(deptFormationText({ seats: 5, elapsedSec: 30, state: "seated" }).body).toBe("자리 5개가 모두 붙었습니다 · 걸린 시간 1분 미만");
    expect(deptFormationText({ seats: 5, elapsedSec: 60, state: "seated" }).body).toBe("자리 5개가 모두 붙었습니다 · 걸린 시간 1분");
  });
  it("★check(설치 여부를 모르는 15분 상한) — 제목 「팀원 켜기 — 15분 경과」 · 본문 「붙은 자리 M개 — 설치하지 않은 프로그램(claude·agy·codex)의 자리는 생기지 않습니다. 그 밖이면 Control Center 에서 자리 상태를 확인하세요」", () => {
    expect(deptFormationText({ seats: 3, elapsedSec: 900, state: "check" })).toEqual({ title: "팀원 켜기 — 15분 경과", body: CHECK(3) });
    expect(deptFormationText({ seats: 0, elapsedSec: 905, state: "check" }).body).toBe(CHECK(0));
    expect(deptFormationText({ seats: 1, elapsedSec: 905, state: "check" }).body).toBe(CHECK(1)); // CLI 가 하나도 없는 PC — 부서장 자리 1개만 붙는다(윈도우 11 러너 실측)
    // ★옛 제목 「확인 필요」 는 이 문구에 없다 — 설치하지 않은 프로그램의 자리가 안 생기는 것은 정상 종결이다(편성 도구도 일반 알림으로 알린다)
    expect(deptFormationText({ seats: 3, elapsedSec: 900, state: "check" }).title.includes("확인 필요")).toBe(false);
    // 경과는 제목이 '15분' 으로 말하고 본문에는 따로 싣지 않는다 — 상태가 check 이면 경과 입력과 무관하게 같은 문구
    expect(deptFormationText({ seats: 3, elapsedSec: 5000, state: "check" })).toEqual(deptFormationText({ seats: 3, elapsedSec: 900, state: "check" }));
  });
  it("★silent(좌석 목록을 못 받는 채 15분 상한) — 제목 「팀 데몬이 응답하지 않습니다 — 확인 필요」 · 본문 「좌석 목록을 받지 못했습니다 — Control Center 에서 팀 상태를 확인하세요 · 경과 T」", () => {
    expect(deptFormationText({ seats: 0, elapsedSec: 900, state: "silent" })).toEqual({ title: "팀 데몬이 응답하지 않습니다 — 확인 필요", body: SILENT("15분") });
    expect(deptFormationText({ seats: 4, elapsedSec: 1000, state: "silent" }).body).toBe(SILENT("16분")); // 자리 수는 말하지 않는다 — 목록을 못 받았다
    expect(deptFormationText({ seats: 4, elapsedSec: 1000, state: "silent" }).body.includes("자리")).toBe(false);
  });
  it("★stall(자리가 3분 동안 더 붙지 않음) — 제목 「팀원 켜기 — 자리가 더 붙지 않습니다」 · 본문 「3분 동안 자리가 더 붙지 않았습니다 — 붙은 자리 M개 · 경과 T. 설치하지 않은 프로그램(claude·agy·codex)의 자리는 생기지 않습니다. 더 붙어야 한다면 Control Center 에서 자리 상태를 확인하세요」", () => {
    expect(deptFormationText({ seats: 3, elapsedSec: 280, state: "stall" })).toEqual({ title: "팀원 켜기 — 자리가 더 붙지 않습니다", body: STALL(3, "4분") });
    expect(deptFormationText({ seats: 1, elapsedSec: 200, state: "stall" }).body).toBe(STALL(1, "3분")); // 부서장 자리 1개만(CLI 0개) — 경과도 분 단위 내림
    expect(deptFormationText({ seats: 4, elapsedSec: 885, state: "stall" }).body).toBe(STALL(4, "14분"));
    // 자리 수는 '붙은 의무 역할 수' 이고 경과는 분 단위 — 문구에 초 단위가 없다
    expect(deptFormationText({ seats: 3, elapsedSec: 280, state: "stall" }).body.includes("초")).toBe(false);
    // 경고 문구(「확인 필요」)가 아니다 — 설치하지 않은 프로그램의 자리가 안 생기는 것은 정상 종결이다
    expect(deptFormationText({ seats: 3, elapsedSec: 280, state: "stall" }).title.includes("확인 필요")).toBe(false);
  });
  it("★문구의 「3분」 은 DEPT_FORMATION_STALL_SECS 에서 파생한다(15분 문구와 같은 방식) — 상수를 바꾸면 문구가 따라가고, 소스에 「3분 동안」 을 박지 않는다", () => {
    const m = deptFormationText({ seats: 3, elapsedSec: 280, state: "stall" }).body;
    expect(m.startsWith(`${DEPT_FORMATION_STALL_SECS / 60}분 동안 자리가 더 붙지 않았습니다`)).toBe(true);
    const code = read("./deptprogress.ts")
      .replace(/\/\*[\s\S]*?\*\//g, "")
      .split("\n")
      .map((l) => (l.trimStart().startsWith("//") ? "" : l.replace(/\s\/\/.*$/, "")))
      .join("\n");
    expect(code.includes("Math.floor(DEPT_FORMATION_STALL_SECS / 60)")).toBe(true);
    expect(code.includes("3분 동안")).toBe(false); // 박지 않았다
    expect(code.includes("Math.floor(DEPT_FORMATION_CAP_SECS / 60)")).toBe(true); // 같은 방식인 15분 문구
  });
  it("★살아 있음을 지어내지 않는다 — 자리 판정 문구(seated·check·silent·stall)에 완료·준비됐·정상·켜졌습니다·성공 낱말이 없다(에이전트가 실제로 떴는지는 화면이 모른다)", () => {
    for (const state of ["seated", "check", "silent", "stall"] as DeptFormationState[]) {
      const t = deptFormationText({ seats: 1, elapsedSec: 30, state });
      for (const lie of ["완료", "준비됐", "정상", "켜졌습니다", "성공"]) expect({ state, 낱말: lie, 있음: (t.title + t.body).includes(lie) }).toEqual({ state, 낱말: lie, 있음: false });
    }
    // 켜는 중 문구도 '곧 켜진다'가 아니라 조건(설치된 프로그램)·최대 자리·보통 시간·붙은 자리 수만 말한다
    expect(deptFormationText({ seats: 1, elapsedSec: 30 }).body.includes("완료")).toBe(false);
  });
  it("모르는 상태(옛 partial·pending·failed·complete 같은 값)는 던지지 않고 켜는 중 문구 — 사유(detail)를 싣던 경로는 없다", () => {
    for (const old of ["partial", "pending", "failed", "complete", "", "x"]) {
      const t = deptFormationText({ seats: 2, elapsedSec: 10, state: old as unknown as DeptFormationState, detail: "사유" } as never);
      expect({ 상태: old, 값: t }).toEqual({ 상태: old, 값: { title: "팀원을 켜는 중", body: BOOT(2, "1분 미만") } });
      expect(t.body.includes("사유")).toBe(false);
    }
  });
  it("자리 수·경과가 이상해도 던지지 않고 0 으로 접는다 — 다섯 상태 모두", () => {
    expect(deptFormationText({ seats: -2, elapsedSec: -5 }).body).toBe(BOOT(0, "1분 미만"));
    expect(deptFormationText({ seats: Number.NaN, elapsedSec: Number.NaN, state: "seated" }).body).toBe("자리 5개가 모두 붙었습니다 · 걸린 시간 1분 미만");
    expect(deptFormationText({ seats: Number.NaN, elapsedSec: Number.NaN, state: "check" }).body).toBe(CHECK(0));
    expect(deptFormationText({ seats: Number.NaN, elapsedSec: Number.NaN, state: "silent" }).body).toBe(SILENT("1분 미만"));
    expect(deptFormationText({ seats: Number.NaN, elapsedSec: Number.NaN, state: "stall" }).body).toBe(STALL(0, "1분 미만"));
    expect(deptFormationText({ seats: 1e9, elapsedSec: 0, state: "stall" }).body).toBe(STALL(999, "1분 미만"));
    expect(deptFormationText({ seats: 2.9, elapsedSec: 125.9 }).body).toBe(BOOT(2, "2분"));
    expect(deptFormationText({ seats: 1e9, elapsedSec: 0 }).body).toBe(BOOT(999, "1분 미만"));
    expect(deptFormationText({ seats: 1e9, elapsedSec: 0, state: "check" }).body).toBe(CHECK(999));
  });
  it("★옛 거짓 약속 「십여 초」 는 팀원 안내에도 없다", () => {
    for (const state of STATES) {
      const t = deptFormationText({ seats: 3, elapsedSec: 100, state });
      expect((t.title + t.body).includes("십여")).toBe(false);
    }
  });
  it("★R2F-UI(A2 B-1 · A3 n13): 옛 문구가 어떤 상태·입력에서도 다시 나오지 않는다 — 「부서장·CSO·워커·리뷰어가 차례로 켜집니다」(조건 없는 단정) · 「지금 N자리」(탭의 칸 수) · 「아직 N자리입니다」(경고색 확인 필요)", () => {
    for (const state of STATES)
      for (const seats of [0, 1, 3, 5])
        for (const sec of [0, 90, 900]) {
          const t = deptFormationText({ seats, elapsedSec: sec, state });
          for (const old of ["부서장·CSO·워커·리뷰어가 차례로 켜집니다", "지금 ", "아직 ", "자리입니다"]) {
            expect({ state, seats, sec, 옛: old, 있음: (t.title + t.body).includes(old) }).toEqual({ state, seats, sec, 옛: old, 있음: false });
          }
          expect(t.title).not.toBe("팀원 켜기 — 확인 필요"); // 옛 경고 제목(15분 상한은 이제 일반 알림 「15분 경과」)
        }
    // 제품 소스(문구 모듈과 배선)에도 옛 문구·옛 템플릿이 없다 — 주석을 걷은 코드 줄만 본다
    const code = (rel: string): string =>
      read(rel)
        .split("\n")
        .map((l) => (l.trimStart().startsWith("//") || l.trimStart().startsWith("*") || l.trimStart().startsWith("/*") ? "" : l.replace(/\s\/\/.*$/, "")))
        .join("\n");
    for (const rel of ["./deptprogress.ts", "./main.ts"]) {
      const c = code(rel);
      for (const old of ["부서장·CSO·워커·리뷰어가 차례로 켜집니다", "지금 ${seats}자리", "아직 ${seats}자리입니다", "팀원 켜기 — 확인 필요"]) {
        expect({ 파일: rel, 옛: old, 있음: c.includes(old) }).toEqual({ 파일: rel, 옛: old, 있음: false });
      }
    }
  });
});

describe("deptFormationNoticeKind — 알림 등급(silent 만 경고 · 15분 경과는 일반 알림)", () => {
  it("★silent → watchdog(경고) · booting·seated·check·stall·상태 생략 → feed(일반 알림) — 15분 경과(check)와 자리 정체(stall)는 설치하지 않은 프로그램 때문일 수 있어 경고가 아니다", () => {
    expect(deptFormationNoticeKind("silent")).toBe("watchdog");
    expect(deptFormationNoticeKind("stall")).toBe("feed"); // 후속(정체 판정): 경고가 아닌 일반 알림
    for (const st of ["booting", "seated", "check", "stall", undefined] as (DeptFormationState | undefined)[])
      expect({ 상태: String(st), 등급: deptFormationNoticeKind(st) }).toEqual({ 상태: String(st), 등급: "feed" });
  });
  it("모르는 값은 던지지 않고 feed 로 접는다", () => {
    for (const bad of ["partial", "", "WATCHDOG", "check ", null, 5, {}]) expect({ 값: String(bad), 등급: deptFormationNoticeKind(bad as unknown as DeptFormationState) }).toEqual({ 값: String(bad), 등급: "feed" });
  });
});

describe("deptFormationListSilent — 좌석 목록을 연속으로 60초 넘게 못 받았는가", () => {
  const T = 5_000_000;
  it("★정확히 60초는 아직 아니고(경계) 60.001초부터 true — 마지막 수신 시각과 지금의 차", () => {
    expect(deptFormationListSilent(T, T)).toBe(false);
    expect(deptFormationListSilent(T, T + 59_999)).toBe(false);
    expect(deptFormationListSilent(T, T + 60_000)).toBe(false);
    expect(deptFormationListSilent(T, T + 60_001)).toBe(true);
    expect(deptFormationListSilent(T, T + 3_600_000)).toBe(true);
  });
  it("모르는 입력(수신 시각이 없거나 유한한 수가 아님 · 시계가 거꾸로 감)으로 '응답 없음'을 지어내지 않는다", () => {
    for (const bad of [undefined, Number.NaN, Number.POSITIVE_INFINITY, Number.NEGATIVE_INFINITY, "x" as unknown as number, null as unknown as number])
      expect({ 값: String(bad), 침묵: deptFormationListSilent(bad, T + 999_999) }).toEqual({ 값: String(bad), 침묵: false });
    expect(deptFormationListSilent(T, Number.NaN)).toBe(false);
    expect(deptFormationListSilent(T, undefined as unknown as number)).toBe(false);
    expect(deptFormationListSilent(T + 100_000, T)).toBe(false); // 시계가 거꾸로 갔다(차 < 0)
  });
});

describe("deptFormationStalled — 자리 정체 판정(시간과 자리 수만 본다 — 목록 수신·상한 전 여부는 호출측)", () => {
  const T = 7_000_000;
  const MS = DEPT_FORMATION_STALL_SECS * 1000;
  it("★경계 — 마지막으로 늘어난 때부터 179.999초는 아직, 정확히 180초부터 true(180초 이상) · 그 뒤로도 true", () => {
    for (const seated of [1, 2, 3, 4]) {
      expect({ seated, t: "179.999s", 정체: deptFormationStalled(seated, T, T + MS - 1) }).toEqual({ seated, t: "179.999s", 정체: false });
      expect({ seated, t: "180s", 정체: deptFormationStalled(seated, T, T + MS) }).toEqual({ seated, t: "180s", 정체: true });
      expect({ seated, t: "180.001s", 정체: deptFormationStalled(seated, T, T + MS + 1) }).toEqual({ seated, t: "180.001s", 정체: true });
      expect({ seated, t: "한 시간", 정체: deptFormationStalled(seated, T, T + 3_600_000) }).toEqual({ seated, t: "한 시간", 정체: true });
    }
    expect(deptFormationStalled(3, T, T)).toBe(false); // 방금 늘었다
  });
  it("★자리 수 조건 — 0(부서장 자리도 없음)은 정체가 아니다 · 다섯(모두 붙음)은 정체가 아니다 · 그 사이(1~4)만 해당 · 음수·소수 이하도 0 과 같다", () => {
    expect(deptFormationStalled(0, T, T + 10 * MS)).toBe(false);
    expect(deptFormationStalled(-1, T, T + 10 * MS)).toBe(false);
    expect(deptFormationStalled(0.5, T, T + 10 * MS)).toBe(false);
    expect(deptFormationStalled(5, T, T + 10 * MS)).toBe(false);
    expect(deptFormationStalled(6, T, T + 10 * MS)).toBe(false);
    for (const seated of [1, 2, 3, 4]) expect(deptFormationStalled(seated, T, T + MS)).toBe(true);
  });
  it("모르는 입력(자리 수·기준 시각·지금이 유한한 수가 아님 · 기준이 없음 · 시계가 거꾸로 감)으로 정체를 지어내지 않는다 — 던지지 않고 false", () => {
    for (const bad of [Number.NaN, Number.POSITIVE_INFINITY, "3" as unknown as number, null as unknown as number, undefined as unknown as number]) {
      expect({ 값: String(bad), 정체: deptFormationStalled(bad, T, T + 10 * MS) }).toEqual({ 값: String(bad), 정체: false });
    }
    for (const bad of [undefined, Number.NaN, Number.POSITIVE_INFINITY, Number.NEGATIVE_INFINITY, "x" as unknown as number, null as unknown as number])
      expect({ 기준: String(bad), 정체: deptFormationStalled(3, bad, T + 10 * MS) }).toEqual({ 기준: String(bad), 정체: false });
    for (const bad of [Number.NaN, Number.POSITIVE_INFINITY, undefined as unknown as number, "x" as unknown as number]) expect(deptFormationStalled(3, T, bad)).toBe(false);
    expect(deptFormationStalled(3, T + 10 * MS, T)).toBe(false); // 시계가 거꾸로 갔다(차 < 0)
  });
});

describe("갱신 열쇠 · 상한 · 첫 자리 창", () => {
  it("deptFormationNoticeKey — 자리 수가 바뀌면 달라진다", () => {
    expect(deptFormationNoticeKey({ seats: 1, elapsedSec: 10 })).not.toBe(deptFormationNoticeKey({ seats: 2, elapsedSec: 10 }));
  });
  it("경과 '분'이 바뀌면 달라진다(59→60) · 같은 분·같은 45초 칸 안에서는 같다(46~59)", () => {
    expect(deptFormationNoticeKey({ seats: 1, elapsedSec: 59 })).not.toBe(deptFormationNoticeKey({ seats: 1, elapsedSec: 60 }));
    const base = deptFormationNoticeKey({ seats: 1, elapsedSec: 46 });
    for (let s = 46; s <= 59; s++) expect({ 초: s, 열쇠: deptFormationNoticeKey({ seats: 1, elapsedSec: s }) }).toEqual({ 초: s, 열쇠: base });
  });
  it("45초 칸이 바뀌어도 달라진다(44→45) — 갱신 사이가 토스트 기본 수명(60초) 밑으로 유지된다", () => {
    expect(deptFormationNoticeKey({ seats: 1, elapsedSec: 44 })).not.toBe(deptFormationNoticeKey({ seats: 1, elapsedSec: 45 }));
    expect(deptFormationNoticeKey({ seats: 1, elapsedSec: 89 })).not.toBe(deptFormationNoticeKey({ seats: 1, elapsedSec: 90 }));
  });
  it("상태가 바뀌면 달라진다(silent 포함) · 상태를 생략하면 booting", () => {
    expect(deptFormationNoticeKey({ seats: 1, elapsedSec: 10, state: "check" })).not.toBe(deptFormationNoticeKey({ seats: 1, elapsedSec: 10, state: "booting" }));
    expect(deptFormationNoticeKey({ seats: 1, elapsedSec: 10, state: "silent" })).not.toBe(deptFormationNoticeKey({ seats: 1, elapsedSec: 10, state: "check" }));
    expect(deptFormationNoticeKey({ seats: 1, elapsedSec: 10, state: "stall" })).not.toBe(deptFormationNoticeKey({ seats: 1, elapsedSec: 10, state: "booting" }));
    expect(deptFormationNoticeKey({ seats: 1, elapsedSec: 10 })).toBe(deptFormationNoticeKey({ seats: 1, elapsedSec: 10, state: "booting" }));
  });
  it("★3초 틱으로 5분을 돌려도 열쇠가 바뀌는 횟수는 한 자릿수·두 갱신 사이는 47초 이하(초 단위 호출 없음 · 토스트가 사라지지 않음)", () => {
    let keyChanges = 0;
    let last = deptFormationNoticeKey({ seats: 0, elapsedSec: 0 });
    let lastAt = 0;
    let maxGap = 0;
    for (let t = 3; t <= 300; t += 3) {
      const seats = t >= 250 ? 5 : t >= 190 ? 4 : t >= 130 ? 3 : t >= 70 ? 2 : t >= 10 ? 1 : 0;
      const k = deptFormationNoticeKey({ seats, elapsedSec: t });
      if (k !== last) {
        keyChanges++;
        maxGap = Math.max(maxGap, t - lastAt);
        last = k;
        lastAt = t;
      }
    }
    expect(keyChanges).toBeLessThan(25); // 100 틱 중 갱신은 일부뿐
    expect(keyChanges).toBeGreaterThan(5);
    expect(maxGap).toBeLessThan(48); // 47초 이하(틱이 3초 격자라 정수)
  });
  it("deptFormationCapped — 900초(15분) 이상이면 접는다", () => {
    expect(deptFormationCapped(0)).toBe(false);
    expect(deptFormationCapped(899)).toBe(false);
    expect(deptFormationCapped(899.9)).toBe(false);
    expect(deptFormationCapped(900)).toBe(true);
    expect(deptFormationCapped(5000)).toBe(true);
    expect(deptFormationCapped(Number.NaN)).toBe(false);
    expect(deptFormationCapped(-1)).toBe(false);
  });
  it("deptFirstSeatPending — 이 세션에서 만든 지 0 이상 60초 미만일 때만(createdAt 없음·시계 역행은 false)", () => {
    const t0 = 1_000_000;
    expect(deptFirstSeatPending(t0, t0)).toBe(true);
    expect(deptFirstSeatPending(t0, t0 + 59_999)).toBe(true);
    expect(deptFirstSeatPending(t0, t0 + 60_000)).toBe(false);
    expect(deptFirstSeatPending(t0, t0 + 3_600_000)).toBe(false);
    expect(deptFirstSeatPending(t0, t0 - 1)).toBe(false); // 시계가 거꾸로 갔다
    expect(deptFirstSeatPending(undefined, t0)).toBe(false);
    expect(deptFirstSeatPending(Number.NaN, t0)).toBe(false);
    expect(deptFirstSeatPending(t0, Number.NaN)).toBe(false);
    expect(DEPT_FIRST_SEAT_TEXT).toBe("첫 자리를 붙이는 중입니다 — 잠시만 기다려 주세요");
  });
  it("deptFirstSeatRemainingMs — 창이 끝나기까지 남은 ms(0 이상)", () => {
    const t0 = 5_000;
    expect(deptFirstSeatRemainingMs(t0, t0)).toBe(60_000);
    expect(deptFirstSeatRemainingMs(t0, t0 + 20_000)).toBe(40_000);
    expect(deptFirstSeatRemainingMs(t0, t0 + 60_000)).toBe(0);
    expect(deptFirstSeatRemainingMs(t0, t0 + 90_000)).toBe(0);
    expect(deptFirstSeatRemainingMs(undefined, t0)).toBe(0);
    expect(deptFirstSeatRemainingMs(t0, Number.NaN)).toBe(0);
  });
});

describe("새 순수 모듈 deptprogress.ts — 불변식(최상위 부수효과 0 · 문서/창/저장소 0 · 구형 WKWebView 비호환 문법 0)", () => {
  const raw = read("./deptprogress.ts");
  const stripLine = (s: string): string =>
    s
      .split("\n")
      .map((l) => (l.trimStart().startsWith("//") ? "" : l.replace(/\s\/\/.*$/, "")))
      .join("\n");
  const code = stripLine(raw.replace(/\/\*[\s\S]*?\*\//g, ""));
  it("원문(주석 포함)에 document·window·localStorage 문자열이 없다", () => {
    for (const w of ["document", "window", "localStorage"]) expect({ 낱말: w, 있음: raw.includes(w) }).toEqual({ 낱말: w, 있음: false });
  });
  it("전역 부수효과 표면(저장소·문서·창·타이머·IPC) 0", () => {
    for (const bad of ["localStorage", "sessionStorage", "indexedDB", "document.", "window.", "navigator", "setInterval", "setTimeout", "clearInterval", "__TAURI__", "invoke(", "fetch(", "Date.now", "new Date"])
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
  it("화면·저장소·조치를 일으키는 코드가 없다 — 순수 함수만(HTML 삽입·키 주입·강제 배달·화면 전환 낱말 0)", () => {
    for (const bad of ["send_input", "send_key", "queue.deliver", "force", "jumpToSurface", "setFocus", "innerHTML", "textContent", "outerHTML", "insertAdjacentHTML"])
      expect({ 낱말: bad, 있음: code.includes(bad) }).toEqual({ 낱말: bad, 있음: false });
  });
});
