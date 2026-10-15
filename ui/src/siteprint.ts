// 0.14.51 TOAST 축 — 「남긴 자리」의 **도달 가능성 지문** (순수 · DOM/Tauri 불요 · 본체에서 import 되지 않는다).
//
// 왜 필요한가(44 적대 리뷰 major1): 남김 단언이 「그 줄이 그 문장과 같다」만 보면, **같은 팔 앞에 early return·조건이 들어와도 초록**이고,
//   가드 시험이 `alertsQuiet(` 토큰만 보면 **category 비교로 만든 가드는 적색이 안 난다**. 지문은 토큰이 아니라 **효과**를 잰다:
//   그 문장에 이르는 길 위에 있는 **제어 흐름 줄**(조건·분기·반환·던짐·건너뜀)과 **opener 사슬**(그 문장을 감싸는 블록들)을 정규화해 해시한다.
//   가드의 모양(alertsQuiet · category · 삼항 · && · 래핑 블록 · 앞선 return)이 무엇이든 그 문장에 이르는 길이 바뀌면 지문이 바뀐다.
//
// 지문 = h64( opener 사슬 줄들 + 각 조상 수준의 선행 제어 흐름 줄들 + 문장 자신 )  — 줄은 trim · 줄 끝 주석 제거 · 빈 줄/주석 줄 제외.
//   · 선행 제어 흐름 줄 = 같은 수준(들여쓰기가 같거나 조상 수준)에서 문장보다 앞에 있는 줄 중 `if/else/for/while/switch/case/try/catch/return/throw/continue/break` 를 담은 줄과
//     `alertsQuiet`·`category` 를 담은 줄. 더 깊이 들여쓴 줄(다른 블록의 속)은 건너뛴다 — 그 블록이 열린 줄(opener) 이 위에서 이미 걸린다.
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

/** 함수 본문 전체의 효과 지문 — 줄 끝 주석·빈 줄·주석 줄을 걷고 trim 한 줄들의 해시(통짜 가드 시험이 토큰이 아니라 본문 전체를 본다). */
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
