// mousefilter.ts 순수 함수 회귀 테스트 (bun test — 신규 의존성 0).
//
// 회귀 핀의 계약: 이 필터는 main.ts에서 **Windows일 때만** 배선된다(macOS 무변경).
// 여기 테스트는 플랫폼과 무관한 '분류 판단'만 고정한다 — 배선 조건은 main.ts 주석이 지킨다.
//
// 지키는 것 두 가지:
//  (1) 마우스 보고는 전부 잡아낸다 — 놓치면 Windows에서 `[555;98;34M` 리터럴 유출이 재발한다.
//  (2) 마우스 아닌 것은 절대 안 잡는다 — 오탐하면 사용자 입력(특히 한글 IME 경로)이 소실된다.
import { describe, it, expect } from "bun:test";
import { classifyMouseReport } from "./mousefilter";

const ESC = "\x1b";
// SGR(1006/1016) 보고 조립기. press=M(눌림)/m(뗌).
const sgr = (b: number, x: number, y: number, end: "M" | "m" = "M") => `${ESC}[<${b};${x};${y}${end}`;
// urxvt(1015): 버튼·좌표에 32 오프셋.
const urxvt = (b: number, x: number, y: number) => `${ESC}[${b + 32};${x + 32};${y + 32}M`;
// X10 레거시: ESC[M + 원시 3바이트(전부 +32).
const x10 = (b: number, x: number, y: number) =>
  `${ESC}[M${String.fromCharCode(b + 32, x + 32, y + 32)}`;

describe("마우스 보고 폐기 — 인코딩 변형 전부", () => {
  it("SGR 누름(M)·뗌(m) 둘 다 drop", () => {
    expect(classifyMouseReport(sgr(0, 10, 20, "M"))).toEqual({ kind: "drop" });
    expect(classifyMouseReport(sgr(0, 10, 20, "m"))).toEqual({ kind: "drop" });
  });
  it("SGR-Pixels(1016) 대형 픽셀 좌표(현장 제보의 555 등)도 drop", () => {
    // 실기에서 유출된 문자열이 `[555;98;34M` 형태였다 — 좌표 자릿수에 상한을 두면 안 된다.
    expect(classifyMouseReport(sgr(32, 555, 98, "M"))).toEqual({ kind: "drop" });
    expect(classifyMouseReport(sgr(0, 1920, 1080, "M"))).toEqual({ kind: "drop" });
  });
  it("SGR 모션 보고(1003 배칭의 주범, 버튼 35)도 drop", () => {
    expect(classifyMouseReport(sgr(35, 12, 34))).toEqual({ kind: "drop" });
  });
  it("urxvt(1015) drop", () => {
    expect(classifyMouseReport(urxvt(0, 10, 20))).toEqual({ kind: "drop" });
    expect(classifyMouseReport(urxvt(35, 200, 100))).toEqual({ kind: "drop" });
  });
  it("X10 레거시(ESC[M + 원시 3바이트) drop", () => {
    expect(classifyMouseReport(x10(0, 10, 20))).toEqual({ kind: "drop" });
    expect(classifyMouseReport(x10(3, 1, 1))).toEqual({ kind: "drop" });
  });
  it("좌우 휠(66/67)은 세로 번역 불가 → 스크롤 없이 drop", () => {
    expect(classifyMouseReport(sgr(66, 10, 20))).toEqual({ kind: "drop" });
    expect(classifyMouseReport(sgr(67, 10, 20))).toEqual({ kind: "drop" });
  });
  it("확장 버튼(128 비트)은 휠이 아니라 drop", () => {
    expect(classifyMouseReport(sgr(128, 10, 20))).toEqual({ kind: "drop" });
  });
});

describe("휠 보고 → 로컬 스크롤 번역", () => {
  it("휠업(64) → dir -1(위로), 휠다운(65) → dir +1(아래로)", () => {
    expect(classifyMouseReport(sgr(64, 10, 20))).toEqual({ kind: "wheel", dir: -1, count: 1 });
    expect(classifyMouseReport(sgr(65, 10, 20))).toEqual({ kind: "wheel", dir: 1, count: 1 });
  });
  it("모디파이어 가산(shift 4·alt 8·ctrl 16)도 휠로 판정", () => {
    expect(classifyMouseReport(sgr(64 | 4, 1, 1))).toEqual({ kind: "wheel", dir: -1, count: 1 });
    expect(classifyMouseReport(sgr(64 | 8, 1, 1))).toEqual({ kind: "wheel", dir: -1, count: 1 });
    expect(classifyMouseReport(sgr(64 | 16, 1, 1))).toEqual({ kind: "wheel", dir: -1, count: 1 });
    expect(classifyMouseReport(sgr(65 | 4 | 16, 1, 1))).toEqual({ kind: "wheel", dir: 1, count: 1 });
    expect(classifyMouseReport(sgr(65 | 4 | 8 | 16, 1, 1))).toEqual({ kind: "wheel", dir: 1, count: 1 });
  });
  it("휠 뗌(m) 표기도 동일 판정", () => {
    expect(classifyMouseReport(sgr(64, 5, 5, "m"))).toEqual({ kind: "wheel", dir: -1, count: 1 });
  });
  it("urxvt·X10 인코딩의 휠도 번역", () => {
    expect(classifyMouseReport(urxvt(64, 10, 20))).toEqual({ kind: "wheel", dir: -1, count: 1 });
    expect(classifyMouseReport(x10(65, 10, 20))).toEqual({ kind: "wheel", dir: 1, count: 1 });
  });
});

describe("한 청크 배칭 — 고빈도 이벤트가 뭉쳐 들어올 때", () => {
  it("모션 보고 20개 연속 청크는 통째로 drop", () => {
    let chunk = "";
    for (let i = 0; i < 20; i++) chunk += sgr(35, 10 + i, 20);
    expect(classifyMouseReport(chunk)).toEqual({ kind: "drop" });
  });
  it("휠다운 5연속 → count 5", () => {
    const chunk = sgr(65, 1, 1).repeat(5);
    expect(classifyMouseReport(chunk)).toEqual({ kind: "wheel", dir: 1, count: 5 });
  });
  it("휠업 5연속 → dir -1 count 5", () => {
    const chunk = sgr(64, 1, 1).repeat(5);
    expect(classifyMouseReport(chunk)).toEqual({ kind: "wheel", dir: -1, count: 5 });
  });
  it("휠 + 모션이 섞인 마우스 전용 청크는 휠만 집계", () => {
    const chunk = sgr(35, 1, 1) + sgr(65, 1, 1) + sgr(35, 2, 2) + sgr(65, 2, 2);
    expect(classifyMouseReport(chunk)).toEqual({ kind: "wheel", dir: 1, count: 2 });
  });
  it("위/아래가 정확히 상쇄되면 스크롤할 것이 없어 drop", () => {
    expect(classifyMouseReport(sgr(64, 1, 1) + sgr(65, 1, 1))).toEqual({ kind: "drop" });
  });
  it("인코딩이 섞인 마우스 전용 청크도 전량 인식", () => {
    expect(classifyMouseReport(sgr(0, 1, 1) + urxvt(3, 2, 2) + x10(0, 3, 3))).toEqual({ kind: "drop" });
  });
});

describe("통과 보장 — 오탐하면 사용자 입력이 소실된다", () => {
  const passes: [string, string][] = [
    ["빈 문자열", ""],
    ["일반 ASCII 텍스트", "hello world"],
    ["한글 완성 음절", "너"],
    ["한글 문장", "안녕하세요 마스터"],
    ["개행", "\r"],
    ["Ctrl-C", "\x03"],
    ["방향키 위", `${ESC}[A`],
    ["방향키 오른쪽", `${ESC}[C`],
    ["Ctrl+오른쪽(모디파이어 CSI)", `${ESC}[1;5C`],
    ["Home/End 계열", `${ESC}[1~`],
    ["F5 기능키", `${ESC}[15~`],
    ["Alt 조합", `${ESC}b`],
    ["CSI지만 M으로 끝나는 DL(파라미터 1개)", `${ESC}[3M`],
    ["세 파라미터지만 32 미만이라 마우스 아님", `${ESC}[1;2;3M`],
    ["ESC 단독", ESC],
    ["미완성 CSI(청크 경계)", `${ESC}[`],
    ["미완성 SGR 보고(청크 경계)", `${ESC}[<35;10`],
    ["미완성 X10(3바이트 못 채움)", `${ESC}[M!`],
  ];
  for (const [name, input] of passes) {
    it(`${name} → pass`, () => {
      expect(classifyMouseReport(input)).toEqual({ kind: "pass" });
    });
  }

  it("bracketed paste 블록은 통째로 pass", () => {
    expect(classifyMouseReport(`${ESC}[200~붙여넣은 텍스트${ESC}[201~`)).toEqual({ kind: "pass" });
  });
  it("붙여넣기 안에 마우스 보고처럼 보이는 문자열이 있어도 pass(텍스트 보존 우선)", () => {
    expect(classifyMouseReport(`${ESC}[200~[555;98;34M${ESC}[201~`)).toEqual({ kind: "pass" });
  });
  it("마우스 보고 + 일반 텍스트 혼합 청크는 pass — 텍스트 소실보다 유출 1회가 낫다", () => {
    expect(classifyMouseReport(sgr(35, 10, 20) + "a")).toEqual({ kind: "pass" });
    expect(classifyMouseReport("a" + sgr(64, 10, 20))).toEqual({ kind: "pass" });
  });
});
