// wheelgate.ts 휠 억제 술어 회귀 테스트 (스펙 D4 — 매트릭스 고정).
//
// 억제 = [alt ∧ 장부 트래킹 요청 ∧ xterm 트래킹 미진입 ∧ !킬스위치 ∧ !Windows] 전부 참일
// 때 **만**이다. 한 항이라도 어긋나면 xterm 기본 처리(방향키 합성·보고 전송·로컬 스크롤)를
// 보존해야 한다 — less/man 의 휠 페이지 넘김과 win vim mouse=a 휠이 이 계약에 걸려 있다.
import { describe, it, expect } from "bun:test";
import { shouldSuppressWheel, type WheelGateState } from "./wheelgate";

const base: WheelGateState = {
  altActive: true,
  ledgerWantsMouse: true,
  xtermTracking: false,
  allowAppMouse: false,
  isWindows: false,
};

describe("휠 억제 매트릭스 — 시나리오 명명 케이스", () => {
  it("claude fullscreen 주입 파싱 창(alt·장부 요청·xterm 미진입·mac) → 억제", () => {
    // 이 창의 휠이 방향키로 합성되면 프롬프트 히스토리가 오염된다(원 결함) — 유일한 억제 케이스.
    expect(shouldSuppressWheel(base)).toBe(true);
  });
  it("less/man(트래킹 무요청 alt) → 비억제 — 방향키 합성 보존(휠 페이지 넘김 무회귀)", () => {
    expect(shouldSuppressWheel({ ...base, ledgerWantsMouse: false })).toBe(false);
  });
  it("vim mouse=a·claude 정착 후(xterm 트래킹 진입 완료) → 비억제 — 보고 경로 그대로", () => {
    expect(shouldSuppressWheel({ ...base, xtermTracking: true })).toBe(false);
  });
  it("primary 화면(비 alt) → 비억제 — 로컬 스크롤백 경로 보존", () => {
    expect(shouldSuppressWheel({ ...base, altActive: false })).toBe(false);
  });
  it("킬스위치 allowAppMouse(pane 캡처값) → 비억제 — 앱이 마우스를 갖는다", () => {
    expect(shouldSuppressWheel({ ...base, allowAppMouse: true })).toBe(false);
  });
  it("Windows → 비억제 — 방향키 합성 유지(vim 휠 무회귀·main.ts 는 핸들러 자체 미등록)", () => {
    expect(shouldSuppressWheel({ ...base, isWindows: true })).toBe(false);
  });
});

describe("휠 억제 매트릭스 — 32조합 전수(억제는 정확히 1조합)", () => {
  const bools = [false, true];
  for (const altActive of bools)
    for (const ledgerWantsMouse of bools)
      for (const xtermTracking of bools)
        for (const allowAppMouse of bools)
          for (const isWindows of bools) {
            const s = { altActive, ledgerWantsMouse, xtermTracking, allowAppMouse, isWindows };
            const want =
              altActive && ledgerWantsMouse && !xtermTracking && !allowAppMouse && !isWindows;
            it(`${JSON.stringify(s)} → ${want ? "억제" : "통과"}`, () => {
              expect(shouldSuppressWheel(s)).toBe(want);
            });
          }
});
