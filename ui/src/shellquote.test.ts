// shellquote.ts 순수 함수 회귀 테스트 (bun test — 신규 의존성 0).
//
// PTY로 타이핑되는 경로가 셸에서 '단일 인자'로 복원되는지 — 공백·작은따옴표·한글·다중 파일·Windows 큰따옴표.
import { describe, it, expect } from "bun:test";
import { shellQuotePosix, shellQuoteWindows, shellQuote, shellQuoteJoin } from "./shellquote";

describe("shellQuotePosix — 작은따옴표 감싸기", () => {
  it("공백 경로", () => {
    expect(shellQuotePosix("/tmp/a b.png")).toBe("'/tmp/a b.png'");
  });
  it("작은따옴표 포함 경로는 '\\'' 로 끊어 이스케이프", () => {
    expect(shellQuotePosix("/tmp/it's.png")).toBe("'/tmp/it'\\''s.png'");
  });
  it("한글 경로는 그대로 리터럴", () => {
    expect(shellQuotePosix("/사용자/바탕 화면/그림.png")).toBe("'/사용자/바탕 화면/그림.png'");
  });
  it("빈 문자열도 유효한 빈 인자", () => {
    expect(shellQuotePosix("")).toBe("''");
  });
  it("$·\"·\\ 등 특수문자는 작은따옴표 안에서 리터럴", () => {
    expect(shellQuotePosix('/tmp/$a "b" \\c')).toBe("'/tmp/$a \"b\" \\c'");
  });
});

describe("shellQuoteWindows — 큰따옴표 감싸기", () => {
  it("공백 경로", () => {
    expect(shellQuoteWindows("C:\\Users\\x y\\img.png")).toBe('"C:\\Users\\x y\\img.png"');
  });
  it("큰따옴표 포함은 \"\" 로 이스케이프", () => {
    expect(shellQuoteWindows('C:\\x"y.png')).toBe('"C:\\x""y.png"');
  });
  it("한글 경로", () => {
    expect(shellQuoteWindows("C:\\사용자\\바탕 화면\\그림.png")).toBe('"C:\\사용자\\바탕 화면\\그림.png"');
  });
});

describe("shellQuote — 플랫폼 분기", () => {
  it("isWindows=false → POSIX", () => {
    expect(shellQuote("/tmp/a b.png", false)).toBe("'/tmp/a b.png'");
  });
  it("isWindows=true → Windows", () => {
    expect(shellQuote("C:\\a b.png", true)).toBe('"C:\\a b.png"');
  });
});

describe("shellQuoteJoin — 다중 파일 공백 연결", () => {
  it("여러 경로 각각 인용 후 공백 연결(POSIX)", () => {
    expect(shellQuoteJoin(["/tmp/a b.png", "/tmp/c.txt"], false)).toBe("'/tmp/a b.png' '/tmp/c.txt'");
  });
  it("빈 배열은 빈 문자열", () => {
    expect(shellQuoteJoin([], false)).toBe("");
  });
  it("Windows 다중", () => {
    expect(shellQuoteJoin(["C:\\a b.png", "C:\\c.txt"], true)).toBe('"C:\\a b.png" "C:\\c.txt"');
  });
});
