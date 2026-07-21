// web pane 순수 로직 — DOM 무의존(bun test 대상). main.ts의 WebPaneView(iframe·타이틀 스트립)는
// 이 모듈의 URL 가드·URL 조립·레이아웃 버전 판정만 호출하고, 부수효과(iframe.src·localStorage)는
// 호출측이 쥔다. seam 커밋(PaneView)에 이어 web pane 편입의 검증 가능한 판단부를 여기 모은다.

// 레이아웃 저장 키. v3=web 노드 포함 신 포맷. v2=구 포맷(구조 동일 — web 노드만 신규라 passthrough).
// ★다운그레이드 안전: 신 빌드는 v3에만 쓰고 v2는 읽기만 한다(삭제·수정 금지). 구 빌드로 롤백해도
// v2 스냅샷을 그대로 읽어 부팅한다(업그레이드 이후 v2는 프리즈된 마지막 구-포맷 상태).
export const LAYOUT_KEY_V2 = "cys-layout-v2";
export const LAYOUT_KEY_V3 = "cys-layout-v3";

// web pane 노드 — pane(터미널 sid)과 달리 sid가 없고 wid(단조증가 고유 id)로 식별한다.
// url = 뷰어 사이드카 앱 URL(loopback+토큰). 직렬화 대상(레이아웃 트리에 저장).
export type WebNode = { type: "web"; url: string; title?: string; wid: number };

export function makeWebNode(wid: number, url: string, title?: string): WebNode {
  return title === undefined ? { type: "web", url, wid } : { type: "web", url, title, wid };
}

// URL 하드 가드 — iframe src로 실릴 수 있는 것은 loopback(127.0.0.1|localhost)+명시 포트+경로뿐.
// 임의 인터넷 사이트·file://·https·userinfo(@) 위장 host는 전부 차단(예외 없음). 포트 뒤 첫 문자가
// '/'여야 하므로 `http://127.0.0.1:80@evil/`·`http://127.0.0.1:80.evil/`는 매칭 실패 → 거부.
export function isAllowedWebPaneUrl(url: string): boolean {
  return /^http:\/\/(127\.0\.0\.1|localhost):\d+\//.test(url);
}

// 뷰어 앱 URL 조립 — 사이드카가 넘긴 {port, token}과 렌더 대상 파일 경로로 만든다.
// token은 secrets.token_urlsafe(무-예약문자)라 그대로 삽입(app.js가 pathname 첫 세그먼트로 raw 대조).
export function viewerAppUrl(port: number, token: string, path: string): string {
  return `http://127.0.0.1:${port}/${token}/app/?path=${encodeURIComponent(path)}`;
}

// 저장된(스테일 token) web URL에서 원 파일 경로만 회수 — 복원 시 새 {port,token}으로 재조립하기 위함.
export function extractViewerPath(url: string): string | null {
  try {
    return new URL(url).searchParams.get("path");
  } catch {
    return null;
  }
}

// 레이아웃 트리에서 web 노드의 뷰어 경로를 전부 수집(collectWebWids 관례 승계 — 순수 walk).
// viewer.open dup 판정의 범위를 '현재 워크스페이스 트리'로 좁히는 데 쓴다: 전역 webPanes 스캔은
// 다른 워크스페이스 pane 때문에 "이미 열려 있음"인데 현재 화면엔 없는 UX 결함을 만든다(시뮬 F1).
export function collectWebPaths(node: any, out: (string | null)[] = []): (string | null)[] {
  if (!node) return out;
  if (node.type === "web") out.push(extractViewerPath(node.url));
  else if (node.type === "split") {
    collectWebPaths(node.a, out);
    collectWebPaths(node.b, out);
  }
  return out;
}

// viewer.open 데몬 이벤트 판정 — DOM 무의존 순수 판정부(main.ts 핸들러가 소비, bun test 대상).
// not-ready: 워크스페이스 미준비(pending) — 무음 드롭 금지, 호출측이 toast로 알린다.
// stale: 데몬 재접속 replay가 과거 viewer.open을 되살리는 창 차단 — maxAgeSecs 초과 이벤트 무시.
// dup: 같은 경로 pane 존재 — 새로 만들지 않는다(중복 pane·이벤트 증폭 방지).
// cap: 뷰어 pane 총량 상한 — pane 홍수의 UI측 방벽(데몬 rate-limit의 짝).
export type ViewerOpenDecision = "open" | "dup" | "cap" | "stale" | "not-ready";
export function decideViewerOpen(args: {
  path: string;
  existingPaths: (string | null)[];
  paneCount: number;
  maxPanes: number;
  eventEpoch: number;
  nowEpoch: number;
  maxAgeSecs: number;
  wsReady: boolean;
}): ViewerOpenDecision {
  if (!args.wsReady) return "not-ready";
  if (args.nowEpoch - args.eventEpoch > args.maxAgeSecs) return "stale";
  if (args.existingPaths.includes(args.path)) return "dup";
  if (args.paneCount >= args.maxPanes) return "cap";
  return "open";
}

// 레이아웃 트리에서 web 노드 wid를 전부 수집(순수 walk — DOM 무의존). teardown/복원 경로가
// 이걸로 dispose 대상을 뽑는다. split은 양쪽 재귀, pane(터미널 sid)·null은 건너뛴다.
export function collectWebWids(node: any, out: number[] = []): number[] {
  if (!node) return out;
  if (node.type === "web") out.push(node.wid);
  else if (node.type === "split") {
    collectWebWids(node.a, out);
    collectWebWids(node.b, out);
  }
  return out;
}

// 레이아웃 로드 — v3 우선, 없으면 v2 읽어 마이그레이션(구조 동일 passthrough). 손상 저장본은 null.
// v2는 절대 쓰지/지우지 않는다(다운그레이드 안전).
// ★손상 v3 폴백(F5): v3가 존재하나 JSON 손상이면 전손실 대신 v2 스냅샷으로 폴백한다
// (업그레이드 직후 v2는 프리즈된 마지막 구-포맷 상태라 부팅 가능한 유효 후보다).
export function loadPersistedLayout(getItem: (key: string) => string | null): any | null {
  const rawV3 = getItem(LAYOUT_KEY_V3);
  if (rawV3) {
    try {
      return JSON.parse(rawV3);
    } catch {
      // v3 손상 → v2 폴백 시도(아래 공통 경로로 낙하). v2도 없거나 손상이면 최종 null.
    }
  }
  const rawV2 = getItem(LAYOUT_KEY_V2); // v2→v3 마이그레이션(첫 v3 로드 전까지 구 저장본 승계) + v3 손상 폴백
  if (rawV2) {
    try {
      return JSON.parse(rawV2);
    } catch {
      return null;
    }
  }
  return null;
}

// 레이아웃 저장 — v3에만 쓴다(v2 미변경 = 다운그레이드 안전 불변식). 저장 직전 cast 노드의 실 토큰을
// pending으로 치환한다(sanitizeLayoutForPersist) — v3를 쓰는 단일 choke point라 우회 저장 경로가 없다.
export function persistLayout(setItem: (key: string, value: string) => void, data: unknown): void {
  setItem(LAYOUT_KEY_V3, JSON.stringify(sanitizeLayoutForPersist(data)));
}

// ─── cast web pane (browserd CDP screencast) ──────────────────────────────
// 뷰어 web pane과 같은 iframe 표면·같은 WebNode 스키마(url 내용만 다름)를 쓰되, URL이 browserd가
// 자기 origin에서 서빙하는 screencast 앱을 가리킨다. 뷰어와 구분되는 순수 판단부만 여기 모은다
// (DOM 무의존·bun test 대상). isAllowedWebPaneUrl 하드가드는 cast URL도 그대로 통과한다.

export const CAST_PROTOCOL_VERSION = 2;

export type CastPanePhase =
  | "PENDING"
  | "SHELL_READY"
  | "FRAME_READY"
  | "LIVE"
  | "DEGRADED"
  | "FAILED"
  | "CLOSED";

export interface CastPaneState {
  phase: CastPanePhase;
  generation: number;
  paintedFrames: number;
  errorCode?: string;
}

export type CastPaneEvent =
  | { type: "IFRAME_LOADED"; generation: number }
  | { type: "SHELL_READY"; generation: number }
  | { type: "FRAME_READY"; generation: number }
  | { type: "LIVE"; generation: number }
  | { type: "TIMEOUT"; generation: number }
  | { type: "DISCONNECTED"; generation: number }
  | { type: "FAIL"; generation: number; errorCode: string }
  | { type: "CLOSED"; generation: number };

export function initialCastPaneState(generation: number): CastPaneState {
  return { phase: "PENDING", generation, paintedFrames: 0 };
}

// Cast pane 수명주기의 순수 전이 경계. iframe load는 관측 신호일 뿐 성공이 아니며,
// generation이 다른 document의 늦은 이벤트는 기존 객체를 그대로 반환해 무효화한다.
export function reduceCastPaneState(state: CastPaneState, event: CastPaneEvent): CastPaneState {
  if (event.generation !== state.generation) return state;
  switch (event.type) {
    case "IFRAME_LOADED":
      return state;
    case "SHELL_READY":
      return state.phase === "PENDING" || state.phase === "DEGRADED"
        ? { ...state, phase: "SHELL_READY", errorCode: undefined }
        : state;
    case "FRAME_READY":
      if (state.phase !== "SHELL_READY") {
        return { ...state, phase: "FAILED", errorCode: "PROTOCOL_ORDER" };
      }
      return { ...state, phase: "FRAME_READY", paintedFrames: 1 };
    case "LIVE":
      return state.phase === "FRAME_READY" || state.phase === "LIVE"
        ? { ...state, phase: "LIVE", paintedFrames: Math.max(1, state.paintedFrames) }
        : { ...state, phase: "FAILED", errorCode: "PROTOCOL_ORDER" };
    case "TIMEOUT":
      if (state.phase === "PENDING") return { ...state, phase: "FAILED", errorCode: "SHELL_TIMEOUT" };
      if (state.phase === "SHELL_READY") return { ...state, phase: "FAILED", errorCode: "FIRST_FRAME_TIMEOUT" };
      return state;
    case "DISCONNECTED":
      return state.paintedFrames > 0
        ? { ...state, phase: "DEGRADED", errorCode: "WS_DISCONNECTED" }
        : { ...state, phase: "FAILED", errorCode: "WS_CLOSED_BEFORE_FRAME" };
    case "FAIL":
      return { ...state, phase: "FAILED", errorCode: event.errorCode };
    case "CLOSED":
      return { ...state, phase: "CLOSED" };
  }
}

// postMessage의 phase 필드를 내부 상태 이벤트로 좁힌다. 문자열을 그대로 UI 상태/오류 코드로
// 사용하지 않아 미지 phase의 조용한 폴백과 무제한 오류 문자열 렌더를 함께 막는다.
export function castPhaseEvent(data: unknown, generation: number): CastPaneEvent | null {
  if (!Number.isSafeInteger(generation) || generation < 1 || !data || typeof data !== "object") return null;
  const payload = data as { phase?: unknown; errorCode?: unknown };
  switch (payload.phase) {
    case "SHELL_READY":
    case "FRAME_READY":
    case "LIVE":
      return { type: payload.phase, generation };
    case "DEGRADED":
      return { type: "DISCONNECTED", generation };
    case "CLOSED":
      return { type: "CLOSED", generation };
    case "FAILED":
      if (typeof payload.errorCode !== "string" || !/^[A-Z][A-Z0-9_]{0,63}$/.test(payload.errorCode)) return null;
      return { type: "FAIL", generation, errorCode: payload.errorCode };
    default:
      return null;
  }
}

export interface CastEmbedOptions {
  protocolVersion: number;
  embedGeneration: number;
  parentOrigin: string;
  embedTicket: string;
}

export function newCastEmbedTicket(): string {
  const bytes = new Uint8Array(32);
  globalThis.crypto.getRandomValues(bytes);
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

// Tauri custom protocol의 location.origin은 WebKit에서 "null"일 수 있으므로 protocol+host로
// canonical origin을 만든다. 그 외에는 현재 배포 origin 또는 명시 포트의 loopback 개발 origin만 허용한다.
export function castParentOrigin(location: {
  protocol: string;
  host: string;
  origin: string;
}): string | null {
  if (location.protocol === "tauri:" && location.host === "localhost") return "tauri://localhost";
  if (location.protocol === "http:" && location.host === "tauri.localhost" && location.origin === "http://tauri.localhost") {
    return location.origin;
  }
  if (location.protocol === "http:" && /^((127\.0\.0\.1)|localhost):\d+$/.test(location.host)) {
    const expected = `http://${location.host}`;
    return location.origin === expected ? expected : null;
  }
  return null;
}

// cast 앱 URL 조립 — 저장용 3인자 호출은 기존 URL 바이트를 보존한다. 실제 iframe 로드는
// protocol/generation/parentOrigin 을 함께 실어, 같은 contentWindow가 navigation 뒤 재사용돼도
// 이전 document의 늦은 postMessage를 generation으로 거부할 수 있게 한다.
export function castAppUrl(port: number, token: string, context: string, embed?: CastEmbedOptions): string {
  const base = `http://127.0.0.1:${port}/${token}/cast/?context=${encodeURIComponent(context)}`;
  if (!embed) return base;
  return `${base}&protocolVersion=${encodeURIComponent(String(embed.protocolVersion))}`
    + `&embedGeneration=${encodeURIComponent(String(embed.embedGeneration))}`
    + `&parentOrigin=${encodeURIComponent(embed.parentOrigin)}`
    + `&embedTicket=${encodeURIComponent(embed.embedTicket)}`;
}

// 부모가 처리할 cast postMessage의 단일 검증 경계. origin "형태"가 아니라 현재 iframe URL의
// 정확 origin을 대조하고, source 일치와 protocol/embed generation을 모두 요구한다.
export function acceptsCastMessage(args: {
  frameUrl: string;
  eventOrigin: unknown;
  sourceMatches: boolean;
  data: unknown;
}): boolean {
  if (!args.sourceMatches || typeof args.eventOrigin !== "string") return false;
  let url: URL;
  try {
    url = new URL(args.frameUrl);
  } catch {
    return false;
  }
  if (!isLoopbackOrigin(url.origin) || args.eventOrigin !== url.origin) return false;
  const protocolVersion = Number(url.searchParams.get("protocolVersion"));
  const embedGeneration = Number(url.searchParams.get("embedGeneration"));
  const embedTicket = url.searchParams.get("embedTicket");
  if (
    protocolVersion !== CAST_PROTOCOL_VERSION
    || !Number.isSafeInteger(embedGeneration)
    || embedGeneration < 1
    || !embedTicket
    || !/^[A-Za-z0-9_-]{43,128}$/.test(embedTicket)
  ) return false;
  if (!args.data || typeof args.data !== "object") return false;
  const data = args.data as { protocolVersion?: unknown; embedGeneration?: unknown; embedTicket?: unknown };
  return data.protocolVersion === protocolVersion
    && data.embedGeneration === embedGeneration
    && data.embedTicket === embedTicket;
}

// cast URL 판정 — pathname 2번째 세그먼트가 "cast"면 true(토큰 무관·순수). 뷰어 URL(2번째="app")·
// 손상 URL은 false. 조립 전 pending URL(port 0·token "pending")도 cast로 인식해야 한다.
export function isCastUrl(url: string): boolean {
  try {
    const segs = new URL(url).pathname.split("/").filter((s) => s.length > 0);
    return segs[1] === "cast";
  } catch {
    return false;
  }
}

// cast 노드의 context 회수 — 복원·재연결 시 같은 context로 재조립하기 위함. 없으면 "default".
export function castContextOf(url: string): string {
  try {
    return new URL(url).searchParams.get("context") ?? "default";
  } catch {
    return "default";
  }
}

// cast 앱이 보고한 페이지 제목 → pane 헤더 문구(순수). 값의 출처가 **3자 페이지**라 공백 정규화 +
// 길이 상한으로 헤더 파괴를 막는다(문자열로만 다루므로 마크업 주입 표면은 없다 — 호출측 textContent).
// 빈 값·비문자열은 ""를 돌려 호출측이 기본 제목("브라우저")을 유지하게 한다.
export function castPaneTitle(title: unknown, maxLen = 60): string {
  if (typeof title !== "string") return "";
  const one = title.replace(/\s+/g, " ").trim();
  return one.length <= maxLen ? one : `${one.slice(0, maxLen)}…`;
}

// cast 앱이 보고한 페이지 URL → 헤더 축약 표시(순수). 스킴·쿼리를 걷어 host+path만 보인다(전체 URL은
// 호출측이 title 속성으로 단다). 비URL(about:blank 등)은 원문을 축약해 그대로 쓴다.
export function castDisplayUrl(url: unknown, maxLen = 60): string {
  if (typeof url !== "string") return "";
  const one = url.replace(/\s+/g, " ").trim();
  if (!one) return "";
  let shown = one;
  try {
    const u = new URL(one);
    if (u.host) shown = `${u.host}${u.pathname === "/" ? "" : u.pathname}`;
  } catch {
    // 비URL은 원문 축약으로 낙하(무음 실패 아님 — 사용자는 보고된 값을 그대로 본다)
  }
  return shown.length <= maxLen ? shown : `${shown.slice(0, maxLen)}…`;
}

// postMessage origin 하드 가드 — origin은 경로 없는 `scheme://host:port` 형태다. isAllowedWebPaneUrl과
// 같은 원칙(loopback+명시 포트·양끝 고정)으로 userinfo(@)·서브도메인 위장을 차단한다. 포트는 가변이라
// 값이 아니라 형태만 본다 — 발신자 실체 대조는 ownsSource(iframe contentWindow)가 맡는 2중 게이트.
export function isLoopbackOrigin(origin: unknown): boolean {
  return typeof origin === "string" && /^http:\/\/(127\.0\.0\.1|localhost):\d+$/.test(origin);
}

// cast 노드의 저장용 URL 치환(순수) — browserd 토큰의 localStorage 평문 영속을 없앤다.
// 근거: cast pane은 로드할 때마다 browserd_state/ensure_browserd_cast로 최신 {port,token}을 다시 받아
// URL을 재조립하므로 **저장된 토큰은 한 번도 쓰이지 않는다**(순수 잔여물 — 지워도 기능 손실 0). 반면 이
// 토큰은 RPC eval로 임의 JS 실행이 가능해 뷰어 토큰과 권한 등급이 다르다. context는 복원 시 같은
// 컨텍스트로 재연결해야 하므로 **반드시 보존**한다.
// ★변경 없는 서브트리는 원본 참조를 그대로 돌려준다 — 뷰어·터미널 노드 직렬화는 바이트 불변이고,
// 런타임 트리는 변형하지 않는다(실 토큰은 메모리에만 남아 iframe 로드·재조립이 그대로 동작).
export function sanitizeCastNodes(node: any): any {
  if (!node) return node;
  if (node.type === "web") {
    if (!isCastUrl(node.url)) return node; // 뷰어 노드 무변경
    return { ...node, url: castAppUrl(0, "pending", castContextOf(node.url)) };
  }
  if (node.type === "split") {
    const a = sanitizeCastNodes(node.a);
    const b = sanitizeCastNodes(node.b);
    return a === node.a && b === node.b ? node : { ...node, a, b };
  }
  return node; // 터미널 pane 등 무변경
}

// 레이아웃 전체에 위 치환 적용(workspaces[].tree만 훑는다). 저장 시점 전용 — 런타임 상태 무영향.
export function sanitizeLayoutForPersist(data: any): any {
  if (!data || !Array.isArray(data.workspaces)) return data;
  return {
    ...data,
    workspaces: data.workspaces.map((w: any) => {
      if (!w || !w.tree) return w;
      const tree = sanitizeCastNodes(w.tree);
      return tree === w.tree ? w : { ...w, tree };
    }),
  };
}

// ensure_browserd_cast 실패 원인을 사용자 노출용 한 줄 문구로 정규화 — 무음 금지의 순수부.
// 원인을 UI에서 버리면 신선 머신 최빈 실패("bun 미설치 — https://bun.sh")가 "브라우저 꺼짐"으로
// 위장돼 사용자가 클릭만 반복하며 원인을 영영 모른다. 개행·연속 공백을 접고 maxLen 초과분은 "…"로
// 절단한다(placeholder가 좁아 앞부분만 싣는다 — 전문은 toast가 나른다). 빈 에러·null은 ""(호출측이
// 원인 없는 기본 문구로 낙하).
export function castFailureReason(err: unknown, maxLen = 120): string {
  const one = (err === null || err === undefined ? "" : String(err)).replace(/\s+/g, " ").trim();
  return one.length <= maxLen ? one : `${one.slice(0, maxLen)}…`;
}

// 레이아웃 트리에서 web 노드 url을 전부 수집(collectWebPaths 동형 순수 walk). openCastPane의 dup
// 판정(현재 ws 트리에 cast pane 이미 있으면 새 pane 대신 포커스만)에 쓴다.
export function collectWebUrls(node: any, out: string[] = []): string[] {
  if (!node) return out;
  if (node.type === "web") out.push(node.url);
  else if (node.type === "split") {
    collectWebUrls(node.a, out);
    collectWebUrls(node.b, out);
  }
  return out;
}
