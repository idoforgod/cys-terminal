// wheelgate.ts 휠 억제 술어 회귀 테스트 (스펙 D4 — 매트릭스 고정).
//
// 이 파일은 술어 **둘**과 그 **배선 계층**을 고정한다. 앞 두 describe = mac 경로
// (shouldSuppressWheel), 가운데 두 describe = Windows 경로(shouldSuppressWheelWin), 마지막 두
// describe = 배선(wheelHandlerKind·win/macGateInputs). 아래 매트릭스의 "!Windows" 항을 앱 전체의
// 계약으로 읽지 마라 — 'Windows 는 억제하지 않는다'는 **mac 술어에 한정된 참**이다.
//
// mac 술어의 억제 = [alt ∧ 장부 트래킹 요청 ∧ xterm 트래킹 미진입 ∧ !킬스위치 ∧ !Windows]
// 전부 참일 때 **만**이다. 한 항이라도 어긋나면 xterm 기본 처리(방향키 합성·보고 전송·로컬
// 스크롤)를 보존해야 한다 — less/man 의 휠 페이지 넘김이 이 계약에 걸려 있다.
import { describe, it, expect } from "bun:test";
import { shouldSuppressWheel, type WheelGateState } from "./wheelgate";
// Windows 전용 술어는 별도 import 로 둔다 — 위 줄(기존 38 단언의 입력)에 손대지 않기 위함.
import { shouldSuppressWheelWin, type WinWheelGateState } from "./wheelgate";
import {
  wheelHandlerKind,
  winGateInputs,
  macGateInputs,
  type WheelTermView,
  type WheelLedgerView,
} from "./wheelgate";

const base: WheelGateState = {
  altActive: true,
  ledgerWantsMouse: true,
  xtermTracking: false,
  allowAppMouse: false,
  isWindows: false,
};

describe("휠 억제 매트릭스 — 시나리오 명명 케이스", () => {
  it("claude fullscreen 주입 파싱 창(alt·장부 요청·xterm 미진입·mac) → 억제", () => {
    // 이 창의 휠이 방향키로 합성되면 프롬프트 히스토리가 오염된다(원 결함) — 유일한 억제 케이스.
    expect(shouldSuppressWheel(base)).toBe(true);
  });
  it("less/man(트래킹 무요청 alt) → 비억제 — 방향키 합성 보존(휠 페이지 넘김 무회귀)", () => {
    expect(shouldSuppressWheel({ ...base, ledgerWantsMouse: false })).toBe(false);
  });
  it("vim mouse=a·claude 정착 후(xterm 트래킹 진입 완료) → 비억제 — 보고 경로 그대로", () => {
    expect(shouldSuppressWheel({ ...base, xtermTracking: true })).toBe(false);
  });
  it("primary 화면(비 alt) → 비억제 — 로컬 스크롤백 경로 보존", () => {
    expect(shouldSuppressWheel({ ...base, altActive: false })).toBe(false);
  });
  it("킬스위치 allowAppMouse(pane 캡처값) → 비억제 — 앱이 마우스를 갖는다", () => {
    expect(shouldSuppressWheel({ ...base, allowAppMouse: true })).toBe(false);
  });
  // ★제목 정정(2026-08-17): 종전 제목은 "…main.ts 는 핸들러 자체 미등록"이었으나 main.ts 는
  // 이제 Windows 에도 휠 핸들러를 등록한다(win 전용 술어로 — 이 파일 아래 describe 가 고정).
  // 단언은 무수정 — 이 술어가 Windows 에서 절대 억제하지 않는다는 계약은 그대로다.
  it("Windows → 이 mac 술어는 항상 비억제 — win 억제는 shouldSuppressWheelWin 이 맡는다", () => {
    expect(shouldSuppressWheel({ ...base, isWindows: true })).toBe(false);
  });
});

describe("휠 억제 매트릭스 — 32조합 전수(억제는 정확히 1조합)", () => {
  const bools = [false, true];
  for (const altActive of bools)
    for (const ledgerWantsMouse of bools)
      for (const xtermTracking of bools)
        for (const allowAppMouse of bools)
          for (const isWindows of bools) {
            const s = { altActive, ledgerWantsMouse, xtermTracking, allowAppMouse, isWindows };
            const want =
              altActive && ledgerWantsMouse && !xtermTracking && !allowAppMouse && !isWindows;
            it(`${JSON.stringify(s)} → ${want ? "억제" : "통과"}`, () => {
              expect(shouldSuppressWheel(s)).toBe(want);
            });
          }
});

// ─────────────────────────────────────────────────────────────────────────────
// Windows 전용 술어 회귀 테스트 (스펙 C-3). 위 38 단언의 `expect` 는 무수정 — mac 경로 계약은
// 그대로 두고 여기에만 추가한다(wheelgate.ts 가 술어를 분리한 이유와 같다).
//
// ★위 32조합 루프의 `want` 식에 대한 정확한 평가(2026-08-17 실측 정정 — 종전 이 자리에는
//   "술어가 바뀌면 기대값도 함께 바뀌어 회귀를 못 잡는다"고 적혀 있었고 그것은 **거짓**이었다):
//   그 `want` 는 **이 테스트 파일 안의 리터럴 식**이고 shouldSuppressWheel 을 호출하지 않는다.
//   wheelgate.ts 를 고쳐도 자동으로 따라오지 않으므로 술어 변이를 실제로 잡는다 — 측정으로
//   확인했다(항 제거 5종·상수화 1종 = 6 변이 전건에서 이 루프가 FAIL). 다만 술어를 고치는
//   **사람**이 want 를 기계적으로 베껴 맞추면 무력해진다. ∴ 의미 회귀의 정본 감시선은 위 명명
//   6종이고, 이 루프는 조합 누락 방지용이다. 거짓 자기평가를 남기면 "어차피 못 잡는 루프"라며
//   38 단언을 걷어낼 근거로 오용될 수 있어 문장을 사실로 교체했다.
//   아래 Windows 표는 애초에 그 위험이 없다 — 기대값이 손으로 적은 진리표 리터럴이라 술어를
//   고치면 표가 깨져야 정상이다. 표를 고쳐 맞추지 말고 설계 이탈인지 먼저 따져라.
//
// ★★정정 이력(2026-08-17): 위 mac 블록의 산문 두 곳이 배선 변경으로 거짓이 됐었다 —
//   파일 머리의 "억제 = […∧ !Windows] 전부 참일 때만"(앱 전체의 계약처럼 읽혔다)과 it 제목의
//   "main.ts 는 핸들러 자체 미등록"(이제 등록한다). **둘 다 이 라운드에서 고쳤다.**
//   · 무수정 계약의 범위: 스펙 C-3 이 못박은 것은 기존 38 단언의 `expect` 식이다 — 그것이
//     mac 경로 회귀 감시선이고, 이번 수정에서 단언은 한 줄도 바뀌지 않았다(제목·주석만).
//     거짓이 된 제목을 보존하는 것은 감시선 보존이 아니라 결함이다(주석=계약 규약).
//   · 여전히 참인 사실: shouldSuppressWheel 은 isWindows=true 를 항상 불충족으로 만든다.
//     그것은 **그 술어에 한정된 참**이고 앱 전체의 참이 아니다 — Windows 의 억제는 아래
//     describe 가 고정하는 shouldSuppressWheelWin 이 맡는다.
// ★이 describe 전체의 지위(정직 고지 — 성찰3 테스트렌즈 note): 아래 명명 케이스는 **전건이
//   다음 describe 진리표의 특정 행과 입력·기대값이 완전히 같다**(ⓐ=11000 상당 … 측정으로
//   확인: 술어 변이 전종에서 '명명 fail ⊆ 진리표 fail'). ∴ 커버리지로 세지 마라 — 이 블록의
//   값은 판별력이 아니라 **의도 문서화**다(어떤 시나리오가 왜 그 기대값인지). 회귀를 실제로
//   잡는 것은 손으로 적은 진리표 쪽이다.
describe("Windows 휠 억제 술어 — 시나리오 명명 케이스", () => {
  // ⓐ 억제 기준선: claude fullscreen 이 1003(any-motion)을 켠 직후, xterm 은 아직 미진입.
  const claudeFullscreen: WinWheelGateState = {
    altActive: true,
    ledgerWantsAnyMotion: true,
    xtermTracking: false,
    allowAppMouse: false,
  };

  it("ⓐ claude fullscreen(alt ∧ 장부 1003 ∧ xterm 미진입) → 억제", () => {
    // Claude Code 는 alt 진입 시 1000+1002+1003+1006("full")을 켠다 → 1003 이 장부에 오른다.
    // 이 창의 휠이 방향키로 합성되면 프롬프트 히스토리가 오염된다(원 결함).
    expect(shouldSuppressWheelWin(claudeFullscreen)).toBe(true);
  });

  it("ⓑ vim mouse=a(1000/1002/1006 만 — 1003 부재) → 비억제 ★이 파일의 핵심 단언", () => {
    // Windows 에서 vim 휠(방향키 합성)은 현행 UX 다. 판별자를 1003 으로 좁힌 유일한 이유가
    // 이 케이스이며, 여기가 깨지면 Windows vim 휠이 조용히 죽는다 = 설계 실패 신호.
    // (미확정: 저장소에 vim 의 실제 DECSET 캡처가 0건 — wheelgate.ts (d) 참조.)
    expect(shouldSuppressWheelWin({ ...claudeFullscreen, ledgerWantsAnyMotion: false })).toBe(false);
  });

  it("ⓒ less/man(트래킹 무요청 — 장부 공백) → 비억제 — 휠 페이지 넘김 무회귀", () => {
    // ★독립 판별력 0 고지(정직): 이 상태 모델에서 ⓒ 는 ⓑ 와 **입력·기대값이 완전히 같다**.
    // WinWheelGateState 가 장부에 대해 갖는 정보는 '1003 이 켜졌는가' 한 비트뿐이라,
    // "1003 만 없다"(vim)와 "아무것도 없다"(less/man)가 같은 벡터로 접힌다. 그래도 두 케이스를
    // 따로 두는 이유는 ①스펙 C-3 이 명명 6종을 요구했고 ②둘은 **깨지는 방식이 다르기** 때문이다:
    // 판별자를 ledgerWantsMouse(아무 h 나) 로 되돌리면 ⓑ 만 깨지고 ⓒ 는 살아남는다.
    // ∴ ⓒ 는 회귀 감시가 아니라 '이 시나리오도 비억제여야 한다'는 **문서적 핀**이다 —
    // 커버리지 계산에 두 건으로 세지 마라. 벡터를 spread 로 감추지 않고 전 필드를 적어 둔다.
    expect(
      shouldSuppressWheelWin({
        altActive: true, // alt 화면(less/man 도 alt 를 쓴다)
        ledgerWantsAnyMotion: false, // 장부 공백 — 트래킹 DECSET 자체가 없다
        xtermTracking: false, // 앱이 안 켰으니 xterm 도 진입 없음
        allowAppMouse: false,
      }),
    ).toBe(false);
  });

  it("ⓓ primary 화면(비 alt) → 비억제 — 로컬 스크롤백 경로 보존", () => {
    expect(shouldSuppressWheelWin({ ...claudeFullscreen, altActive: false })).toBe(false);
  });

  it("ⓔ 킬스위치 allowAppMouse(pane 캡처값) → 비억제 — 앱이 마우스를 갖는다", () => {
    expect(shouldSuppressWheelWin({ ...claudeFullscreen, allowAppMouse: true })).toBe(false);
  });

  it("ⓕ xterm 트래킹 진입 완료 → 비억제 — 이 술어에서 xterm 진입은 '해제항'이다", () => {
    // ★도달 가능성: Windows 에서 이 상태는 **발생하지 않는다**(trackfilter win 인스턴스가
    //   마우스 DECSET 을 전량 스트리핑 → mouseTrackingMode 상수 "none"). 그래도 명명 층에
    //   남기는 이유는 의도 문서화다 — 이 항이 '억제를 거는 조건'이 아니라 '억제를 푸는 조건'
    //   임을 코드 독자가 오해하지 않게(wheelgate.ts (b) 의 단일 방어선 고지와 짝).
    expect(shouldSuppressWheelWin({ ...claudeFullscreen, xtermTracking: true })).toBe(false);
  });

  // ※ 종전 이 자리에 있던 ⓕ(PAGE 모드 절대 상한)·ⓔ+ⓕ 두 케이스는 술어에서 `pageMode` 항을
  //   제거하면서 함께 삭제했다 — 상태에 없는 필드를 검사할 수는 없다. 제거 판단의 근거 전문은
  //   wheelgate.ts (c) 에 남겼다(요지: 그 항이 단독으로 덮는 영역은 이중 가정의 사각뿐인데,
  //   PAGE 를 보고하는 엔진에서는 vim·less·man 의 alt 휠을 전멸시키고 그 탈출구가 '가드 전체
  //   끄기'=원 결함 복원 하나뿐이었다). 되살리려면 그 문단의 재도입 조건을 먼저 충족시켜라.
});

describe("Windows 휠 억제 술어 — 16조합 전수(손으로 적은 진리표)", () => {
  // 비트 문자열 자릿수 = [altActive, ledgerWantsAnyMotion, xtermTracking, allowAppMouse].
  // 기대값은 손으로 적었다 — 술어 식을 재계산하지 않는다(위 ★함정 주석).
  // ※ 종전에는 5비트 32행이었다(끝자리 = pageMode). 술어에서 그 항을 뺐으므로 표도 4비트
  //   16행으로 줄었다 — 술어의 상태 공간을 그대로 따라간 것이지 감시선을 걷어낸 것이 아니다
  //   (억제 행은 3행 → 1행: 제거된 2행은 전부 '1003 부재인데 PAGE 라서 억제' 계열이었다).
  //
  // ★도달 가능성 고지(행 주석을 실제 시나리오로 읽지 마라): xterm 비트=1 인 행들은 술어의 전수
  //   계약을 지키기 위한 것이지 **Windows 에서 실제로 발생할 수 있는 상태가 아니다**. trackfilter
  //   의 win 인스턴스는 consume=false 라 마우스 DECSET(9·1000·1002·1003·1005·1006·1015·1016)을
  //   전부 스트리핑하고, 그래서 xterm 은 Windows 에서 트래킹에 진입할 수 없다
  //   (term.modes.mouseTrackingMode 상수 "none"). 유일한 예외 경로(allowAppMouse=true, 원문
  //   그대로 write)는 킬스위치 비트가 이미 술어를 끈다. ∴ Windows 억제의 실질 판별자는 1003
  //   휴리스틱 **하나뿐**이고, 이 표의 xterm 행은 '이중 방어가 있다'는 증거가 아니다
  //   (wheelgate.ts (b) 의 단일 방어선 고지 참조).
  const 진리표: Array<[string, boolean]> = [
    // alt=0 — primary 화면에서는 무엇을 켜든 억제하지 않는다(로컬 스크롤백 보존).
    ["0000", false],
    ["0001", false],
    ["0010", false],
    ["0011", false],
    ["0100", false],
    ["0101", false],
    ["0110", false],
    ["0111", false],
    // alt=1 — 억제는 정확히 1조합(1100).
    ["1000", false], // less/man: 장부 공백 → 통과(휠 페이지 넘김 무회귀)
    ["1001", false], // 킬스위치
    ["1010", false], // xterm 진입 완료 = 보고 경로
    ["1011", false],
    ["1100", true], // ⓐ claude fullscreen 기준선 = 유일한 억제 조합
    ["1101", false], // 킬스위치가 1003 보다 세다
    ["1110", false], // vim/claude 정착 후(xterm 진입 완료)
    ["1111", false],
  ];

  it("표 자체의 무결성 — 16행 · 억제 1행(오타 방지 자물쇠)", () => {
    expect(진리표.length).toBe(16);
    expect(진리표.filter(([, want]) => want).length).toBe(1);
    expect(new Set(진리표.map(([bits]) => bits)).size).toBe(16);
  });

  // 비트열 → 필드 매핑을 만드는 자리. 아래 매핑 핀이 이 함수를 직접 검사한다.
  const stateOf = (bits: string): WinWheelGateState => ({
    altActive: bits[0] === "1",
    ledgerWantsAnyMotion: bits[1] === "1",
    xtermTracking: bits[2] === "1",
    allowAppMouse: bits[3] === "1",
  });

  it("비트열↔필드 매핑 자물쇠 — 진리표로는 영원히 못 잡는 축(대칭 술어)", () => {
    // ★왜 따로 못박는가: 술어는 `!xtermTracking && !allowAppMouse` 라 **두 항의 교환에
    //   불변**이다. 즉 위 stateOf 에서 3·4번째 비트를 맞바꿔도 16행 기대값 집합이 그대로
    //   유지돼 표는 전부 초록이다. 그러면 행 주석의 "킬스위치" / "xterm 진입 완료" 라벨과
    //   그 위 ★도달 가능성 고지(xterm 비트=1 행은 Windows 에서 도달 불가)가 조용히 뒤바뀐다
    //   — 주석=계약인 저장소에서 라벨 오염은 결함이다. 매핑을 여기서 직접 확인한다.
    const s = stateOf("1010");
    expect(s.altActive).toBe(true);
    expect(s.ledgerWantsAnyMotion).toBe(false);
    expect(s.xtermTracking).toBe(true); // 3번째 비트 = xterm(도달 불가 축)
    expect(s.allowAppMouse).toBe(false); // 4번째 비트 = 킬스위치(도달 가능 축)
  });

  for (const [bits, want] of 진리표) {
    const s = stateOf(bits);
    it(`${bits}(alt·1003·xterm·killswitch) → ${want ? "억제" : "통과"}`, () => {
      expect(shouldSuppressWheelWin(s)).toBe(want);
    });
  }
});

// ─────────────────────────────────────────────────────────────────────────────
// 배선 계층 — 술어에 **무엇을 먹이는가**와 **어느 술어를 등록하는가**를 고정한다
// (성찰3 테스트렌즈 major ×2: 종전 이 두 판정은 main.ts 인라인이라 단언 0건이었고,
//  오배선을 잡는 단언이 저장소 **전 스위트에 한 건도 없었다**).
// ★수치 정정(2026-08-17): 종전 이 자리에는 "오배선 시 410 단언 중 어느 것도 울지 않았다"고
//   적혀 있었으나 **410 은 어느 시점의 실측과도 맞지 않는다** — 이 라운드 직전 트리(HEAD)의
//   `bun test` 는 **353건**이고 현재는 **411건**이다. 그래서 절대 건수를 지웠다. 계약으로
//   읽어야 할 것은 개수가 아니라 관계다: **배선을 보는 단언이 0이었다**(그래서 스위트가 몇
//   건이든 전건 초록인 채로 오배선이 통과했다). 이 파일의 다른 수치 주석과 같은 규약이다.
describe("휠 핸들러 선택 — 8조합 전수(배타성을 반환 타입으로 강제)", () => {
  // 기대값은 손으로 적었다(술어 재계산 금지). 자릿수 = [isWindows, reconcile, winWheelGuardOff].
  const 표: Array<[string, "mac" | "win" | "none"]> = [
    ["000", "none"], // mac ∧ 정합기 롤백 → 미등록(억제도 끈다)
    ["001", "none"], // 〃 (win 게이트는 mac 에서 무의미)
    ["010", "mac"], // mac 기본 경로
    ["011", "mac"], // 〃 — winWheelGuardOff 는 mac 판정에 영향이 없다
    ["100", "win"], // Windows 기본 경로 — reconcile 을 보지 않는다
    ["101", "none"], // Windows 롤백 게이트 ON → 미등록 = 종전(방향키 합성) 동작
    ["110", "win"], // reconcile 이 켜져 있어도 Windows 판정은 동일
    ["111", "none"], // 게이트가 이긴다
  ];

  it("표 자체의 무결성 — 8행 · mac 2 · win 2 · none 4", () => {
    expect(표.length).toBe(8);
    expect(new Set(표.map(([bits]) => bits)).size).toBe(8);
    expect(표.filter(([, k]) => k === "mac").length).toBe(2);
    expect(표.filter(([, k]) => k === "win").length).toBe(2);
    expect(표.filter(([, k]) => k === "none").length).toBe(4);
  });

  for (const [bits, want] of 표) {
    const s = {
      isWindows: bits[0] === "1",
      reconcile: bits[1] === "1",
      winWheelGuardOff: bits[2] === "1",
    };
    it(`${bits}(isWindows·reconcile·guardOff) → ${want}`, () => {
      expect(wheelHandlerKind(s)).toBe(want);
    });
  }

  it("★배타성 — 어떤 입력에서도 결과는 정확히 하나다(둘 다 등록되는 상태가 표현 불가)", () => {
    // 종전 배선은 `if (…) {등록} else if (…) {등록}` 이었고, `else` 한 단어만 지우면 두
    // 핸들러가 다 등록돼 **뒤가 앞을 조용히 덮었다**(attachCustomWheelEventHandler 는
    // 인스턴스당 단일 슬롯). 그 회귀를 잡는 단언이 저장소에 0건이었다. 이제 값이 하나뿐이라
    // 그 상태는 타입으로 표현할 수 없다 — 이 단언은 그 사실을 문서화하고, 반환값이 셋 중
    // 하나임을 전수로 고정한다.
    for (const bits of ["000", "001", "010", "011", "100", "101", "110", "111"]) {
      const k = wheelHandlerKind({
        isWindows: bits[0] === "1",
        reconcile: bits[1] === "1",
        winWheelGuardOff: bits[2] === "1",
      });
      expect(["mac", "win", "none"].filter((x) => x === k).length).toBe(1);
    }
  });
});

describe("술어 입력 조립 — 오배선 검출(장부 접근자·xterm 리터럴)", () => {
  const termView = (alt: boolean, tracking: boolean): WheelTermView => ({
    buffer: { active: { type: alt ? "alternate" : "normal" } },
    modes: { mouseTrackingMode: tracking ? "any" : "none" },
  });
  // ★두 페이크의 요점: 한쪽만 true 인 장부를 만들어 **어느 접근자를 읽었는지** 드러낸다.
  //  vim 장부(1000/1002/1006) = wantsMouse true · wantsAnyMotion false.
  //  claude 장부(…+1003)       = 둘 다 true.
  const vimLedger: WheelLedgerView = {
    ledgerWantsMouse: () => true,
    ledgerWantsAnyMotion: () => false,
  };
  const claudeLedger: WheelLedgerView = {
    ledgerWantsMouse: () => true,
    ledgerWantsAnyMotion: () => true,
  };

  it("★win 배선이 ledgerWantsAnyMotion 을 읽는다 — wantsMouse 로 바꾸면 Windows vim 휠이 죽는다", () => {
    // 이 단언이 이 describe 의 존재 이유다. 두 접근자는 같은 클래스의 인접 메서드이고
    // 타입이 둘 다 boolean 이라, main.ts 인라인 시절엔 바꿔 써도 아무 테스트도 울지 않았다.
    expect(winGateInputs(termView(true, false), vimLedger, false).ledgerWantsAnyMotion).toBe(false);
    expect(winGateInputs(termView(true, false), claudeLedger, false).ledgerWantsAnyMotion).toBe(true);
    // 조립 → 술어까지 이어 붙인 최종 계약(이 두 줄이 실사용 경로 전체다).
    expect(shouldSuppressWheelWin(winGateInputs(termView(true, false), vimLedger, false))).toBe(false);
    expect(shouldSuppressWheelWin(winGateInputs(termView(true, false), claudeLedger, false))).toBe(true);
  });

  it("mac 배선은 ledgerWantsMouse 를 읽는다(win 과 반대 — 넓은 판별자가 mac 계약)", () => {
    expect(macGateInputs(termView(true, false), vimLedger, false, false).ledgerWantsMouse).toBe(true);
    expect(shouldSuppressWheel(macGateInputs(termView(true, false), vimLedger, false, false))).toBe(true);
  });

  it("xterm 리터럴 — alt 판정은 \"alternate\", 트래킹 판정은 !==\"none\"", () => {
    // 리터럴 오타(예: "alt")는 억제를 영구 불발시키고, `=== "none"` 반전은 억제를 뒤집는다.
    expect(winGateInputs(termView(false, false), claudeLedger, false).altActive).toBe(false);
    expect(winGateInputs(termView(true, false), claudeLedger, false).altActive).toBe(true);
    expect(winGateInputs(termView(true, true), claudeLedger, false).xtermTracking).toBe(true);
    expect(winGateInputs(termView(true, false), claudeLedger, false).xtermTracking).toBe(false);
  });

  it("킬스위치·isWindows 는 조립이 판단하지 않고 그대로 통과시킨다(캡처값 계약)", () => {
    expect(winGateInputs(termView(true, false), claudeLedger, true).allowAppMouse).toBe(true);
    const m = macGateInputs(termView(true, false), claudeLedger, true, true);
    expect(m.allowAppMouse).toBe(true);
    expect(m.isWindows).toBe(true); // 현 배선에서는 도달 불가 — 술어 인터페이스 동결의 잔여항
  });
});
