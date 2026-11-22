// S1 수정 2 — 「피드백 보내기」 기능 **부재** 핀. 끝 상태를 줄 번호 핀이 아니라 **측정**으로 말한다.
//
// ★무엇을 지키는가: 오너 지시로 그 기능을 통째로 걷었다 — 사이드바 단추, 작성 창, 묶음 생성기(Rust),
//   Tauri 명령 12개, 설명서의 절, 그리고 그 기능만 보던 검체. 여기 실패는 "걷어낸 것이 되살아났다"는 뜻이다
//   (병합·되돌리기·부분 복원 — 예: 모듈은 지웠는데 `mod` 선언이나 설명서 절이 남았다).
// ★왜 줄 핀이 아닌가: "main.ts 의 몇 번째 줄에 그 import 가 없다" 같은 핀은 줄이 한 칸 밀리면 아무것도 지키지
//   않는다. 아래 값은 전부 **모집단을 걸어서 센 것**이다 — 파일 실존, import 문의 대상, 명령 등재 블록의 항목,
//   출하 소스·문서의 줄. 모집단이 비면(경로가 옮겨졌다 = 아무것도 보지 않았다) 그 자체가 적색이다(하한 단언).
// ★실패 출력 계약: 소스 파일을 실패 출력에 쏟지 않는다 — 건수와 `경로:줄 종류 «걸린 낱말»` 표본(상한 8)만 낸다.
// ★일부러 지키지 **않는** 것:
//   · `~/.cys/feedback` 을 아는 완전 초기화 인벤토리(src/factory_reset.rs 의 "feedback" 항목과 그 검체 두 줄) —
//     구 판(0.14.41~0.15.1)이 남긴 폴더를 격리해야 하므로 **남아야 한다**. 아래 판정은 그 줄을 잡지 않는다(⑦ 음성 대조).
//   · 과거 릴리스 노트(docs/RELEASE_NOTES_*) — 그때 무엇을 냈는지의 이력이다. 모집단에서 규칙으로 뺀다.
//   · 검체 파일(tests/ · *.test.ts · *_tests.rs)과 제3자 코드(cysjavis-pack/web/vendor/) — 출하되는 제품의 말이 아니다.
//   · '피드백'이라는 보통 낱말(클릭에 대한 화면 반응 · 리뷰 피드백 · 메모리 유형 `feedback`).
import { describe, it, expect } from "bun:test";
import { existsSync, readFileSync, readdirSync } from "node:fs";

const REPO_ROOT = new URL("../../", import.meta.url);
/** 저장소 상대 경로 → URL(cwd 비의존). 조각마다 인코딩한다 — 이름에 `#`·`?`·`%`·공백이 있어도 다른 파일을 가리키지 않게. */
const repoUrl = (rel: string): URL => new URL(rel.split("/").map(encodeURIComponent).join("/"), REPO_ROOT);
const read = (rel: string): string => readFileSync(repoUrl(rel), "utf8");
const SELF = "ui/src/" + (new URL(import.meta.url).pathname.split("/").pop() ?? "");

const SKIP_DIRS = ["node_modules", "target", "dist", "__pycache__", ".git"];
/** `dir`(끝에 `/`) 아래 모든 파일의 저장소 상대 경로. 심볼릭 링크는 따라가지 않는다(디렉터리도 파일도 아닌 항목). */
function walk(dir: string): string[] {
  const out: string[] = [];
  for (const e of readdirSync(repoUrl(dir), { withFileTypes: true })) {
    if (e.isDirectory()) {
      if (!SKIP_DIRS.includes(e.name)) out.push(...walk(`${dir}${e.name}/`));
    } else if (e.isFile()) out.push(`${dir}${e.name}`);
  }
  return out.sort();
}
const baseName = (p: string): string => p.slice(p.lastIndexOf("/") + 1);
/** 검체 경로(제품의 말이 아니다) — cysjavis-pack/bin/tests/test_content_pins_parity.py 의 deferral_class 'out' 과 같은 규칙. */
const isTestPath = (p: string): boolean =>
  /(^|\/)(tests?|__tests__|fixtures)\//.test(p) || /(\.test\.ts|\.spec\.ts|_tests?\.rs)$/.test(p);
/** 글로 읽을 파일 — 확장자 목록 + 확장자 없는 스크립트(cysjavis-pack/bin/cys-dept). 그림·아이콘은 세지 않는다. */
const isText = (p: string): boolean =>
  /\.(ts|css|html|rs|json|toml|plist|nsh|md|py|sh|js|cjs|cmd|txt|template|example|yml|yaml)$/.test(p) || !baseName(p).includes(".");

/** 그 기능을 **이름으로 부르는** 낱말들(한국어·영어·식별자). 보통 낱말 '피드백/feedback' 은 여기에 없다. */
const FEATURE_NAMES: [string, RegExp][] = [
  ["이름", /피드백\s*보내기/],
  ["단추·창", /피드백\s*(단추|버튼|창|슬롯)/],
  ["DOM", /btn-feedback|wsbar-feedback|feedback-(modal|overlay)/],
  ["작성 창 CSS", /(^|[^0-9A-Za-z_-])fb-[a-z]/],
  ["UI 식별자", /(mount|open)Feedback|feedbackOverlay|feedbackDeps|FeedbackFacts|FEEDBACK_(TO|OVERLAY|MAX|MIN|ALLOWED|CHUNK)/],
  ["Tauri 명령", /\bfeedback_(draft_new|attach_(path|begin|chunk|commit)|detach|discard|diag_preview|submit|open_mail(_app)?|reveal)\b/],
  ["Rust 모듈", /\bmod\s+feedback\b|\bfeedback::/],
  ["지운 파일 이름", /\bfeedback(modal|flow|wiring)?(\.test)?\.(ts|rs)\b/],
  ["이름(영)", /send(ing)?[\s_-]+feedback|feedback[\s_-]+(button|form|modal|window|dialog|overlay|slot)/i],
];
/** 그 낱말 자체(보통 낱말 포함) — 그 낱말이 **하나도 없어야 하는** 자리(index.html · src-tauri)에만 쓴다. */
const ANY_MENTION: [string, RegExp][] = [["낱말", /feedback|피드백/i]];
/** 위 판정들이 **모두** 품는 공통 조각 — scan 의 빠른 길(모집단 27MB 를 판정마다 훑지 않게). 판정이 이 조각 밖으로 나가면 ⑦ 이 적색이다. */
const COMMON = /feedback|피드백|fb-/i;

/** 줄마다 판정해 `경로:줄 종류 «걸린 낱말»` 을 모은다(줄 내용은 싣지 않는다). */
function scan(paths: string[], pats: [string, RegExp][]): string[] {
  const hits: string[] = [];
  for (const p of paths) {
    const text = read(p);
    if (!COMMON.test(text)) continue; // 빠른 길 — 공통 조각이 없는 파일은 줄을 세지 않는다
    const lines = text.split(/\r?\n/);
    for (let i = 0; i < lines.length; i++) {
      for (const [kind, re] of pats) {
        const m = re.exec(lines[i] as string);
        if (m) {
          hits.push(`${p}:${i + 1} ${kind} «${m[0].trim().slice(0, 40)}»`);
          break;
        }
      }
    }
  }
  return hits;
}
/** 실패 출력용 요약 — 건수·파일 수·표본 8줄. */
const brief = (hits: string[]) => ({
  건수: hits.length,
  파일수: new Set(hits.map((h) => h.replace(/(:\d+ | ← ).*$/, ""))).size,
  표본: hits.slice(0, 8),
});
const NONE = { 건수: 0, 파일수: 0, 표본: [] as string[] };

describe("「피드백 보내기」 기능은 제품에 없다 — 끝 상태 측정", () => {
  it("① 기능 파일이 트리에 없다 — UI 모듈 셋·Rust 묶음 생성기, 그리고 그 이름으로 시작하는 어떤 파일도", () => {
    const ui = walk("ui/src/");
    const rs = walk("src-tauri/src/");
    expect({ "ui/src 파일": ui.length > 60, "src-tauri/src 파일": rs.length > 2 }).toEqual({ "ui/src 파일": true, "src-tauri/src 파일": true });
    const named = ["ui/src/feedback.ts", "ui/src/feedbackmodal.ts", "ui/src/feedbackflow.ts", "src-tauri/src/feedback.rs"];
    expect({
      "남은 파일": named.filter((p) => existsSync(repoUrl(p))),
      // 목록 밖의 변형(다른 이름의 조각·그 검체)도 센다 — 이름을 적어 두지 않고 디렉터리를 걸어서.
      "이름이 feedback 으로 시작": [...ui, ...rs].filter((p) => /^feedback/i.test(baseName(p))),
    }).toEqual({ "남은 파일": [], "이름이 feedback 으로 시작": [] });
  });

  it("② ui/index.html 에 단추도, 단추가 달리던 자리(슬롯)도 없다 — 그 낱말이 한 줄도 없다", () => {
    expect(read("ui/index.html").includes('<div id="wsbar-foot">')).toBe(true); // 슬롯이 있던 꼬리 칸은 그대로 — 옳은 파일을 보고 있다
    expect(brief(scan(["ui/index.html"], ANY_MENTION))).toEqual(NONE);
  });

  it("③ main.ts 는 ./feedback* 에서 아무것도 들여오지 않는다 — 다른 UI 모듈·검체도", () => {
    /** 정적 import·재수출(`from "x"` · `import "x"`)과 동적 `import("x")` 의 대상. */
    const specs = (src: string): string[] => {
      const out: string[] = [];
      const re = /(?:\bfrom\s*|\bimport\s*\(\s*|\bimport\s+)["']([^"'\n]+)["']/g;
      let m: RegExpExecArray | null;
      while ((m = re.exec(src)) !== null) out.push(m[1] as string);
      return out;
    };
    const isFeature = (s: string): boolean => /^\.\/feedback/i.test(s);
    const mainSpecs = specs(read("ui/src/main.ts"));
    expect(mainSpecs.length).toBeGreaterThan(40); // 수집기가 import 문을 실제로 본다
    const others: string[] = [];
    for (const p of walk("ui/src/")) {
      if (!p.endsWith(".ts") || p === "ui/src/main.ts" || p === SELF) continue;
      for (const s of specs(read(p))) if (isFeature(s)) others.push(`${p} ← ${s}`);
    }
    expect({ "main.ts": mainSpecs.filter(isFeature), "다른 모듈": brief(others) }).toEqual({ "main.ts": [], "다른 모듈": NONE });
  });

  it("④ src-tauri: `mod feedback` 선언·명령 등재·include_str! 가 없다 — 그 낱말이 한 줄도 없다", () => {
    const main = read("src-tauri/src/main.rs");
    const prod = main.slice(0, main.indexOf("#[cfg(test)]\nmod tests {"));
    const i = prod.indexOf("tauri::generate_handler![");
    const end = prod.indexOf("\n        ])", i);
    const entries = prod
      .slice(i, end)
      .split("\n")
      .slice(1)
      .map((l) => l.trim())
      .filter((l) => l.length > 0 && !l.startsWith("//"));
    expect({ "등재 블록": i >= 0 && end > i, "항목 60 초과": entries.length > 60 }).toEqual({ "등재 블록": true, "항목 60 초과": true });
    const tauri = walk("src-tauri/").filter((p) => isText(p) && !p.startsWith("src-tauri/gen/"));
    const sources = tauri.filter((p) => p.endsWith(".rs"));
    expect(sources.length).toBeGreaterThan(2);
    const count = (re: RegExp): number => sources.reduce((n, p) => n + read(p).split(/\r?\n/).filter((l) => re.test(l)).length, 0);
    expect({
      "mod 선언": count(/^\s*(pub(\([a-z]+\))?\s+)?mod\s+feedback\b/),
      "등재된 명령": entries.filter((l) => /feedback/i.test(l)),
      // 검체 빌드 전체가 여기에 걸려 있다 — 없는 파일을 include_str! 하면 한 검체가 아니라 `cys-app` 검체 빌드가 통째로 죽는다.
      'include_str!("feedback…")': count(/include_str!\(\s*"[^"]*feedback/),
      "그 낱말이 있는 줄": brief(scan(tauri, ANY_MENTION)),
    }).toEqual({ "mod 선언": 0, "등재된 명령": [], 'include_str!("feedback…")': 0, "그 낱말이 있는 줄": NONE });
  });

  it("⑤ 출하 소스의 어느 줄도 그 기능을 이름으로 부르지 않는다 — UI·앱·데몬·팩", () => {
    const pop = [
      "ui/index.html",
      ...walk("ui/src/"),
      ...walk("src-tauri/").filter((p) => !p.startsWith("src-tauri/gen/")),
      ...walk("src/"),
      ...walk("cysjavis-pack/").filter((p) => !p.startsWith("cysjavis-pack/web/vendor/")),
    ].filter((p) => isText(p) && !isTestPath(p));
    const n = (prefix: string): number => pop.filter((p) => p.startsWith(prefix)).length;
    // 하한은 계약이 아니다(공허 방지) — 나무가 옮겨져 아무것도 걷지 못하면 여기서 적색이 된다.
    expect({ ui: n("ui/") > 40, "src-tauri": n("src-tauri/") > 5, src: n("src/") > 80, pack: n("cysjavis-pack/") > 300 }).toEqual({
      ui: true,
      "src-tauri": true,
      src: true,
      pack: true,
    });
    expect(brief(scan(pop, FEATURE_NAMES))).toEqual(NONE);
  });

  it("⑥ 사용자 문서의 어느 줄도 그 기능을 안내하지 않는다 — 설명서·README·docs/(과거 릴리스 노트 제외)", () => {
    const rootDocs = readdirSync(REPO_ROOT, { withFileTypes: true })
      .filter((e) => e.isFile() && e.name.endsWith(".md"))
      .map((e) => e.name)
      .sort();
    const docs = walk("docs/").filter((p) => isText(p) && !baseName(p).startsWith("RELEASE_NOTES_"));
    expect({ 설명서: rootDocs.includes("USER-MANUAL.md"), "루트 문서": rootDocs.length > 3, "docs/": docs.length > 40 }).toEqual({
      설명서: true,
      "루트 문서": true,
      "docs/": true,
    });
    expect(brief(scan([...rootDocs, ...docs], FEATURE_NAMES))).toEqual(NONE);
  });

  it("⑦ 판정 자기검증 — 이름은 잡고(양성), 남아야 하는 줄·보통 낱말은 잡지 않는다(음성)", () => {
    const kind = (line: string): string | null => {
      for (const [k, re] of FEATURE_NAMES) if (re.test(line)) return k;
      return null;
    };
    const positive: [string, string][] = [
      ["### 4.11 피드백 보내기 (v0.14.41)", "이름"],
      ["사이드바 바닥에 피드백 단추를 넣었습니다.", "단추·창"],
      ["Use Send Feedback to tell us.", "이름(영)"],
      ['<div id="wsbar-feedback-slot"></div>', "DOM"],
      [".fb-head { display: flex; }", "작성 창 CSS"],
      ["mountFeedbackButton(slot, open);", "UI 식별자"],
      ["            feedback::feedback_submit,", "Tauri 명령"],
      ["mod feedback;", "Rust 모듈"],
      ['let s = include_str!("feedback.rs");', "지운 파일 이름"],
    ];
    for (const [line, k] of positive) expect({ line, kind: kind(line), 빠른길: COMMON.test(line) }).toEqual({ line, kind: k, 빠른길: true });
    expect(positive.map(([, k]) => k).sort()).toEqual(FEATURE_NAMES.map(([k]) => k).sort()); // 판정마다 양성 검체가 하나씩 있다
    const negative = [
      '    "feedback",', // src/factory_reset.rs — 완전 초기화 인벤토리 항목(남아야 한다)
      '        mk(&r.cys_base.join("feedback/fb-20260923-120000-ab12"));', // 그 검체의 남은 묶음 씨앗
      '            ".cys/feedback",', // 그 검체의 격리 기대값
      "  // 클릭 즉시 placeholder 탭 push+render — launch await 동안 시각 피드백 제공.", // 화면 반응이라는 보통 낱말
      "javis_memory.py add --type feedback --name some-slug", // 메모리 유형
      "- [자율 진행 권한 템플릿](feedback_autonomous-pilot-mandate.md)", // 메모리 파일 이름
      "step 5fb-a: fallback boot", // 다른 약어(fallback)
    ];
    for (const line of negative) expect({ line, kind: kind(line) }).toEqual({ line, kind: null });
    // scan 이 실제 파일에서 줄 번호를 센다 — 이 파일 자신의 머리말 첫 줄에 그 이름이 있다(공허하지 않다는 증거).
    expect(scan([SELF], FEATURE_NAMES)[0]).toBe(`${SELF}:1 이름 «피드백 보내기»`);
  });
});
