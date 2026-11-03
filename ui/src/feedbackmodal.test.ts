// F58 C4-b(DESIGN-F58-r1 C4-b · U3) 피드백 창의 자가 수리 안내 — 행동 검체 + 배선 대조.
//
// 창 맨 앞 첫 줄 「먼저 자비스에게 맡겨 보셨나요? 대부분은 자비스가 직접 고칩니다.」, 단추 [자비스에게 맡길 문장 복사]
// (클릭 = 「자비스, 이 문제를 직접 고쳐 줘: <설명 첫 줄>」 을 클립보드에), 기록 위치 안내
// 「자비스가 만든 자가 수리 기록: ~/.cys/local/self-service/<최근>/ (vendor-report.txt · attempts.json 을 첨부하세요)」.
// 사람은 거절되지 않는다(자가 수리 기록이 없어도 보내기는 그대로) · 데몬·에이전트 호출 0(클립보드만 —
// feedbackwiring.test.ts ⑤ 의 feedback_* 핀이 그대로 초록이어야 한다).
// DOM 없이 돈다: 순수 함수는 직접 부르고, 창 배선은 소스를 데이터로 읽어 대조한다(feedbackwiring 관례).
import { describe, it, expect } from "bun:test";
import { readFileSync } from "node:fs";
import * as M from "./feedbackmodal";
import { FEEDBACK_MIN_DESC_CHARS, canSubmit, submitBlockReason } from "./feedback";

const FIRST = "먼저 자비스에게 맡겨 보셨나요? 대부분은 자비스가 직접 고칩니다.";
const LABEL = "자비스에게 맡길 문장 복사";
const HINT =
  "자비스가 만든 자가 수리 기록: ~/.cys/local/self-service/<최근>/ (vendor-report.txt · attempts.json 을 첨부하세요)";
const ASK = "자비스, 이 문제를 직접 고쳐 줘: ";

const mod = M as unknown as Record<string, unknown>;
const src = readFileSync(new URL("./feedbackmodal.ts", import.meta.url), "utf-8");

const stripComments = (s: string): string =>
  s
    .split("\n")
    .map((l) => (l.trimStart().startsWith("//") ? "" : l.replace(/\s\/\/.*$/, "")))
    .join("\n");

function fnBody(s: string, header: string): string {
  const a = s.indexOf(header);
  expect(a).toBeGreaterThanOrEqual(0);
  const open = s.indexOf("{", a);
  let depth = 0;
  for (let i = open; i < s.length; i++) {
    if (s[i] === "{") depth++;
    else if (s[i] === "}") {
      depth--;
      if (depth === 0) return s.slice(a, i + 1);
    }
  }
  throw new Error(`unbalanced: ${header}`);
}

const code = stripComments(src);
const tplStart = code.indexOf("const MODAL_HTML =");
const tpl = code.slice(tplStart, code.indexOf("function errText(", tplStart));

type Clip = { writeText: (s: string) => Promise<void> } | null | undefined;
const sentence = (d: string): string => (mod.selfServiceSentence as (d: string) => string)(d);
const copy = (d: string, c: Clip): Promise<string> =>
  (mod.copySelfServiceSentence as (d: string, c: Clip) => Promise<string>)(d, c);

describe("C4-b 문구(글자 그대로)", () => {
  it("첫 줄", () => expect(mod.SELF_SERVICE_FIRST_LINE).toBe(FIRST));
  it("단추 이름", () => expect(mod.SELF_SERVICE_COPY_LABEL).toBe(LABEL));
  it("기록 위치 안내", () => expect(mod.SELF_SERVICE_HINT).toBe(HINT));
});

describe("C4-b 맡길 문장 = 「자비스, 이 문제를 직접 고쳐 줘: <설명 첫 줄>」", () => {
  it("설명 첫 줄만 넣는다", () => {
    expect(typeof mod.selfServiceSentence).toBe("function");
    expect(sentence("앱이 멈춥니다\n두 번째 줄")).toBe(ASK + "앱이 멈춥니다");
  });
  it("앞쪽 빈 줄과 앞뒤 공백은 건너뛴다(CRLF 포함)", () => {
    expect(sentence("\n\r\n  화면이 깨져요  \r\n둘째")).toBe(ASK + "화면이 깨져요");
  });
  it("설명이 비어 있어도 문장 틀은 그대로다", () => {
    expect(sentence("")).toBe(ASK);
    expect(sentence("  \n ")).toBe(ASK);
  });
});

describe("C4-b 단추 클릭 → 클립보드(그 외 경로 0)", () => {
  it("클립보드에 그 문장 하나만 쓴다", async () => {
    expect(typeof mod.copySelfServiceSentence).toBe("function");
    const written: string[] = [];
    const clip = {
      writeText: (s: string) => {
        written.push(s);
        return Promise.resolve();
      },
    };
    const got = await copy("앱이 멈춥니다\n둘째", clip);
    expect(written).toEqual([ASK + "앱이 멈춥니다"]);
    expect(got).toBe(written[0]);
  });
  it("클립보드가 없으면 거절로 끝난다(창은 상태줄에 문장을 보여 준다)", async () => {
    expect(typeof mod.copySelfServiceSentence).toBe("function");
    let rejected = false;
    await copy("앱이 멈춥니다", undefined).catch(() => {
      rejected = true;
    });
    expect(rejected).toBe(true);
  });
  it("창의 단추 클릭은 copySelfServiceSentence(desc.value, navigator.clipboard) 를 부르고 invoke 는 부르지 않는다", () => {
    const h = fnBody(code, 'ssCopyBtn.addEventListener("click", () =>');
    expect(h).toContain("copySelfServiceSentence(desc.value, navigator.clipboard)");
    expect(/invoke|listen\(|deps\./.test(h)).toBe(false);
    expect(code).toContain('const ssCopyBtn = q<HTMLButtonElement>(".fb-ss-copy");');
  });
});

describe("C4-b 창 배치 — 작성 칸 맨 앞", () => {
  it("첫 줄·단추·안내 칸이 고정 틀에 있고 fb-intro 보다 앞이다", () => {
    expect(tplStart).toBeGreaterThan(0);
    const at = (cls: string) => tpl.search(new RegExp(`class="[^"]*\\b${cls}\\b`));
    for (const cls of ["fb-ss", "fb-ss-line", "fb-ss-copy", "fb-ss-hint"]) expect(`${cls}:${at(cls) > 0}`).toBe(`${cls}:true`);
    expect(at("fb-compose")).toBeLessThan(at("fb-ss"));
    expect(at("fb-ss")).toBeLessThan(at("fb-intro"));
    expect(/<button type="button" class="[^"]*\bfb-ss-copy\b/.test(tpl)).toBe(true);
  });
  it("문구는 상수에서 textContent 로 넣는다(고정 틀에 사본 0)", () => {
    expect(code).toContain('q<HTMLElement>(".fb-ss-line").textContent = SELF_SERVICE_FIRST_LINE;');
    expect(code).toContain("ssCopyBtn.textContent = SELF_SERVICE_COPY_LABEL;");
    expect(code).toContain('q<HTMLElement>(".fb-ss-hint").textContent = SELF_SERVICE_HINT;');
    expect(tpl.includes(FIRST) || tpl.includes(LABEL) || tpl.includes(HINT)).toBe(false);
  });
});

describe("C4-b 사람은 거절되지 않는다(자가 수리 기록 없이도 보내기)", () => {
  it("보내기 판정은 설명만 본다", () => {
    const st = { desc: "가".repeat(FEEDBACK_MIN_DESC_CHARS), pending: 0, submitting: false };
    expect(canSubmit(st)).toBe(true);
    expect(submitBlockReason(st)).toBeNull();
  });
  it("submit·refresh 는 자가 수리 이름을 보지 않는다 — 보내기 단추 판정 줄은 그대로", () => {
    const sub = fnBody(code, "const submit = async () =>");
    const ref = fnBody(code, "const refresh = () =>");
    for (const body of [sub, ref]) expect(/selfService|SELF_SERVICE|fb-ss|ssCopy/.test(body)).toBe(false);
    expect(ref).toContain("sendBtn.disabled = done || !canSubmit(st);");
  });
});
