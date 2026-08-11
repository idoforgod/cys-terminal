// PTY→xterm 출력 스트림에서 '마우스 트래킹 모드 전환' 시퀀스를 제거한다 — 근본 수리(B안).
//
// 왜 필요한가(2026-08 현장 제보 — A안의 잔여 반쪽):
// Claude Code TUI가 DECSET 1003h/1006h 로 마우스 트래킹을 켜면 xterm.js 는 ①휠을 로컬 스크롤
// 대신 보고로 앱에 보내고 ②선택(selection)을 끈다. A안(mousefilter 휠 번역 + Option드래그)은
// 스크롤과 보조키 선택을 복구했지만, **일반 드래그 선택**은 xterm 이 트래킹 모드에 들어가는 한
// 구조적으로 불가능하다. 그래서 모드 전환 자체를 UI 계층(term.write 직전)에서 걷어낸다 —
// xterm 이 트래킹 모드에 아예 진입하지 않으면 휠·드래그 선택·복사 전부 기본 동작이다.
//
// ★배치 계층 계약: 이 필터는 **UI 렌더 직전에만** 존재한다. 데몬 계층에 두면 read_text 가상
// 스크린을 소비하는 readiness 판정·거버넌스 프로브 전체가 파급권에 들어간다(부트 체인 접촉 금지).
//
// ★fail-open 계약(ABSOLUTE ANCHOR ④ — "터미널 백지화"류 결함 봉인):
//  · 홀드백(청크 경계에 걸린 미완성 후보)은 CARRY_CAP 바이트까지만 — 초과 시 원문 그대로 방류.
//  · 판정 불가·비정형 시퀀스는 전부 원문 통과. 이 필터가 바이트를 '지연'시킬 수 있는 유일한
//    경우는 (합법 DECSET 접두의 미완성 꼬리 ≤ CARRY_CAP)이고, 다음 청크가 오면 즉시 해소된다.
//  · 킬스위치: localStorage.cysAllowAppMouse = "1" → 필터 전면 우회(앱이 마우스를 갖는다 —
//    vim mouse=a 등 TUI 마우스가 필요한 사용자용. 새 pane 부터 적용). 배선은 main.ts.
//
// 트레이드오프(의도된 결정): 필터가 켜진 동안 TUI 앱은 마우스 이벤트를 받지 못한다.
// 이 제품의 surface 는 AI 에이전트 감시·읽기·복사가 1순위 용도다(오너 버그 수정 지시).

// 제거 대상 DECSET/DECRST 파라미터 — 마우스 트래킹 계열 전부.
//   9=X10 · 1000=press/release · 1002=drag · 1003=any-motion · 1005=UTF-8 좌표 ·
//   1006=SGR 좌표 · 1015=urxvt 좌표 · 1016=SGR-Pixels 좌표.
// ★1004(focus in/out)·1049(alt screen)·2004(bracketed paste) 등은 마우스가 아니다 — 보존.
const MOUSE_PARAMS = new Set([9, 1000, 1002, 1003, 1005, 1006, 1015, 1016]);

// 미완성 후보 홀드백 상한 — 합법 `ESC[?` + 파라미터 나열이 이 길이를 넘는 실전 사례는 없다.
// 넘으면 후보가 아니라고 보고 원문 방류(fail-open — 지연·소실 0).
const CARRY_CAP = 32;

const ESC = 0x1b;
const BRACKET = 0x5b; // '['
const QMARK = 0x3f; // '?'
const SEMI = 0x3b; // ';'

function isDigit(b: number): boolean {
  return b >= 0x30 && b <= 0x39;
}

// 한 청크를 필터한다 — (출력 바이트, 다음 청크에 이어붙일 미완성 꼬리) 순수 계산.
// 호출측(MouseTrackingFilter)이 carry 를 앞에 이어붙여 재호출하는 것으로 상태를 만든다.
export function filterChunk(input: Uint8Array): { out: Uint8Array; carry: Uint8Array } {
  const out: number[] = [];
  let i = 0;
  const n = input.length;
  while (i < n) {
    const b = input[i];
    if (b !== ESC) {
      out.push(b);
      i++;
      continue;
    }
    // ESC 발견 — DECSET/DECRST 후보(`ESC [ ? params h|l`)인지 전방 탐색.
    // 후보 확정 전에 청크가 끝나면 carry(캡 이내)로 보류한다.
    if (i + 1 >= n) {
      return { out: new Uint8Array(out), carry: input.slice(i) };
    }
    if (input[i + 1] !== BRACKET) {
      out.push(b); // ESC 이지만 CSI 아님 — 원문 통과, 다음 바이트부터 재스캔
      i++;
      continue;
    }
    if (i + 2 >= n) {
      return { out: new Uint8Array(out), carry: input.slice(i) };
    }
    if (input[i + 2] !== QMARK) {
      // 일반 CSI — 손대지 않는다. `ESC[` 두 바이트를 통과시키고 이어서 스캔
      // (파라미터부에 ESC 는 올 수 없으므로 이어지는 바이트는 일반 루프가 그대로 통과시킨다).
      out.push(input[i], input[i + 1]);
      i += 2;
      continue;
    }
    // `ESC [ ?` 확정 — 파라미터(digits·';')를 소비하고 최종 바이트를 본다.
    let j = i + 3;
    while (j < n && (isDigit(input[j]) || input[j] === SEMI)) j++;
    if (j >= n) {
      // 미완성 — 캡 이내면 보류, 초과면 후보 아님(fail-open 원문 방류)
      if (n - i <= CARRY_CAP) {
        return { out: new Uint8Array(out), carry: input.slice(i) };
      }
      for (let k = i; k < n; k++) out.push(input[k]);
      i = n;
      continue;
    }
    const fin = input[j];
    if (fin !== 0x68 && fin !== 0x6c) {
      // 'h'/'l' 아님(DECRQM `$p` 등) — 스캔한 원문 전체 통과
      for (let k = i; k <= j; k++) out.push(input[k]);
      i = j + 1;
      continue;
    }
    // DECSET(h)/DECRST(l) — 마우스 파라미터만 제거, 나머지는 보존해 재조립.
    const paramText = String.fromCharCode(...input.slice(i + 3, j));
    const kept = paramText
      .split(";")
      .filter((p) => p !== "" && !MOUSE_PARAMS.has(Number(p)));
    if (kept.length === paramText.split(";").filter((p) => p !== "").length) {
      // 마우스 파라미터 전무 — 원문 그대로(바이트 무변경 계약)
      for (let k = i; k <= j; k++) out.push(input[k]);
    } else if (kept.length > 0) {
      // 부분 제거 — 비마우스 파라미터(1049·2004 등)는 순서 보존 재조립
      const rebuilt = `\x1b[?${kept.join(";")}${String.fromCharCode(fin)}`;
      for (let k = 0; k < rebuilt.length; k++) out.push(rebuilt.charCodeAt(k));
    }
    // kept.length === 0 → 시퀀스 통째 제거(방출 없음)
    i = j + 1;
  }
  return { out: new Uint8Array(out), carry: new Uint8Array(0) };
}

// pane 수명 전체를 관통하는 스테이트풀 래퍼 — attach 스냅샷 재생과 라이브 스트림이 같은
// 인스턴스를 지나야 한다(스냅샷 안의 1003h 가 살아남으면 재생 순간 트래킹 모드 진입).
export class MouseTrackingFilter {
  private carry: Uint8Array = new Uint8Array(0);

  feed(chunk: Uint8Array): Uint8Array {
    let input = chunk;
    if (this.carry.length > 0) {
      input = new Uint8Array(this.carry.length + chunk.length);
      input.set(this.carry, 0);
      input.set(chunk, this.carry.length);
    }
    const { out, carry } = filterChunk(input);
    this.carry = carry;
    return out;
  }

  // 스트림 종료(surface exited) 시 잔여 carry 방류 — 앱이 시퀀스 중간에서 죽어도 바이트
  // 영구 소실 0 을 보장한다(미완성 후보는 어차피 렌더 무해한 접두다). 호출측: main.ts exited.
  flush(): Uint8Array {
    const rest = this.carry;
    this.carry = new Uint8Array(0);
    return rest;
  }
}
