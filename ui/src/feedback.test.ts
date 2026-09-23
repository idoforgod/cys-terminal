// U6(0.14.41) 피드백 1단계 — 순수 판정 핀(DOM·Tauri 무관).
//
// 이 모듈이 정하는 것: 첨부 형식·크기 상한, [메일로 보내기] 활성 조건, 파일 조각 계획, 문구.
// 상한 값은 Rust(src-tauri/src/feedback.rs)가 **다시** 강제한다 — UI 는 먼저 알려 주는 쪽이고
// 최종 판정은 Rust 다. 두 쪽 값이 갈라지면 "UI 는 통과, Rust 는 거부"가 되어 사용자는 이유를
// 모른 채 실패만 본다 — 그래서 값 동일성을 여기서 소스로 대조한다(아래 '상한 파리티').
import { describe, it, expect } from "bun:test";
import { readFileSync } from "node:fs";
import {
  FEEDBACK_ALLOWED_EXT,
  FEEDBACK_CHUNK_BYTES,
  FEEDBACK_MAX_DESC_CHARS,
  FEEDBACK_MAX_FILE_BYTES,
  FEEDBACK_MAX_FILES,
  FEEDBACK_MAX_TOTAL_BYTES,
  FEEDBACK_MIN_DESC_CHARS,
  attachmentKind,
  canSubmit,
  captureTips,
  chunkRanges,
  descLength,
  extOf,
  feedbackImageExt,
  formatBytes,
  mailSizeNote,
  shouldConfirmDiscard,
  submitBlockReason,
  validateAttachment,
} from "./feedback";

const MB = 1024 * 1024;

describe("첨부 형식", () => {
  it("확장자는 마지막 점 뒤 소문자 — 경로 구분자(맥 / · 윈도우 \\) 둘 다 인식", () => {
    expect(extOf("/Users/a/Desktop/화면 기록.MOV")).toBe("mov");
    expect(extOf("C:\\Users\\홍길동\\Pictures\\shot.PNG")).toBe("png");
    expect(extOf("archive.tar.gz")).toBe("gz");
    expect(extOf("noext")).toBe("");
    expect(extOf(".hidden")).toBe(""); // 점으로 시작하는 이름은 확장자가 아니다
    expect(extOf("dir.v2/README")).toBe(""); // 폴더 이름의 점을 확장자로 읽지 않는다
  });

  it("사진·영상·텍스트만 허용하고 실행형·서버 스크립트는 거부", () => {
    expect(attachmentKind("a.png")).toBe("image");
    expect(attachmentKind("a.HEIC")).toBe("image");
    expect(attachmentKind("a.mov")).toBe("video");
    expect(attachmentKind("a.webm")).toBe("video");
    expect(attachmentKind("cysd.log")).toBe("text");
    for (const bad of ["a.exe", "a.php", "a.sh", "a.command", "a.app", "a.zip", "a.html", "noext", "a.svg"]) {
      expect(attachmentKind(bad)).toBeNull();
    }
    // 목록 자체의 핀 — 여기에 실행형이 끼어들면 red.
    for (const e of FEEDBACK_ALLOWED_EXT) {
      expect(["exe", "bat", "cmd", "ps1", "sh", "php", "js", "html", "svg", "app", "command"]).not.toContain(e);
    }
  });
});

describe("첨부 상한", () => {
  const empty = { count: 0, total: 0 };
  it("파일 하나 200MB 경계", () => {
    expect(validateAttachment("a.mp4", FEEDBACK_MAX_FILE_BYTES, empty).ok).toBe(true);
    const r = validateAttachment("a.mp4", FEEDBACK_MAX_FILE_BYTES + 1, empty);
    expect(r.ok).toBe(false);
    if (!r.ok) expect(r.reason).toContain("200");
  });
  it("합계 300MB 를 넘으면 거부", () => {
    const r = validateAttachment("b.mp4", 150 * MB, { count: 1, total: 151 * MB });
    expect(r.ok).toBe(false);
    if (!r.ok) expect(r.reason).toContain("300");
    expect(validateAttachment("b.mp4", 149 * MB, { count: 1, total: 151 * MB }).ok).toBe(true);
  });
  it("11번째 파일은 거부", () => {
    const r = validateAttachment("a.png", 10, { count: FEEDBACK_MAX_FILES, total: 100 });
    expect(r.ok).toBe(false);
    if (!r.ok) expect(r.reason).toContain(String(FEEDBACK_MAX_FILES));
  });
  it("빈 파일·형식 거부는 이유를 말한다", () => {
    const z = validateAttachment("a.png", 0, empty);
    expect(z.ok).toBe(false);
    const x = validateAttachment("setup.exe", 10, empty);
    expect(x.ok).toBe(false);
    if (!x.ok) expect(x.reason).toContain(".exe");
  });
});

describe("[메일로 보내기] 활성 조건", () => {
  it("설명은 공백을 뺀 글자 수로 센다(한글 1자 = 1)", () => {
    expect(descLength("  안녕하세요  ")).toBe(5);
    expect(descLength("\n\n")).toBe(0);
  });
  it("4자 → 비활성 · 5자 → 활성", () => {
    expect(FEEDBACK_MIN_DESC_CHARS).toBe(5);
    expect(canSubmit({ desc: "abcd", pending: 0, submitting: false })).toBe(false);
    expect(canSubmit({ desc: "abcde", pending: 0, submitting: false })).toBe(true);
  });
  it("첨부 중이거나 보내는 중이면 비활성", () => {
    expect(canSubmit({ desc: "충분히 긴 설명", pending: 1, submitting: false })).toBe(false);
    expect(canSubmit({ desc: "충분히 긴 설명", pending: 0, submitting: true })).toBe(false);
  });
  it("상한을 넘는 설명은 비활성 + 이유", () => {
    const long = "가".repeat(FEEDBACK_MAX_DESC_CHARS + 1);
    expect(canSubmit({ desc: long, pending: 0, submitting: false })).toBe(false);
    expect(submitBlockReason({ desc: long, pending: 0, submitting: false })).toContain(String(FEEDBACK_MAX_DESC_CHARS));
  });
  it("비활성 이유는 사람이 읽는 문장이고, 활성이면 null", () => {
    expect(submitBlockReason({ desc: "", pending: 0, submitting: false })).toContain("5자");
    expect(submitBlockReason({ desc: "충분히 긴 설명", pending: 2, submitting: false })).toContain("첨부");
    expect(submitBlockReason({ desc: "충분히 긴 설명", pending: 0, submitting: false })).toBeNull();
  });
});

describe("버리기 확인", () => {
  it("쓴 것이 없으면 묻지 않고 닫는다 · 설명이나 첨부가 있으면 묻는다 · 묶음을 만든 뒤엔 묻지 않는다", () => {
    expect(shouldConfirmDiscard({ desc: "  ", attachments: 0, done: false })).toBe(false);
    expect(shouldConfirmDiscard({ desc: "a", attachments: 0, done: false })).toBe(true);
    expect(shouldConfirmDiscard({ desc: "", attachments: 1, done: false })).toBe(true);
    expect(shouldConfirmDiscard({ desc: "a", attachments: 1, done: true })).toBe(false);
  });
});

describe("파일 조각 계획(파일 고르기·붙여넣기 → 원시 IPC)", () => {
  it("4MB 조각으로 빈틈·겹침 없이 덮는다", () => {
    expect(FEEDBACK_CHUNK_BYTES).toBe(4 * MB);
    const size = 9 * MB + 7;
    const r = chunkRanges(size, FEEDBACK_CHUNK_BYTES);
    expect(r.length).toBe(3);
    expect(r[0]).toEqual([0, 4 * MB]);
    expect(r[2]).toEqual([8 * MB, size]);
    let cur = 0;
    for (const [a, b] of r) {
      expect(a).toBe(cur);
      expect(b).toBeGreaterThan(a);
      cur = b;
    }
    expect(cur).toBe(size);
  });
  it("정확히 나눠떨어지는 크기와 0바이트", () => {
    expect(chunkRanges(8 * MB, 4 * MB)).toEqual([
      [0, 4 * MB],
      [4 * MB, 8 * MB],
    ]);
    expect(chunkRanges(0, 4 * MB)).toEqual([]);
  });
  it("조각 크기가 0 이하면 빈 계획(무한 루프 금지)", () => {
    expect(chunkRanges(10, 0)).toEqual([]);
    expect(chunkRanges(10, -1)).toEqual([]);
  });
});

describe("문구", () => {
  it("바이트 표기", () => {
    expect(formatBytes(512)).toBe("512B");
    expect(formatBytes(1536)).toBe("1.5KB");
    expect(formatBytes(84 * MB)).toBe("84.0MB");
  });
  it("붙여넣기 이미지 확장자 — 이미지 MIME 만, 나머지는 null", () => {
    expect(feedbackImageExt("image/png")).toBe("png");
    expect(feedbackImageExt("image/jpeg")).toBe("jpg");
    expect(feedbackImageExt("IMAGE/WEBP")).toBe("webp");
    expect(feedbackImageExt("image/gif")).toBe("gif");
    expect(feedbackImageExt("image/svg+xml")).toBeNull();
    expect(feedbackImageExt("text/plain")).toBeNull();
  });
  it("메일 첨부 한도(보통 25MB)를 넘길 묶음에는 미리 알린다", () => {
    expect(mailSizeNote(5 * MB)).toBeNull();
    expect(mailSizeNote(30 * MB)).toContain("25MB");
  });
  it("캡처 요령은 플랫폼 단축키를 쓴다", () => {
    expect(captureTips("mac")).toContain("⌘⇧4");
    expect(captureTips("mac")).toContain("⌘⇧5");
    expect(captureTips("win")).toContain("Win+Shift+S");
  });
});

describe("상한 파리티 — UI 와 Rust 가 같은 값을 쓴다", () => {
  const rs = readFileSync(new URL("../../src-tauri/src/feedback.rs", import.meta.url), "utf-8");
  const constOf = (name: string): string => {
    const m = rs.match(new RegExp(`const ${name}: [a-z0-9]+ = ([^;]+);`));
    expect(m).not.toBeNull();
    return (m as RegExpMatchArray)[1].replace(/_/g, "").trim();
  };
  it("개수·크기·설명 상한", () => {
    expect(constOf("MAX_FILES")).toBe(String(FEEDBACK_MAX_FILES));
    expect(constOf("MAX_FILE_BYTES")).toBe("200 * 1024 * 1024");
    expect(FEEDBACK_MAX_FILE_BYTES).toBe(200 * MB);
    expect(constOf("MAX_TOTAL_BYTES")).toBe("300 * 1024 * 1024");
    expect(FEEDBACK_MAX_TOTAL_BYTES).toBe(300 * MB);
    expect(constOf("MIN_DESC_CHARS")).toBe(String(FEEDBACK_MIN_DESC_CHARS));
    expect(constOf("MAX_DESC_CHARS")).toBe(String(FEEDBACK_MAX_DESC_CHARS));
  });
  it("허용 확장자 목록이 같다", () => {
    const m = rs.match(/const ALLOWED_EXT: \[&str; \d+\] = \[([^\]]+)\]/);
    expect(m).not.toBeNull();
    const rsList = (m as RegExpMatchArray)[1]
      .split(",")
      .map((s) => s.trim().replace(/"/g, ""))
      .filter(Boolean);
    expect([...rsList].sort()).toEqual([...FEEDBACK_ALLOWED_EXT].sort());
  });
});
