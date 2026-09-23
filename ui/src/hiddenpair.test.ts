// `[hidden]` 짝 규칙 정적 핀 (U9 · 0.14.41) — style.css·index.html·main.ts 를 데이터로 읽어 단언한다
// (wswiring.test.ts 관례: 런타임 코드 0줄 · 본체에서 import 되지 않는다 · DOM/Tauri 불요).
//
// ★왜 필요한가 (U9 반박 검증 확정 · 2026-09-23):
// 작성자 CSS 에 `display` 를 명시하면(예: `.badge { display:inline-block }`) HTML `hidden` 속성의
// UA 규칙(`[hidden]{display:none}` — 명시도 열세)을 이긴다. 그러면 코드가 `el.hidden = true` 를 해도
// **화면에는 그대로 남는다**. `.badge` 는 첫 커밋부터 짝이 없어서
//   · Update 옆 빨간 `!` 가 확인 전부터 보이고(index.html 초기 글자),
//   · 팩 설치를 마쳐도 `↻` 가 남고(pack-updated 의 badge.hidden = true 가 무효),
//   · Control Center 옆 빨간 `0` 이 승인 대기 0건에도 늘 보였다.
// 누르면 다시 확인해서 "최신"이 뜨므로 오너 증상("숫자가 떠 있는데 눌러 보면 최신")이 그대로 재현됐다.
// WebKit(맥)·Blink(윈도우 WebView2) 두 엔진에서 같은 계산 스타일(display=inline-block)을 실측했다.
//
// 이 저장소는 같은 함정을 이미 두 번 겪고 style.css 주석에 "짝 규칙(Directive)"으로 적어 두었다.
// 사람이 눈으로 지키던 그 규칙을 기계가 센다: **display 를 명시한 셀렉터가 hidden 을 쓰는 요소에
// 걸리면, 그 요소를 이기는 `[hidden]{display:none}` 짝이 있어야 한다.**
import { describe, it, expect } from "bun:test";
import { readFileSync } from "node:fs";

const CSS = readFileSync(new URL("./style.css", import.meta.url), "utf-8");
const HTML = readFileSync(new URL("../index.html", import.meta.url), "utf-8");
const MAIN = readFileSync(new URL("./main.ts", import.meta.url), "utf-8");

type Rule = { sel: string; body: string; idx: number };
type El = { tag: string; id: string | null; classes: string[]; where: string };
type Spec = [number, number, number];

/** 주석을 걷고 가장 안쪽 `셀렉터 { 선언 }` 블록만 모은다(@media 안의 규칙 포함 · @규칙 머리는 제외). */
export function cssRules(css: string): Rule[] {
  const nc = css.replace(/\/\*[\s\S]*?\*\//g, "");
  const out: Rule[] = [];
  const re = /([^{}]+)\{([^{}]*)\}/g;
  let m: RegExpExecArray | null;
  let idx = 0;
  while ((m = re.exec(nc))) {
    const sels = m[1].trim();
    idx++;
    if (sels.startsWith("@")) continue;
    for (const s of sels.split(",")) if (s.trim()) out.push({ sel: s.trim(), body: m[2], idx });
  }
  return out;
}

/** 선언 블록의 display 값(없으면 null). `-webkit-display` 같은 접두 속성은 제외. */
function displayOf(body: string): string | null {
  const m = /(^|[^-\w])display\s*:\s*([^;]+)/.exec(body);
  return m ? m[2].trim() : null;
}

/** 셀렉터의 주체(마지막 복합 셀렉터) — 조상 문맥은 명시도로만 반영한다. */
function subjectOf(sel: string): string {
  const parts = sel.trim().split(/[\s>+~]+/).filter(Boolean);
  return parts.length ? parts[parts.length - 1] : "";
}

function specificity(sel: string): Spec {
  const s = sel.replace(/::[\w-]+/g, " ");
  const ids = (s.match(/#[\w-]+/g) || []).length;
  const cls =
    (s.match(/\.[\w-]+/g) || []).length +
    (s.match(/\[[^\]]*\]/g) || []).length +
    (s.match(/:[\w-]+/g) || []).length;
  const types = s
    .split(/[\s>+~]+/)
    .filter(Boolean)
    .filter((c) => /^[a-zA-Z]/.test(c)).length;
  return [ids, cls, types];
}

function cmpSpec(a: Spec, b: Spec): number {
  for (let i = 0; i < 3; i++) if (a[i] !== b[i]) return a[i] - b[i];
  return 0;
}

/** 주체 복합 셀렉터가 요소에 걸리는가. `[hidden]` 은 떼고 본다(짝 판정용). 모르는 태그는 보수적으로 매치. */
function subjectMatches(subject: string, el: El): boolean {
  const s = subject.replace(/\[hidden\]/g, "");
  if (s === "" && /\[hidden\]/.test(subject)) return true; // 범용 [hidden]{…} = 모든 요소
  const ids = (s.match(/#[\w-]+/g) || []).map((x) => x.slice(1));
  const cls = (s.match(/\.[\w-]+/g) || []).map((x) => x.slice(1));
  const tagM = /^([a-zA-Z][\w-]*)/.exec(s);
  if (!ids.length && !cls.length && !tagM) return false; // '*'·순수 의사 클래스 — 이 핀의 대상 아님
  if (ids.some((i) => i !== el.id)) return false;
  if (cls.some((c) => !el.classes.includes(c))) return false;
  if (tagM && el.tag && tagM[1].toLowerCase() !== el.tag.toLowerCase()) return false;
  return true;
}

/** index.html 에서 `hidden` 속성을 가진 요소. 따옴표 값 안의 'hidden' 단어는 무시한다. */
export function htmlHiddenElements(html: string): El[] {
  const out: El[] = [];
  const re = /<([a-zA-Z][\w-]*)(\s[^>]*)?>/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(html))) {
    const attrs = m[2] || "";
    const bare = attrs.replace(/"[^"]*"/g, '""').replace(/'[^']*'/g, "''");
    if (!/(^|\s)hidden(?=[\s=>/]|$)/.test(bare)) continue;
    const id = /\sid="([^"]+)"/.exec(attrs)?.[1] ?? null;
    const cls = /\sclass="([^"]+)"/.exec(attrs)?.[1] ?? "";
    out.push({ tag: m[1], id, classes: cls.split(/\s+/).filter(Boolean), where: "index.html" });
  }
  return out;
}

/** index.html 의 id → 요소(태그·클래스) 조회 — main.ts 가 동적으로 숨기는 요소의 정적 모양을 얻는다. */
function htmlElementById(html: string, id: string): El | null {
  const re = new RegExp(`<([a-zA-Z][\\w-]*)\\s[^>]*\\bid="${id}"[^>]*>`);
  const m = re.exec(html);
  if (!m) return null;
  const cls = /\sclass="([^"]+)"/.exec(m[0])?.[1] ?? "";
  return { tag: m[1], id, classes: cls.split(/\s+/).filter(Boolean), where: "index.html" };
}

/**
 * main.ts 가 `.hidden` 을 쓰는 요소(근사 · 누락은 사정권 축소일 뿐 오탐은 아니다):
 *   ① getElementById("X")!.hidden / ?.hidden
 *   ② const v = document.getElementById("X") … 30줄 안의 v.hidden =
 *   ③ v = document.createElement("t") … 12줄 안의 v.id / v.className + v.hidden
 */
export function mainHiddenElements(main: string, html: string): El[] {
  const out: El[] = [];
  const add = (el: El) => {
    if (!out.some((o) => o.id === el.id && o.id !== null && o.where === el.where)) out.push(el);
  };
  const byId = (id: string): El => {
    const h = htmlElementById(html, id);
    return h ? { ...h, where: "main.ts" } : { tag: "", id, classes: [], where: "main.ts" };
  };
  for (const m of main.matchAll(/getElementById\("([\w-]+)"\)[!?]?\.hidden\b/g)) add(byId(m[1]));
  const lines = main.split("\n");
  for (let i = 0; i < lines.length; i++) {
    const g = /(?:const|let|var)\s+(\w+)\s*=\s*document\.getElementById\("([\w-]+)"\)/.exec(lines[i]);
    if (g) {
      const win = lines.slice(i, i + 30).join("\n");
      if (new RegExp(`\\b${g[1]}!?\\.hidden\\s*=`).test(win)) add(byId(g[2]));
    }
    const c = /(\w+)\s*=\s*document\.createElement\("([\w-]+)"\)/.exec(lines[i]);
    if (c) {
      const win = lines.slice(i, i + 12).join("\n");
      if (!new RegExp(`\\b${c[1]}!?\\.hidden\\s*=`).test(win)) continue;
      const id = new RegExp(`\\b${c[1]}\\.id\\s*=\\s*"([\\w-]+)"`).exec(win)?.[1] ?? null;
      const cls = new RegExp(`\\b${c[1]}\\.className\\s*=\\s*"([^"]+)"`).exec(win)?.[1] ?? "";
      out.push({ tag: c[2], id, classes: cls.split(/\s+/).filter(Boolean), where: `main.ts:${i + 1}` });
    }
  }
  return out;
}

export type Violation = { el: string; display: string; value: string };

/** 짝 없는 (요소, display 명시 셀렉터) 쌍. 짝 = 같은 요소에 걸리는 `…[hidden]{display:none}` 으로서
 *  명시도가 더 높거나(같으면 뒤에 선언) `!important`. */
export function hiddenPairViolations(css: string, els: El[]): Violation[] {
  const rules = cssRules(css);
  const shows = rules.filter((r) => {
    const d = displayOf(r.body);
    return d !== null && !/^none\b/.test(d) && !/\[hidden\]/.test(subjectOf(r.sel));
  });
  const hides = rules.filter((r) => {
    const d = displayOf(r.body);
    return d !== null && /^none\b/.test(d) && /\[hidden\]/.test(subjectOf(r.sel));
  });
  const out: Violation[] = [];
  for (const el of els) {
    for (const s of shows) {
      if (!subjectMatches(subjectOf(s.sel), el)) continue;
      const paired = hides.some((h) => {
        if (!subjectMatches(subjectOf(h.sel), el)) return false;
        if (/!important/.test(h.body)) return true;
        const c = cmpSpec(specificity(h.sel), specificity(s.sel));
        return c > 0 || (c === 0 && h.idx > s.idx);
      });
      if (!paired) {
        const v = {
          el: el.id ? `#${el.id}` : `${el.tag}.${el.classes.join(".")} (${el.where})`,
          display: s.sel,
          value: displayOf(s.body) ?? "",
        };
        // 같은 요소가 index.html(정적)과 main.ts(동적) 양쪽에서 잡히면 한 번만 센다.
        if (!out.some((o) => o.el === v.el && o.display === v.display)) out.push(v);
      }
    }
  }
  return out;
}

describe("[hidden] 짝 규칙 — 판정기 자체 검증(공허한 핀 방지)", () => {
  const el: El = { tag: "span", id: "b1", classes: ["badge"], where: "t" };
  it("display 명시 + 짝 없음 → 위반", () => {
    expect(hiddenPairViolations(".badge { display: inline-block; }", [el]).length).toBe(1);
  });
  it("짝 있음(.badge[hidden]) → 통과", () => {
    const css = ".badge { display: inline-block; }\n.badge[hidden] { display: none; }";
    expect(hiddenPairViolations(css, [el])).toEqual([]);
  });
  it("짝의 명시도가 조상 문맥 셀렉터보다 낮으면 위반(body.x #p 대 #p[hidden])", () => {
    const p: El = { tag: "aside", id: "p", classes: [], where: "t" };
    const weak = "body.x #p { display: flex; }\n#p[hidden] { display: none; }";
    expect(hiddenPairViolations(weak, [p]).length).toBe(1);
    const strong = weak + "\nbody.x #p[hidden] { display: none; }";
    expect(hiddenPairViolations(strong, [p])).toEqual([]);
  });
  it("display:none 규칙·무관한 요소는 위반이 아니다", () => {
    expect(hiddenPairViolations(".badge { display: none; }", [el])).toEqual([]);
    expect(hiddenPairViolations(".other { display: flex; }", [el])).toEqual([]);
  });
  it("@media 안의 display 도 센다", () => {
    const css = "@media (max-width: 600px) {\n  .badge { display: block; }\n}";
    expect(hiddenPairViolations(css, [el]).length).toBe(1);
  });
  it("속성 값 안의 'hidden' 단어는 hidden 속성이 아니다", () => {
    const h = `<button title="hidden files" id="x">a</button><span id="y" class="badge" hidden>0</span>`;
    expect(htmlHiddenElements(h).map((e) => e.id)).toEqual(["y"]);
  });
});

describe("[hidden] 짝 규칙 — 실제 style.css · index.html · main.ts", () => {
  const els = [...htmlHiddenElements(HTML), ...mainHiddenElements(MAIN, HTML)];

  it("사정권 실존(공허 방지): 숨김 요소를 충분히 찾았고 배지 3종이 그 안에 있다", () => {
    expect(els.length).toBeGreaterThanOrEqual(15);
    const ids = new Set(els.map((e) => e.id));
    for (const id of ["update-badge", "cc-pending-badge", "cc-feed-tabbadge"]) expect(ids.has(id)).toBe(true);
  });

  it("★display 를 명시한 셀렉터가 hidden 요소에 걸리면 [hidden]{display:none} 짝이 있다", () => {
    const v = hiddenPairViolations(CSS, els);
    // 위반 목록을 그대로 보여 준다 — 빨간불일 때 어느 요소·어느 셀렉터인지 바로 읽히게.
    expect(v).toEqual([]);
  });

  it(".badge[hidden] 짝이 실제로 있다(Control Center 빨간 0 · Update ! 의 근본 원인)", () => {
    const pair = cssRules(CSS).find((r) => r.sel === ".badge[hidden]");
    expect(pair).toBeDefined();
    expect(displayOf(pair!.body)).toBe("none");
  });

  it("Update 배지의 초기 마크업은 hidden + 중립 — 확인 전에 '업데이트 있음'(!·↻·숫자)을 말하지 않는다", () => {
    const m = /<span id="update-badge"([^>]*)>([^<]*)<\/span>/.exec(HTML);
    expect(m).not.toBeNull();
    const attrs = m![1];
    const text = m![2].trim();
    expect(/(^|\s)hidden(\s|$)/.test(attrs)).toBe(true);
    expect(["!", "↻"].includes(text)).toBe(false);
    expect(/\d/.test(text)).toBe(false);
    expect(/class="[^"]*\bok\b/.test(attrs)).toBe(true); // 중립색
  });
});
