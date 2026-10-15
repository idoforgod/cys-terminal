// 0.14.51 TOAST 축 — 「남긴 자리」의 **필터된 소스 줄 표류 탐지** (순수 · DOM/Tauri 불요 · 본체에서 import 되지 않는다).
//
// 무엇을 하는가: 그 문장 줄을 둘러싼 **소스 줄 몇 개**를 정규화해 해시한다 — 문장 줄 자신 · 그 문장을 감싸는 블록을 연 줄(opener 사슬) · 같은 들여쓰기(또는 조상) 수준에서 앞서는 줄 중
//   `if/else/for/while/switch/case/try/catch/return/throw/continue/break` · `alertsQuiet` · `category` 를 담은 줄. 줄은 trim · 줄 끝 주석 제거 · 빈 줄/주석 줄 제외.
//   남긴 문장 앞에 early return · category 비교 가드 · 삼항 · && · 래핑 블록 · 같은 줄 접두 가드가 **이 줄들 중 하나로** 들어오면 지문이 바뀐다 — 토큰 하나(alertsQuiet)만 보던 옛 단언이 놓친 모양들이다.
//
// ⛔무엇을 하지 않는가(codex R1 minor 1 · 2026-10-15): **도달 가능성을 증명하지 않는다.** 이 알고리즘은 문장보다 더 깊이 들여쓴 앞선 줄을 건너뛴다(그 블록을 연 opener 가 위에서 걸린다는 가정) —
//   그래서 앞선 다른 블록 *속*에 끼워 넣은 `return;`(예: 안전모드 토스트 직후)이 뒤 자리를 실행 불능으로 만들어도 뒤 자리의 지문은 **그대로**다.
//   toastaxis.test.ts 의 「알려진 한계」 시험이 이 사각을 못박는다. 실행 흐름은 이벤트·싱크 **실행** 시험이 지킨다.
//
// 지문 = h64( opener 사슬 줄들 + 각 조상 수준의 선행 줄들 + 문장 자신 ).
//   · 선행 줄 = 같은 수준(들여쓰기가 같거나 조상 수준)에서 문장보다 앞에 있는 줄 중 위 키워드를 담은 줄. 더 깊이 들여쓴 줄(다른 블록의 속)은 건너뛴다.
//   · 의도된 변경으로 지문이 바뀌면 시험 실패 메시지가 새 값을 찍는다 — 핀 해시를 다시 계산해 넣는다(자리 표는 toastaxis.test.ts).

/** 64비트 FNV-1a(16 hex) — 변조 방어가 아니라 **변경 감지**용이다(암호 해시 불요 · 앰비언트 타입에 node:crypto 가 없다). */
export function h64(text: string): string {
  let h = 0xcbf29ce484222325n;
  const P = 0x100000001b3n;
  const M = 0xffffffffffffffffn;
  for (const b of new TextEncoder().encode(text)) {
    h ^= BigInt(b);
    h = (h * P) & M;
  }
  return h.toString(16).padStart(16, "0");
}

/** 줄 끝 `//` 주석을 따옴표 밖에서만 걷는다(문자열·템플릿 안의 `//` 는 지키는 근사). */
export function stripLineComment(l: string): string {
  let q: string | null = null;
  for (let i = 0; i < l.length; i++) {
    const c = l[i];
    if (q) {
      if (c === "\\") i++;
      else if (c === q) q = null;
    } else if (c === '"' || c === "'" || c === "`") q = c;
    else if (c === "/" && l[i + 1] === "/" && (i === 0 || /\s/.test(l[i - 1]))) return l.slice(0, i);
  }
  return l;
}

export function normLine(l: string): string {
  return stripLineComment(l).trim();
}

const indentOf = (l: string): number => l.length - l.trimStart().length;
const isBlank = (l: string): boolean => normLine(l) === "";
const CONTROL = /\b(if|else|for|while|switch|case|try|catch|return|throw|continue|break)\b|\balertsQuiet\b|\bcategory\b/;

/** `lines[idx]` 에 이르는 길의 지문 재료(정규화된 줄 목록). */
export function pathLines(lines: string[], idx: number): string[] {
  const out: string[] = [];
  let level = indentOf(lines[idx]);
  for (let j = idx - 1; j >= 0 && level > 0; j--) {
    const raw = lines[j];
    if (isBlank(raw)) continue;
    const ind = indentOf(raw);
    if (ind > level) continue; // 더 깊은 블록의 속 — 그 블록의 opener 가 위에서 걸린다
    const n = normLine(raw);
    if (ind < level) {
      out.push(n); // opener(조상 블록을 연 줄)
      level = ind;
    } else if (CONTROL.test(n)) out.push(n); // 같은 수준의 선행 제어 흐름 줄
  }
  out.reverse();
  out.push(normLine(lines[idx]));
  return out;
}

export function fingerprint(lines: string[], idx: number): string {
  return h64(pathLines(lines, idx).join("\n"));
}

/** 함수 본문 전체의 정규화 줄 해시(본문 표류 탐지) — 줄 끝 주석·빈 줄·주석 줄을 걷고 trim 한 줄들의 해시다. 통짜 가드 시험이 토큰이 아니라 본문 줄 전체를 본다(실행 흐름 증명은 아니다). */
export function bodyPrint(src: string): string {
  const norm = src
    .split("\n")
    .map(normLine)
    .filter((l) => l !== "" && !l.startsWith("/*") && !l.startsWith("*"))
    .join("\n");
  return h64(norm);
}

/** `anchor`(정규화된 문장 한 줄)와 같은 줄을 nth(0 시작)번째로 찾는다. 없으면 -1. */
export function findAnchor(lines: string[], anchor: string, nth: number): number {
  let k = 0;
  for (let i = 0; i < lines.length; i++) {
    if (normLine(lines[i]) === anchor) {
      if (k === nth) return i;
      k++;
    }
  }
  return -1;
}
