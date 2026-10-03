// 0.14.43 J2 — 업데이트가 설치되지 않았을 때의 알림(updatenotice.ts 순수 문구·해석 + main.ts·백엔드 배선) 회귀 핀.
//
// 무엇을 지키는가(티켓 J2):
//   · Windows 스마트 앱 컨트롤이 서명 없는 설치 파일을 막아도 인앱 업데이트는 침묵했다 — 설치기를 띄운 뒤 앱이 곧바로 끝나 실패를 알릴
//     코드가 실행되지 않았다. 이제 백엔드가 설치 직전에 시도 기록을 남기고 다시 뜬 앱이 한 번 판정해 알린다.
//   · sacPreflightText: 스마트 앱 컨트롤이 켜짐("on")일 때만 사전 안내 문단을 돌려준다(그 밖은 null → 확인 창 본문 불변).
//   · updateFailedNotice: 공통 첫 문장 + OS 별 사실 고지. **스마트 앱 컨트롤을 끄라는 말이 어디에도 없다**(사실 고지만).
//   · planUpdateAttemptReport: 백엔드 응답의 해석 — failed → 알림 · pending → (wait_secs+5)초 뒤 재-pull 한 번 · 그 밖 → 없음.
//   · main.ts 배선: pull 은 bundle_integrity pull 바로 뒤에서 fire-and-forget · 재-pull 은 한 번뿐(타이머 1개·재귀 없음) ·
//     설치 확인 창은 조회 실패·초과에도 열린다(문단 없으면 본문 바이트 동일).
//   · 백엔드(src-tauri) 쪽 계약(명령 등재·응답 키)이 UI 가 읽는 것과 같다.
import { describe, it, expect } from "bun:test";
import { readFileSync } from "node:fs";
import {
  sacPreflightText,
  updateFailedNotice,
  planUpdateAttemptReport,
  UPDATE_FAILED_TITLE,
  UPDATE_FAILED_TOAST_ID,
  UPDATE_ATTEMPT_RETRY_SLACK_SECS,
  UPDATE_ATTEMPT_WAIT_DEFAULT_SECS,
  UPDATE_ATTEMPT_WAIT_MAX_SECS,
} from "./updatenotice";

const read = (rel: string) => readFileSync(new URL(rel, import.meta.url), "utf-8");

/** main.ts 소스(주석 제거본) — 주석에만 적힌 장치로 핀이 통과하지 않게 한다. */
const stripComments = (s: string): string =>
  s
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .split("\n")
    .map((l) => (l.trimStart().startsWith("//") ? "" : l.replace(/\s\/\/.*$/, "")))
    .join("\n");
const mainCode = stripComments(read("./main.ts"));
/** `async function name(` / `function name(` 부터 다음 최상위 `\n}\n` 직전까지(끝 중괄호는 뺀다). */
const fnBodyOf = (name: string): string => {
  const a = mainCode.search(new RegExp(`(async )?function ${name}\\(`));
  expect({ 함수: name, 존재: a >= 0 }).toEqual({ 함수: name, 존재: true });
  const b = mainCode.indexOf("\n}\n", a);
  expect(b).toBeGreaterThan(a);
  return mainCode.slice(a, b);
};
const count = (hay: string, needle: string): number => hay.split(needle).length - 1;

/** 티켓이 정한 문안 전문 — 모듈의 문구가 바뀌면 이 핀이 빨개진다(손으로 옮긴 사본이 아니라 티켓 원문이다). */
const SAC_PREFLIGHT =
  "이 PC 는 Windows '스마트 앱 컨트롤'이 켜져 있습니다. 지금 cys 설치 파일에는 코드 서명이 없어 Windows 가 설치를 막을 수 있습니다. " +
  "막히면 경고 없이 지금 버전이 그대로 남고, 앱을 다시 열면 '설치되지 않았습니다' 알림이 뜹니다. " +
  "스마트 앱 컨트롤이 켜진 PC 에서는 홈페이지에서 받은 설치 파일도 같은 이유로 막힙니다.";
const WIN_PARAGRAPH =
  "Windows 가 서명 없는 설치 파일을 막았을 수 있습니다(스마트 앱 컨트롤 · Defender). " +
  "확인: 이벤트 뷰어 → 응용 프로그램 및 서비스 로그 → Microsoft → Windows → CodeIntegrity → Operational 의 이벤트 3033·3077. " +
  "설치기는 임시 폴더의 cys-<to>-updater-… 아래에 풀립니다.";
const SAC_ON_LINE = "이 PC 의 스마트 앱 컨트롤: 켜짐 — 켜져 있는 동안은 서명 없는 설치 파일이 수동 설치에서도 막힙니다.";
const SAC_EVAL_LINE = "이 PC 의 스마트 앱 컨트롤: 평가 모드(차단하지 않음) — 다른 원인(Defender 등)을 확인해 주세요.";
const OTHER_OS_LINE = "홈페이지(www.cysinsight.com)에서 설치 파일을 받아 직접 설치해 주세요.";
const first = (from: string, to: string) => `${to} 업데이트가 설치되지 않았습니다 — 지금 버전은 ${from} 그대로입니다.`;
const win = (to: string) => WIN_PARAGRAPH.replace("<to>", to);

/** 끄라는 지시·끄는 방법으로 읽힐 수 있는 낱말 — 문구는 사실만 말한다(티켓 '하지 않는 것'). */
const DISABLE_WORDS = ["끄", "끌", "꺼", "비활성", "해제", "disable", "turn off"];
const hasDisableWord = (s: string): string[] => DISABLE_WORDS.filter((w) => s.toLowerCase().includes(w));

describe("sacPreflightText — 켜짐(on)일 때만 문단", () => {
  it("\"on\" → 티켓 문안 전문(「스마트 앱 컨트롤」·「코드 서명」 포함)", () => {
    const t = sacPreflightText("on");
    expect(t).toBe(SAC_PREFLIGHT);
    expect((t ?? "").includes("스마트 앱 컨트롤")).toBe(true);
    expect((t ?? "").includes("코드 서명")).toBe(true);
  });

  it("\"eval\"·\"off\"·null·undefined·\"\" → null", () => {
    for (const v of ["eval", "off", null, undefined, ""] as const) {
      expect({ 입력: v, 결과: sacPreflightText(v) }).toEqual({ 입력: v, 결과: null });
    }
  });

  it("모르는 값(대소문자 다른 \"ON\"·공백 낀 \"on \"·숫자 모양)도 null — 정확히 \"on\" 만 켜짐으로 읽는다", () => {
    for (const v of ["ON", "On", " on", "on ", "1", "0x1", "true", "yes", "enabled"]) {
      expect({ 입력: v, 결과: sacPreflightText(v) }).toEqual({ 입력: v, 결과: null });
    }
  });

  it("사실만 말한다 — 끄라는 말·끄는 방법이 없다 · 설치를 막는 말(금지·불가·중단)도 없다", () => {
    const t = sacPreflightText("on") ?? "";
    expect(hasDisableWord(t)).toEqual([]);
    for (const w of ["금지", "불가", "중단", "하지 마"]) expect({ 낱말: w, 있음: t.includes(w) }).toEqual({ 낱말: w, 있음: false });
  });
});

describe("updateFailedNotice — 제목 · 공통 첫 문장 · OS 별 분기", () => {
  it("제목은 「업데이트가 설치되지 않았습니다」", () => {
    expect(UPDATE_FAILED_TITLE).toBe("업데이트가 설치되지 않았습니다");
    expect(updateFailedNotice({ from: "0.14.42", to: "0.14.43", os: "windows" }).title).toBe("업데이트가 설치되지 않았습니다");
    expect(updateFailedNotice({ from: "0.14.42", to: "0.14.43", os: "macos" }).title).toBe("업데이트가 설치되지 않았습니다");
  });

  it("windows + on: 첫 문장 · 서명 없는 설치 파일 사실 · 이벤트 3033·3077 · 임시 폴더 이름 · 켜짐 한 줄", () => {
    const n = updateFailedNotice({ from: "0.14.42", to: "0.14.43", os: "windows", sac: "on" });
    expect(n.body).toBe([first("0.14.42", "0.14.43"), win("0.14.43"), SAC_ON_LINE].join("\n"));
    for (const must of ["CodeIntegrity", "3033", "3077", "cys-0.14.43-updater-", "스마트 앱 컨트롤", "Defender", "이벤트 뷰어"]) {
      expect({ 낱말: must, 있음: n.body.includes(must) }).toEqual({ 낱말: must, 있음: true });
    }
    expect(n.body.includes(OTHER_OS_LINE)).toBe(false);
    expect(n.body.includes(SAC_EVAL_LINE)).toBe(false);
  });

  it("windows + eval: 평가 모드 한 줄(차단하지 않음) · 켜짐 줄은 없다", () => {
    const n = updateFailedNotice({ from: "0.14.42", to: "0.14.43", os: "windows", sac: "eval" });
    expect(n.body).toBe([first("0.14.42", "0.14.43"), win("0.14.43"), SAC_EVAL_LINE].join("\n"));
    expect(n.body.includes("켜짐")).toBe(false);
    expect(n.body.includes("평가 모드(차단하지 않음)")).toBe(true);
  });

  it("windows + null·undefined·off·모르는 값: 스마트 앱 컨트롤 줄 없음(윈도우 문단까지만)", () => {
    for (const sac of [null, undefined, "off", "", "weird"] as const) {
      const n = updateFailedNotice({ from: "0.14.42", to: "0.14.43", os: "windows", sac });
      expect({ sac, body: n.body }).toEqual({ sac, body: [first("0.14.42", "0.14.43"), win("0.14.43")].join("\n") });
      expect(n.body.includes("이 PC 의 스마트 앱 컨트롤")).toBe(false);
    }
    // sac 키 자체가 없어도 같다
    expect(updateFailedNotice({ from: "0.14.42", to: "0.14.43", os: "windows" }).body).toBe(
      [first("0.14.42", "0.14.43"), win("0.14.43")].join("\n"),
    );
  });

  it("macos: 홈페이지에서 직접 설치 안내 · 윈도우 문구(스마트 앱 컨트롤·CodeIntegrity·Defender)는 없다 · sac 값이 와도 무시", () => {
    const n = updateFailedNotice({ from: "0.14.42", to: "0.14.43", os: "macos" });
    expect(n.body).toBe([first("0.14.42", "0.14.43"), OTHER_OS_LINE].join("\n"));
    for (const w of ["스마트 앱 컨트롤", "CodeIntegrity", "Defender", "이벤트 뷰어", "3033", "updater-"]) {
      expect({ 낱말: w, 있음: n.body.includes(w) }).toEqual({ 낱말: w, 있음: false });
    }
    expect(updateFailedNotice({ from: "0.14.42", to: "0.14.43", os: "macos", sac: "on" }).body).toBe(n.body);
  });

  it("linux·os 없음·모르는 OS·대소문자 다른 Windows: 윈도우가 아니면 홈페이지 안내 · \"Windows\"·공백 낀 값은 윈도우로 읽는다", () => {
    const other = [first("1.0.0", "1.1.0"), OTHER_OS_LINE].join("\n");
    for (const os of ["linux", undefined, "", "freebsd", "darwin"]) {
      expect({ os, body: updateFailedNotice({ from: "1.0.0", to: "1.1.0", os }).body }).toEqual({ os, body: other });
    }
    for (const os of ["Windows", " windows ", "WINDOWS"]) {
      expect({ os, 윈도우: updateFailedNotice({ from: "1.0.0", to: "1.1.0", os }).body.includes("CodeIntegrity") }).toEqual({ os, 윈도우: true });
    }
  });

  it("<from>·<to> 치환 — 자리표시자가 남지 않고 임시 폴더 이름에도 새 버전이 들어간다", () => {
    for (const os of ["windows", "macos"]) {
      const b = updateFailedNotice({ from: "0.9.1", to: "0.9.2", os, sac: "on" }).body;
      expect(b.startsWith("0.9.2 업데이트가 설치되지 않았습니다 — 지금 버전은 0.9.1 그대로입니다.")).toBe(true);
      expect(b.includes("<from>") || b.includes("<to>") || b.includes("undefined") || b.includes("[object")).toBe(false);
    }
    expect(updateFailedNotice({ from: "0.9.1", to: "0.9.2", os: "windows" }).body.includes("cys-0.9.2-updater-…")).toBe(true);
  });

  it("★문구에 끄라는 지시가 없다 — 모든 OS × 스마트 앱 컨트롤 상태 조합(사실 고지만)", () => {
    for (const os of ["windows", "macos", "linux", undefined]) {
      for (const sac of ["on", "off", "eval", null, undefined]) {
        const n = updateFailedNotice({ from: "0.14.42", to: "0.14.43", os, sac });
        expect({ os, sac, 낱말: hasDisableWord(n.title + "\n" + n.body) }).toEqual({ os, sac, 낱말: [] });
      }
    }
  });

  it("버전 문자열이 이상해도 한 줄로 접고 40자로 자른다 — 문자열이 아니거나 비면 \"?\"", () => {
    const hostile = updateFailedNotice({ from: "0.14.42\n‮<b>x</b>", to: "9".repeat(200), os: "windows" }).body;
    expect(hostile.split("\n").length).toBe(2); // 첫 문장 + 윈도우 문단(버전의 개행이 줄을 늘리지 않는다)
    expect(hostile.includes("‮")).toBe(false);
    expect(hostile.includes("9".repeat(40))).toBe(false); // 39자 + …
    expect(hostile.includes("9".repeat(39) + "…")).toBe(true);
    const unknown = updateFailedNotice({ from: undefined as unknown as string, to: "" , os: "macos" }).body;
    expect(unknown).toBe(first("?", "?") + "\n" + OTHER_OS_LINE);
    // HTML 은 해석하지 않고 글자 그대로 둔다 — 화면은 textContent 로만 그린다
    expect(hostile.includes("<b>x</b>")).toBe(true);
  });

  it("돌려주는 객체의 키는 정확히 title·body", () => {
    expect(Object.keys(updateFailedNotice({ from: "a", to: "b", os: "windows" })).sort()).toEqual(["body", "title"]);
  });
});

describe("planUpdateAttemptReport — 백엔드 응답 해석(알림 · 재-pull 한 번 · 없음)", () => {
  it("failed → 알림(제목·본문은 updateFailedNotice 와 같다)", () => {
    const r = { failed: true, from: "0.14.42", to: "0.14.43", at: 1_760_000_000, os: "windows", sac: "on" };
    const want = updateFailedNotice({ from: "0.14.42", to: "0.14.43", os: "windows", sac: "on" });
    expect(planUpdateAttemptReport(r)).toEqual({ kind: "notice", title: want.title, body: want.body });
    const m = planUpdateAttemptReport({ failed: true, from: "0.14.42", to: "0.14.43", at: 1, os: "macos", sac: null });
    expect(m).toEqual({ kind: "notice", title: UPDATE_FAILED_TITLE, body: [first("0.14.42", "0.14.43"), OTHER_OS_LINE].join("\n") });
  });

  it("failed 인데 필드가 모자라면 \"?\" 로 알린다 — 실패를 침묵하지 않는다", () => {
    const p = planUpdateAttemptReport({ failed: true });
    expect(p).toEqual({ kind: "notice", title: UPDATE_FAILED_TITLE, body: first("?", "?") + "\n" + OTHER_OS_LINE });
    // sac 가 문자열이 아니면 null 로(켜짐 줄 없음) · os 가 문자열이 아니면 윈도우 아님
    const q = planUpdateAttemptReport({ failed: true, from: "a", to: "b", os: 7, sac: 1 });
    expect(q).toEqual({ kind: "notice", title: UPDATE_FAILED_TITLE, body: first("a", "b") + "\n" + OTHER_OS_LINE });
  });

  it("pending → (wait_secs + 5)초 뒤 재-pull — 80초면 85000ms", () => {
    expect(UPDATE_ATTEMPT_RETRY_SLACK_SECS).toBe(5);
    expect(planUpdateAttemptReport({ pending: true, wait_secs: 80 })).toEqual({ kind: "retry", delayMs: 85_000 });
    expect(planUpdateAttemptReport({ pending: true, wait_secs: 1 })).toEqual({ kind: "retry", delayMs: 6_000 });
    expect(planUpdateAttemptReport({ pending: true, wait_secs: 90 })).toEqual({ kind: "retry", delayMs: 95_000 });
    expect(planUpdateAttemptReport({ pending: true, wait_secs: 0 })).toEqual({ kind: "retry", delayMs: 5_000 });
    expect(planUpdateAttemptReport({ pending: true, wait_secs: 0.2 })).toEqual({ kind: "retry", delayMs: 6_000 }); // 올림
  });

  it("pending 의 wait_secs 가 숫자가 아니거나 음수·무한이면 판정 보류 창 전체(90초)를 기다린다 · 300초 상한(타이머 한계 방어)", () => {
    expect(UPDATE_ATTEMPT_WAIT_DEFAULT_SECS).toBe(90);
    expect(UPDATE_ATTEMPT_WAIT_MAX_SECS).toBe(300);
    for (const w of [undefined, null, "80", NaN, Infinity, -Infinity, -1, {}, [], true]) {
      expect({ w, 결과: planUpdateAttemptReport({ pending: true, wait_secs: w }) }).toEqual({ w, 결과: { kind: "retry", delayMs: 95_000 } });
    }
    expect(planUpdateAttemptReport({ pending: true })).toEqual({ kind: "retry", delayMs: 95_000 });
    expect(planUpdateAttemptReport({ pending: true, wait_secs: 1e12 })).toEqual({ kind: "retry", delayMs: 305_000 });
    expect(planUpdateAttemptReport({ pending: true, wait_secs: 300 })).toEqual({ kind: "retry", delayMs: 305_000 });
    expect(planUpdateAttemptReport({ pending: true, wait_secs: 301 })).toEqual({ kind: "retry", delayMs: 305_000 });
  });

  it("null·undefined·원시값·배열·빈 객체·모르는 모양 → none", () => {
    for (const r of [null, undefined, "failed", 0, 1, true, [], [{ failed: true }], {}, { failed: false }, { pending: false }, { failed: "true" }, { pending: 1 }, { ok: true }]) {
      expect({ r, 결과: planUpdateAttemptReport(r) }).toEqual({ r, 결과: { kind: "none" } });
    }
  });

  it("failed 와 pending 이 함께 오면(있을 수 없는 응답) 알림이 이긴다", () => {
    const p = planUpdateAttemptReport({ failed: true, pending: true, wait_secs: 80, from: "a", to: "b", os: "macos" });
    expect(p.kind).toBe("notice");
  });

  it("백엔드가 모르는 키를 더 보내도 무시한다(가산 호환)", () => {
    const p = planUpdateAttemptReport({ pending: true, wait_secs: 10, future_key: { x: 1 } });
    expect(p).toEqual({ kind: "retry", delayMs: 15_000 });
  });
});

describe("신뢰할 수 없는 응답 — 결정론 난수 500판(던지지 않고, 알림 문자열은 제어문자 없음)", () => {
  // 결정론 의사난수(LCG) — 실행마다 같은 입력.
  let seed = 20261003;
  const rnd = (n: number): number => {
    seed = (seed * 1664525 + 1013904223) % 4294967296;
    return seed % n;
  };
  const pool: unknown[] = [
    null, undefined, true, false, 0, -1, 90, 1e12, NaN, "", "x", "0.14.42", "windows", "macos", "on", "eval", "off", "a\nb", "\u0000\u001f", "‮", "😀".repeat(30),
    [], [1], {}, { a: 1 },
  ];
  const pick = (): unknown => pool[rnd(pool.length)];
  const BAD_CTRL = /[\u{0}-\u{9}\u{b}-\u{1f}\u{7f}-\u{9f}\u{200b}-\u{200f}\u{2028}\u{2029}\u{202a}-\u{202e}\u{2066}-\u{2069}\u{feff}]/u;
  it("어떤 조합이든 none·retry·notice 중 하나 — 알림의 제목·본문에 개행 외 제어문자 0 · 재시도 지연은 0 초과 305000 이하의 정수", () => {
    for (let i = 0; i < 500; i++) {
      const r = { failed: pick(), pending: pick(), wait_secs: pick(), from: pick(), to: pick(), os: pick(), sac: pick() };
      const p = planUpdateAttemptReport(i % 7 === 0 ? pick() : r);
      expect(["none", "retry", "notice"].includes(p.kind)).toBe(true);
      if (p.kind === "notice") {
        expect(BAD_CTRL.test(p.title)).toBe(false);
        expect(BAD_CTRL.test(p.body)).toBe(false);
        expect(p.title).toBe(UPDATE_FAILED_TITLE);
        expect(hasDisableWord(p.body)).toEqual([]);
      }
      if (p.kind === "retry") {
        expect(p.delayMs > 0 && p.delayMs <= 305_000 && Number.isInteger(p.delayMs)).toBe(true);
      }
    }
  });
});

describe("새 순수 모듈 updatenotice.ts — 불변식(최상위 부수효과 0 · 문서/창/저장소/타이머/IPC 0 · 구형 WKWebView 비호환 문법 0)", () => {
  const raw = read("./updatenotice.ts");
  const stripLine = (s: string): string =>
    s
      .split("\n")
      .map((l) => (l.trimStart().startsWith("//") ? "" : l.replace(/\s\/\/.*$/, "")))
      .join("\n");
  const code = stripLine(raw.replace(/\/\*[\s\S]*?\*\//g, ""));
  it("원문(주석 포함)에 문서 객체·저장소·타이머·IPC 낱말이 없다", () => {
    for (const w of ["document", "localStorage", "sessionStorage", "indexedDB", "setTimeout", "setInterval", "__TAURI__", "invoke(", "fetch(", "navigator"])
      expect({ 낱말: w, 있음: raw.includes(w) }).toEqual({ 낱말: w, 있음: false });
  });
  it("코드에 창 객체 접근(window.) 0", () => {
    expect({ 창접근: code.includes("window.") }).toEqual({ 창접근: false });
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
  it("화면·조치를 일으키는 코드가 없다 — 문구와 해석만(HTML 삽입·토스트·재기동·설치 호출 낱말 0)", () => {
    for (const bad of ["innerHTML", "textContent", "stickyToast", "confirmModal", "install_update", "send_input", "app.restart"])
      expect({ 낱말: bad, 있음: code.includes(bad) }).toEqual({ 낱말: bad, 있음: false });
  });
  it("모듈 원문에도 끄라는 낱말이 없다(주석 포함 — 문구 사본이 따로 생기지 않게)", () => {
    // 설명 주석의 '끄라는 지시도 끄는 방법도 없다' 문장은 부정문이라 낱말 핀에서 제외하고, 문자열 리터럴(실제 문구)만 본다.
    const literals = (code.match(/"(?:[^"\\\n]|\\.)*"|`(?:[^`\\]|\\.)*`/g) ?? []).join("\n");
    expect(hasDisableWord(literals)).toEqual([]);
  });
});

describe("main.ts 배선 — 기동 pull(bundle_integrity 바로 뒤 · fire-and-forget · 재-pull 한 번) · 설치 확인 창의 사전 고지", () => {
  const code = mainCode;
  const fnBody = fnBodyOf;

  it("순수 모듈의 도우미를 import 한다", () => {
    expect(/import\s*\{[^}]*\bplanUpdateAttemptReport\b[^}]*\}\s*from\s*["']\.\/updatenotice["']/.test(code)).toBe(true);
    expect(/import\s*\{[^}]*\bsacPreflightText\b[^}]*\}\s*from\s*["']\.\/updatenotice["']/.test(code)).toBe(true);
    expect(/import\s*\{[^}]*\bUPDATE_FAILED_TOAST_ID\b[^}]*\}\s*from\s*["']\.\/updatenotice["']/.test(code)).toBe(true);
  });

  it("★start() 의 update_attempt_report pull 은 첫 bundle_integrity pull 뒤 · 다음 pull(claude_missing_hint) 앞 — fire-and-forget(await 하지 않는다)", () => {
    const s = fnBody("start");
    const bundle = s.indexOf('invoke("bundle_integrity")');
    const call = s.indexOf("void pullUpdateAttemptReport();");
    const next = s.indexOf('invoke("claude_missing_hint")');
    expect(bundle).toBeGreaterThanOrEqual(0);
    expect(call).toBeGreaterThan(bundle);
    expect(next).toBeGreaterThan(call);
    // 호출은 기동 경로에 정확히 한 곳 — 어디서도 await 하지 않는다(기동 지연 0)
    expect(count(code, "pullUpdateAttemptReport()")).toBe(2); // 정의 1 + 호출 1
    expect(/await\s+pullUpdateAttemptReport\(/.test(code)).toBe(false);
    // 첫 bundle_integrity pull 과 이 호출 사이에는 그 pull 의 try/catch 닫힘뿐이다(바로 뒤)
    const between = s.slice(bundle, call);
    expect(count(between, 'invoke("')).toBe(1);
    // 파일에 적힌 순서로도 뒤다 — update_attempt_report 호출은 어느 bundle_integrity pull 보다도 아래에 있다(정의가 start() 바로 아래)
    const pull = code.indexOf('invoke("update_attempt_report")');
    expect(pull).toBeGreaterThan(code.lastIndexOf('invoke("bundle_integrity")'));
    expect(code.indexOf("async function pullUpdateAttemptReport(")).toBeGreaterThan(code.indexOf("async function start()") + s.length);
  });

  it("★pullUpdateAttemptReport: 재-pull 은 한 번뿐 — update_attempt_report 호출 2회 · setTimeout 1개 · 재귀·반복·인터벌 없음", () => {
    const b = fnBody("pullUpdateAttemptReport");
    expect(count(b, 'invoke("update_attempt_report")')).toBe(2);
    expect(count(b, "setTimeout(")).toBe(1);
    expect(b.includes("setInterval(")).toBe(false);
    expect(b.includes("pullUpdateAttemptReport(")).toBe(true); // 선언부 자신의 이름(재귀 호출은 아래에서 따로 센다)
    expect(count(b, "pullUpdateAttemptReport(")).toBe(1); // 본문에 자기 호출 0 — 선언의 이름뿐
    expect(/\b(while|for)\s*\(/.test(b)).toBe(false);
    // 해석은 순수 함수가 한다 — 응답마다 planUpdateAttemptReport 를 거친다(2회)
    expect(count(b, "planUpdateAttemptReport(")).toBe(2);
    // 재시도 지연은 계획이 정한 값(delayMs)을 쓴다 — 두 번째 pull 은 그 대기 뒤에 있다
    const wait = b.indexOf("setTimeout(");
    const second = b.lastIndexOf('invoke("update_attempt_report")');
    const firstCall = b.indexOf('invoke("update_attempt_report")');
    expect(firstCall).toBeLessThan(wait);
    expect(wait).toBeLessThan(second);
    expect(/plan\.kind === "retry"/.test(b)).toBe(true);
    expect(b.includes("plan.delayMs")).toBe(true);
  });

  it("★알림은 stickyToast(UPDATE_FAILED_TOAST_ID, \"health\", 제목, 본문) 한 곳 — id 는 순수 모듈의 상수 · textContent 경로(HTML 삽입 없음) · 알림일 때만", () => {
    const b = fnBody("pullUpdateAttemptReport");
    expect(count(b, "stickyToast(")).toBe(1);
    expect(b.includes('if (plan.kind === "notice") stickyToast(UPDATE_FAILED_TOAST_ID, "health", plan.title, plan.body);')).toBe(true);
    expect(b.includes("innerHTML")).toBe(false);
    // id 문자열의 정의처는 updatenotice.ts 하나다 — main.ts 에는 리터럴이 없고, 상수 식별자는 import 1 + 사용 1 이다(같은 id 로 다른 곳에서 띄우지 않는다)
    expect(count(code, '"update-not-installed"')).toBe(0);
    expect(count(code, "UPDATE_FAILED_TOAST_ID")).toBe(2);
    // 상수의 값(수명·만료 배너를 거는 toastttl.ts 와 같아야 한다 — 두 값을 함께 재는 검체는 toastttl.test.ts)
    expect(UPDATE_FAILED_TOAST_ID).toBe("update-not-installed");
  });

  it("★조회 실패는 조용히 무시한다 — try/catch 로 감싸고 catch 안에서 던지거나 토스트를 내지 않는다(부팅 무영향)", () => {
    const b = fnBody("pullUpdateAttemptReport");
    const t = b.indexOf("try {");
    const c = b.indexOf("} catch {");
    expect(t).toBeGreaterThanOrEqual(0);
    expect(c).toBeGreaterThan(b.lastIndexOf('invoke("update_attempt_report")')); // 두 pull 모두 try 안
    const handler = b.slice(c);
    expect(handler.includes("throw")).toBe(false);
    expect(handler.includes("toast(")).toBe(false);
  });

  it("★promptBinaryPatch: smart_app_control 조회는 rpcT(T_SAC) 상한 + catch 로 접히고, 확인 창(confirmModal)보다 앞이다", () => {
    const b = fnBody("promptBinaryPatch");
    const q = b.indexOf('invoke("smart_app_control")');
    const modal = b.indexOf("confirmModal(");
    expect(q).toBeGreaterThanOrEqual(0);
    expect(q).toBeLessThan(modal);
    expect(b.includes('await rpcT(invoke("smart_app_control"), T_SAC)')).toBe(true);
    expect(count(b, 'invoke("smart_app_control")')).toBe(1);
    // try { … } catch { sacNote = null } — 창이 이 조회 때문에 멈추거나 실패하지 않는다
    const tryAt = b.lastIndexOf("try {", q);
    const catchAt = b.indexOf("} catch {", q);
    expect(tryAt).toBeGreaterThanOrEqual(0);
    expect(catchAt).toBeGreaterThan(q);
    expect(catchAt).toBeLessThan(modal);
    expect(b.slice(catchAt, modal).includes("sacNote = null")).toBe(true);
    expect(b.includes("sacPreflightText(")).toBe(true);
    // 상한 상수의 정의처가 있다(리터럴 흩뿌리기 금지 — 값마다 근거 주석)
    expect(/const T_SAC = winScaled\(\d[\d_]*\);/.test(code)).toBe(true);
  });

  it("★확인 창 본문: 문단이 없으면 종전과 바이트 동일 · 있으면 맨 끝에 한 줄 띄우고 붙는다 · 설치를 막지 않는다(`if (!ok) return;` 그대로)", () => {
    const b = fnBody("promptBinaryPatch");
    // 종전 본문 조각 3개가 그대로 있다(앞부분을 건드리지 않았다)
    for (const piece of [
      "`새 본체 버전 ${v} — 패치 설치`",
      "`새 본체(앱) ${v}을 패치 방식으로 설치합니다: 저장(drain) 신호 후 다운로드·서명 검증·교체하고 앱을 ` +",
      "`재시작합니다. 부서·노드는 재시작 후 자동 복원됩니다(대화 기억 포함). 마지막 미저장분은 손실될 수 ` +",
      "`있습니다.\\n\\n지금 설치하시겠습니까? (수동 설치는 홈페이지 www.cysinsight.com)` +",
    ])
      expect({ 조각: piece, 있음: b.includes(piece) }).toEqual({ 조각: piece, 있음: true });
    // 덧붙임 식: 실제 소스 조각을 꺼내 실행해 두 경우를 잰다(문자열 핀을 우회하는 변형 차단)
    const a = b.indexOf("(sacNote ? ");
    expect(a).toBeGreaterThan(0);
    const tail = '"")';
    const e = b.indexOf(tail, a) + tail.length;
    const expr = b.slice(a, e);
    const run = (note: string | null): unknown => (new Function("sacNote", `return ${expr};`) as (n: string | null) => unknown)(note);
    expect(run(null)).toBe(""); // 문단 없음 → 아무것도 붙지 않는다(본문 바이트 동일)
    expect(run("")).toBe("");
    expect(run("고지 문단")).toBe("\n\n고지 문단"); // 한 줄 띄우고
    expect(run(SAC_PREFLIGHT)).toBe("\n\n" + SAC_PREFLIGHT);
    // 덧붙임이 본문 인자의 맨 끝이고 다음 인자는 확인 버튼 라벨이다
    expect(b.slice(e).trimStart().startsWith(',\n    "설치",')).toBe(true);
    // 설치를 막지 않는다 — 확인(사용자 선택)이 유일한 관문이다
    expect(b.includes("if (!ok) return;")).toBe(true);
    expect(b.includes('await invoke("install_update", { force: true });')).toBe(true);
  });

  it("스마트 앱 컨트롤 조회·시도 판정 외에 업데이트 경로를 바꾸지 않는다 — install_update 호출 위치는 종전 둘(패치 설치·자동 테스트)뿐", () => {
    expect(count(code, 'invoke("install_update"')).toBe(2);
  });
});

describe("백엔드(src-tauri) 계약 — UI 가 부르는 명령이 등재돼 있고 응답 키가 UI 가 읽는 것과 같다", () => {
  const rust = read("../../src-tauri/src/main.rs");
  const prod = rust.slice(0, rust.indexOf("#[cfg(test)]\nmod tests {"));
  const regStart = prod.indexOf("tauri::generate_handler![");
  const reg = prod.slice(regStart, prod.indexOf("\n        ])", regStart));
  it("update_attempt_report·smart_app_control 이 #[tauri::command] 로 정의되고 invoke_handler 에 등재된다", () => {
    expect(regStart).toBeGreaterThan(0);
    for (const name of ["update_attempt_report", "smart_app_control"]) {
      expect({ 명령: name, 정의: prod.includes(`#[tauri::command]\nasync fn ${name}(`) }).toEqual({ 명령: name, 정의: true });
      expect({ 명령: name, 등재: reg.split("\n").some((l) => l.trim() === `${name},`) }).toEqual({ 명령: name, 등재: true });
    }
  });
  it("백엔드 보고 함수가 싣는 키(failed·pending·wait_secs·from·to·at·os·sac)를 UI 가 읽는다", () => {
    const a = prod.indexOf("fn update_attempt_report_at(");
    const body = prod.slice(a, prod.indexOf("\n}\n", a));
    const ui = read("./updatenotice.ts");
    for (const key of ["failed", "pending", "wait_secs", "from", "to", "os", "sac"]) {
      expect({ 키: key, 백엔드: body.includes(`"${key}"`) }).toEqual({ 키: key, 백엔드: true });
      expect({ 키: key, UI: ui.includes(key) }).toEqual({ 키: key, UI: true });
    }
  });
  it("백엔드 판정 보류 창(90초)과 UI 의 기본 대기(90초)가 같은 값이다", () => {
    const m = prod.match(/const UPDATE_ATTEMPT_MIN_AGE_SECS: u64 = (\d+);/);
    expect(m === null ? null : Number(m[1])).toBe(UPDATE_ATTEMPT_WAIT_DEFAULT_SECS);
  });
});

// ── 실제 함수 본문을 대역 위에서 실행 — 문자열 핀을 우회하는 변형(낱말은 그대로 두고 흐름만 깨는 것)을 막는다 ──────────────
// starvednotice.test.ts 와 같은 방법: main.ts 에서 함수 하나를 소스 그대로 꺼내 `new Function` + `with (deps)` 로 실행한다. 함수 본문의
// 자유 변수(invoke·stickyToast·rpcT·confirmModal …)는 전부 대역이 채운다. TS 전용 표기는 아는 곳만 걷는다 — 걷을 표기를 못 찾으면(소스가
// 바뀌면) 이 검체가 먼저 실패해 알린다.
describe("main.ts 실행 — pullUpdateAttemptReport 를 대역 위에서 돌린다(알림 · 재-pull 한 번 · 실패 무시)", () => {
  const flush = (): Promise<void> => new Promise((res) => globalThis.setTimeout(res, 0));

  function load<T>(name: string, strip: [string, string][], deps: Record<string, unknown>): T {
    let js = fnBodyOf(name) + "\n}";
    for (const [from, to] of strip) {
      expect({ 걷을_표기: from, 존재: js.includes(from) }).toEqual({ 걷을_표기: from, 존재: true });
      js = js.replace(from, to);
    }
    return new Function("deps", `with (deps) {\n${js}\nreturn ${name};\n}`)(deps) as T;
  }

  type Timer = { fn: () => void; ms: number };
  /** responses[i] 는 i 번째 `invoke("update_attempt_report")` 의 결과(Error 면 거부). 모자라면 마지막 값을 되풀이한다. */
  async function runPull(responses: unknown[], opts: { toastThrows?: boolean } = {}) {
    const invokes: string[] = [];
    const toasts: unknown[][] = [];
    const timers: Timer[] = [];
    let n = 0;
    const deps = {
      invoke: (cmd: string): Promise<unknown> => {
        invokes.push(cmd);
        const r = responses[Math.min(n++, responses.length - 1)];
        return r instanceof Error ? Promise.reject(r) : Promise.resolve(r);
      },
      planUpdateAttemptReport,
      UPDATE_FAILED_TOAST_ID,
      stickyToast: (...a: unknown[]): void => {
        toasts.push(a);
        if (opts.toastThrows) throw new Error("toast boom");
      },
      setTimeout: (fn: () => void, ms: number): number => {
        timers.push({ fn, ms });
        return 0;
      },
    };
    const pull = load<() => Promise<void>>(
      "pullUpdateAttemptReport",
      [
        ["async function pullUpdateAttemptReport(): Promise<void> {", "async function pullUpdateAttemptReport() {"],
        ["new Promise<void>(", "new Promise("],
      ],
      deps,
    );
    let settled = false;
    let error: unknown = null;
    pull().then(
      () => {
        settled = true;
      },
      (e) => {
        settled = true;
        error = e;
      },
    );
    await flush();
    // 걸린 타이머를 순서대로 발화시킨다(최대 5개 — 한 번만 걸려야 정상이다. 더 걸리면 그만큼 invokes 가 늘어 아래 단언이 알린다).
    let fired = 0;
    while (timers.length > fired && fired < 5) {
      timers[fired].fn();
      fired++;
      await flush();
    }
    await flush();
    return { invokes, toasts, timers, settled, error };
  }

  const FAILED = { failed: true, from: "0.14.42", to: "0.14.43", at: 1_760_000_000, os: "windows", sac: "on" };
  const PENDING = { pending: true, wait_secs: 80 };
  const WANT = updateFailedNotice({ from: "0.14.42", to: "0.14.43", os: "windows", sac: "on" });

  it("응답 null(알릴 것 없음) → pull 1회 · 알림 0 · 타이머 0", async () => {
    const r = await runPull([null]);
    expect({ 호출: r.invokes, 알림: r.toasts.length, 타이머: r.timers.length, 끝남: r.settled }).toEqual({
      호출: ["update_attempt_report"],
      알림: 0,
      타이머: 0,
      끝남: true,
    });
  });

  it("failed → 알림 1회(id 'update-not-installed' · health · 제목 · 본문) · 재-pull 0 · 타이머 0", async () => {
    const r = await runPull([FAILED]);
    expect(r.invokes).toEqual(["update_attempt_report"]);
    expect(r.toasts).toEqual([["update-not-installed", "health", WANT.title, WANT.body]]);
    expect(r.timers.length).toBe(0);
  });

  it("pending → (wait_secs+5)초 타이머 1개 → 재-pull 1회 → failed 면 그때 알림 1회", async () => {
    const r = await runPull([PENDING, FAILED]);
    expect(r.invokes).toEqual(["update_attempt_report", "update_attempt_report"]);
    expect(r.timers.map((t) => t.ms)).toEqual([85_000]);
    expect(r.toasts).toEqual([["update-not-installed", "health", WANT.title, WANT.body]]);
    expect(r.settled).toBe(true);
  });

  it("★pending → 재-pull 에서도 pending 이면 더 하지 않는다 — pull 정확히 2회 · 타이머 1개 · 알림 0 · 끝난다", async () => {
    const r = await runPull([PENDING, PENDING, PENDING, PENDING]);
    expect(r.invokes.length).toBe(2);
    expect(r.timers.length).toBe(1);
    expect(r.toasts.length).toBe(0);
    expect(r.settled).toBe(true);
  });

  it("pending → 재-pull 이 null 이면 알림 없음", async () => {
    const r = await runPull([PENDING, null]);
    expect({ 호출: r.invokes.length, 타이머: r.timers.length, 알림: r.toasts.length }).toEqual({ 호출: 2, 타이머: 1, 알림: 0 });
  });

  it("★조회 실패(첫 pull 이 거부)는 조용히 무시한다 — 던지지 않고 알림·재시도 0", async () => {
    const r = await runPull([new Error("command not found")]);
    expect({ 끝남: r.settled, 오류: r.error, 알림: r.toasts.length, 타이머: r.timers.length, 호출: r.invokes.length }).toEqual({
      끝남: true,
      오류: null,
      알림: 0,
      타이머: 0,
      호출: 1,
    });
  });

  it("재-pull 이 거부돼도 조용히 무시한다 — 던지지 않고 알림 0", async () => {
    const r = await runPull([PENDING, new Error("boom")]);
    expect({ 끝남: r.settled, 오류: r.error, 알림: r.toasts.length, 호출: r.invokes.length }).toEqual({ 끝남: true, 오류: null, 알림: 0, 호출: 2 });
  });

  it("알림을 띄우다 던져도 삼킨다(부팅 무영향) · 모르는 모양의 응답은 아무 일도 하지 않는다", async () => {
    const t = await runPull([FAILED], { toastThrows: true });
    expect({ 끝남: t.settled, 오류: t.error, 시도: t.toasts.length }).toEqual({ 끝남: true, 오류: null, 시도: 1 });
    for (const junk of [undefined, "x", 7, [], [FAILED], { ok: true }]) {
      const r = await runPull([junk]);
      expect({ junk, 알림: r.toasts.length, 타이머: r.timers.length, 호출: r.invokes.length, 끝남: r.settled }).toEqual({
        junk,
        알림: 0,
        타이머: 0,
        호출: 1,
        끝남: true,
      });
    }
  });
});

describe("main.ts 실행 — promptBinaryPatch 의 설치 전 고지를 대역 위에서 돌린다(본문 바이트 동일 · 조회 실패·초과에도 창은 열린다)", () => {
  /** 이 변경 전의 확인 창 본문(리터럴) — '문단이 없으면 종전과 바이트 동일'의 기준. */
  const BASE_BODY = (v: string): string =>
    `새 본체(앱) ${v}을 패치 방식으로 설치합니다: 저장(drain) 신호 후 다운로드·서명 검증·교체하고 앱을 ` +
    `재시작합니다. 부서·노드는 재시작 후 자동 복원됩니다(대화 기억 포함). 마지막 미저장분은 손실될 수 ` +
    `있습니다.\n\n지금 설치하시겠습니까? (수동 설치는 홈페이지 www.cysinsight.com)`;

  type Opts = {
    sac?: unknown;
    sacRejects?: boolean;
    sacHangs?: boolean;
    ok?: boolean;
    installRejects?: boolean;
    blocked?: boolean;
    noBin?: boolean;
  };
  async function runPrompt(o: Opts) {
    const calls: string[] = [];
    const modal: unknown[][] = [];
    const invokes: unknown[][] = [];
    const toasts: unknown[][] = [];
    const dismissed: string[] = [];
    const caps: number[] = [];
    const deps = {
      daemonActionBlocked: (): boolean => !!o.blocked,
      binActionable: (): { version: string } | null => (o.noBin ? null : { version: "0.14.43" }),
      updState: {},
      openUpdatePanel: (): void => {
        calls.push("openUpdatePanel");
      },
      refreshUpdateState: async (): Promise<void> => {
        calls.push("refreshUpdateState");
      },
      invoke: (cmd: string, args?: unknown): Promise<unknown> => {
        invokes.push([cmd, args]);
        if (cmd === "smart_app_control") {
          if (o.sacHangs) return new Promise<unknown>(() => {}); // 영영 안 끝나는 조회
          return o.sacRejects ? Promise.reject(new Error("boom")) : Promise.resolve(o.sac ?? null);
        }
        if (cmd === "install_update") return o.installRejects ? Promise.reject("install boom") : Promise.resolve(undefined);
        return Promise.resolve(null);
      },
      // rpcT 대역: 상한 값을 기록하고, 영영 안 끝나는 조회는 상한이 지난 것으로 보고 즉시 거부한다(진짜 rpcT 와 같은 계약).
      rpcT: (p: Promise<unknown>, ms: number): Promise<unknown> => {
        caps.push(ms);
        return o.sacHangs ? Promise.reject(new Error("rpc timeout")) : p;
      },
      T_SAC: 4242,
      sacPreflightText,
      confirmModal: (...a: unknown[]): Promise<boolean> => {
        modal.push(a);
        return Promise.resolve(o.ok !== false);
      },
      dismissToast: (id: string): void => {
        dismissed.push(id);
      },
      toast: (...a: unknown[]): void => {
        toasts.push(a);
      },
    };
    let js = fnBodyOf("promptBinaryPatch") + "\n}";
    const strip: [string, string] = ["let sacNote: string | null = null;", "let sacNote = null;"];
    expect({ 걷을_표기: strip[0], 존재: js.includes(strip[0]) }).toEqual({ 걷을_표기: strip[0], 존재: true });
    js = js.replace(strip[0], strip[1]);
    const fn = new Function("deps", `with (deps) {\n${js}\nreturn promptBinaryPatch;\n}`)(deps) as () => Promise<void>;
    await fn();
    return { calls, modal, invokes, toasts, dismissed, caps };
  }

  it("★켜짐(on): 확인 창 본문 맨 끝에 한 줄 띄우고 문단이 붙는다 — 제목·확인 라벨은 그대로 · 조회 상한은 T_SAC", async () => {
    const r = await runPrompt({ sac: "on" });
    expect(r.modal.length).toBe(1);
    expect(r.modal[0]).toEqual(["새 본체 버전 0.14.43 — 패치 설치", BASE_BODY("0.14.43") + "\n\n" + sacPreflightText("on"), "설치"]);
    expect(r.caps).toEqual([4242]);
    expect(r.invokes[0]).toEqual(["smart_app_control", undefined]);
    // 이어서 설치는 사용자가 확인했으므로 그대로 진행된다(문단이 설치를 막지 않는다)
    expect(r.invokes[1]).toEqual(["install_update", { force: true }]);
    expect(r.invokes.length).toBe(2);
  });

  it("★켜짐이 아니면(off·eval·null·모르는 값·문자열 아님) 본문은 종전과 바이트 동일", async () => {
    for (const sac of ["off", "eval", null, undefined, "", "weird", "ON", 1, true, { on: true }]) {
      const r = await runPrompt({ sac });
      expect({ sac, 본문: r.modal[0]?.[1] }).toEqual({ sac, 본문: BASE_BODY("0.14.43") });
      expect(r.modal[0]?.[0]).toBe("새 본체 버전 0.14.43 — 패치 설치");
      expect(r.modal[0]?.[2]).toBe("설치");
    }
  });

  it("★조회가 실패하거나(거부) 상한을 넘겨도(영영 안 끝남) 확인 창은 종전 본문으로 열린다 — 이 조회 때문에 창이 멈추지 않는다", async () => {
    for (const o of [{ sacRejects: true }, { sacHangs: true }] as Opts[]) {
      const r = await runPrompt(o);
      expect(r.modal.length).toBe(1);
      expect(r.modal[0]?.[1]).toBe(BASE_BODY("0.14.43"));
      expect(r.caps).toEqual([4242]); // 상한(rpcT) 아래에서 불렀다
      expect(r.invokes[r.invokes.length - 1]).toEqual(["install_update", { force: true }]); // 사용자가 확인하면 설치는 진행
    }
  });

  it("사용자가 거절하면(아니오) 설치하지 않는다 — 스마트 앱 컨트롤 조회는 설치를 막지도 부르지도 않는다", async () => {
    const r = await runPrompt({ sac: "on", ok: false });
    expect(r.modal.length).toBe(1);
    expect(r.invokes.map((i) => i[0])).toEqual(["smart_app_control"]);
  });

  it("데몬 작업 차단 중이면 아무것도 하지 않고 · 설치할 본체가 없으면 패널을 열 뿐 조회도 창도 없다(종전 거동)", async () => {
    const b = await runPrompt({ blocked: true, sac: "on" });
    expect({ 호출: b.invokes.length, 창: b.modal.length }).toEqual({ 호출: 0, 창: 0 });
    const n = await runPrompt({ noBin: true, sac: "on" });
    expect({ 호출: n.invokes.length, 창: n.modal.length, 패널: n.calls }).toEqual({
      호출: 0,
      창: 0,
      패널: ["openUpdatePanel", "refreshUpdateState"],
    });
  });

  it("설치 호출이 실패하면 종전처럼 진행 토스트를 내리고 '패치 설치 실패' 토스트를 낸다", async () => {
    const r = await runPrompt({ sac: "off", installRejects: true });
    expect(r.dismissed).toEqual(["upd-bin"]);
    expect(r.toasts).toEqual([["health", "패치 설치 실패", "install boom"]]);
  });
});
