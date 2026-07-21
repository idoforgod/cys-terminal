// cast.test.ts — cast 순수 로직 단위 테스트(bun test). 브라우저 미기동.
// castRoute·parseClientMsg·mapInput·CAST_APP_HTML 마커.
import { test, expect } from "bun:test";
import {
  castRoute,
  parseClientMsg,
  mapInput,
  CAST_APP_HTML,
  CAST_PROTOCOL_VERSION,
  resolveCastParentOrigin,
  castContentSecurityPolicy,
  LatestFrameFlow,
  RECONNECT_GRACE_MS,
  reconnectGraceDecision,
  CastEmbedTicketRegistry,
} from "../cast";

const TOK = "abc123def456";

test("Cast embed policy — 플랫폼별 exact parent origin과 wildcard 없는 CSP/targetOrigin", () => {
  expect(CAST_PROTOCOL_VERSION).toBe(2);
  expect(resolveCastParentOrigin({ platform: "darwin", mode: "production", requested: "tauri://localhost" }))
    .toBe("tauri://localhost");
  expect(resolveCastParentOrigin({ platform: "win32", mode: "production", requested: "http://tauri.localhost" }))
    .toBe("http://tauri.localhost");
  expect(resolveCastParentOrigin({ platform: "darwin", mode: "production", requested: "http://tauri.localhost" }))
    .toBeNull();
  expect(resolveCastParentOrigin({
    platform: "darwin",
    mode: "development",
    requested: "http://localhost:1420",
    developmentOrigin: "http://localhost:1420",
  })).toBe("http://localhost:1420");
  expect(resolveCastParentOrigin({
    platform: "darwin",
    mode: "development",
    requested: "http://evil.example",
    developmentOrigin: "http://evil.example",
  })).toBeNull();

  const csp = castContentSecurityPolicy(51234, "tauri://localhost");
  expect(csp).toContain("frame-ancestors tauri://localhost");
  expect(csp).not.toContain("frame-ancestors 'self'");
  expect(csp).not.toContain("*");
  expect(CAST_APP_HTML).not.toContain("postMessage({ type: 'cys-cast-reconnect' }, '*')");
  expect(CAST_APP_HTML).not.toContain("}, '*')");
});

test("LatestFrameFlow — bounded latest-frame-wins와 fid별 CDP ack exactly-once", () => {
  const flow = new LatestFrameFlow<string>(1);
  expect(flow.offer(1, "cdp-1")).toEqual([]);
  expect(flow.offer(2, "cdp-2")).toEqual(["cdp-1"]); // 구 프레임은 즉시 drop+ack 대상
  expect(flow.size).toBe(1);
  expect(flow.acknowledge(1)).toBeNull(); // 이미 eviction으로 처리됨
  expect(flow.acknowledge(2)).toBe("cdp-2");
  expect(flow.acknowledge(2)).toBeNull(); // 같은 fid 두 번째 ack는 릴레이 금지
  expect(flow.size).toBe(0);

  flow.offer(3, "cdp-3");
  expect(flow.drain()).toEqual(["cdp-3"]); // detach에서도 미확인 프레임을 정확히 한 번 회수
  expect(flow.drain()).toEqual([]);
});

test("reconnectGraceDecision — 순간 WS 0명은 grace 동안 유지하고 경계 뒤 cleanup한다", () => {
  const emptySince = 10_000;
  expect(reconnectGraceDecision({ clientCount: 1, emptySince, now: 99_999 })).toBe("keep");
  expect(reconnectGraceDecision({ clientCount: 0, emptySince, now: emptySince + RECONNECT_GRACE_MS - 1 })).toBe("keep");
  expect(reconnectGraceDecision({ clientCount: 0, emptySince, now: emptySince + RECONNECT_GRACE_MS })).toBe("cleanup");
  expect(reconnectGraceDecision({ clientCount: 0, emptySince: null, now: emptySince + RECONNECT_GRACE_MS })).toBe("cleanup");
});

test("CastEmbedTicketRegistry — runtime/context/generation 결합, TTL, WS 1회 소비와 replay 차단", () => {
  const registry = new CastEmbedTicketRegistry(15_000, 32);
  const base = {
    ticket: "A".repeat(43),
    runtimeId: "runtime-a",
    context: "default",
    protocolVersion: CAST_PROTOCOL_VERSION,
    embedGeneration: 7,
    paneId: "pane".padEnd(64, "a"),
    parentOrigin: "tauri://localhost",
  };

  expect(registry.issue(base, 1_000)).toBe("issued");
  expect(registry.issue(base, 1_001)).toBe("duplicate"); // iframe URL replay도 거부
  expect(registry.consume({ ...base, embedGeneration: 8 }, 1_002)).toBe("mismatch");
  expect(registry.consume({ ...base, paneId: "other".padEnd(64, "b") }, 1_002)).toBe("mismatch");
  expect(registry.consume(base, 1_003)).toBe("accepted");
  expect(registry.consume(base, 1_004)).toBe("replayed");

  const expired = { ...base, ticket: "B".repeat(43), embedGeneration: 8 };
  expect(registry.issue(expired, 2_000)).toBe("issued");
  expect(registry.consume(expired, 17_000)).toBe("expired"); // 경계값은 만료
  expect(registry.size).toBeLessThanOrEqual(32);
});

// --- castRoute ---
test("castRoute: /<token>/cast/ → app (끝 슬래시 필수)", () => {
  expect(castRoute("/" + TOK + "/cast/")).toEqual({ kind: "app", token: TOK });
  // 설계 계약은 끝 슬래시 있는 형태뿐 — 없는 경로는 거부(엄격화).
  expect(castRoute("/" + TOK + "/cast")).toBeNull();
});

test("castRoute: /<token>/cast/ws → ws", () => {
  expect(castRoute("/" + TOK + "/cast/ws")).toEqual({ kind: "ws", token: TOK });
});

test("castRoute: 빈 토큰·세그먼트 초과·rpc 경로는 null", () => {
  expect(castRoute("//cast/")).toBeNull(); // 빈 토큰
  expect(castRoute("/" + TOK + "/cast/ws/extra")).toBeNull(); // 세그먼트 초과
  expect(castRoute("/" + TOK + "/rpc")).toBeNull(); // rpc 경로
  expect(castRoute("/cast/ws")).toBeNull(); // 토큰 없음(cast 가 첫 세그먼트)
  expect(castRoute("/")).toBeNull();
});

// --- parseClientMsg ---
test("parseClientMsg: 전 type 정상 통과", () => {
  expect(parseClientMsg(JSON.stringify({ type: "ack", fid: 3 }))).toEqual({ type: "ack", fid: 3 });
  expect(
    parseClientMsg(
      JSON.stringify({ type: "mouse", kind: "pressed", x: 10, y: 20, cw: 800, ch: 600, button: "left", clickCount: 1, modifiers: 8, deltaX: 0, deltaY: 0 })
    )
  ).toEqual({ type: "mouse", kind: "pressed", x: 10, y: 20, cw: 800, ch: 600, button: "left", clickCount: 1, modifiers: 8, deltaX: 0, deltaY: 0 });
  expect(parseClientMsg(JSON.stringify({ type: "key", kind: "down", key: "a", code: "KeyA", text: "a", modifiers: 0 }))).toEqual({
    type: "key",
    kind: "down",
    key: "a",
    code: "KeyA",
    text: "a",
    modifiers: 0,
  });
  expect(parseClientMsg(JSON.stringify({ type: "insertText", text: "안녕" }))).toEqual({ type: "insertText", text: "안녕" });
  expect(parseClientMsg(JSON.stringify({ type: "navigate", url: "https://x.com" }))).toEqual({ type: "navigate", url: "https://x.com" });
  const leaseId = "c".repeat(64);
  expect(parseClientMsg(JSON.stringify({ type: "control", action: "release", leaseId }))).toEqual({
    type: "control",
    action: "release",
    leaseId,
  });
});

test("parseClientMsg: 미지 type·불량 JSON은 null", () => {
  expect(parseClientMsg(JSON.stringify({ type: "explode" }))).toBeNull();
  expect(parseClientMsg("not json")).toBeNull();
  expect(parseClientMsg(JSON.stringify(42))).toBeNull();
  expect(parseClientMsg(JSON.stringify({ type: "control", action: "acquire", leaseId: "c".repeat(64) }))).toBeNull(); // release 만 허용
  expect(parseClientMsg(JSON.stringify({ type: "control", action: "release" }))).toBeNull(); // owner lease 증명 필수
});

test("parseClientMsg: 좌표 NaN·비유한은 null", () => {
  expect(parseClientMsg(JSON.stringify({ type: "mouse", kind: "moved", x: "NaN", y: 0, cw: 1, ch: 1 }))).toBeNull();
  expect(parseClientMsg(JSON.stringify({ type: "mouse", kind: "moved", x: null, y: 0, cw: 1, ch: 1 }))).toBeNull();
  // JSON 은 Infinity 를 못 실으므로 큰 유한값은 통과, 문자열 좌표는 거부
  expect(parseClientMsg('{"type":"mouse","kind":"moved","x":1e308,"y":0,"cw":1,"ch":1}')).not.toBeNull();
  expect(parseClientMsg(JSON.stringify({ type: "mouse", kind: "bogus", x: 0, y: 0, cw: 1, ch: 1 }))).toBeNull(); // kind enum
});

test("parseClientMsg: 길이 상한 초과는 null", () => {
  expect(parseClientMsg(JSON.stringify({ type: "insertText", text: "x".repeat(4097) }))).toBeNull();
  expect(parseClientMsg(JSON.stringify({ type: "key", kind: "down", key: "a", code: "KeyA", text: "x".repeat(4097) }))).toBeNull();
  expect(parseClientMsg(JSON.stringify({ type: "navigate", url: "x".repeat(2049) }))).toBeNull();
});

test("parseClientMsg: ack 는 fid 정수만 — sessionId 스키마는 거부", () => {
  expect(parseClientMsg(JSON.stringify({ type: "ack", fid: 1.5 }))).toBeNull(); // 정수 아님
  expect(parseClientMsg(JSON.stringify({ type: "ack", fid: "3" }))).toBeNull();
  // 구 스키마(sessionId)는 통과 금지 — CDP sessionId 는 세션 상수라 dedup 키로 쓰면 스트림이 스톨한다.
  expect(parseClientMsg(JSON.stringify({ type: "ack", sessionId: 3 }))).toBeNull();
});

// --- mapInput ---
test("mapInput: 정확 fit(letterbox 없음) 좌표 그대로", () => {
  const meta = { deviceWidth: 1280, deviceHeight: 800 };
  expect(mapInput({ x: 100, y: 200, cw: 1280, ch: 800 }, meta)).toEqual({ x: 100, y: 200 });
});

test("mapInput: 가로 letterbox(좌우 여백) 변환·경계 null", () => {
  const meta = { deviceWidth: 1280, deviceHeight: 800 };
  // canvas 1280×400 → scale 0.5, dispW 640, offX 320. 중앙 클릭.
  expect(mapInput({ x: 640, y: 200, cw: 1280, ch: 400 }, meta)).toEqual({ x: 640, y: 400 });
  expect(mapInput({ x: 100, y: 200, cw: 1280, ch: 400 }, meta)).toBeNull(); // 좌측 여백
  expect(mapInput({ x: 1000, y: 200, cw: 1280, ch: 400 }, meta)).toBeNull(); // 우측 여백
  expect(mapInput({ x: 320, y: 0, cw: 1280, ch: 400 }, meta)).toEqual({ x: 0, y: 0 }); // 좌상 모서리 경계
});

test("mapInput: 세로 letterbox(상하 여백) 변환·경계 null", () => {
  const meta = { deviceWidth: 1280, deviceHeight: 800 };
  // canvas 640×800 → scale 0.5, dispH 400, offY 200.
  expect(mapInput({ x: 320, y: 400, cw: 640, ch: 800 }, meta)).toEqual({ x: 640, y: 400 });
  expect(mapInput({ x: 320, y: 100, cw: 640, ch: 800 }, meta)).toBeNull(); // 상단 여백
});

test("mapInput: metadata 미보유·불량 크기는 null", () => {
  expect(mapInput({ x: 1, y: 1, cw: 100, ch: 100 }, { deviceWidth: 0, deviceHeight: 0 })).toBeNull();
  expect(mapInput({ x: 1, y: 1, cw: 0, ch: 0 }, { deviceWidth: 1280, deviceHeight: 800 })).toBeNull();
});

test("mapInput: 우/하 끝 경계는 배제 — 뷰포트 밖 매핑 금지", () => {
  const meta = { deviceWidth: 1280, deviceHeight: 800 };
  // x=1280 은 마지막 픽셀 다음 → 뷰포트 밖이므로 null 이어야 한다.
  expect(mapInput({ x: 1280, y: 400, cw: 1280, ch: 800 }, meta)).toBeNull();
  expect(mapInput({ x: 640, y: 800, cw: 1280, ch: 800 }, meta)).toBeNull();
  expect(mapInput({ x: 1279, y: 799, cw: 1280, ch: 800 }, meta)).toEqual({ x: 1279, y: 799 }); // 안쪽 끝은 유효
});

test("mapInput: offsetTop·pageScaleFactor 반영(항등값이면 결과 불변)", () => {
  const base = { deviceWidth: 1280, deviceHeight: 800 };
  // 항등값 — 기존 결과와 동일해야 한다.
  expect(mapInput({ x: 100, y: 200, cw: 1280, ch: 800 }, { ...base, offsetTop: 0, pageScaleFactor: 1 })).toEqual({ x: 100, y: 200 });
  // pageScaleFactor=2 (핀치줌) → CSS px 는 절반.
  expect(mapInput({ x: 100, y: 200, cw: 1280, ch: 800 }, { ...base, pageScaleFactor: 2 })).toEqual({ x: 50, y: 100 });
  // offsetTop=50 → 콘텐츠가 50DIP 아래에서 시작하므로 y 가 그만큼 당겨진다.
  expect(mapInput({ x: 100, y: 200, cw: 1280, ch: 800 }, { ...base, offsetTop: 50 })).toEqual({ x: 100, y: 150 });
  // offsetTop 위쪽(콘텐츠 영역 밖) 클릭은 무시.
  expect(mapInput({ x: 100, y: 10, cw: 1280, ch: 800 }, { ...base, offsetTop: 50 })).toBeNull();
});

// --- CAST_APP_HTML 마커 ---
test("CAST_APP_HTML: 필수 마커 존재", () => {
  for (const marker of ["canvas", "WebSocket", "insertText", "cys-cast-reconnect", "isComposing"]) {
    expect(CAST_APP_HTML).toContain(marker);
  }
});

test("CAST_APP_HTML: 인증 WS 뒤 SHELL_READY, 유효 canvas paint 뒤 FRAME_READY/LIVE를 보고한다", () => {
  expect(CAST_APP_HTML).toContain("phase: 'SHELL_READY'");
  expect(CAST_APP_HTML).toContain("phase: 'FRAME_READY'");
  expect(CAST_APP_HTML).toContain("phase: 'LIVE'");
  expect(CAST_APP_HTML).toContain("painted");
});
