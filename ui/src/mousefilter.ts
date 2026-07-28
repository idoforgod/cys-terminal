// xterm.js가 PTY로 보내려는 '마우스 보고' 시퀀스를 분류한다 — 순수 함수.
//
// 왜 필요한가(현장 결함 1호 · Parallels Windows 실기 제보):
// Claude Code TUI가 마우스 트래킹(1003h/1006h)을 켜면 xterm.js는 마우스 이벤트마다
// `ESC[<b;x;y M/m` 같은 보고를 onData로 발화해 PTY로 보낸다. macOS는 앱이 이를 정상 소비하지만,
// Windows는 ConPTY 입력 계층이 시퀀스를 깨뜨려 선두 ESC가 소실된 `[555;98;34M...` 문자열이
// Claude Code 입력창에 리터럴로 무한 타이핑된다. ∴ Windows에서는 보고를 PTY로 보내지 않는다.
//
// 이 모듈은 '판단'만 한다(플랫폼 판별·스크롤 실행은 호출측 main.ts). 순수 함수라 인코딩 변형·
// 배칭·오탐 경계를 결정론으로 회귀 테스트할 수 있다(mousefilter.test.ts).

// pass = 마우스 보고가 아니다 → 호출측은 바이트 하나 건드리지 말고 기존 경로로 보낸다.
// drop = 마우스 보고다 → 무음 폐기.
// wheel = 휠 보고다 → PTY 전송 대신 로컬 스크롤로 번역.
//   dir 부호는 xterm의 term.scrollLines() 규약과 같다: -1 = 위로(휠업), +1 = 아래로(휠다운).
//   count = 청크에 담긴 휠 노치의 순증감 절댓값(고빈도 배칭 시 여러 개가 한 청크로 온다).
export type MouseVerdict =
  | { kind: "pass" }
  | { kind: "drop" }
  | { kind: "wheel"; dir: 1 | -1; count: number };

// 마우스 보고 인코딩 4종. 전부 CSI(ESC[)로 시작하며 sticky(y) 플래그로 '주어진 위치에서만' 매칭한다.
// SGR(1006)과 SGR-Pixels(1016)는 좌표 단위만 다르고 문법이 동일하므로 같은 정규식이 덮는다
// (픽셀 좌표는 555 같은 대형 십진수 — 자릿수 제한을 두지 않는 이유).
const RE_SGR = /\x1b\[<(\d+);(\d+);(\d+)([Mm])/y; // ESC[<b;x;yM  /  ESC[<b;x;ym (릴리스)
const RE_URXVT = /\x1b\[(\d+);(\d+);(\d+)M/y; //     ESC[Cb;Cx;CyM (1015)
const RE_X10 = /\x1b\[M[\s\S]{3}/y; //               ESC[M + 원시 3바이트 (레거시)

// urxvt·X10은 좌표/버튼을 32 오프셋으로 실어 보낸다(제어문자 회피). SGR만 버튼이 날것.
const OFFSET = 32;

// 버튼 코드 → 휠 방향. 휠 비트는 64, 모디파이어는 shift=4·alt=8·ctrl=16으로 가산되므로
// 하위 2비트로만 방향을 읽는다(64|mods=휠업, 65|mods=휠다운).
// 66/67(좌우 휠)은 세로 스크롤로 번역할 수 없어 0(=폐기)으로 떨어뜨린다.
// 128 비트는 확장 버튼(8~11)이라 휠이 아니므로 0xC0 마스크로 함께 배제한다.
function wheelDelta(button: number): -1 | 0 | 1 {
  if ((button & 0xc0) !== 0x40) return 0;
  const low = button & 3;
  if (low === 0) return -1; // 휠업 → 위로 스크롤
  if (low === 1) return 1; // 휠다운 → 아래로 스크롤
  return 0;
}

interface Hit {
  end: number; // 보고가 끝나는 인덱스(다음 스캔 시작점)
  wheel: -1 | 0 | 1;
}

// data[from]에서 시작하는 마우스 보고 하나를 매칭한다. 아니면 null.
function matchReport(data: string, from: number): Hit | null {
  RE_SGR.lastIndex = from;
  const sgr = RE_SGR.exec(data);
  if (sgr) return { end: RE_SGR.lastIndex, wheel: wheelDelta(Number(sgr[1])) };

  RE_X10.lastIndex = from;
  if (RE_X10.exec(data)) {
    // ESC[M 뒤 3바이트 중 첫 바이트가 버튼(+32). ESC[M 접두가 유일해 오탐 위험이 없다.
    const button = data.charCodeAt(from + 3) - OFFSET;
    return { end: RE_X10.lastIndex, wheel: wheelDelta(button) };
  }

  RE_URXVT.lastIndex = from;
  const urx = RE_URXVT.exec(data);
  if (urx) {
    const button = Number(urx[1]) - OFFSET;
    // urxvt 문법(`ESC[n;n;nM`)은 접두가 평범해 오탐 여지가 있다. 실제 보고는 버튼·좌표가 모두
    // 32 이상이므로 그 하한을 만족할 때만 마우스로 인정한다(미달이면 non-match → 청크는 pass).
    if (button >= 0 && Number(urx[2]) >= OFFSET && Number(urx[3]) >= OFFSET) {
      return { end: RE_URXVT.lastIndex, wheel: wheelDelta(button) };
    }
  }
  return null;
}

// 입력 청크를 분류한다. 청크 전체가 마우스 보고로만 구성될 때만 drop/wheel이고, 그 외엔 전부 pass.
//
// ★혼합 청크(마우스 보고 + 일반 텍스트)를 pass로 두는 이유:
// xterm.js는 마우스 이벤트당 onData를 1회 발화하므로 혼합은 이론상 발생하지 않는다. 그럼에도
// 방어적으로 pass를 택하는 것은 두 오류의 비대칭 때문이다 — 잘못 폐기하면 사용자가 친 텍스트가
// 조용히 사라지지만(복구 불가·원인 불명), 잘못 통과시키면 깨진 보고 1회가 화면에 보일 뿐이다.
// 텍스트를 잃는 것보다 유출 1회가 낫다.
//
// 미완성 보고(청크 경계에서 잘린 X10 3바이트 등)도 같은 이유로 non-match → pass다.
export function classifyMouseReport(data: string): MouseVerdict {
  if (!data) return { kind: "pass" }; // 빈 청크는 손대지 않는다

  let i = 0;
  let reports = 0;
  let net = 0; // 휠 노치 순증감(+아래 / -위) — 고빈도 배칭 청크를 한 번의 스크롤로 접는다
  while (i < data.length) {
    const hit = matchReport(data, i);
    if (!hit) return { kind: "pass" }; // 마우스 아닌 바이트가 하나라도 있으면 통째로 통과
    reports++;
    net += hit.wheel;
    i = hit.end;
  }
  if (reports === 0) return { kind: "pass" }; // 도달 불가(빈 문자열은 위에서 컷) — 방어
  if (net === 0) return { kind: "drop" }; // 휠 아님, 또는 위/아래가 상쇄돼 스크롤할 것이 없음
  return { kind: "wheel", dir: net > 0 ? 1 : -1, count: Math.abs(net) };
}
