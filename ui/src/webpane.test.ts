// webpane.ts 순수 로직 회귀 테스트 (bun test — 신규 의존성 0).
//
// ①레이아웃 직렬화 왕복(web 노드 포함) ②v2→v3 마이그레이션 ③다운그레이드 불변식
// ④URL 하드 가드. DOM(iframe·타이틀 스트립)은 main.ts WebPaneView가 담당하고, 여기선
// 검증 대상인 순수 판단부만 돌린다.
import { describe, it, expect } from "bun:test";
import {
  LAYOUT_KEY_V2,
  LAYOUT_KEY_V3,
  isAllowedWebPaneUrl,
  makeWebNode,
  viewerAppUrl,
  extractViewerPath,
  decideViewerOpen,
  collectWebPaths,
  loadPersistedLayout,
  persistLayout,
  collectWebWids,
  castAppUrl,
  isCastUrl,
  castContextOf,
  castFailureReason,
  castPaneTitle,
  castDisplayUrl,
  isLoopbackOrigin,
  CAST_PROTOCOL_VERSION,
  acceptsCastMessage,
  castParentOrigin,
  newCastEmbedTicket,
  initialCastPaneState,
  reduceCastPaneState,
  castPhaseEvent,
  sanitizeCastNodes,
  sanitizeLayoutForPersist,
  collectWebUrls,
} from "./webpane";

// 테스트용 in-memory localStorage 대역(getItem/setItem만).
function fakeStore(init: Record<string, string> = {}) {
  const m = new Map<string, string>(Object.entries(init));
  return {
    getItem: (k: string) => (m.has(k) ? m.get(k)! : null),
    setItem: (k: string, v: string) => void m.set(k, v),
    raw: m,
  };
}

// web 노드가 섞인 레이아웃 트리(split 아래 터미널 pane + web pane).
function layoutWithWeb() {
  return {
    workspaces: [
      {
        id: 1,
        name: "ws1",
        tree: {
          type: "split",
          dir: "row",
          ratio: 0.5,
          a: { type: "pane", sid: 7 },
          b: makeWebNode(3, "http://127.0.0.1:51234/tok/app/?path=%2Ftmp%2Fa.md", "a.md"),
        },
      },
    ],
    groups: [],
    active: 0,
    counter: 2,
    groupCounter: 1,
  };
}

describe("① 레이아웃 직렬화 왕복 — web 노드 보존", () => {
  it("web 노드 포함 트리를 v3에 저장→로드하면 동일하다", () => {
    const s = fakeStore();
    const data = layoutWithWeb();
    persistLayout(s.setItem, data);
    const loaded = loadPersistedLayout(s.getItem);
    expect(loaded).toEqual(data);
    // web 노드 필드가 JSON 왕복에서 유실되지 않았는지 직접 확인
    const web = loaded.workspaces[0].tree.b;
    expect(web).toEqual({
      type: "web",
      wid: 3,
      url: "http://127.0.0.1:51234/tok/app/?path=%2Ftmp%2Fa.md",
      title: "a.md",
    });
  });

  it("저장은 v3 키에만 쓴다(v2 미기록)", () => {
    const s = fakeStore();
    persistLayout(s.setItem, layoutWithWeb());
    expect(s.raw.has(LAYOUT_KEY_V3)).toBe(true);
    expect(s.raw.has(LAYOUT_KEY_V2)).toBe(false);
  });
});

describe("② v2→v3 마이그레이션", () => {
  it("v3 없고 v2만 있으면 v2를 읽어온다(passthrough)", () => {
    const v2data = { workspaces: [{ id: 1, name: "old", tree: { type: "pane", sid: 1 } }], active: 0 };
    const s = fakeStore({ [LAYOUT_KEY_V2]: JSON.stringify(v2data) });
    const loaded = loadPersistedLayout(s.getItem);
    expect(loaded).toEqual(v2data);
  });

  it("v2 로드 후 v3로 저장해도 v2 원본은 그대로 보존된다", () => {
    const v2raw = JSON.stringify({ workspaces: [{ id: 1, name: "old", tree: null }], active: 0 });
    const s = fakeStore({ [LAYOUT_KEY_V2]: v2raw });
    const loaded = loadPersistedLayout(s.getItem);
    persistLayout(s.setItem, { ...loaded, migrated: true });
    // v3 신규 기록
    expect(s.raw.has(LAYOUT_KEY_V3)).toBe(true);
    // v2 원본은 바이트 단위로 불변
    expect(s.raw.get(LAYOUT_KEY_V2)).toBe(v2raw);
  });

  it("v3가 있으면 v2를 무시하고 v3를 읽는다(v3 우선)", () => {
    const s = fakeStore({
      [LAYOUT_KEY_V2]: JSON.stringify({ tag: "v2" }),
      [LAYOUT_KEY_V3]: JSON.stringify({ tag: "v3" }),
    });
    expect(loadPersistedLayout(s.getItem)).toEqual({ tag: "v3" });
  });

  it("손상 저장본은 null(폴백)", () => {
    expect(loadPersistedLayout(fakeStore({ [LAYOUT_KEY_V3]: "{bad" }).getItem)).toBeNull();
    expect(loadPersistedLayout(fakeStore().getItem)).toBeNull();
  });

  it("손상 v3 + 정상 v2 → v2로 폴백 복원한다(F5)", () => {
    const v2data = { workspaces: [{ id: 1, name: "before-upgrade", tree: null }], active: 0 };
    const s = fakeStore({
      [LAYOUT_KEY_V3]: "{corrupt json",
      [LAYOUT_KEY_V2]: JSON.stringify(v2data),
    });
    // v3 손상이어도 전손실(null) 대신 v2 스냅샷으로 부팅한다
    expect(loadPersistedLayout(s.getItem)).toEqual(v2data);
  });

  it("손상 v3 + 손상 v2 → null(최종 폴백)", () => {
    const s = fakeStore({ [LAYOUT_KEY_V3]: "{bad", [LAYOUT_KEY_V2]: "{also bad" });
    expect(loadPersistedLayout(s.getItem)).toBeNull();
  });
});

describe("③ 다운그레이드 불변식 — 구 빌드는 v2를 읽는다", () => {
  it("신 빌드가 v3에 써도, v2만 읽는 구 빌드는 업그레이드 전 스냅샷을 본다", () => {
    const v2raw = JSON.stringify({ workspaces: [{ id: 1, name: "before-upgrade", tree: null }], active: 0 });
    const s = fakeStore({ [LAYOUT_KEY_V2]: v2raw });
    // 신 빌드: v2 마이그레이션 로드 → web 포함 레이아웃을 v3에 저장
    loadPersistedLayout(s.getItem);
    persistLayout(s.setItem, layoutWithWeb());
    // 구 빌드 시뮬레이션: v2 키만 읽는다
    const oldBuildRead = JSON.parse(s.getItem(LAYOUT_KEY_V2)!);
    expect(oldBuildRead).toEqual(JSON.parse(v2raw));
    // v3는 존재하지만 구 빌드는 이 키를 모른다
    expect(s.raw.has(LAYOUT_KEY_V3)).toBe(true);
  });
});

describe("④ URL 하드 가드", () => {
  it("허용: loopback+포트+경로", () => {
    expect(isAllowedWebPaneUrl("http://127.0.0.1:51234/tok/app/?path=x")).toBe(true);
    expect(isAllowedWebPaneUrl("http://localhost:8642/app/")).toBe(true);
  });
  it("차단: https·file·임의 host·포트없음·userinfo·port위장", () => {
    expect(isAllowedWebPaneUrl("https://127.0.0.1:51234/app/")).toBe(false);
    expect(isAllowedWebPaneUrl("file:///etc/passwd")).toBe(false);
    expect(isAllowedWebPaneUrl("http://evil.com/app/")).toBe(false);
    expect(isAllowedWebPaneUrl("http://127.0.0.1/app/")).toBe(false); // 포트 없음
    expect(isAllowedWebPaneUrl("http://localhost/app/")).toBe(false); // 포트 없음
    expect(isAllowedWebPaneUrl("http://127.0.0.1:80@evil.example.com/")).toBe(false); // userinfo 위장
    expect(isAllowedWebPaneUrl("http://127.0.0.1:80.evil.com/")).toBe(false); // port 위장
    expect(isAllowedWebPaneUrl("http://127.0.0.1.evil.com:80/")).toBe(false); // host 위장
    expect(isAllowedWebPaneUrl("")).toBe(false);
  });
});

describe("⑤ web pane 정리(teardown) — collectWebWids", () => {
  it("split 아래 섞인 web 노드 wid를 전부 수집한다", () => {
    const tree = {
      type: "split",
      dir: "row",
      ratio: 0.5,
      a: makeWebNode(3, "http://127.0.0.1:1/tok/app/?path=a", "a"),
      b: {
        type: "split",
        dir: "col",
        ratio: 0.5,
        a: { type: "pane", sid: 7 }, // 터미널 pane은 건너뛴다
        b: makeWebNode(9, "http://127.0.0.1:1/tok/app/?path=b", "b"),
      },
    };
    expect(collectWebWids(tree).sort((x, y) => x - y)).toEqual([3, 9]);
  });

  it("web 노드 없는 트리(터미널만)는 빈 배열", () => {
    const tree = { type: "split", dir: "row", ratio: 0.5, a: { type: "pane", sid: 1 }, b: { type: "pane", sid: 2 } };
    expect(collectWebWids(tree)).toEqual([]);
  });

  it("단일 web 리프·null 처리", () => {
    expect(collectWebWids(makeWebNode(5, "http://127.0.0.1:1/t/app/?path=x"))).toEqual([5]);
    expect(collectWebWids(null)).toEqual([]);
  });
});

describe("보조 — URL 조립·경로 회수 왕복", () => {
  it("viewerAppUrl은 가드를 통과하고 extractViewerPath로 원 경로가 복원된다", () => {
    const url = viewerAppUrl(51234, "tok-en_123", "/tmp/report file.md");
    expect(isAllowedWebPaneUrl(url)).toBe(true);
    expect(extractViewerPath(url)).toBe("/tmp/report file.md");
  });
  it("extractViewerPath는 비URL에 null", () => {
    expect(extractViewerPath("not a url")).toBeNull();
  });
});

describe("viewer.open 이벤트 판정 — decideViewerOpen", () => {
  const base = {
    path: "/r/_round/report.md",
    existingPaths: [] as (string | null)[],
    paneCount: 0,
    maxPanes: 8,
    eventEpoch: 1000,
    nowEpoch: 1001,
    maxAgeSecs: 30,
    wsReady: true,
  };
  it("정상 이벤트는 open", () => {
    expect(decideViewerOpen(base)).toBe("open");
  });
  it("워크스페이스 미준비(pending)는 not-ready — 무음 드롭 금지의 전제", () => {
    expect(decideViewerOpen({ ...base, wsReady: false })).toBe("not-ready");
  });
  it("maxAgeSecs 초과 이벤트는 stale — 재접속 replay의 스테일 pane 부활 차단", () => {
    expect(decideViewerOpen({ ...base, eventEpoch: 1000, nowEpoch: 1031 })).toBe("stale");
    // 경계: 정확히 maxAgeSecs까지는 유효
    expect(decideViewerOpen({ ...base, eventEpoch: 1000, nowEpoch: 1030 })).toBe("open");
  });
  it("같은 경로 pane이 이미 있으면 dup — 중복 pane 미생성", () => {
    expect(
      decideViewerOpen({ ...base, existingPaths: ["/other.md", "/r/_round/report.md"], paneCount: 2 }),
    ).toBe("dup");
    // extractViewerPath 실패분(null)이 섞여도 판정은 무사하다
    expect(decideViewerOpen({ ...base, existingPaths: [null, "/other.md"], paneCount: 2 })).toBe("open");
  });
  it("pane 총량 상한 도달 시 cap — pane 홍수의 UI측 방벽", () => {
    expect(decideViewerOpen({ ...base, paneCount: 8 })).toBe("cap");
  });
  it("collectWebPaths — 트리에서 web 노드 뷰어 경로만 수집(dup 판정의 현재-ws 범위 재료)", () => {
    const w1 = makeWebNode(1, viewerAppUrl(1, "t", "/a.md"));
    const w2 = makeWebNode(2, viewerAppUrl(1, "t", "/b.md"));
    const tree = { type: "split", dir: "row", a: { type: "pane", sid: 7 }, b: { type: "split", dir: "col", a: w1, b: w2 } };
    expect(collectWebPaths(tree)).toEqual(["/a.md", "/b.md"]);
    expect(collectWebPaths(null)).toEqual([]);
    // 비URL(손상 저장본)은 null로 수집 — decideViewerOpen 판정은 null 혼입에 무사(기존 핀과 정합)
    expect(collectWebPaths({ type: "web", wid: 3, url: "not a url" })).toEqual([null]);
  });
  it("판정 우선순위: not-ready > stale > dup > cap", () => {
    const all = {
      ...base,
      wsReady: false,
      nowEpoch: 9999,
      existingPaths: [base.path],
      paneCount: 8,
    };
    expect(decideViewerOpen(all)).toBe("not-ready");
    expect(decideViewerOpen({ ...all, wsReady: true })).toBe("stale");
    expect(decideViewerOpen({ ...all, wsReady: true, nowEpoch: 1001 })).toBe("dup");
    expect(decideViewerOpen({ ...all, wsReady: true, nowEpoch: 1001, existingPaths: [] })).toBe("cap");
  });
});

describe("cast web pane — URL 조립·판정(browserd screencast)", () => {
  it("Cast Pane 상태 머신 — load/spawn은 성공이 아니며 SHELL_READY 뒤 실제 painted frame만 열린다", () => {
    let s = initialCastPaneState(11);
    expect(s.phase).toBe("PENDING");

    // iframe load 자체는 성공 신호가 아니며 상태를 바꾸지 않는다.
    s = reduceCastPaneState(s, { type: "IFRAME_LOADED", generation: 11 });
    expect(s.phase).toBe("PENDING");

    // 순서를 건너뛴 FRAME_READY는 fail-closed다.
    const skipped = reduceCastPaneState(s, { type: "FRAME_READY", generation: 11 });
    expect(skipped).toMatchObject({ phase: "FAILED", errorCode: "PROTOCOL_ORDER" });

    s = reduceCastPaneState(s, { type: "SHELL_READY", generation: 11 });
    expect(s.phase).toBe("SHELL_READY");
    s = reduceCastPaneState(s, { type: "FRAME_READY", generation: 11 });
    expect(s.phase).toBe("FRAME_READY");
    expect(s.paintedFrames).toBe(1);
    s = reduceCastPaneState(s, { type: "LIVE", generation: 11 });
    expect(s.phase).toBe("LIVE");

    // 이전 document의 늦은 메시지는 현재 generation을 절대 움직이지 못한다.
    expect(reduceCastPaneState(s, { type: "CLOSED", generation: 10 })).toBe(s);
  });

  it("Cast Pane 실패 전이 — 셸/첫 프레임 timeout을 구분하고 live 단절은 마지막 화면을 유지한다", () => {
    const pending = initialCastPaneState(3);
    expect(reduceCastPaneState(pending, { type: "TIMEOUT", generation: 3 })).toMatchObject({
      phase: "FAILED",
      errorCode: "SHELL_TIMEOUT",
    });

    const shell = reduceCastPaneState(pending, { type: "SHELL_READY", generation: 3 });
    expect(reduceCastPaneState(shell, { type: "TIMEOUT", generation: 3 })).toMatchObject({
      phase: "FAILED",
      errorCode: "FIRST_FRAME_TIMEOUT",
    });

    const frame = reduceCastPaneState(shell, { type: "FRAME_READY", generation: 3 });
    const degraded = reduceCastPaneState(frame, { type: "DISCONNECTED", generation: 3 });
    expect(degraded).toMatchObject({ phase: "DEGRADED", paintedFrames: 1, errorCode: "WS_DISCONNECTED" });
    expect(reduceCastPaneState(degraded, { type: "SHELL_READY", generation: 3 }).phase).toBe("SHELL_READY");

    expect(reduceCastPaneState(pending, {
      type: "FAIL",
      generation: 3,
      errorCode: "PROTOCOL_MISMATCH",
    })).toMatchObject({ phase: "FAILED", errorCode: "PROTOCOL_MISMATCH" });
  });

  it("Cast phase payload — 허용 phase와 bounded error code만 상태 이벤트로 변환한다", () => {
    expect(castPhaseEvent({ phase: "SHELL_READY" }, 5)).toEqual({ type: "SHELL_READY", generation: 5 });
    expect(castPhaseEvent({ phase: "FRAME_READY" }, 5)).toEqual({ type: "FRAME_READY", generation: 5 });
    expect(castPhaseEvent({ phase: "LIVE" }, 5)).toEqual({ type: "LIVE", generation: 5 });
    expect(castPhaseEvent({ phase: "DEGRADED" }, 5)).toEqual({ type: "DISCONNECTED", generation: 5 });
    expect(castPhaseEvent({ phase: "FAILED", errorCode: "CDP_FAILED" }, 5)).toEqual({
      type: "FAIL",
      generation: 5,
      errorCode: "CDP_FAILED",
    });
    expect(castPhaseEvent({ phase: "FAILED", errorCode: "x".repeat(65) }, 5)).toBeNull();
    expect(castPhaseEvent({ phase: "PWNED" }, 5)).toBeNull();
  });

  it("Cast Protocol v2 — 현재 embed generation·정확 origin·실제 iframe source만 수용한다", () => {
    const ticket = "a".repeat(64);
    const url = castAppUrl(51234, "tok", "default", {
      protocolVersion: CAST_PROTOCOL_VERSION,
      embedGeneration: 7,
      parentOrigin: "tauri://localhost",
      embedTicket: ticket,
    });
    const good = {
      frameUrl: url,
      eventOrigin: "http://127.0.0.1:51234",
      sourceMatches: true,
      data: { type: "cys-cast-title", protocolVersion: CAST_PROTOCOL_VERSION, embedGeneration: 7, embedTicket: ticket },
    };

    expect(acceptsCastMessage(good)).toBe(true);
    expect(acceptsCastMessage({ ...good, eventOrigin: "http://localhost:51234" })).toBe(false);
    expect(acceptsCastMessage({ ...good, sourceMatches: false })).toBe(false);
    expect(acceptsCastMessage({ ...good, data: { ...good.data, protocolVersion: 1 } })).toBe(false);
    expect(acceptsCastMessage({ ...good, data: { ...good.data, embedGeneration: 6 } })).toBe(false);
    expect(acceptsCastMessage({ ...good, data: { ...good.data, embedTicket: "b".repeat(64) } })).toBe(false);
    expect(new URL(url).searchParams.get("embedTicket")).toBe(ticket);
    expect(newCastEmbedTicket()).toMatch(/^[a-f0-9]{64}$/);
  });

  it("castParentOrigin — 패키징 Tauri와 명시 loopback 개발 origin만 exact 값으로 만든다", () => {
    expect(castParentOrigin({ protocol: "tauri:", host: "localhost", origin: "null" })).toBe("tauri://localhost");
    expect(castParentOrigin({ protocol: "http:", host: "tauri.localhost", origin: "http://tauri.localhost" }))
      .toBe("http://tauri.localhost");
    expect(castParentOrigin({ protocol: "http:", host: "localhost:1420", origin: "http://localhost:1420" }))
      .toBe("http://localhost:1420");
    expect(castParentOrigin({ protocol: "https:", host: "evil.example", origin: "https://evil.example" })).toBeNull();
  });

  it("castAppUrl 형식 — loopback+토큰+/cast/+context, context는 encodeURIComponent", () => {
    expect(castAppUrl(51234, "tok-en_123", "default")).toBe(
      "http://127.0.0.1:51234/tok-en_123/cast/?context=default",
    );
    // context 특수문자는 인코딩(주소 위장 방지)
    expect(castAppUrl(8080, "t", "부서 1/a")).toBe("http://127.0.0.1:8080/t/cast/?context=%EB%B6%80%EC%84%9C%201%2Fa");
  });

  it("castAppUrl은 isAllowedWebPaneUrl 하드가드를 통과한다(loopback+포트+경로)", () => {
    expect(isAllowedWebPaneUrl(castAppUrl(51234, "tok", "default"))).toBe(true);
    expect(isAllowedWebPaneUrl(castAppUrl(0, "pending", "default"))).toBe(true); // pending도 통과
  });

  it("isCastUrl — 정상 cast=true·pending=true·뷰어 URL=false·손상=false", () => {
    expect(isCastUrl(castAppUrl(51234, "tok", "default"))).toBe(true);
    expect(isCastUrl("http://127.0.0.1:0/pending/cast/?context=default")).toBe(true); // 조립 전 pending
    expect(isCastUrl("http://127.0.0.1:51234/tok/cast/ws?context=default")).toBe(true); // ws 경로도 cast 계열
    expect(isCastUrl(viewerAppUrl(51234, "tok", "/tmp/a.md"))).toBe(false); // 뷰어(2번째="app")
    expect(isCastUrl("not a url")).toBe(false); // 손상
    expect(isCastUrl("http://127.0.0.1:51234/")).toBe(false); // 세그먼트 부족
  });

  it("castContextOf — 지정 context 회수·없으면 default", () => {
    expect(castContextOf(castAppUrl(51234, "tok", "dept-2"))).toBe("dept-2");
    expect(castContextOf("http://127.0.0.1:51234/tok/cast/")).toBe("default"); // context 파라미터 없음
    expect(castContextOf("not a url")).toBe("default"); // 손상 URL도 default
  });

  it("collectWebUrls — 트리에서 web 노드 url 전부 수집(cast dup 판정 재료)", () => {
    const cast = makeWebNode(1, castAppUrl(51234, "tok", "default"), "브라우저");
    const viewer = makeWebNode(2, viewerAppUrl(51234, "tok", "/a.md"), "a.md");
    const tree = {
      type: "split", dir: "row",
      a: { type: "pane", sid: 7 }, // 터미널 pane은 건너뜀
      b: { type: "split", dir: "col", a: cast, b: viewer },
    };
    expect(collectWebUrls(tree)).toEqual([cast.url, viewer.url]);
    expect(collectWebUrls(tree).some(isCastUrl)).toBe(true); // dup 판정 형태
    expect(collectWebUrls(null)).toEqual([]);
  });

  it("castFailureReason — ensure 실패 원인이 사용자 노출 문구로 보존된다(무음 금지 회귀 핀)", () => {
    // 신선 머신 최빈 실패: 이 문자열이 UI에서 소실되면 "브라우저 꺼짐"으로 위장된다(P1-1).
    expect(castFailureReason("bun 미설치 — https://bun.sh")).toBe("bun 미설치 — https://bun.sh");
    // Rust가 stderr/stdout을 여러 줄로 올려도 placeholder 한 줄로 접힌다
    expect(castFailureReason("ensure 실패\n  stderr: bun: command not found\n")).toBe(
      "ensure 실패 stderr: bun: command not found",
    );
    // Error 객체도 메시지가 살아남는다
    expect(castFailureReason(new Error("state.json 없음"))).toBe("Error: state.json 없음");
  });

  it("castFailureReason — 긴 원인은 절단(placeholder 폭)·경계는 무절단·빈 값은 빈 문자열", () => {
    expect(castFailureReason("x".repeat(120))).toBe("x".repeat(120)); // 경계: 정확히 maxLen이면 그대로
    expect(castFailureReason("x".repeat(121))).toBe(`${"x".repeat(120)}…`); // 초과분만 "…"
    expect(castFailureReason("abcdef", 3)).toBe("abc…"); // maxLen 주입
    // 원인이 비면 "" — 호출측이 원인 없는 기본 문구로 낙하한다(빈 괄호 노출 방지)
    expect(castFailureReason("")).toBe("");
    expect(castFailureReason("   \n  ")).toBe("");
    expect(castFailureReason(null)).toBe("");
    expect(castFailureReason(undefined)).toBe("");
  });

  it("castPaneTitle — 3자 페이지 제목의 헤더 반영(정규화·상한·빈값은 기본 제목에 양보)", () => {
    expect(castPaneTitle("예시 문서 — example.com")).toBe("예시 문서 — example.com");
    expect(castPaneTitle("줄바꿈\n섞인   제목\t")).toBe("줄바꿈 섞인 제목"); // 헤더 파괴 방지
    expect(castPaneTitle("가".repeat(60))).toBe("가".repeat(60)); // 경계: 무절단
    expect(castPaneTitle("가".repeat(61))).toBe(`${"가".repeat(60)}…`);
    // 빈값·비문자열은 "" → 호출측이 기본 "브라우저"를 유지한다(제목 소실 방지)
    expect(castPaneTitle("")).toBe("");
    expect(castPaneTitle("   ")).toBe("");
    expect(castPaneTitle(null)).toBe("");
    expect(castPaneTitle(undefined)).toBe("");
    expect(castPaneTitle(123)).toBe("");
  });

  it("castDisplayUrl — 페이지 URL 축약 표시(스킴·쿼리 제거·비URL 폴백)", () => {
    expect(castDisplayUrl("https://example.com/docs/a?x=1#f")).toBe("example.com/docs/a"); // 스킴·쿼리·해시 제거·경로는 보존
    expect(castDisplayUrl("https://example.com/")).toBe("example.com"); // 루트 경로는 생략
    expect(castDisplayUrl("about:blank")).toBe("about:blank"); // host 없는 스킴은 원문 유지
    expect(castDisplayUrl("not a url")).toBe("not a url"); // 비URL 폴백
    expect(castDisplayUrl(`https://example.com/${"p".repeat(80)}`)).toHaveLength(61); // 60자+"…"
    expect(castDisplayUrl("")).toBe("");
    expect(castDisplayUrl(null)).toBe("");
  });

  it("isLoopbackOrigin — postMessage origin 하드 가드(spawn 유발 핸들러의 1차 게이트)", () => {
    expect(isLoopbackOrigin("http://127.0.0.1:51234")).toBe(true);
    expect(isLoopbackOrigin("http://localhost:8642")).toBe(true);
    expect(isLoopbackOrigin("http://evil.example")).toBe(false);
    expect(isLoopbackOrigin("http://127.0.0.1.evil.com")).toBe(false); // 서브도메인 위장
    expect(isLoopbackOrigin("http://127.0.0.1:80@evil")).toBe(false); // userinfo 위장
    expect(isLoopbackOrigin("https://127.0.0.1:51234")).toBe(false); // https는 우리 origin이 아니다
    expect(isLoopbackOrigin("http://127.0.0.1")).toBe(false); // 포트 없음
    expect(isLoopbackOrigin("http://127.0.0.1:51234/tok/cast/")).toBe(false); // origin에 경로가 붙을 수 없다
    expect(isLoopbackOrigin("")).toBe(false);
    expect(isLoopbackOrigin(null)).toBe(false);
    expect(isLoopbackOrigin(undefined)).toBe(false);
  });

  it("다운그레이드 안전 — 구 빌드가 cast 노드를 읽으면 extractViewerPath=null → 무해 placeholder 낙하", () => {
    // cast URL엔 ?path= 가 없다. 구 빌드(cast 미인지)의 ensureAndLoad는 extractViewerPath에서 null을
    // 받아 "잘못된 뷰어 경로" placeholder로 무해하게 떨어진다(트리·v2 불변·auto-spawn 없음).
    expect(extractViewerPath(castAppUrl(51234, "tok", "default"))).toBeNull();
  });
});

describe("cast 토큰 평문 영속 제거 — sanitizeCastNodes / sanitizeLayoutForPersist", () => {
  const REAL_TOKEN = "s3cr3t-browserd-token_XYZ";

  it("cast 노드 url에서 실 토큰이 사라지고 context는 보존된다", () => {
    const cast = makeWebNode(1, castAppUrl(51234, REAL_TOKEN, "dept-2"), "브라우저");
    const out = sanitizeCastNodes(cast);
    expect(out.url).not.toContain(REAL_TOKEN); // 평문 영속 금지 — 이 토큰은 RPC eval 권한을 준다
    expect(out.url).toBe(castAppUrl(0, "pending", "dept-2"));
    expect(castContextOf(out.url)).toBe("dept-2"); // context는 복원 재연결에 필요 → 보존
    expect(isCastUrl(out.url)).toBe(true); // 복원 시 cast 분기가 성립해야 한다
    expect(out.wid).toBe(1); // 나머지 필드 보존
    expect(out.title).toBe("브라우저");
  });

  it("뷰어 노드·터미널 노드·split 구조는 무변경(참조까지 동일 = 직렬화 바이트 불변)", () => {
    const viewer = makeWebNode(2, viewerAppUrl(51234, REAL_TOKEN, "/a.md"), "a.md");
    const term = { type: "pane", sid: 7 };
    const tree = { type: "split", dir: "row", ratio: 0.5, a: term, b: viewer };
    const out = sanitizeCastNodes(tree);
    expect(out).toBe(tree); // cast가 없으면 원본 참조 그대로
    expect(JSON.stringify(out)).toBe(JSON.stringify(tree));
    expect(viewer.url).toContain(REAL_TOKEN); // 뷰어 토큰 관례는 이 브리프 범위 밖 — 건드리지 않았다
  });

  it("혼합 트리 — cast만 치환·형제는 참조 보존·★런타임 원본 트리는 변형되지 않는다", () => {
    const cast = makeWebNode(1, castAppUrl(51234, REAL_TOKEN, "default"), "브라우저");
    const viewer = makeWebNode(2, viewerAppUrl(51234, REAL_TOKEN, "/a.md"), "a.md");
    const term = { type: "pane", sid: 7 };
    const tree = {
      type: "split", dir: "row", ratio: 0.5, a: term,
      b: { type: "split", dir: "col", ratio: 0.25, a: cast, b: viewer },
    };
    const before = JSON.stringify(tree);
    const out = sanitizeCastNodes(tree);
    expect(out).not.toBe(tree); // cast가 있으니 새 트리
    expect(out.a).toBe(term); // 터미널 서브트리 참조 보존
    expect(out.b.b).toBe(viewer); // 뷰어 노드 참조 보존
    expect(out.b.a.url).toBe(castAppUrl(0, "pending", "default"));
    expect(out.dir).toBe("row"); // split 메타 보존
    expect(out.b.ratio).toBe(0.25);
    // ★런타임 트리 무변형 — 실 토큰은 메모리에만 남아 iframe 로드·재조립이 그대로 동작한다
    expect(JSON.stringify(tree)).toBe(before);
    expect(cast.url).toContain(REAL_TOKEN);
  });

  it("persistLayout 저장물에 browserd 토큰이 없다(단일 choke point 관통 핀)", () => {
    const s = fakeStore();
    const cast = makeWebNode(1, castAppUrl(51234, REAL_TOKEN, "dept-2"), "브라우저");
    persistLayout(s.setItem, { workspaces: [{ id: 1, name: "ws1", tree: cast }], groups: [], active: 0 });
    expect(s.raw.get(LAYOUT_KEY_V3)!).not.toContain(REAL_TOKEN);
    // 복원 경로는 성립해야 한다 — 되읽으면 cast로 판정되고 context가 살아 있다
    const loaded = loadPersistedLayout(s.getItem);
    expect(isCastUrl(loaded.workspaces[0].tree.url)).toBe(true);
    expect(castContextOf(loaded.workspaces[0].tree.url)).toBe("dept-2");
  });

  it("cast 없는 레이아웃은 저장 바이트가 불변(기존 직렬화 회귀 0)", () => {
    const data = layoutWithWeb(); // 뷰어 노드만 담긴 기존 픽스처
    const s = fakeStore();
    persistLayout(s.setItem, data);
    expect(s.raw.get(LAYOUT_KEY_V3)).toBe(JSON.stringify(data)); // sanitize 전후 동일 바이트
    expect(sanitizeLayoutForPersist(data)).toEqual(data);
  });

  it("비정형 입력에도 안전(workspaces 없음·null·tree null)", () => {
    expect(sanitizeLayoutForPersist(null)).toBeNull();
    expect(sanitizeLayoutForPersist({ tag: "no-workspaces" })).toEqual({ tag: "no-workspaces" });
    expect(sanitizeLayoutForPersist({ workspaces: [{ id: 1, name: "x", tree: null }] })).toEqual({
      workspaces: [{ id: 1, name: "x", tree: null }],
    });
    expect(sanitizeCastNodes(null)).toBeNull();
  });
});
