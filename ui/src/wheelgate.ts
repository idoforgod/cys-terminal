// 휠 억제 술어 — 순수 함수 (배선은 main.ts 의 term.attachCustomWheelEventHandler).
//
// 왜 필요한가(스펙 D4+A8 — 문제2의 잔여 창 봉인): 정합기(trackfilter)가 alt 진입 시 장부의
// 트래킹 DECSET 을 재생 주입해도, xterm 의 write 파싱은 비동기라 '1049h 는 파싱됐고 주입
// DECSET 은 아직'인 찰나가 있다. 그 창에서 xterm 은 alt buffer 의 휠을 방향키(CUU/CUD)로
// 합성해 앱에 보내고 — Claude Code 는 그것을 프롬프트 히스토리 탐색으로 소비해 입력
// 히스토리가 오염된다(원 결함). ∴ [alt ∧ 장부 트래킹 요청 ∧ xterm 트래킹 미진입]이면 휠을
// 통째로 소비한다. 억제 방향의 오차 비용은 휠 1노치 손실 — 오염 대비 안전측이다(스펙 R1 S8).
//
// 계약(wheelgate 매트릭스 — wheelgate.test.ts 가 고정):
//  · less/man 류(트래킹 무요청 alt): ledgerWantsMouse=false → 불충족 — 방향키 합성 보존
//    (휠로 페이지가 굴러가는 현행 UX 무회귀).
//  · vim mouse=a·claude 정착 후(트래킹 진입 완료): xtermTracking=true → 불충족 — 보고 경로.
//  · 킬스위치(allowAppMouse): 앱이 마우스를 갖는다 — 억제하지 않는다(pane 생성 시 캡처값).
//  · Windows: 억제 비활성 — 방향키 합성 유지(vim mouse=a 휠 무회귀·문제2는 win claude 기본
//    inline 이라 미발현). main.ts 는 핸들러 자체를 win 에 미등록하지만 술어도 자체 방어한다.
//  · 롤백 스위치(cysMouseReconcilerOff): main.ts 가 핸들러를 등록하지 않는다(win 경로 재사용).

export interface WheelGateState {
  altActive: boolean; // term.buffer.active.type === "alternate"
  ledgerWantsMouse: boolean; // 정합기 장부에 희망 h 파라미터 존재(trackFilter.ledgerWantsMouse())
  xtermTracking: boolean; // term.modes.mouseTrackingMode !== "none"
  allowAppMouse: boolean; // pane 생성 시 캡처된 킬스위치(라이브 재판독 금지)
  isWindows: boolean;
}

// true = 휠 소비. 호출측은 xterm 커스텀 휠 핸들러에서 `!shouldSuppressWheel(...)` 를 반환해
// 기본 처리(방향키 합성·보고 전송·로컬 스크롤)를 차단한다(return false = xterm 미처리).
export function shouldSuppressWheel(s: WheelGateState): boolean {
  return s.altActive && s.ledgerWantsMouse && !s.xtermTracking && !s.allowAppMouse && !s.isWindows;
}
