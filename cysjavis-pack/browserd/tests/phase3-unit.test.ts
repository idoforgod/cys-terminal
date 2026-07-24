// phase3-unit.test.ts — Phase 3 순수 로직(브라우저 미기동).
// 메시지 계약(4-T-11) · 내비 URL 게이트(PRE-3) · 뷰포트 캡(4-T-5) · 에러 번역 · 앱 렌더 규칙(4-T-3).
import { test, expect } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import {
  parseClientMsg,
  navErrorMessage,
  navigableUrlError,
  clampViewportSide,
  fitViewport,
  fitHumanViewport,
  FIT_MIN_WIDTH,
  jpegQualityFor,
  mapInput,
  msgNav,
  msgTabs,
  msgErr,
  MSG,
  CMSG,
  VIEWPORT_MIN,
  VIEWPORT_MAX,
  VIEWPORT_MAX_AREA,
  UI_TEXT_CAP,
  CAST_APP_HTML,
} from "../cast";

// ════════ 4-T-11 메시지 계약: 타입명 단일 소스 ════════
test("4-T-11: S→C·C→S 타입명 상수가 CAST_APP_HTML 에 JSON 으로 주입된다(단일 소스)", () => {
  // 앱은 문자열 리터럴이 아니라 주입된 상수를 쓴다 — 서버가 이름을 바꾸면 앱도 함께 바뀐다.
  expect(CAST_APP_HTML).toContain(`var MSG = ${JSON.stringify(MSG)}`);
  expect(CAST_APP_HTML).toContain(`var CMSG = ${JSON.stringify(CMSG)}`);
  // 생성 헬퍼가 계약 필드를 빠짐없이 싣는다(필드 오타를 타입이 잡는 자리).
  const nav: any = msgNav({
    seq: 3,
    tabId: "t1",
    url: "http://x/",
    title: "T",
    canBack: true,
    canForward: false,
    loading: true,
    viewportPinned: false,
  });
  expect(nav.type).toBe(MSG.NAV);
  for (const k of ["seq", "tabId", "url", "title", "canBack", "canForward", "loading", "viewportPinned"]) {
    expect(Object.prototype.hasOwnProperty.call(nav, k)).toBe(true);
  }
  const tabs: any = msgTabs(7, [{ id: "t1", title: "a", url: "u", active: true }]);
  expect(tabs.type).toBe(MSG.TABS);
  expect(tabs.seq).toBe(7);
  expect(msgErr("X", "m", "d", true)).toEqual({ type: MSG.ERR, code: "X", message: "m", detail: "d", retry: true } as any);
});

// ════════ parseClientMsg 신규 type ════════
test("parseClientMsg: nav-action 은 4종 enum 만 통과", () => {
  for (const a of ["back", "forward", "reload", "stop"]) {
    expect(parseClientMsg(JSON.stringify({ type: CMSG.NAV_ACTION, action: a }))).toEqual({
      type: CMSG.NAV_ACTION,
      action: a,
    } as any);
  }
  expect(parseClientMsg(JSON.stringify({ type: CMSG.NAV_ACTION, action: "goto" }))).toBeNull();
  expect(parseClientMsg(JSON.stringify({ type: CMSG.NAV_ACTION }))).toBeNull();
});

test("parseClientMsg: tab 은 action enum + id 계약(미지 id 는 서버가 err)", () => {
  expect(parseClientMsg(JSON.stringify({ type: CMSG.TAB, action: "new" }))).toEqual({
    type: CMSG.TAB,
    action: "new",
    id: "",
  } as any);
  expect(parseClientMsg(JSON.stringify({ type: CMSG.TAB, action: "activate", id: "t3" }))).toEqual({
    type: CMSG.TAB,
    action: "activate",
    id: "t3",
  } as any);
  expect(parseClientMsg(JSON.stringify({ type: CMSG.TAB, action: "activate" }))).toBeNull(); // id 필수
  expect(parseClientMsg(JSON.stringify({ type: CMSG.TAB, action: "close", id: 3 }))).toBeNull();
  expect(parseClientMsg(JSON.stringify({ type: CMSG.TAB, action: "close", id: "x".repeat(65) }))).toBeNull();
  expect(parseClientMsg(JSON.stringify({ type: CMSG.TAB, action: "explode", id: "t1" }))).toBeNull();
});

test("parseClientMsg: viewport 는 유한 양수 + unpin 불리언", () => {
  expect(parseClientMsg(JSON.stringify({ type: CMSG.VIEWPORT, width: 900, height: 600 }))).toEqual({
    type: CMSG.VIEWPORT,
    width: 900,
    height: 600,
    unpin: false,
  } as any);
  expect(parseClientMsg(JSON.stringify({ type: CMSG.VIEWPORT, width: 900, height: 600, unpin: true }))).toEqual({
    type: CMSG.VIEWPORT,
    width: 900,
    height: 600,
    unpin: true,
  } as any);
  expect(parseClientMsg(JSON.stringify({ type: CMSG.VIEWPORT, width: 0, height: 600 }))).toBeNull();
  expect(parseClientMsg(JSON.stringify({ type: CMSG.VIEWPORT, width: "900", height: 600 }))).toBeNull();
});

test("parseClientMsg: dialog-reply 계약", () => {
  expect(parseClientMsg(JSON.stringify({ type: CMSG.DIALOG_REPLY, id: 1, action: "accept", text: "hi" }))).toEqual({
    type: CMSG.DIALOG_REPLY,
    id: 1,
    action: "accept",
    text: "hi",
  } as any);
  expect(parseClientMsg(JSON.stringify({ type: CMSG.DIALOG_REPLY, id: 1, action: "explode" }))).toBeNull();
  expect(parseClientMsg(JSON.stringify({ type: CMSG.DIALOG_REPLY, id: "1", action: "accept" }))).toBeNull();
});

// ════════ PRE-3 단일 내비 게이트 ════════
test("Browser v2 navigableUrlError: 제품 내비게이션은 http·https만 통과", () => {
  expect(navigableUrlError("http://example.com/")).toBeNull();
  expect(navigableUrlError("https://example.com/x?y=1")).toBeNull();
  expect(navigableUrlError("HtTps://EXAMPLE.com/")).toBeNull(); // 스킴 대소문자 무관
  expect(navigableUrlError("data:text/html,<h1>x</h1>")).toContain("거부");
  expect(navigableUrlError("about:blank")).toContain("거부"); // blank는 서버 내부 생성 경로만 사용
});

test("PRE-3 navigableUrlError: ★로컬 파일·특권 스킴은 거부(get text 로 읽어내는 경로 차단)", () => {
  // "cast 는 픽셀만 나가니 안전"이라는 Phase 2 논거는 조회 동사 도입 순간 무너진다 —
  // file:// 을 렌더한 뒤 get text 로 읽으면 그대로 유출된다.
  for (const bad of [
    "file:///Users/x/.ssh/id_rsa",
    "file:///etc/hosts",
    "javascript:alert(1)",
    "chrome://settings",
    "view-source:http://x/",
    "blob:http://x/abc",
    "about:config",
    "ftp://x/",
  ]) {
    expect(navigableUrlError(bad)).not.toBeNull();
  }
  expect(navigableUrlError("")).not.toBeNull();
  expect(navigableUrlError(null)).not.toBeNull();
  expect(navigableUrlError(123)).not.toBeNull();
  expect(navigableUrlError("notaurl")).not.toBeNull(); // 스킴 없는 상대 주소
  expect(navigableUrlError("https://x/" + "a".repeat(2100))).not.toBeNull(); // 길이 상한
});

// ════════ 4-T-5 뷰포트 캡 ════════
test("4-T-5 clampViewportSide: 1~4096 로 접고 반올림", () => {
  expect(clampViewportSide(900)).toBe(900);
  expect(clampViewportSide(0)).toBe(VIEWPORT_MIN);
  expect(clampViewportSide(-100)).toBe(VIEWPORT_MIN);
  expect(clampViewportSide(99999)).toBe(VIEWPORT_MAX);
  expect(clampViewportSide(800.6)).toBe(801);
});

test("4-T-5 fitViewport: ★면적 상한 — 변 클램프만으로는 4096×4096(16.7MP)이 통과해 동사 한 줄 DoS", () => {
  const big = fitViewport(4096, 4096);
  expect(big.width * big.height).toBeLessThanOrEqual(VIEWPORT_MAX_AREA);
  // 종횡비 유지 — 깨지면 mapInput letterbox 역변환과 화면이 어긋난다.
  expect(Math.abs(big.width / big.height - 1)).toBeLessThan(0.01);
});

test("4-T-5 fitViewport: 캡 이하는 그대로 · 종횡비 보존 · jpeg 품질 강등", () => {
  expect(fitViewport(1280, 800)).toEqual({ width: 1280, height: 800 });
  expect(fitViewport(900, 600)).toEqual({ width: 900, height: 600 });
  const wide = fitViewport(4000, 2000); // 8MP → 축소
  expect(wide.width * wide.height).toBeLessThanOrEqual(VIEWPORT_MAX_AREA);
  expect(Math.abs(wide.width / wide.height - 2)).toBeLessThan(0.05); // 2:1 유지
  // 큰 뷰포트일수록 품질이 내려간다(바이트 예산).
  expect(jpegQualityFor(1280, 800)).toBe(75);
  expect(jpegQualityFor(1600, 900)).toBeLessThan(75);
  expect(jpegQualityFor(1920, 1080)).toBeLessThanOrEqual(jpegQualityFor(1600, 900));
});

test("4-T-5 jpegQualityFor: fit-to-width 실사고 면적이 45 로 떨어지지 않고 60 하한 보장(2026-07-25 오너 실사고)", () => {
  // fit-to-width 실사고 뷰포트 1080×1747=1.886MP — 이전 사다리(>1.6MP→45)에서 흐렸다.
  expect(jpegQualityFor(1080, 1747)).toBe(60);
  // 회귀 방지: 기본 뷰포트 1280×800=1.024MP 는 75 불변.
  expect(jpegQualityFor(1280, 800)).toBe(75);
  // 안전망: 면적 캡(2.1MP) 초과 합성값은 45(fitViewport 가 실제로는 캡하므로 도달 불가).
  expect(jpegQualityFor(2000, 1500)).toBe(45); // 3.0MP > 2.1MP
});

// ════════ 4-T-5 확장: 사람 뷰포트 fit-to-width ════════
test("4-T-5 fitHumanViewport: 좁은 pane 은 데스크톱 폭까지 확대 — ★네이버 실사고 재현(810 폭에서 1080 전폭)", () => {
  // 오너 실사고: pane CSS 810px 폭에서 네이버(최소 ≈1080px)가 사이트 차원에서 우측 절단.
  // 이 수정의 정의 = 810 폭 pane 에서 1080 전폭이 보이게 되는 것.
  const fit = fitHumanViewport(810, 1310);
  // k=1080/810=1.3333 → h=round(1310×1080/810)=1747 · 면적 1080×1747=1.886MP<2.1MP → 그대로.
  expect(fit).toEqual({ width: FIT_MIN_WIDTH, height: 1747 });
});

test("4-T-5 fitHumanViewport: 넓은 pane 은 무확대(fitViewport 그대로 통과)", () => {
  expect(fitHumanViewport(1400, 900)).toEqual(fitViewport(1400, 900)); // {1400,900} — 면적 1.26MP
});

test("4-T-5 fitHumanViewport: 경계 — FIT_MIN_WIDTH 이상은 그대로, 1px 미만이면 확대 발동", () => {
  expect(fitHumanViewport(FIT_MIN_WIDTH, 800)).toEqual({ width: 1080, height: 800 }); // 경계 포함=무확대
  const narrow = fitHumanViewport(FIT_MIN_WIDTH - 1, 800); // 1079 → 확대 발동
  expect(narrow.width).toBe(FIT_MIN_WIDTH);
  // k=1080/1079 → h=round(800×1080/1079)=801 · 면적 865,080<2.1MP → 그대로.
  expect(narrow.height).toBe(801);
});

test("4-T-5 fitHumanViewport: 면적 캡 상호작용 — 확대 후 2.1MP 초과분은 fitViewport 가 종횡비 유지로 축소", () => {
  // (500,1300): k=1080/500=2.16 → h=round(1300×2.16)=2808 → fitViewport(1080,2808).
  //   면적 1080×2808=3,032,640>2,100,000 → k2=sqrt(2.1M/3.03264M)=0.8321459.
  //   width=floor(1080×0.8321459)=floor(898.72)=898 · height=floor(2808×0.8321459)=floor(2336.67)=2336.
  const fit = fitHumanViewport(500, 1300);
  expect(fit).toEqual({ width: 898, height: 2336 });
  // 면적 캡 준수.
  expect(fit.width * fit.height).toBeLessThanOrEqual(VIEWPORT_MAX_AREA);
  // 종횡비 보존(floor 로 ±1px 오차 이내로 1080/2808 을 유지 — 깨지면 mapInput letterbox 와 어긋난다).
  expect(Math.abs(fit.width / fit.height - FIT_MIN_WIDTH / 2808)).toBeLessThan(0.001);
});

test("4-T-5 fitHumanViewport: 퇴화 입력은 fitViewport 폴백(확대 없음)", () => {
  expect(fitHumanViewport(0, 500)).toEqual(fitViewport(0, 500));   // w<=0 → 폴백
  expect(fitHumanViewport(800, 0)).toEqual(fitViewport(800, 0));   // h<=0 → 폴백(확대 안 함)
});

// ════════ 4-T-5 reset 정합 — 에이전트 RPC viewport action=reset 도 사람 경로와 동일 fit 정책 ════════
// reset 경로는 순수함수로 분리돼 있지 않고 RPC 디스패치에 인라인이다 → 서버 배선을 grep 으로 검증한다.
test("4-T-5 reset 정합: viewport action=reset 은 fitHumanViewport 로 복원(사람 경로와 동일)·리터럴 계약은 불변", () => {
  const server = readFileSync(join(import.meta.dir, "..", "server.ts"), "utf8");
  // reset 블록 추출: `=== "reset"` 부터 그 블록의 return 까지.
  const start = server.indexOf('=== "reset"');
  expect(start).toBeGreaterThan(-1);
  const resetBlock = server.slice(start, server.indexOf("pinned: false", start));
  // reset=사람 뷰포트 복원 → fit-to-width 를 통과시켜 applyViewport 한다.
  expect(resetBlock).toContain("fitHumanViewport(");
  expect(resetBlock).toMatch(/applyViewport\(cid, bv\.width, bv\.height\)/);
  // 에이전트 리터럴 계약(width/height 지정)은 fit 없이 그대로 적용 — 회귀 0.
  expect(server).toContain("await applyViewport(cid, w, h)");
});

// ════════ 4-S-1-4 rebind 직후 입력 무시 ════════
test("4-S-1-4 mapInput: metadata 미보유(null)면 좌표를 만들지 않는다 — 오클릭보다 무시가 안전", () => {
  expect(mapInput({ x: 10, y: 10, cw: 100, ch: 100 }, null)).toBeNull();
  expect(mapInput({ x: 10, y: 10, cw: 100, ch: 100 }, { deviceWidth: 0, deviceHeight: 0 })).toBeNull();
  // 정상 metadata 는 기존과 동일하게 동작(회귀 0)
  expect(mapInput({ x: 100, y: 200, cw: 1280, ch: 800 }, { deviceWidth: 1280, deviceHeight: 800 })).toEqual({ x: 100, y: 200 });
});

// ════════ 에러 번역 ════════
test("navErrorMessage: 원인별로 문장이 갈린다(상수가 아니다)", () => {
  const dns = navErrorMessage("page.goto: net::ERR_NAME_NOT_RESOLVED at http://x.invalid/");
  const refused = navErrorMessage("net::ERR_CONNECTION_REFUSED");
  const cert = navErrorMessage("net::ERR_CERT_DATE_INVALID");
  const timeout = navErrorMessage("Timeout 30000ms exceeded.");
  expect(dns).toContain("찾을 수 없");
  expect(refused).toContain("거부");
  expect(cert).toContain("인증서");
  expect(timeout).toContain("시간");
  expect(new Set([dns, refused, cert, timeout]).size).toBe(4);
  expect(navErrorMessage("something unexpected")).toBe("페이지를 열지 못했습니다.");
});

// ════════ 4-T-3 cast 앱 렌더 규칙(토큰 보유 origin XSS 차단) ════════
test("4-T-3: cast 앱은 innerHTML 을 쓰지 않는다 — 탭 제목은 페이지가 통제하는 문자열이다", () => {
  // img-src 가 data: 뿐이라 <img src=x> 로드는 반드시 실패하고 → onerror 가 확실히 발화한다.
  // 그 실행 위치가 WS 토큰을 쥔 origin 이라 임의 내비게이션·입력 주입이 전부 열린다.
  expect(CAST_APP_HTML).not.toContain("innerHTML");
  expect(CAST_APP_HTML).not.toContain("insertAdjacentHTML");
  expect(CAST_APP_HTML).not.toContain("outerHTML");
  expect(CAST_APP_HTML).not.toContain("document.write");
  // 렌더는 textContent 로만 — 탭 제목·에러·다이얼로그 지점이 전부 그렇다.
  expect(CAST_APP_HTML).toContain("s.textContent = cap(");
  expect(CAST_APP_HTML).toContain("bMsg.textContent =");
  expect(CAST_APP_HTML).toContain("dlgMsg.textContent =");
  expect(CAST_APP_HTML).toContain(`var UI_CAP = ${JSON.stringify(UI_TEXT_CAP)}`); // 길이 캡 주입
});

test("Phase 2 앱 마커 유지 + Phase 3 UI 마커 존재(회귀 0)", () => {
  for (const marker of ["canvas", "WebSocket", "insertText", "cys-cast-reconnect", "isComposing"]) {
    expect(CAST_APP_HTML).toContain(marker);
  }
  for (const marker of ["renderTabs", "applyNav", "progbar", "ResizeObserver", "showBanner", "showDialog", "seenSeq", "vunpin"]) {
    expect(CAST_APP_HTML).toContain(marker);
  }
  // 뒤로/앞으로는 기본 비활성(서버가 알려주기 전에 눌려 빈 화면으로 가지 않게).
  expect(CAST_APP_HTML).toContain('id="back" title="뒤로" disabled');
  expect(CAST_APP_HTML).toContain('id="fwd" title="앞으로" disabled');
  // 4-T-5: 클라이언트는 stage CSS 크기만 측정한다(프레임 metadata 파생값이면 무한 루프).
  expect(CAST_APP_HTML).toContain("stage.clientWidth");
  expect(CAST_APP_HTML).not.toContain("deviceWidth");
  // fitCanvas 는 캔버스 백킹스토어에 DPR 을 반영한다(레티나 흐림 해소·2026-07-25) — 브라우저
  // 전용 인라인이라 단위테스트 불가, CAST_APP_HTML 문자열에 배선됐는지 grep 으로 검증한다.
  expect(CAST_APP_HTML).toContain("devicePixelRatio");
});

// ════════════════════════════════════════════════════════════════════════
// 4-T-10 ③ 결정론 exit 계약 — 서버 에러 코드 ↔ CLI EXIT_BY_ERROR ↔ README 3면 일치
// ════════════════════════════════════════════════════════════════════════

// ★"미등재는 전부 exit 1 로 뭉개져 결정론 계약이 죽는다"(4-T-10 ③)를 **자동으로** 잡는다.
//   숫자 3개를 박아 두는 핀은 다음에 코드가 늘면 또 뚫린다 — 규칙 자체를 핀으로 만든다.
test("4-T-10 ③: 서버가 던지는 모든 RpcError 코드가 CLI exit 표에 등재돼 있다(고유 번호·README 일치)", () => {
  const server = readFileSync(join(import.meta.dir, "..", "server.ts"), "utf8");
  const cli = readFileSync(join(import.meta.dir, "..", "..", "bin", "javis_browser.py"), "utf8");
  const readme = readFileSync(join(import.meta.dir, "..", "README.md"), "utf8");

  const thrown = new Set([...server.matchAll(/new RpcError\("([A-Z_]+)"/g)].map((m) => m[1]));
  expect(thrown.size).toBeGreaterThan(10); // 정규식이 헛돌면 빈 집합으로 '통과'한다 — 하한을 건다

  const table = cli.slice(cli.indexOf("EXIT_BY_ERROR = {"), cli.indexOf("}", cli.indexOf("EXIT_BY_ERROR = {")));
  const registered = new Map<string, number>(
    [...table.matchAll(/"([A-Z_]+)":\s*(\d+)/g)].map((m) => [m[1], Number(m[2])] as [string, number])
  );

  const missing = [...thrown].filter((c) => !registered.has(c));
  console.log(`[4-T-10③] 서버 코드 ${thrown.size}종 · CLI 등재 ${registered.size}종 · 미등재 ${missing.length}종`);
  expect(missing).toEqual([]); // 미등재 = 그 거부가 exit 1 로 뭉개진다

  // 보안 거부 3종은 **고유 코드**여야 한다(같은 위반이 방향에 따라 다른 코드로 갈리지 않게 함께).
  for (const code of ["HUMAN_CID_RESERVED", "HUMAN_CID_REQUIRED", "PROFILE_MISMATCH"]) {
    expect(registered.has(code)).toBe(true);
    expect(registered.get(code)).toBeGreaterThan(1); // 1(기타)이면 등재의 의미가 없다
    expect(readme).toContain(code); // 3면 일치 — README exit 표에도 있다
  }
  // 번호 충돌 금지: BUSY(2) 와 argparse(9) 충돌이 이 계약이 생긴 이유다.
  const nums = [...registered.values()];
  const dup = nums.filter((n, i) => nums.indexOf(n) !== i && n !== 9); // 9 는 사용례 오류 계열 공유(의도)
  expect(dup).toEqual([]);
});
