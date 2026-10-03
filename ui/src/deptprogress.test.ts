// 0.14.43 GU — 「팀 직접 만들기」 대기 문구(경과·단계)와 팀원 부팅 안내(deptprogress.ts 순수 도우미) 회귀 핀.
//
// 무엇을 지키는가(티켓 GU):
//   · 대기 문구는 거짓 약속을 하지 않는다 — 옛 "최대 십여 초" 는 실측(25~30초 · 느린 PC 1분+)과 달랐다. 90초 미만은 실측 범위, 90초부터 '평소보다 오래'.
//   · 단계 표지(`[cys-dept] @stage <키>`)는 키 7종만 한글 줄이 되고 모르는 키·빈 값은 단계 줄 없이 경과만 보인다(구 팩에도 견딘다).
//   · 표지 파서는 Rust 쪽 같은 이름의 파서와 **같은 벡터표**를 잰다(이 파일이 src-tauri/src/main.rs 의 표를 소스에서 읽어 자기 표와 대조한다).
//   · 팀원 부팅 안내: 5상태 문구 · '확인 필요' 는 받은 사유를 그대로(없으면 일반 안내) · 갱신 열쇠(자리 수·경과 분·45초 칸) · 15분 상한 · 첫 자리 창(60초).
//   · 신뢰할 수 없는 입력(이벤트 payload·feed 본문)은 걸러 낸다 — 단계 키 정규식 · 사유의 제어문자·길이.
//   · 순수 모듈 불변식: 최상위 부수효과 0 · 문서/창/저장소 낱말 0 · 구형 WKWebView 비호환 문법 0.
import { describe, it, expect } from "bun:test";
import { readFileSync } from "node:fs";
import {
  DEPT_TYPICAL_SECS,
  DEPT_SLOW_FACTOR,
  DEPT_FORMATION_CAP_SECS,
  DEPT_FORMATION_TOAST_PREFIX,
  DEPT_FORMATION_REFRESH_SECS,
  DEPT_FIRST_SEAT_WINDOW_SECS,
  DEPT_FIRST_SEAT_TEXT,
  DEPT_FORMATION_DETAIL_MAX,
  deptStageLabel,
  deptPendingText,
  formatDeptElapsed,
  formatDeptMinutes,
  parseDeptStageLine,
  sanitizeDeptStageKey,
  deptProgressId,
  parseDeptProgressPayload,
  deptFormationToastId,
  deptFormationStateOfKind,
  deptFormationDetail,
  deptFormationText,
  deptFormationNoticeKey,
  deptFormationCapped,
  deptFirstSeatPending,
  deptFirstSeatRemainingMs,
  type DeptFormationState,
} from "./deptprogress";

const read = (rel: string) => readFileSync(new URL(rel, import.meta.url), "utf-8");
const cpLen = (s: string): number => Array.from(s).length;

/** 보이지 않는 문자 — 원문에 raw 로 두지 않고 코드로 만든다. */
const BIDI = String.fromCharCode(0x202e);
const ZWSP = String.fromCharCode(0x200b);
const NUL = String.fromCharCode(0);

// ── 문안 전문(티켓 문구 그대로 — 모듈 상수로 다시 짓지 않고 리터럴로 적는다: 문구가 바뀌면 여기서 적색) ──
const NORMAL = (elapsed: string): string => `팀을 만드는 중입니다 — 보통 30초 안팎, 컴퓨터에 따라 1분 넘게 걸릴 수 있어요 · 경과 ${elapsed}`;
const SLOW = (elapsed: string): string => `평소보다 오래 걸리고 있습니다 — 그대로 기다려 주세요(중간에 닫으면 만들던 팀이 정리됩니다) · 경과 ${elapsed}`;
const STAGES: [string, string][] = [
  ["reserve", "팀 번호를 잡는 중"],
  ["probe", "이미 켜진 데몬이 있는지 확인하는 중(최대 12초)"],
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

describe("deptFormationText — 5상태 × detail 유/무", () => {
  const BOOT = (seats: number, elapsed: string): string => `부서장·CSO·워커·리뷰어가 차례로 켜집니다(보통 3~5분) · 지금 ${seats}자리 · 경과 ${elapsed}`;
  const DEFAULT_DETAIL = "일부 자리가 아직 켜지지 않았습니다 — Control Center 에서 자리 상태를 확인하세요";
  const CHECK_TITLE = "팀원 켜기 — 확인 필요";

  it("booting(기본) — 제목 「팀원을 켜는 중」 · 본문에 순서·보통 시간·자리 수·경과(분)", () => {
    expect(deptFormationText({ seats: 0, elapsedSec: 0 })).toEqual({ title: "팀원을 켜는 중", body: BOOT(0, "1분 미만") });
    expect(deptFormationText({ seats: 3, elapsedSec: 125, state: "booting" })).toEqual({ title: "팀원을 켜는 중", body: BOOT(3, "2분") });
    expect(deptFormationText({ seats: 5, elapsedSec: 60 }).body).toBe(BOOT(5, "1분"));
    // state 를 생략한 것과 booting 은 같다 · detail 은 booting 에서 쓰지 않는다
    expect(deptFormationText({ seats: 2, elapsedSec: 10 })).toEqual(deptFormationText({ seats: 2, elapsedSec: 10, state: "booting" }));
    expect(deptFormationText({ seats: 2, elapsedSec: 10, state: "booting", detail: "사유" }).body).toBe(BOOT(2, "1분 미만"));
  });
  it("complete — 제목 「팀 준비 완료」 · 본문 「<자리>자리 · <시간> 걸렸습니다」(초까지)", () => {
    expect(deptFormationText({ seats: 5, elapsedSec: 252, state: "complete" })).toEqual({ title: "팀 준비 완료", body: "5자리 · 4분 12초 걸렸습니다" });
    expect(deptFormationText({ seats: 5, elapsedSec: 30, state: "complete" }).body).toBe("5자리 · 30초 걸렸습니다");
    expect(deptFormationText({ seats: 4, elapsedSec: 60, state: "complete" }).body).toBe("4자리 · 1분 0초 걸렸습니다");
    // detail 은 complete 에서 쓰지 않는다(완료를 사유로 덮지 않는다)
    expect(deptFormationText({ seats: 5, elapsedSec: 30, state: "complete", detail: "무시됨" }).body).toBe("5자리 · 30초 걸렸습니다");
  });
  for (const state of ["partial", "pending", "failed"] as DeptFormationState[]) {
    it(`${state} — 제목 「팀원 켜기 — 확인 필요」 · 본문 = 받은 사유 **그대로** + 지금 자리·경과(분)`, () => {
      const detail = "worker·reviewer-codex 자리가 아직 안 떴습니다(CLI 미설치?)  — 공백 두 칸 · 그대로";
      expect(deptFormationText({ seats: 2, elapsedSec: 200, state, detail })).toEqual({
        title: CHECK_TITLE,
        body: `${detail} · 지금 2자리 · 경과 3분`,
      });
      // 사유는 다듬지 않는다 — 앞뒤 공백·줄바꿈까지 받은 그대로(정제는 deptFormationDetail 의 일이고 이 함수는 전하기만 한다)
      expect(deptFormationText({ seats: 2, elapsedSec: 200, state, detail: "  앞뒤 공백 유지  " }).body).toBe("  앞뒤 공백 유지   · 지금 2자리 · 경과 3분");
      expect(deptFormationText({ seats: 2, elapsedSec: 200, state, detail: "첫 줄\n둘째 줄" }).body).toBe("첫 줄\n둘째 줄 · 지금 2자리 · 경과 3분");
      // 사유가 없으면(없음·null·빈 문자열·공백뿐) 일반 안내
      for (const none of [undefined, null, "", "   ", "\n\t"]) {
        expect({ 사유: none, 값: deptFormationText({ seats: 1, elapsedSec: 5, state, detail: none }) }).toEqual({
          사유: none,
          값: { title: CHECK_TITLE, body: `${DEFAULT_DETAIL} · 지금 1자리 · 경과 1분 미만` },
        });
      }
      expect(deptFormationText({ seats: 1, elapsedSec: 5, state, detail: 7 as unknown as string }).body).toContain(DEFAULT_DETAIL);
    });
  }
  it("★살아 있음을 지어내지 않는다 — '확인 필요' 문구에는 받은 사유 말고 성공·정상을 암시하는 낱말이 없다", () => {
    for (const state of ["partial", "pending", "failed"] as DeptFormationState[]) {
      const t = deptFormationText({ seats: 1, elapsedSec: 30, state, detail: "사유X" });
      expect(t.body).toBe("사유X · 지금 1자리 · 경과 1분 미만");
      for (const lie of ["완료", "정상", "준비됐", "켜졌습니다", "성공"]) expect({ 낱말: lie, 있음: t.title.includes(lie) || t.body.includes(lie) }).toEqual({ 낱말: lie, 있음: false });
    }
    // 일반 안내도 '아직 켜지지 않았다'만 말한다.
    expect(deptFormationText({ seats: 1, elapsedSec: 30, state: "partial" }).body).toContain("아직 켜지지 않았습니다");
  });
  it("자리 수·경과가 이상해도 던지지 않고 0 으로 접는다", () => {
    expect(deptFormationText({ seats: -2, elapsedSec: -5 }).body).toBe(BOOT(0, "1분 미만"));
    expect(deptFormationText({ seats: Number.NaN, elapsedSec: Number.NaN, state: "complete" }).body).toBe("0자리 · 0초 걸렸습니다");
    expect(deptFormationText({ seats: 2.9, elapsedSec: 125.9 }).body).toBe(BOOT(2, "2분"));
    expect(deptFormationText({ seats: 1e9, elapsedSec: 0 }).body).toBe(BOOT(999, "1분 미만"));
  });
  it("★옛 거짓 약속 「십여 초」 는 팀원 안내에도 없다", () => {
    for (const state of ["booting", "complete", "partial", "pending", "failed"] as DeptFormationState[]) {
      const t = deptFormationText({ seats: 3, elapsedSec: 100, state });
      expect((t.title + t.body).includes("십여")).toBe(false);
    }
  });
});

describe("편성 feed → 안내 상태 · 사유 정제", () => {
  it("deptFormationStateOfKind — formation-* 4종만", () => {
    expect(deptFormationStateOfKind("formation-complete")).toBe("complete");
    expect(deptFormationStateOfKind("formation-partial")).toBe("partial");
    expect(deptFormationStateOfKind("formation-pending")).toBe("pending");
    expect(deptFormationStateOfKind("formation-failed")).toBe("failed");
    for (const bad of ["formation", "formation-", "formation-complete ", "Formation-complete", "approval", "formation-other", "", null, undefined, 7, {}, []]) {
      expect({ kind: bad, 상태: deptFormationStateOfKind(bad) }).toEqual({ kind: bad, 상태: null });
    }
  });
  it("deptFormationDetail — 본문 우선 · 없으면 제목 · 둘 다 없으면 null", () => {
    expect(deptFormationDetail("본문입니다", "제목")).toBe("본문입니다");
    expect(deptFormationDetail("", "제목")).toBe("제목");
    expect(deptFormationDetail("   ", "제목")).toBe("제목");
    expect(deptFormationDetail(undefined, "제목")).toBe("제목");
    expect(deptFormationDetail(null, null)).toBeNull();
    expect(deptFormationDetail("", "")).toBeNull();
    expect(deptFormationDetail(5, {})).toBeNull();
  });
  it("제어문자·줄바꿈·양방향 제어는 공백으로 접고 앞뒤를 다듬는다 · HTML 은 해석하지 않고 글자 그대로", () => {
    expect(deptFormationDetail(`a${NUL}b\n c\t d  ${BIDI}e${ZWSP}f`, "")).toBe("a b c d e f");
    expect(deptFormationDetail("<b>굵게</b> & <img src=x onerror=alert(1)>", "")).toBe("<b>굵게</b> & <img src=x onerror=alert(1)>");
  });
  it("300자(코드 포인트)에서 자른다 — 넘으면 앞 299자 + …", () => {
    expect(DEPT_FORMATION_DETAIL_MAX).toBe(300);
    const exact = "가".repeat(300);
    expect(deptFormationDetail(exact, "")).toBe(exact);
    const over = deptFormationDetail("가".repeat(301), "") as string;
    expect(cpLen(over)).toBe(300);
    expect(over.endsWith("…")).toBe(true);
    const emoji = deptFormationDetail("😀".repeat(400), "") as string;
    expect(cpLen(emoji)).toBe(300); // 서로게이트 쌍을 반으로 자르지 않는다
    expect(emoji.includes("\ud83d ")).toBe(false);
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
  it("상태가 바뀌면 달라진다 · 상태를 생략하면 booting", () => {
    expect(deptFormationNoticeKey({ seats: 1, elapsedSec: 10, state: "partial" })).not.toBe(deptFormationNoticeKey({ seats: 1, elapsedSec: 10, state: "booting" }));
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
