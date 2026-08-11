// xterm.js가 PTY로 보내려는 '마우스 보고' 시퀀스를 분류한다 — 순수 함수.
//
// 왜 필요한가 — 현장 결함 2건이 이 필터의 존재 이유다:
//  · 결함 1호(Parallels Windows 실기 제보): Claude Code TUI가 마우스 트래킹(1003h/1006h)을 켜면
//    xterm.js는 마우스 이벤트마다 `ESC[<b;x;y M/m` 같은 보고를 onData로 발화해 PTY로 보낸다.
//    Windows는 ConPTY 입력 계층이 시퀀스를 깨뜨려 선두 ESC가 소실된 `[555;98;34M...` 문자열이
//    Claude Code 입력창에 리터럴로 무한 타이핑된다. ∴ Windows에서는 보고를 PTY로 보내지 않는다.
//  · 결함(macOS 스크롤백 접근 불가, 2026-08 현장 제보): 마우스 트래킹 중 xterm은 휠을 로컬
//    스크롤 대신 보고로 앱에 보내므로, Claude Code가 떠 있는 동안 사용자가 스크롤백을 읽을 수
//    없다. ∴ 휠 보고만은 **모든 플랫폼**에서 로컬 스크롤로 번역한다(비-휠 보고는 macOS에서
//    기존대로 앱에 전달 — 클릭 소비 보존·오폐기=입력 소실 비대칭 회피).
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
    // ESC[M 뒤 3바이트는 순서대로 버튼·x·y이고, 규약상 **셋 다** 원시값이 OFFSET(0x20) 이상이다
    // (제어문자 회피를 위해 32를 더해 싣기 때문). 정규식은 자릿수만 세므로 여기서 값 하한을 검증한다 —
    // 무검증으로 3 코드유닛을 삼키면 `ESC[M` 뒤에 우연히 붙은 사용자 입력·제어문자까지 마우스 보고로
    // 오인해 폐기하게 된다(입력 소실). 하나라도 미달이면 non-match → 청크는 기존 정책대로 pass.
    const b = data.charCodeAt(from + 3);
    const x = data.charCodeAt(from + 4);
    const y = data.charCodeAt(from + 5);
    if (b >= OFFSET && x >= OFFSET && y >= OFFSET) {
      return { end: RE_X10.lastIndex, wheel: wheelDelta(b - OFFSET) };
    }
    return null;
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

// 휠 노치 하나가 굴리는 줄 수 — 통상 터미널 관용치.
const WHEEL_LINES = 3;

// onData 청크를 어디로 보낼지에 대한 '결정'. 실행(스크롤·전송)은 호출측 main.ts가 한다.
//   forward = 기존 IME 경로로 원문 그대로(data는 입력과 동일 참조 — 바이트 하나 건드리지 않는다는 계약)
//   scroll  = PTY로 보내지 말고 term.scrollLines(lines)로 로컬 스크롤
//   discard = 무음 폐기
export type OnDataRoute =
  | { action: "forward"; data: string }
  | { action: "scroll"; lines: number }
  | { action: "discard" };

// 플랫폼 분기까지 포함한 배선 결정을 순수 함수로 뽑아낸다 — main.ts에는 실행만 남기고
// '어떤 조건에서 무엇을 하는가'는 전부 여기서 테스트로 고정한다(배선 회귀를 테스트가 지키게).
//
// ★플랫폼 계약(2026-08 개정 — 구계약 "macOS는 분류조차 않고 forward"를 의도적으로 파기):
//  · wheel  → **모든 플랫폼**에서 로컬 스크롤. macOS도 마우스 트래킹 중 스크롤백을 읽을 수
//    있어야 한다(현장 제보). 방향키 시퀀스 주입은 금지(의도된 결정 — Claude Code의 입력
//    히스토리를 오염시킨다).
//  · drop(비-휠 보고·상쇄된 휠) → Windows만 폐기(ConPTY 깨짐 방지). macOS는 **forward 유지**:
//    앱이 클릭·모션 보고를 정상 소비하므로 빼앗을 이유가 없고, 오폐기의 비용(입력 소실)이
//    유출 1회보다 크다는 비대칭 원칙도 그대로다.
//  · pass  → forward(원문 동일 참조 — 바이트 하나 건드리지 않는다는 계약 불변).
//
// ★opts 계약(2026-08-12 R2/R3 확정 — 두 결함의 동시 봉합):
//  · allowAppMouse: 킬스위치가 켜지면 **분류 없이 원문 forward**(참조 동일). 종전엔 킬스위치가
//    출력측(trackfilter)만 우회해 앱이 트래킹은 켜는데 보고(클릭·드래그·휠)는 이 필터가 계속
//    가로채·폐기해 — 킬스위치를 켠 사용자의 TUI(vim mouse=a)가 마우스를 하나도 못 받았다.
//    킬스위치의 의미는 '앱이 마우스를 갖는다'이므로 입·출력 양측을 함께 우회해야 참이다.
//  · altScreen: 대체 화면(스크롤백 없는 전체화면 TUI)에서 휠 로컬 스크롤은 no-op이다 — 트래킹
//    보고가 유출 경로(스트리핑 누락·경계 결함)로 발생한 경우, 비-Windows 는 앱에 forward(앱이
//    트래킹을 켰으니 소비 의사가 있다)하고 Windows 는 폐기(ConPTY 깨짐이 휠 보고에도 적용 —
//    no-op 로컬 스크롤과 실효 동일하나 정직한 폐기).
export function routeOnData(
  data: string,
  isWindows: boolean,
  opts?: { allowAppMouse?: boolean; altScreen?: boolean },
): OnDataRoute {
  if (opts?.allowAppMouse) return { action: "forward", data };
  const verdict = classifyMouseReport(data);
  if (verdict.kind === "wheel") {
    if (opts?.altScreen) {
      return isWindows ? { action: "discard" } : { action: "forward", data };
    }
    return { action: "scroll", lines: verdict.dir * WHEEL_LINES * verdict.count };
  }
  if (verdict.kind === "drop" && isWindows) return { action: "discard" };
  return { action: "forward", data };
}
