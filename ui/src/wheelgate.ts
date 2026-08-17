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
//  · Windows: 이 mac 술어는 isWindows=true 를 항상 불충족으로 만든다 — **이 함수는 win 에서
//    아무것도 억제하지 않는다**. Windows 의 억제는 아래 별도 술어(shouldSuppressWheelWin)가
//    맡고, main.ts 는 두 술어를 OS 로 **배타 분기**해 각각 등록한다.
//    ★★`isWindows` 항의 현재 지위(설계 정직 고지 — 성찰3 설계렌즈 minor): 이 항은 **현 배선에서
//      true 가 될 수 없다**. OS 분기의 소유자는 이제 아래 `wheelHandlerKind` 하나이고, 그것이
//      "mac" 을 돌려줄 때만 이 술어가 호출되기 때문이다(=isWindows 는 항상 false). 그래도 항과
//      32조합 진리표를 남겨 두는 이유는 **설계 필요가 아니라 회귀 감시선 동결**이다(스펙 C-3 이
//      기존 38 단언의 expect 를 무수정 대상으로 못박았다). 동결이 풀리면 `isWindows` 를
//      인터페이스에서 제거하고 OS 판정을 wheelHandlerKind 단일 소유로 정리하라 — 그때 이 술어의
//      진리표는 16행으로 줄어든다. 그전까지 "mac 술어가 OS 판정에 참여한다"고 읽지 마라.
//    ★정정(2026-08-17 실측 · 스펙 C-6): 종전 이 자리에는 "문제2는 win claude 가 기본 inline
//    이라 미발현" + "main.ts 는 핸들러 자체를 win 에 미등록" 이라고 적혀 있었다. **둘 다 이제
//    거짓이다.** ①전제 반증: Claude Code 2.1.233 의 fullscreen 판정 함수 `ra()` 에 순수
//    Windows→inline 분기가 없고 Windows 관련 분기는 `Windows ∧ SSH` 하나뿐이며, settings 의
//    `tui` 키가 부재하면 최종 판정을 서버측 기능게이트가 한다 — 즉 화면 모드는 OS 가 아니라
//    **계정·롤아웃**이 결정한다(저장소에 이 전제를 뒷받침하는 버전 핀·감지 코드는 0건이었다).
//    ②배선 변경: 그래서 main.ts 는 이제 Windows 에도 휠 핸들러를 등록한다(win 전용 술어로).
//    ※ 2026-08-17 후속: wheelgate.test.ts 의 옛 표현("win 미등록")도 **정정 완료**다. 무수정
//      계약의 대상은 기존 38 단언의 `expect` 식(회귀 감시선)이지 그것을 설명하는 산문이 아니다
//      — 거짓이 된 제목을 남기는 것이 감시선 보존이 아니라 결함이다. 단언은 한 줄도 바뀌지
//      않았고 제목·주석만 현재 사실에 맞췄다.
//  · 롤백 스위치(cysMouseReconcilerOff): main.ts 가 mac 핸들러를 등록하지 않는다. Windows
//    가드의 롤백은 이것이 아니라 별도 게이트다(아래 (d) — win-wheel-guard-off).

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

// ─────────────────────────────────────────────────────────────────────────────
// Windows 전용 휠 억제 술어 (스펙 C-2). 위 mac 경로와 **분리된 순수 함수**다.
//
// (a) 왜 별도 함수인가 — 위 shouldSuppressWheel 은 wheelgate.test.ts 의 38 단언(명명 6종 +
//     32조합 전수)이 한 조합 단위로 고정하고 있다. 술어에 항을 하나라도 끼워 넣으면 그
//     매트릭스가 통째로 재작성돼 회귀 감시선이 사라진다 — mac 경로의 검증된 계약(특히
//     less/man 의 방향키 합성 보존)을 무손실로 남기려면 새 술어를 옆에 세우는 편이 옳다.
//     ∴ 위 함수·인터페이스는 무수정, 아래는 추가만. 두 술어는 main.ts 에서 OS 로 분기한다.
//
// (b) 왜 1003 판별자인가 — mac 경로는 '장부에 h 가 하나라도 있으면'(ledgerWantsMouse) 이라
//     Windows 에 그대로 쓰면 vim mouse=a 까지 삼킨다. Windows 에서는 vim 휠(방향키 합성)이
//     현행 UX 이고 그 회귀가 mac 의 히스토리 오염보다 즉시 체감되므로 판별자를 좁힌다:
//       · Claude Code 는 fullscreen 진입 시 1000+1002+1003+1006("full")을 켠다 → 1003 이
//         반드시 장부에 오른다 → 술어 충족 → 억제.
//       · vim `mouse=a` 는 통상 1000/1002/1006 만 켠다(any-motion 불요) → 1003 부재 →
//         술어 불충족 → 방향키 합성 보존 = 현행 vim 휠 UX 무회귀.
//       · less/man 은 트래킹 자체를 안 켠다 → 장부 공백 → 불충족.
//     ★단일 방어선 고지(다중 방어가 아니다): `!xtermTracking` 항은 **Windows 에서 상수 true**
//     다 — trackfilter 는 win 인스턴스를 consume=false 로 만들고(`os==="mac" ∧ reconcile`),
//     소비 비활성 경로는 MOUSE_PARAMS(9·1000·1002·1003·1005·1006·1015·1016)를 전부
//     스트리핑하므로 xterm 은 Windows 에서 마우스 DECSET 을 **결코 보지 못한다** →
//     `term.modes.mouseTrackingMode` 는 항상 "none". 유일한 예외인 allowAppMouse=true 경로는
//     원문을 그대로 term.write 하지만, 그 경우 `!allowAppMouse` 항이 이미 술어를 끈다.
//     ∴ Windows 억제의 실질 판별자는 **1003 휴리스틱 하나뿐**이고, 그것이 (d) 의 미확정과
//     직결된다. 아래 테스트 진리표의 xterm=1 행들은 계약(전수 16조합)을 지키기 위한 것이지
//     Windows 에서 실제로 도달 가능한 상태가 아니다 — 그 행들을 근거로 "이중 방어가 있다"고
//     판단하지 마라.
//     ★★노출 시간의 비대칭(mac 경험으로 Windows 를 추정하지 마라 — 적대검증 2R 지적):
//       위 문단의 따름정리로, **mac 과 Windows 는 억제가 지속되는 시간이 근본적으로 다르다.**
//       · mac: alt 진입 즉시 정합기가 장부를 xterm 에 재생 주입하므로(consume=true)
//         `term.modes.mouseTrackingMode` 가 곧 "none" 이 아니게 되고 `!xtermTracking` 이 꺼진다
//         → 억제는 **주입이 파싱될 때까지의 수 ms 과도구간**뿐이다. mac 에서 "휠이 죽는다"는
//         신고가 없었던 것은 술어가 옳아서가 아니라 **노출 창이 사실상 없어서**일 수 있다.
//       · Windows: `!xtermTracking` 이 상수 true 이므로 해제항이 영원히 오지 않는다
//         → 억제는 **1003 이 장부에 오른 뒤 alt 인 동안 내내**다(그 alt 세션 전체).
//       ∴ mac 의 무사고 이력은 Windows 노출의 근거가 되지 못한다. Windows 는 검증되지 않은
//         **새 위험군**이고, 그래서 (d) 의 롤백 게이트와 실기 스모크가 릴리스 조건이다.
//       ※ 그 대신 '영구 고착'은 봉인돼 있다 — trackfilter 가 win 인스턴스에서 alt 이탈
//         파라미터(`?1049l`·47l·1047l)나 RIS 를 **관측**하면 장부를 비우므로(trackfilter.ts 의
//         `!consume` 분기 — 내부 alt 추적 상태와 무관해 재부착 pane 까지 덮는다), 전체화면 앱이
//         정상 종료·이탈하면 판정은 반드시 풀린다. 남는 잔여는 앱이 이탈 시퀀스를 못 보내고
//         즉사하는 경우뿐이고, 그 pane 은 이미 alt 화면에 갇힌 상태라 사용자 처방이
//         "pane 을 새로 여세요"로 동일하다(USER-MANUAL §4.6b 에 명기).
//     ★그래도 허용 가능한가 — **가능하다**고 판단한다(2026-08-17). 근거 셋:
//       ①`!xtermTracking` 이 상수인 것은 사고가 아니라 정합이다. 그 항의 의미는 "xterm 이 이미
//         트래킹에 들어갔으니 앱에 보고로 넘겨라"인데, Windows 에는 넘길 보고 경로가 애초에
//         없다(전량 스트리핑). 즉 빠진 것은 '억제 실패를 막는 방어'가 아니라 '과잉 억제를 푸는
//         해제항'이다 — 단일 판별자가 걸려 있는 위험 방향은 **과잉 억제** 한쪽뿐이다.
//       ②그 방향의 오차 비용은 되돌릴 수 있고 즉시 눈에 보인다(아래 관측 형태) — 반대 방향의
//         비용(프롬프트 히스토리 오염)처럼 조용히 누적되지 않는다.
//       ③다중 방어를 만들려면 win 에서도 xterm 이 마우스 DECSET 을 보게 해야 하는데, 그것은
//         ConPTY 결함 1호(마우스 보고 리터럴 타이핑)를 되살린다 = 치료가 병보다 나쁘다.
//         ∴ 단일 방어선은 게으름이 아니라 구조적 강제다.
//     ★실패 시 관측 형태(둘을 혼동하지 마라):
//       · 과잉 억제(1003 오판 — 예: vim 빌드가 1003 을 켠다): Windows 전체화면 TUI 에서 휠이
//         스크롤도 방향키도 아닌 **완전 무동작**. 사용자 신고 문구는 "휠이 아무것도 안 한다".
//       · 억제 실패(1003 을 켜지 않는 fullscreen 앱): 휠을 굴리면 프롬프트가 히스토리를 오간다
//         = 원 결함 재현. 이 경우 판별자를 1003 에서 넓히는 것이 수리 방향이다.
//     ★롤백 경로: 어느 쪽이든 최종 사용자는 (d) 의 게이트(CYS_WIN_WHEEL_GUARD_OFF=1 또는
//       ~/.cys/win-wheel-guard-off)로 이 가드 전체를 끄고 종전(무가드) 동작으로 돌아갈 수 있다.
//
// (c) ★pageMode(deltaMode=DOM_DELTA_PAGE) 항은 **싣지 않는다** — 2026-08-17 최종 결정.
//     초안 술어에는 `|| s.pageMode` 절대 상한이 있었고 진리표도 32행이었다. 성찰3 설계렌즈의
//     지적(롤백 결합)을 계기로 재검토해 **제거**했다. 아래는 그 판단의 전문이다(되살리려는
//     다음 조사자를 위해 근거를 남긴다 — 근거 없이 되돌리지 마라).
//     ★곱셈 자체는 **저장소 안에서 확인된 사실**이다(제거해도 이 사실은 그대로다) — 벤더 번들
//       node_modules/@xterm/xterm/lib/xterm.js 의 Viewport.getLinesScrolled 에
//       `e.deltaMode===WheelEvent.DOM_DELTA_PAGE&&(t*=this._bufferService.rows)` 가 있고,
//       스크롤백 없는 버퍼(=alt)의 wheel 경로가 `for(...e<Math.abs(t)...)` 로 CUU/CUD 를 그만큼
//       반복 전송한다. t = deltaY × scrollSensitivity × rows 이므로 |deltaY|=1·기본 감도면
//       80×24 화면에서 노치당 방향키 24개 남짓이다.
//     ★발생(=이 엔진이 PAGE 를 실제로 보고하는가)은 **[미확정]**이다. 저장소에 deltaMode 캡처·
//       계측은 0건이고, 이 UI 가 도는 엔진(Tauri = WebView2/WKWebView)이 PAGE 를 보고한다는
//       증거도 보고하지 않는다는 증거도 없다.
//     ★제거 근거 넷:
//       ①**방어선이 아니라 배율이다.** 원 결함의 피해자는 '방향키를 프롬프트 히스토리 탐색으로
//         소비하는 앱'(Claude Code 류)인데 그 부류는 alt 진입 시 1003 을 켠다 → 앞 항이 이미
//         전량 억제한다. 즉 이 항이 단독으로 덮는 영역은 "1003 을 켜지 않는데 방향키를 히스토리로
//         쓰는 앱 ∧ 엔진이 PAGE 보고" 라는 이중 가정의 사각뿐이다. 그런 앱이 있다면 LINE 모드에서
//         이미 (배율만 낮게) 오염되고 있으므로, 옳은 수리는 절대 상한이 아니라 **판별자 확장**이다.
//       ②**비용은 가정이 아니라 확정이다.** PAGE 를 보고하는 환경이면 1003 을 켜지 않는 전
//         앱(vim·less·man)의 alt 휠이 **전멸**한다. alt 엔 스크롤백이 없어 로컬 스크롤이라는
//         대안조차 없다 — 그 사용자에게 휠은 '아무것도 안 하는 키'가 된다.
//       ③**탈출구가 결합돼 있었다.** 그 피해의 유일한 스위치가 (d) 의 win-wheel-guard-off 인데
//         그것은 가드 전체를 끈다 = **원 결함 복원**. '페이저 휠이 죽는다'는 신고에 원 결함을
//         되살리는 것 말고 답이 없는 항은 릴리스에 실을 수 없다. 전용 게이트를 새로 파는 대안도
//         있었으나(커맨드·env·파일·문서·pane attach 의 invoke 왕복이 각각 하나씩 늘어난다)
//         **증거 0건인 항에 개념을 셋 더 세우는 것**이라 채택하지 않았다.
//       ④**릴리스 게이트 정합.** Windows 실기 스모크의 "less/man → 종전 스크롤" 항이 엔진의
//         deltaMode 보고에 의존하지 않게 된다(제거 전에는 그 항의 통과 여부가 엔진 사정이었다).
//     ★재도입 조건(그전에는 되돌리지 마라): 실환경 deltaMode 를 계측해 ⓐPAGE 가 실제로 관측되고
//       ⓑ1003 을 켜지 않으면서 방향키를 히스토리로 소비하는 앱의 실사례가 나올 것. ⓐ만으로는
//       근거가 되지 않는다(위 ①) — 그 경우 필요한 것은 상한이 아니라 그 앱을 판별자에 넣는 수리다.
//     ※ 계측 지시는 유효하다: Windows 실기에서 alt 화면 휠 이벤트의 deltaMode 분포를 한 번은
//        찍어 두라. 0건이면 이 문단은 영구 종결이고, 관측되면 위 ⓑ를 조사하라.
//     ※ 토큰버킷·자체 누산기·자체 라인 산술도 도입하지 않는다 — 그것을 정당화할 실기 데이터가
//        저장소에 없다. 여기 있는 것은 술어 하나뿐이다(스펙 C-2 명시 제약).
//
// (d) ★미확정 위험과 롤백 경로 — 이 설계의 최대 미확정은 (b) 의 'vim 은 1003 을 켜지 않는다'
//     이다. 저장소에 vim 의 실제 DECSET 캡처가 **0건**이라 문헌·관례 근거일 뿐 실측이 아니다.
//     vim 배포판·플러그인(ttymouse=sgr 등)이 1003 을 켜는 조합이 있다면 Windows vim 휠이
//     조용히 죽는다. ∴ 최종 사용자 롤백 탈출구를 코드 밖에 둔다: 환경변수
//     CYS_WIN_WHEEL_GUARD_OFF=1 또는 ~/.cys/win-wheel-guard-off 파일(Tauri 커맨드
//     win_wheel_guard_disabled — 릴리스 빌드엔 devtools 가 없어 localStorage 는 탈출구가 못 된다.
//     사용자 표면인 env·파일 이름은 종전 그대로다 — 바뀐 것은 내부 커맨드 이름뿐).
//     게이트가 꺼지면 wheelHandlerKind 가 "none" 을 돌려주고 main.ts 가 이 술어를 아예 호출하지
//     않는다(아래 배선 계층). 기존 allow-app-mouse 킬스위치를 탈출구로 재사용하면 안 된다 —
//     그것은 입·출력 양측을 열어 Windows ConPTY 결함 1호(마우스 보고 리터럴 타이핑)를 되살린다.
export interface WinWheelGateState {
  altActive: boolean; // term.buffer.active.type === "alternate"
  ledgerWantsAnyMotion: boolean; // 장부에 1003(any-motion) 활성(trackFilter.ledgerWantsAnyMotion())
  xtermTracking: boolean; // term.modes.mouseTrackingMode !== "none"
  allowAppMouse: boolean; // pane 생성 시 캡처된 킬스위치(라이브 재판독 금지)
}

// true = 휠 소비. 호출측 계약은 위 shouldSuppressWheel 과 동일하다(`!shouldSuppressWheelWin(...)`
// 를 커스텀 휠 핸들러의 반환값으로 → return false = xterm 기본 처리 차단).
// 항이 넷뿐인 것은 의도다 — deltaMode 상한을 뺀 이유는 위 (c).
export function shouldSuppressWheelWin(s: WinWheelGateState): boolean {
  return s.altActive && s.ledgerWantsAnyMotion && !s.xtermTracking && !s.allowAppMouse;
}

// ─────────────────────────────────────────────────────────────────────────────
// 배선 계층 — main.ts 가 들고 있던 판정을 순수 함수로 내린다(성찰3 테스트렌즈 major ×2).
//
// 왜 여기로 옮기는가: 술어(위 둘)와 장부 접근자(trackfilter)는 각각 변이 전건에 죽지만, **둘을
// 잇는 한 줄**이 틀리면 어느 단언도 울지 않았다. 실제로 가능했던 오배선 두 종류:
//   ⓐ `ledgerWantsAnyMotion()` 대신 인접 접근자 `ledgerWantsMouse()` 를 먹인다 → Windows vim
//      (1000/1002/1006)에서 술어가 충족돼 **이 파일이 지키려던 바로 그 계약**이 죽는다.
//   ⓑ OS/게이트 분기를 `else if` 에서 `if` 로 바꾼다 → 두 핸들러가 다 등록돼 뒤가 앞을 조용히
//      덮는다(attachCustomWheelEventHandler 는 인스턴스당 단일 슬롯) = mac 억제 소실.
// 둘 다 main.ts 인라인이라 bun test 로 고정할 수 없었다 — main.ts 는 DOM·xterm·Tauri invoke 에
// 묶여 있어 실행 재현 자체가 불가능하다. 그래서 판정만 이 파일로 내리고 main.ts 는 호출만 한다.

// xterm Terminal 에서 이 게이트가 읽는 부분만 추린 구조적 뷰(테스트가 페이크를 만들 수 있게).
// 필드명·리터럴("alternate"·"none")은 xterm 공개 API 계약이다. **휠 게이트가 읽는 두 값에
// 한해** 그 해석을 이 파일이 소유한다 — `mouseTrackingMode` 는 저장소에서 여기서만 읽고,
// `buffer.active.type` 은 main.ts 의 onData 마우스 필터(routeOnData 의 altScreen 인자)에도
// 한 곳 더 있다(그쪽은 입력측 경로라 이 게이트와 소비자가 다르다 — 억지로 합치지 마라).
export interface WheelTermView {
  buffer: { active: { type: string } };
  modes: { mouseTrackingMode: string };
}

// trackfilter 에서 이 게이트가 읽는 부분만 추린 구조적 뷰. **두 접근자를 모두** 요구하는 것이
// 핵심이다 — 하나만 요구하면 오배선(ⓐ)을 타입으로도 테스트로도 잡을 수 없다.
export interface WheelLedgerView {
  ledgerWantsMouse(): boolean; // mac 술어의 입력(장부에 h 가 하나라도)
  ledgerWantsAnyMotion(): boolean; // win 술어의 입력(장부에 1003)
}

// mac 술어 입력 조립. isWindows 는 호출측이 그대로 넘긴다 — 현 배선에서는 항상 false 이지만
// (wheelHandlerKind 가 "mac" 일 때만 호출되므로) 술어 인터페이스가 동결이라 항을 유지한다.
export function macGateInputs(
  term: WheelTermView,
  ledger: WheelLedgerView,
  allowAppMouse: boolean,
  isWindows: boolean,
): WheelGateState {
  return {
    altActive: term.buffer.active.type === "alternate",
    ledgerWantsMouse: ledger.ledgerWantsMouse(),
    xtermTracking: term.modes.mouseTrackingMode !== "none",
    allowAppMouse,
    isWindows,
  };
}

// win 술어 입력 조립. ledgerWantsAnyMotion(1003 한 비트)만 읽는다 — ledgerWantsMouse 를 읽으면
// vim 이 억제 대상이 되어 (b) 의 좁힌 판별자가 무의미해진다(테스트가 두 페이크로 고정한다).
export function winGateInputs(
  term: WheelTermView,
  ledger: WheelLedgerView,
  allowAppMouse: boolean,
): WinWheelGateState {
  return {
    altActive: term.buffer.active.type === "alternate",
    ledgerWantsAnyMotion: ledger.ledgerWantsAnyMotion(),
    xtermTracking: term.modes.mouseTrackingMode !== "none",
    allowAppMouse,
  };
}

// 어느 휠 핸들러를 등록할 것인가 — **배타성을 반환 타입으로 강제한다**. 값이 하나뿐이라
// "둘 다 등록되는" 상태가 구조적으로 표현 불가능하다(종전의 if/else-if 는 한 글자만 고쳐도
// 그 상태가 됐다). main.ts 는 이 값으로만 분기하고 OS·게이트를 다시 읽지 않는다.
//   · reconcile        = 정합기 롤백 스위치(localStorage cysMouseReconcilerOff="1" → false)
//   · winWheelGuardOff = Windows 휠 가드 롤백 게이트(env/파일 — (d))
// mac 은 정합기가 꺼지면 억제도 끈다(재생 주입이 없으면 술어의 전제가 성립하지 않는다).
// Windows 는 reconcile 을 보지 않는다 — 장부 기록은 소비와 무관하게 공통이고, 이 가드의
// 스위치는 winWheelGuardOff 하나다(둘을 섞으면 롤백 의미가 흐려진다).
export function wheelHandlerKind(s: {
  isWindows: boolean;
  reconcile: boolean;
  winWheelGuardOff: boolean;
}): "mac" | "win" | "none" {
  if (!s.isWindows) return s.reconcile ? "mac" : "none";
  return s.winWheelGuardOff ? "none" : "win";
}
