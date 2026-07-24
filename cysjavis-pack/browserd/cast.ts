// cast.ts — browserd in-pane screencast 순수·테스트 가능부.
// lib.ts 관례를 따른다: 부작용 없는 로직·문자열 상수만(브라우저/네트워크 의존 없음).
// server.ts 가 이 모듈의 CAST_APP_HTML 을 서빙하고 castRoute·parseClientMsg·mapInput 을 소비한다.

// ════════════════════════════════════════════════════════════════════════
// 메시지 계약 (4-T-11) — S→C·C→S 타입명을 **단일 소스**로 고정한다.
// 무타입 즉석 리터럴 ↔ 문자열 비교면 필드 오타가 "탭 스트립이 안 뜬다"로만 발현해 진단이
// 불가능하다. 아래 상수를 CAST_APP_HTML 에 JSON.stringify 로 주입해 서버·앱이 같은 값을 쓴다.
// (앱 JS 전체 타입화는 빌드 스텝을 요구해 팩 모델을 깨므로 기각 — 상수 주입까지가 경계다.)
// ════════════════════════════════════════════════════════════════════════

export const MSG = {
  FRAME: "frame",
  NAV: "nav",
  TABS: "tabs",
  CONTROL: "control",
  ERR: "err",
  DIALOG: "dialog",
  CLOSED: "closed",
} as const;

export const CMSG = {
  ACK: "ack",
  MOUSE: "mouse",
  KEY: "key",
  INSERT_TEXT: "insertText",
  NAVIGATE: "navigate",
  NAV_ACTION: "nav-action",
  TAB: "tab",
  VIEWPORT: "viewport",
  CONTROL: "control",
  DIALOG_REPLY: "dialog-reply",
} as const;

export const CAST_PROTOCOL_VERSION = 2;
export const CAST_EMBED_TICKET_TTL_MS = 15_000;

export interface CastEmbedTicketDescriptor {
  ticket: string;
  runtimeId: string;
  context: string;
  protocolVersion: number;
  embedGeneration: number;
  paneId: string;
  parentOrigin: string;
}

export type CastEmbedTicketIssue = "issued" | "duplicate" | "invalid";
export type CastEmbedTicketConsume = "accepted" | "missing" | "expired" | "mismatch" | "replayed";

function validCastEmbedTicket(d: CastEmbedTicketDescriptor): boolean {
  return /^[A-Za-z0-9_-]{43,128}$/.test(d.ticket)
    && d.runtimeId.length >= 8 && d.runtimeId.length <= 128
    && d.context.length >= 1 && d.context.length <= 128 && !/[\u0000-\u001f\u007f]/.test(d.context)
    && d.protocolVersion === CAST_PROTOCOL_VERSION
    && Number.isSafeInteger(d.embedGeneration) && d.embedGeneration > 0
    && /^[A-Za-z0-9_-]{43,128}$/.test(d.paneId)
    && /^(tauri:\/\/localhost|http:\/\/tauri\.localhost|http:\/\/(127\.0\.0\.1|localhost):\d+)$/.test(d.parentOrigin);
}

// iframe 문서 GET에서 issue하고, 그 문서가 여는 인증 WS에서 consume한다. 레지스트리는 browserd
// 프로세스(runtime instance)에 귀속되며 descriptor의 runtime/context/generation을 모두 재대조한다.
// 소비·만료 항목도 bounded tombstone으로 남겨 같은 URL의 재발급/재생을 조용히 허용하지 않는다.
export class CastEmbedTicketRegistry {
  private readonly entries = new Map<string, {
    descriptor: CastEmbedTicketDescriptor;
    expiresAt: number;
    appLoaded: boolean;
    consumed: boolean;
  }>();

  constructor(
    private readonly ttlMs = CAST_EMBED_TICKET_TTL_MS,
    private readonly maxEntries = 4096,
  ) {
    if (!Number.isSafeInteger(ttlMs) || ttlMs < 1) throw new Error("ticket ttl must be a positive integer");
    if (!Number.isSafeInteger(maxEntries) || maxEntries < 1) throw new Error("ticket capacity must be a positive integer");
  }

  get size(): number {
    return this.entries.size;
  }

  issue(descriptor: CastEmbedTicketDescriptor, now = Date.now()): CastEmbedTicketIssue {
    if (!validCastEmbedTicket(descriptor)) return "invalid";
    if (this.entries.has(descriptor.ticket)) return "duplicate";
    while (this.entries.size >= this.maxEntries) {
      const oldest = this.entries.keys().next().value as string;
      this.entries.delete(oldest);
    }
    this.entries.set(descriptor.ticket, {
      descriptor: { ...descriptor },
      expiresAt: now + this.ttlMs,
      appLoaded: false,
      consumed: false,
    });
    return "issued";
  }

  openApp(descriptor: CastEmbedTicketDescriptor, now = Date.now()): CastEmbedTicketConsume {
    const result = this.match(descriptor, now);
    if (result !== "accepted") return result;
    const entry = this.entries.get(descriptor.ticket)!;
    if (entry.appLoaded || entry.consumed) return "replayed";
    entry.appLoaded = true;
    return "accepted";
  }

  consume(descriptor: CastEmbedTicketDescriptor, now = Date.now()): CastEmbedTicketConsume {
    const result = this.match(descriptor, now);
    if (result !== "accepted") return result;
    const entry = this.entries.get(descriptor.ticket)!;
    if (!entry.appLoaded) return "missing";
    if (entry.consumed) return "replayed";
    entry.consumed = true;
    return "accepted";
  }

  private match(descriptor: CastEmbedTicketDescriptor, now: number): CastEmbedTicketConsume {
    if (!validCastEmbedTicket(descriptor)) return "mismatch";
    const entry = this.entries.get(descriptor.ticket);
    if (!entry) return "missing";
    if (now >= entry.expiresAt) return "expired";
    const expected = entry.descriptor;
    if (
      expected.runtimeId !== descriptor.runtimeId
      || expected.context !== descriptor.context
      || expected.protocolVersion !== descriptor.protocolVersion
      || expected.embedGeneration !== descriptor.embedGeneration
      || expected.paneId !== descriptor.paneId
      || expected.parentOrigin !== descriptor.parentOrigin
    ) return "mismatch";
    return "accepted";
  }
}

export type CastBuildMode = "production" | "development";

// Tauri 2의 패키징 origin은 Windows(WebView2)와 그 외 플랫폼(Wry custom protocol)이 다르다.
// 개발 origin은 환경에서 명시한 단 하나의 loopback origin만 추가하며 임의 host/scheme은 받지 않는다.
export function resolveCastParentOrigin(args: {
  platform: string;
  mode: CastBuildMode;
  requested: string | null;
  developmentOrigin?: string | null;
}): string | null {
  const packaged = args.platform === "win32" ? "http://tauri.localhost" : "tauri://localhost";
  if (args.requested === packaged) return packaged;
  if (args.mode !== "development" || !args.developmentOrigin || args.requested !== args.developmentOrigin) return null;
  return /^http:\/\/(127\.0\.0\.1|localhost):\d+$/.test(args.developmentOrigin) ? args.developmentOrigin : null;
}

// cast HTML route 전용 CSP. 호출자는 resolveCastParentOrigin을 통과한 정확 origin만 넘긴다.
export function castContentSecurityPolicy(serverPort: number, parentOrigin: string): string {
  if (!/^(tauri:\/\/localhost|http:\/\/tauri\.localhost|http:\/\/(127\.0\.0\.1|localhost):\d+)$/.test(parentOrigin)) {
    throw new Error("invalid cast parent origin");
  }
  return `default-src 'none'; img-src data:; script-src 'unsafe-inline'; style-src 'unsafe-inline'; `
    + `connect-src ws://127.0.0.1:${serverPort}; frame-ancestors ${parentOrigin}`;
}

// 서버가 push한 fid와 원본 CDP sessionId의 수명주기. offer는 상한을 넘긴 가장 오래된 프레임을
// 즉시 ack 대상으로 돌려주고 최신 항목만 보존한다. acknowledge/drain은 항목을 제거하면서 값을
// 한 번만 반환하므로 다중 client ack와 detach가 겹쳐도 같은 fid를 두 번 릴레이하지 않는다.
export class LatestFrameFlow<T> {
  private readonly pending = new Map<number, T>();

  constructor(private readonly limit = 1) {
    if (!Number.isSafeInteger(limit) || limit < 1) throw new Error("frame flow limit must be a positive integer");
  }

  get size(): number {
    return this.pending.size;
  }

  offer(fid: number, value: T): T[] {
    this.pending.set(fid, value);
    const evicted: T[] = [];
    while (this.pending.size > this.limit) {
      const oldest = this.pending.keys().next().value as number;
      evicted.push(this.pending.get(oldest)!);
      this.pending.delete(oldest);
    }
    return evicted;
  }

  acknowledge(fid: number): T | null {
    const value = this.pending.get(fid);
    if (value === undefined) return null;
    this.pending.delete(fid);
    return value;
  }

  drain(): T[] {
    const values = [...this.pending.values()];
    this.pending.clear();
    return values;
  }
}

export const RECONNECT_GRACE_MS = 5_000;
export type ReconnectGraceDecision = "keep" | "cleanup";

// presence lease의 순수 시간 판정. client가 돌아오면 무조건 유지하고, 마지막 이탈 시각이 없으면
// 예약 근거가 없으므로 정리한다. 경계값은 cleanup으로 고정해 타이머 재예약 루프를 막는다.
export function reconnectGraceDecision(args: {
  clientCount: number;
  emptySince: number | null;
  now: number;
  graceMs?: number;
}): ReconnectGraceDecision {
  if (args.clientCount > 0) return "keep";
  if (args.emptySince === null) return "cleanup";
  const graceMs = args.graceMs ?? RECONNECT_GRACE_MS;
  return args.now - args.emptySince >= graceMs ? "cleanup" : "keep";
}

export interface TabInfo {
  id: string;
  title: string;
  url: string;
  active: boolean;
}

export type ServerMsg =
  | { type: typeof MSG.FRAME; fid: number; data: string; metadata: any }
  | {
      type: typeof MSG.NAV;
      seq: number;
      tabId: string;
      url: string;
      title: string;
      canBack: boolean;
      canForward: boolean;
      loading: boolean;
      viewportPinned: boolean;
    }
  | { type: typeof MSG.TABS; seq: number; tabs: TabInfo[] }
  | { type: typeof MSG.CONTROL; control: "agent" | "human"; leaseId?: string }
  | { type: typeof MSG.ERR; code: string; message: string; detail?: string; retry?: boolean }
  | { type: typeof MSG.DIALOG; id: number; kind: string; message: string; defaultValue: string }
  | { type: typeof MSG.CLOSED; reason: string };

// 생성 헬퍼 — 서버는 S→C 리터럴을 직접 만들지 않고 이 함수만 쓴다(필드 누락을 타입이 잡는다).
export function msgNav(f: {
  seq: number;
  tabId: string;
  url: string;
  title: string;
  canBack: boolean;
  canForward: boolean;
  loading: boolean;
  viewportPinned: boolean;
}): ServerMsg {
  return { type: MSG.NAV, ...f };
}
export function msgTabs(seq: number, tabs: TabInfo[]): ServerMsg {
  return { type: MSG.TABS, seq, tabs };
}
export function msgControl(control: "agent" | "human", leaseId?: string): ServerMsg {
  return leaseId ? { type: MSG.CONTROL, control, leaseId } : { type: MSG.CONTROL, control };
}
export function msgErr(code: string, message: string, detail?: string, retry?: boolean): ServerMsg {
  return { type: MSG.ERR, code, message, detail, retry };
}
export function msgDialog(id: number, kind: string, message: string, defaultValue: string): ServerMsg {
  return { type: MSG.DIALOG, id, kind, message, defaultValue };
}
export function msgClosed(reason: string): ServerMsg {
  return { type: MSG.CLOSED, reason };
}

// --- WS 클라이언트 → 서버 메시지 (검증 후 타입) ---
export type ClientMsg =
  | { type: typeof CMSG.ACK; fid: number }
  | {
      type: typeof CMSG.MOUSE;
      kind: "pressed" | "released" | "moved" | "wheel";
      x: number;
      y: number;
      cw: number;
      ch: number;
      button: "left" | "right" | "middle" | "none";
      clickCount: number;
      modifiers: number;
      deltaX: number;
      deltaY: number;
    }
  | { type: typeof CMSG.KEY; kind: "down" | "up"; key: string; code: string; text: string; modifiers: number }
  | { type: typeof CMSG.INSERT_TEXT; text: string }
  | { type: typeof CMSG.NAVIGATE; url: string }
  | { type: typeof CMSG.CONTROL; action: "release"; leaseId: string }
  | { type: typeof CMSG.NAV_ACTION; action: "back" | "forward" | "reload" | "stop" }
  | { type: typeof CMSG.TAB; action: "activate" | "close" | "new"; id: string }
  | { type: typeof CMSG.VIEWPORT; width: number; height: number; unpin: boolean }
  | { type: typeof CMSG.DIALOG_REPLY; id: number; action: "accept" | "dismiss"; text: string };

const TEXT_LIMIT = 4096;
const URL_LIMIT = 2048;
const KEY_LIMIT = 64; // key/code 는 짧은 DOM 식별자 — 범위 방어
const ID_LIMIT = 64; // 탭 id — 서버 발급 불투명 토큰. 서버는 자기 목록에서만 조회한다

// 경로 파서: /<token>/cast/ → app, /<token>/cast/ws → ws, 그 외 null.
// 순수 함수(bun test). 세그먼트 수 불일치·빈 토큰은 null.
export function castRoute(pathname: string): { kind: "app" | "ws"; token: string } | null {
  const parts = pathname.split("/").filter(Boolean);
  if (parts.length === 2 && parts[1] === "cast" && parts[0]) {
    // 설계 계약은 끝 슬래시가 있는 `/<token>/cast/` 뿐이다. 끝 슬래시 없는 형태는 거부(엄격화)
    // — GUI 가 조립하는 URL(castAppUrl)은 항상 슬래시를 포함한다.
    if (!pathname.endsWith("/")) return null;
    return { kind: "app", token: parts[0] };
  }
  if (parts.length === 3 && parts[1] === "cast" && parts[2] === "ws" && parts[0]) {
    return { kind: "ws", token: parts[0] };
  }
  return null;
}

// WS 클라이언트 메시지 검증. 허용 type·필드 타입·범위만 통과. 미지 type·불량 = null.
// 순수 함수. 신뢰 경계: 이 값이 그대로 CDP Input 으로 흘러가므로 여기서 좁힌다.
export function parseClientMsg(raw: string): ClientMsg | null {
  let m: any;
  try {
    m = JSON.parse(raw);
  } catch {
    return null;
  }
  if (!m || typeof m !== "object") return null;
  const fin = (v: any) => typeof v === "number" && Number.isFinite(v);

  switch (m.type) {
    case CMSG.ACK: {
      // fid = 서버 부여 단조 프레임 id. CDP sessionId 는 세션 상수라 클라이언트에 노출하지 않는다.
      if (!Number.isInteger(m.fid)) return null;
      return { type: CMSG.ACK, fid: m.fid };
    }
    case CMSG.MOUSE: {
      const kinds = ["pressed", "released", "moved", "wheel"];
      const buttons = ["left", "right", "middle", "none"];
      if (!kinds.includes(m.kind)) return null;
      if (!fin(m.x) || !fin(m.y) || !fin(m.cw) || !fin(m.ch)) return null;
      return {
        type: CMSG.MOUSE,
        kind: m.kind,
        x: m.x,
        y: m.y,
        cw: m.cw,
        ch: m.ch,
        button: buttons.includes(m.button) ? m.button : "none",
        clickCount: fin(m.clickCount) ? m.clickCount : 0,
        modifiers: fin(m.modifiers) ? m.modifiers : 0,
        deltaX: fin(m.deltaX) ? m.deltaX : 0,
        deltaY: fin(m.deltaY) ? m.deltaY : 0,
      };
    }
    case CMSG.KEY: {
      if (m.kind !== "down" && m.kind !== "up") return null;
      if (typeof m.key !== "string" || typeof m.code !== "string") return null;
      if (m.key.length > KEY_LIMIT || m.code.length > KEY_LIMIT) return null;
      const text = typeof m.text === "string" ? m.text : "";
      if (text.length > TEXT_LIMIT) return null;
      return { type: CMSG.KEY, kind: m.kind, key: m.key, code: m.code, text, modifiers: fin(m.modifiers) ? m.modifiers : 0 };
    }
    case CMSG.INSERT_TEXT: {
      if (typeof m.text !== "string" || m.text.length > TEXT_LIMIT) return null;
      return { type: CMSG.INSERT_TEXT, text: m.text };
    }
    case CMSG.NAVIGATE: {
      if (typeof m.url !== "string" || m.url.length > URL_LIMIT) return null;
      return { type: CMSG.NAVIGATE, url: m.url };
    }
    case CMSG.CONTROL: {
      if (m.action !== "release") return null;
      // 사람이 획득할 때 서버가 해당 pane/session에만 발급한 불투명 증명이다. action 문자열만으로
      // release를 허용하면 다른(또는 stale) WS가 사람의 조작권을 임의로 반환시킬 수 있다.
      if (typeof m.leaseId !== "string" || !/^[A-Za-z0-9_-]{43,128}$/.test(m.leaseId)) return null;
      return { type: CMSG.CONTROL, action: "release", leaseId: m.leaseId };
    }
    case CMSG.NAV_ACTION: {
      const acts = ["back", "forward", "reload", "stop"];
      if (!acts.includes(m.action)) return null;
      return { type: CMSG.NAV_ACTION, action: m.action };
    }
    case CMSG.TAB: {
      const acts = ["activate", "close", "new"];
      if (!acts.includes(m.action)) return null;
      // activate·close 는 대상 id 필수. new 는 id 없음(빈 문자열로 정규화).
      if (m.action === "new") return { type: CMSG.TAB, action: "new", id: "" };
      if (typeof m.id !== "string" || !m.id || m.id.length > ID_LIMIT) return null;
      return { type: CMSG.TAB, action: m.action, id: m.id };
    }
    case CMSG.VIEWPORT: {
      // 클램프·면적 캡은 서버가 한다 — 여기서는 유한 양수만 통과시킨다(0·음수는 크기가 아니다).
      if (!fin(m.width) || !fin(m.height)) return null;
      if (!(m.width > 0) || !(m.height > 0)) return null;
      return { type: CMSG.VIEWPORT, width: m.width, height: m.height, unpin: m.unpin === true };
    }
    case CMSG.DIALOG_REPLY: {
      if (!Number.isInteger(m.id)) return null;
      if (m.action !== "accept" && m.action !== "dismiss") return null;
      const text = typeof m.text === "string" ? m.text : "";
      if (text.length > TEXT_LIMIT) return null;
      return { type: CMSG.DIALOG_REPLY, id: m.id, action: m.action, text };
    }
    default:
      return null;
  }
}

// canvas 표시 좌표 → letterbox 역변환 → viewport CSS px.
// meta.deviceWidth/Height 를 신뢰(하드코딩 금지 — 시뮬 F4). letterbox 여백 클릭은 null(무시).
// 순수 함수(bun test).
export function mapInput(
  m: { x: number; y: number; cw: number; ch: number },
  meta: { deviceWidth: number; deviceHeight: number; offsetTop?: number; pageScaleFactor?: number } | null
): { x: number; y: number } | null {
  // 4-S-1-4: rebind 직후 lastMeta=null 이면 좌표를 만들지 않는다 — "오클릭보다 무시가 안전".
  if (!meta) return null;
  const dw = meta.deviceWidth;
  const dh = meta.deviceHeight;
  const { x, y, cw, ch } = m;
  if (!(dw > 0) || !(dh > 0) || !(cw > 0) || !(ch > 0)) return null; // metadata 미보유·불량
  // 비율 유지 fit — canvas 안에 프레임을 중앙 letterbox 배치할 때의 배율.
  const scale = Math.min(cw / dw, ch / dh);
  const dispW = dw * scale;
  const dispH = dh * scale;
  const offX = (cw - dispW) / 2;
  const offY = (ch - dispH) / 2;
  const ix = x - offX;
  const iy = y - offY;
  // 우/하 경계는 등호 포함 배제 — ix===dispW 는 마지막 픽셀 다음이라 뷰포트 밖으로 매핑된다.
  if (ix < 0 || iy < 0 || ix >= dispW || iy >= dispH) return null; // letterbox 여백
  // 이미지 px → DIP.
  const dipX = ix / scale;
  const dipY = iy / scale;
  // offsetTop=콘텐츠 상단 오프셋(DIP)·pageScaleFactor=핀치줌. 데스크톱 고정 뷰포트에선 0·1이라
  // 항등이지만, 후속 pane 크기 연동에서 필요하므로 값을 신뢰해 반영한다(하드코딩 금지).
  const psf = meta.pageScaleFactor && meta.pageScaleFactor > 0 ? meta.pageScaleFactor : 1;
  const top = Number.isFinite(meta.offsetTop as number) ? (meta.offsetTop as number) : 0;
  const cssY = (dipY - top) / psf;
  if (cssY < 0) return null; // 콘텐츠 영역 위쪽(브라우저 크롬 자리) 클릭은 무시
  return { x: dipX / psf, y: cssY };
}

// ════════════════════════════════════════════════════════════════════════
// 내비 URL 게이트 (4-T-0 PRE-3) — **모든 내비 진입점이 통과하는 단일 함수**.
// open·goto·navigate(주소창)·tab new·에러 배너 재시도가 전부 여기를 지난다.
// 근거: `open` 이 스킴 무검증이면 `file:///…/.ssh/id_rsa` 를 렌더한 뒤 `get text` 로 읽어낼 수
// 있다 — "cast 는 픽셀만 나가니 안전"이라는 Phase 2 논거가 조회 동사 도입 순간 무너진다.
// Browser v2 제품 허용 = http·https만. about:blank는 서버 내부 탭 생성 경로에서만 사용하고 공개
// RPC/WS 입력에는 허용하지 않는다. data/file/chrome/view-source/javascript/blob 등은 전부 거부한다.
// 순수 함수(bun test). 반환 null=통과, 문자열=거부 사유.
export function navigableUrlError(url: unknown): string | null {
  if (typeof url !== "string" || !url) return "url 필요";
  if (url.length > URL_LIMIT) return `url 길이 상한(${URL_LIMIT}) 초과`;
  let u: URL;
  try {
    u = new URL(url);
  } catch {
    return "URL 형식이 아니다 — 스킴을 포함한 절대 주소여야 한다";
  }
  const scheme = u.protocol.toLowerCase();
  if (scheme === "http:" || scheme === "https:") return null;
  return `스킴 '${u.protocol}' 거부 — http·https 주소만 이동할 수 있다`;
}

// ════════════════════════════════════════════════════════════════════════
// 뷰포트 상한 (4-T-5) — 변 클램프만으로는 4096×4096=16.7MP 가 통과해 **동사 한 줄로 도달하는
// DoS**가 된다(30fps 상한은 프레임 '수'만 막고 '바이트'를 막지 않는다). 면적 캡을 함께 건다.
// ════════════════════════════════════════════════════════════════════════
export const VIEWPORT_MIN = 1;
export const VIEWPORT_MAX = 4096;
export const VIEWPORT_MAX_AREA = 2_100_000; // ≈1920×1080

// ★DPR 결정(4-S-9 이행 · master 판정) — **1차는 devicePixelRatio 미반영**한다.
//   근거: 대역·CPU 우선, 그리고 면적 상한(≤2.1MP)과의 상호작용을 단순하게 유지(캡에 DPR 을 곱하면
//   Retina 에서 실효 면적이 4배가 되어 상한의 의미가 기기마다 달라진다).
//   결과: Retina 에서 CSS px 그대로 캡처하므로 표시 해상도의 절반 — **후속 재검토 대상**이다.
//   이 결정을 코드에 남기는 것까지가 이행이다(무언의 미반영은 미이행).

export function clampViewportSide(v: number): number {
  return Math.max(VIEWPORT_MIN, Math.min(VIEWPORT_MAX, Math.round(v)));
}

// 변 클램프 후 면적 캡을 넘으면 **종횡비를 유지한 채** 축소한다(잘라내지 않는다 —
// 종횡비가 깨지면 mapInput 의 letterbox 역변환과 화면이 어긋난다).
export function fitViewport(w: number, h: number): { width: number; height: number } {
  let width = clampViewportSide(w);
  let height = clampViewportSide(h);
  const area = width * height;
  if (area > VIEWPORT_MAX_AREA) {
    const k = Math.sqrt(VIEWPORT_MAX_AREA / area);
    width = Math.max(VIEWPORT_MIN, Math.floor(width * k));
    height = Math.max(VIEWPORT_MIN, Math.floor(height * k));
  }
  return { width, height };
}

// 사람(pane) 뷰포트 fit-to-width(4-T-5 확장): pane 폭이 FIT_MIN_WIDTH 미만이면
// 데스크톱 고정폭 사이트(≈1080px)가 사이트 차원에서 잘린다(headless 는 가로 스크롤바도
// 안 보인다 — 2026-07-25 오너 실사고). 뷰포트를 FIT_MIN_WIDTH × 비례 높이로 올려 렌더하면
// child contain-fit 이 pane 에 전폭을 축소 표시한다(좌표는 mapInput letterbox 역변환이 흡수).
// 에이전트 pin 뷰포트에는 적용하지 않는다 — 에이전트 지정값은 문자 그대로가 계약.
export const FIT_MIN_WIDTH = 1080;
export function fitHumanViewport(w: number, h: number): { width: number; height: number } {
  if (w >= FIT_MIN_WIDTH || !(w > 0 && h > 0)) return fitViewport(w, h);
  const k = FIT_MIN_WIDTH / w;
  return fitViewport(FIT_MIN_WIDTH, Math.round(h * k));
}

// ★DPR(devicePixelRatio) 결정 — 4-S-9 가 "곱할지 말지 명시적으로 결정하고 근거를 남기라"고 한
//   항목이다. **1차는 DPR 미반영으로 확정한다**(master 판정).
//   근거: ①Retina(devicePixelRatio=2)에서 캡에 DPR 을 곱하면 면적이 4배가 되어 면적 상한
//   (VIEWPORT_MAX_AREA)에 즉시 도달한다 → fitViewport 가 되레 축소를 걸어 **선명도 이득이
//   사라지고** 계산만 늘어난다. ②프레임 바이트·CPU 가 4배로 뛰는데 cast 는 30fps 스트림이라
//   대역이 먼저 무너진다. ③선명도는 jpeg 품질(아래 함수)로 조절하는 편이 예측 가능하다.
//   후속 재검토 여지: pane 이 작아 면적 여유가 큰 경우에만 조건부로 DPR 을 곱하는 안(2차).
// 큰 뷰포트일수록 jpeg 품질을 낮춰 프레임 바이트 예산을 지킨다(대역·CPU 방어).
export function jpegQualityFor(width: number, height: number): number {
  const area = width * height;
  // ★기본 뷰포트 1280×800 = 1.024MP 는 반드시 75 를 유지한다 — 경계를 1.0MP 로 잡으면
  //   Phase 2 와 같은 크기에서 화질이 조용히 떨어진다(회귀).
  if (area <= 1_100_000) return 75;
  if (area <= 1_600_000) return 60;
  return 45;
}

// 내비 실패 원문(Chromium net:: 오류·playwright 타임아웃)을 사람 말로 번역한다.
// 순수 함수(bun test). 원문은 호출측이 detail 로 따로 보존한다 — 번역이 원인을 지우지 않게.
export function navErrorMessage(raw: string): string {
  const s = String(raw || "");
  if (/ERR_NAME_NOT_RESOLVED|ERR_NAME_RESOLUTION_FAILED/.test(s))
    return "주소를 찾을 수 없습니다 — 도메인 이름 조회에 실패했습니다(철자·인터넷 연결 확인).";
  if (/ERR_CONNECTION_REFUSED/.test(s)) return "연결이 거부되었습니다 — 서버가 꺼져 있거나 그 포트가 닫혀 있습니다.";
  if (/ERR_CONNECTION_TIMED_OUT|ERR_TIMED_OUT|Timeout .*exceeded/i.test(s))
    return "시간이 초과되었습니다 — 서버가 응답하지 않습니다.";
  if (/ERR_CERT_|ERR_SSL_|ERR_BAD_SSL/.test(s))
    return "보안 인증서에 문제가 있습니다 — 만료됐거나 이 주소와 맞지 않는 인증서입니다.";
  if (/ERR_INTERNET_DISCONNECTED|ERR_NETWORK_CHANGED|ERR_ADDRESS_UNREACHABLE/.test(s))
    return "네트워크에 연결할 수 없습니다 — 인터넷 연결을 확인하세요.";
  if (/ERR_CONNECTION_RESET|ERR_CONNECTION_CLOSED|ERR_EMPTY_RESPONSE/.test(s))
    return "연결이 끊겼습니다 — 서버가 응답을 중간에 끊었습니다.";
  if (/ERR_UNSAFE_PORT/.test(s)) return "브라우저가 막는 포트입니다 — 다른 포트를 쓰세요.";
  if (/ERR_BLOCKED_BY_|ERR_ACCESS_DENIED/.test(s)) return "접근이 차단되었습니다.";
  if (/ERR_ABORTED/.test(s)) return "이동이 중단되었습니다.";
  if (/ERR_TOO_MANY_REDIRECTS/.test(s)) return "리다이렉트가 너무 많습니다 — 페이지가 자기 자신으로 돌고 있습니다.";
  if (/ERR_FILE_NOT_FOUND|ERR_INVALID_URL/.test(s)) return "잘못된 주소입니다.";
  return "페이지를 열지 못했습니다.";
}

// 사람 UI 표시용 문자열 캡(4-T-3) — 탭 제목·URL 등 웹 통제 문자열의 길이 상한.
export const UI_TEXT_CAP = 120;

// screencast 앱 — 단일 HTML(인라인 CSS/JS, 외부 자산 0). browserd 가 자기 origin 에서 서빙한다.
// canvas letterbox 렌더 + 툴바(뒤로·앞으로·새로고침/중지) + 탭 스트립 + 진행 바 + 오류 배너
// + control 상태 + WS 클라이언트 + 입력 캡처(마우스·키·한글 IME) + 다이얼로그 사람 분기.
// CSP(default-src 'none'; script-src 'unsafe-inline' …)는 server.ts 응답 헤더가 강제.
//
// ★4-T-3 절대 규칙: 웹 유래 문자열(탭 제목·URL·에러 문구·다이얼로그 메시지)은 **textContent 로만**
//   렌더한다. innerHTML·insertAdjacentHTML 금지. 이 앱 origin 은 WS 토큰을 쥐고 있어서
//   탭 제목의 `<img src=x onerror=…>` 하나면 임의 내비게이션·입력 주입이 전부 열린다.
//   (img-src 가 data: 뿐이라 로드가 반드시 실패 → onerror 가 확실히 발화한다 — 직관과 반대다.)
export const CAST_APP_HTML = `<!doctype html>
<html lang="ko">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>cys 브라우저</title>
<style>
  html,body{margin:0;height:100%;background:#1a1a1a;color:#ddd;font:13px -apple-system,BlinkMacSystemFont,"Apple SD Gothic Neo",sans-serif;}
  body{display:flex;flex-direction:column;overflow:hidden;}
  #tabs{display:none;align-items:center;gap:2px;height:28px;padding:0 6px;background:#222;border-bottom:1px solid #000;flex:0 0 auto;overflow-x:auto;}
  #tabs .tab{display:flex;align-items:center;gap:4px;max-width:180px;height:22px;padding:0 6px;border:1px solid #3a3a3a;border-radius:4px;background:#2c2c2c;color:#bbb;cursor:pointer;flex:0 0 auto;}
  #tabs .tab.active{background:#3d4450;color:#fff;border-color:#556;}
  #tabs .tt{overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-size:12px;}
  #tabs .tx,#tabs .tplus{border:0;background:transparent;color:#999;cursor:pointer;font-size:13px;padding:0 3px;line-height:1;}
  #tabs .tx:hover,#tabs .tplus:hover{color:#fff;}
  #bar{display:flex;align-items:center;gap:6px;height:34px;padding:0 8px;background:#2a2a2a;flex:0 0 auto;}
  #addr{flex:1;min-width:60px;height:22px;border:1px solid #444;border-radius:4px;background:#111;color:#eee;padding:0 8px;outline:none;}
  #bar button{height:24px;border:1px solid #444;border-radius:4px;background:#333;color:#eee;cursor:pointer;padding:0 8px;}
  #bar button:hover{background:#3d3d3d;}
  #bar button:disabled{opacity:0.3;cursor:default;background:#2a2a2a;}
  #bar button:disabled:hover{background:#2a2a2a;}
  #nav{display:flex;gap:2px;flex:0 0 auto;}
  #nav button{min-width:26px;padding:0 4px;font-size:14px;}
  #ctrl{font-size:12px;color:#8ab;min-width:56px;text-align:center;}
  #vpin{display:none;align-items:center;gap:4px;font-size:11px;color:#d9a441;flex:0 0 auto;}
  #vpin button{height:20px;font-size:11px;padding:0 6px;}
  #prog{height:2px;background:#2a2a2a;border-bottom:1px solid #000;flex:0 0 auto;overflow:hidden;}
  #progbar{height:100%;width:0;background:#5b9dd9;transition:width 200ms linear;}
  #prog.on #progbar{width:85%;transition:width 6s cubic-bezier(0,.8,.2,1);}
  #banner{display:none;align-items:flex-start;gap:8px;padding:6px 8px;background:#4a2626;border-bottom:1px solid #000;color:#f2c9c9;font-size:12px;flex:0 0 auto;}
  #banner .bmsg{flex:1;word-break:break-word;}
  #banner button{height:22px;border:1px solid #7a4a4a;border-radius:4px;background:#5c2f2f;color:#fff;cursor:pointer;padding:0 8px;font-size:12px;flex:0 0 auto;}
  #stage{flex:1 1 auto;position:relative;overflow:hidden;}
  #screen{position:absolute;inset:0;width:100%;height:100%;display:block;background:#1a1a1a;outline:none;}
  #overlay{position:absolute;inset:0;display:flex;flex-direction:column;gap:10px;align-items:center;justify-content:center;text-align:center;padding:16px;background:rgba(20,20,20,0.85);color:#ccc;}
  #ovbtn{display:none;height:26px;padding:0 14px;border:1px solid #556;border-radius:4px;background:#3d4450;color:#fff;cursor:pointer;}
  #dlg{position:absolute;left:50%;top:20px;transform:translateX(-50%);max-width:80%;display:none;flex-direction:column;gap:8px;padding:12px 14px;background:#2b2f36;border:1px solid #4a5160;border-radius:6px;box-shadow:0 6px 24px rgba(0,0,0,.5);}
  #dlgmsg{white-space:pre-wrap;word-break:break-word;font-size:13px;color:#eee;}
  #dlginput{display:none;height:24px;border:1px solid #444;border-radius:4px;background:#111;color:#eee;padding:0 8px;outline:none;}
  #dlgrow{display:flex;gap:6px;justify-content:flex-end;}
  #dlgrow button{height:24px;border:1px solid #556;border-radius:4px;background:#3d4450;color:#fff;cursor:pointer;padding:0 12px;}
  #proxy{position:absolute;left:-9999px;top:0;width:1px;height:1px;opacity:0;}
</style>
</head>
<body>
<div id="tabs"></div>
<div id="bar">
  <span id="nav">
    <button id="back" title="뒤로" disabled>←</button>
    <button id="fwd" title="앞으로" disabled>→</button>
    <button id="rel" title="새로고침">⟳</button>
  </span>
  <input id="addr" type="text" placeholder="주소 입력 후 Enter" spellcheck="false" autocomplete="off">
  <button id="go">이동</button>
  <span id="vpin"><span>에이전트가 크기 고정</span><button id="vunpin">해제</button></span>
  <span id="ctrl">에이전트</span>
  <button id="hand">에이전트에게 넘기기</button>
</div>
<div id="prog"><div id="progbar"></div></div>
<div id="banner"><span class="bmsg"></span><button class="bretry">다시 시도</button><button class="bclose">닫기</button></div>
<div id="stage">
  <canvas id="screen" tabindex="0"></canvas>
  <div id="overlay"><div id="ovtext">연결 중...</div><button id="ovbtn">다시 연결</button></div>
  <div id="dlg"><div id="dlgmsg"></div><input id="dlginput" type="text"><div id="dlgrow"><button id="dlgcancel">취소</button><button id="dlgok">확인</button></div></div>
</div>
<input id="proxy" type="text" tabindex="-1" aria-hidden="true">
<script>
(function(){
  // ★타입명은 서버(cast.ts)와 단일 소스 — 문자열 리터럴 산재 금지(4-T-11).
  var MSG = ${JSON.stringify(MSG)};
  var CMSG = ${JSON.stringify(CMSG)};
  var UI_CAP = ${JSON.stringify(UI_TEXT_CAP)};
  var SUPPORTED_PROTOCOL = ${JSON.stringify(CAST_PROTOCOL_VERSION)};

  var seg = location.pathname.split('/').filter(Boolean);
  var token = seg[0] || '';
  var params = new URLSearchParams(location.search);
  var context = params.get('context') || 'default';
  var protocolVersion = Number(params.get('protocolVersion'));
  var embedGeneration = Number(params.get('embedGeneration'));
  var paneId = params.get('paneId') || '';
  var parentOrigin = params.get('parentOrigin') || '';
  var embedTicket = params.get('embedTicket') || '';
  var wsParams = new URLSearchParams({
    context: context,
    protocolVersion: String(protocolVersion),
    embedGeneration: String(embedGeneration),
    paneId: paneId,
    parentOrigin: parentOrigin,
    embedTicket: embedTicket
  });
  var wsUrl = (location.protocol === 'https:' ? 'wss://' : 'ws://') + location.host + '/' + token + '/cast/ws?' + wsParams.toString();

  var canvas = document.getElementById('screen');
  var g = canvas.getContext('2d');
  var stage = document.getElementById('stage');
  var addr = document.getElementById('addr');
  var goBtn = document.getElementById('go');
  var handBtn = document.getElementById('hand');
  var ctrlLabel = document.getElementById('ctrl');
  var overlay = document.getElementById('overlay');
  var ovText = document.getElementById('ovtext');
  var ovBtn = document.getElementById('ovbtn');
  var proxy = document.getElementById('proxy');
  var tabsEl = document.getElementById('tabs');
  var backBtn = document.getElementById('back');
  var fwdBtn = document.getElementById('fwd');
  var relBtn = document.getElementById('rel');
  var prog = document.getElementById('prog');
  var banner = document.getElementById('banner');
  var bMsg = banner.querySelector('.bmsg');
  var bRetry = banner.querySelector('.bretry');
  var bClose = banner.querySelector('.bclose');
  var vpin = document.getElementById('vpin');
  var vunpin = document.getElementById('vunpin');
  var dlg = document.getElementById('dlg');
  var dlgMsg = document.getElementById('dlgmsg');
  var dlgInput = document.getElementById('dlginput');
  var dlgOk = document.getElementById('dlgok');
  var dlgCancel = document.getElementById('dlgcancel');

  var ws = null;
  var hasFrame = false;   // 첫 frame(metadata) 전에는 입력 무시(설계 D3)
  var painted = false;    // canvas 유효 draw 완료 — iframe load/process spawn과 분리된 성공 근거
  var reportedLive = false;
  var lastMoveSent = 0;
  var loading = false;
  var seenSeq = -1;       // 4-S-7: 자기가 본 최대 seq 미만 메시지는 무시(늦게 온 옛 상태 차단)
  var dlgId = null;
  var controlLeaseId = null;

  function mods(e){ return (e.altKey?1:0)|(e.ctrlKey?2:0)|(e.metaKey?4:0)|(e.shiftKey?8:0); }
  function btn(e){ return e.button===0?'left':(e.button===1?'middle':(e.button===2?'right':'none')); }
  function send(o){ if (ws && ws.readyState === 1) ws.send(JSON.stringify(o)); }
  function cap(s){ s = (typeof s === 'string') ? s : ''; return s.length > UI_CAP ? s.slice(0, UI_CAP) + '…' : s; }
  function postParent(type, fields){
    if (protocolVersion !== SUPPORTED_PROTOCOL || !Number.isSafeInteger(embedGeneration) || embedGeneration < 1 || !parentOrigin) return;
    if (!/^[A-Za-z0-9_-]{43,128}$/.test(embedTicket)) return;
    if (!/^[A-Za-z0-9_-]{43,128}$/.test(paneId)) return;
    var envelope = Object.assign({
      type:type,
      protocolVersion:protocolVersion,
      embedGeneration:embedGeneration,
      embedTicket:embedTicket,
      paneId:paneId
    }, fields || {});
    try { window.parent.postMessage(envelope, parentOrigin); } catch(e){}
  }

  // --- 연결 상태 오버레이(전면) — 연결 자체가 없을 때만 쓴다 ---
  function showOverlay(text, action){
    ovText.textContent = text;                 // ★textContent 전용(4-T-3)
    overlay.style.display = 'flex';
    overlay._action = action || null;
    ovBtn.style.display = action ? 'block' : 'none';
    overlay.style.cursor = action ? 'pointer' : 'default';
  }
  function hideOverlay(){ overlay.style.display = 'none'; overlay._action = null; ovBtn.style.display = 'none'; }
  function runOverlayAction(){
    if (overlay._action === 'reconnect') {
      // GUI 가 ensure 재발동·iframe 재로드하도록 부모에 알린다.
      postParent('cys-cast-reconnect');
    }
  }
  ovBtn.addEventListener('click', function(e){ e.stopPropagation(); runOverlayAction(); });
  overlay.addEventListener('click', runOverlayAction);

  // --- 오류 배너(4-S-4) — 화면을 가리지 않고, 프레임 도착과 무관하게 사용자가 닫는다 ---
  function showBanner(text, retryable){
    bMsg.textContent = text;                   // ★textContent 전용
    banner.style.display = 'flex';
    bRetry.style.display = retryable ? 'block' : 'none';
  }
  function hideBanner(){ banner.style.display = 'none'; }
  bClose.addEventListener('click', hideBanner);
  bRetry.addEventListener('click', function(){ hideBanner(); send({ type: CMSG.NAV_ACTION, action: 'reload' }); });

  // --- 내비 상태(뒤로/앞으로 활성 여부·로딩 표시) ---
  function applyNav(msg){
    backBtn.disabled = !msg.canBack;
    fwdBtn.disabled = !msg.canForward;
    loading = !!msg.loading;
    relBtn.textContent = loading ? '✕' : '⟳';
    relBtn.title = loading ? '중지' : '새로고침';
    prog.className = loading ? 'on' : '';
    vpin.style.display = msg.viewportPinned ? 'flex' : 'none';
  }

  // --- 탭 스트립(1개면 숨김) · 전부 textContent 렌더 ---
  function renderTabs(tabs){
    tabsEl.textContent = '';
    if (!tabs || tabs.length <= 1) { tabsEl.style.display = 'none'; return; }
    tabsEl.style.display = 'flex';
    tabs.forEach(function(t){
      var d = document.createElement('div');
      d.className = 'tab' + (t.active ? ' active' : '');
      var s = document.createElement('span');
      s.className = 'tt';
      s.textContent = cap(t.title || t.url || '새 탭');   // ★textContent(4-T-3)
      d.title = cap(t.url || '');                         // 속성 대입도 캡
      d.appendChild(s);
      var x = document.createElement('button');
      x.className = 'tx';
      x.textContent = '×';
      x.title = '탭 닫기';
      x.addEventListener('click', function(e){ e.stopPropagation(); send({ type: CMSG.TAB, action: 'close', id: t.id }); });
      d.appendChild(x);
      d.addEventListener('click', function(){ if (!t.active) send({ type: CMSG.TAB, action: 'activate', id: t.id }); });
      tabsEl.appendChild(d);
    });
    var plus = document.createElement('button');
    plus.className = 'tplus';
    plus.textContent = '＋';
    plus.title = '새 탭';
    plus.addEventListener('click', function(){ send({ type: CMSG.TAB, action: 'new' }); });
    tabsEl.appendChild(plus);
  }

  // --- 뷰포트 연동(4-T-5) — ★stage 의 CSS 크기만 측정한다.
  // 프레임 metadata 파생값을 되보내면 서버 리사이즈 → 새 metadata → 재전송의 무한 루프가 된다.
  var vpTimer = null, lastVp = '';
  function measure(){ return { w: Math.round(stage.clientWidth), h: Math.round(stage.clientHeight) }; }
  function sendViewport(unpin){
    var m = measure();
    if (!(m.w > 0 && m.h > 0)) return;
    var key = m.w + 'x' + m.h;
    if (key === lastVp && !unpin) return;
    lastVp = key;
    send({ type: CMSG.VIEWPORT, width: m.w, height: m.h, unpin: !!unpin });
  }
  function scheduleViewport(){          // trailing-edge 디바운스 전용
    if (vpTimer) clearTimeout(vpTimer);
    vpTimer = setTimeout(function(){ sendViewport(false); }, 200);
  }
  window.addEventListener('resize', scheduleViewport);
  if (window.ResizeObserver) { try { new ResizeObserver(scheduleViewport).observe(stage); } catch(e){} }
  vunpin.addEventListener('click', function(){ lastVp = ''; sendViewport(true); });

  function fitCanvas(){ canvas.width = canvas.clientWidth; canvas.height = canvas.clientHeight; }

  function drawFrame(img){
    fitCanvas();
    var cw = canvas.width, ch = canvas.height;
    g.fillStyle = '#1a1a1a';
    g.fillRect(0, 0, cw, ch);
    var iw = img.naturalWidth || img.width, ih = img.naturalHeight || img.height;
    if (!iw || !ih || !(cw > 0 && ch > 0)) return false;
    var scale = Math.min(cw / iw, ch / ih);
    var dw = iw * scale, dh = ih * scale;
    g.drawImage(img, (cw - dw) / 2, (ch - dh) / 2, dw, dh);
    return true;
  }

  function onFrame(msg){
    var img = new Image();
    img.onload = function(){
      if (!drawFrame(img)) {
        send({ type: CMSG.ACK, fid: msg.fid });
        return;
      }
      var first = !hasFrame;
      hasFrame = true;
      painted = true;
      hideOverlay();
      // 4-S-6: 첫 프레임 후 pane 실제 크기를 1회 자발 송신(리사이즈 이벤트만 기다리면 최초
      // pane 크기가 영영 반영되지 않는다).
      if (first) sendViewport(false);
      send({ type: CMSG.ACK, fid: msg.fid }); // 그린 뒤 ack — e2e 배압
      if (first) {
        // drawImage 호출 완료가 아니라 compositor가 최소 한 paint 경계를 지난 뒤에만 FRAME_READY.
        // 두 rAF 사이에 실제 paint가 들어가므로 검은 canvas를 성공으로 오판하지 않는다.
        requestAnimationFrame(function(){ requestAnimationFrame(function(){
          if (!painted || !hasFrame) return;
          postParent('cys-cast-phase', { phase: 'FRAME_READY' });
          requestAnimationFrame(function(){
            if (!reportedLive && painted && hasFrame) {
              reportedLive = true;
              postParent('cys-cast-phase', { phase: 'LIVE' });
            }
          });
        }); });
      }
    };
    // 디코드 실패해도 ack 는 보낸다 — 안 보내면 CDP 스트림이 영구 스톨한다.
    img.onerror = function(){ send({ type: CMSG.ACK, fid: msg.fid }); };
    img.src = 'data:image/jpeg;base64,' + msg.data;
  }

  // --- 다이얼로그 사람 분기(4-T-12) ---
  function showDialog(msg){
    dlgId = msg.id;
    dlgMsg.textContent = cap(msg.message || '') + (msg.kind ? '  [' + msg.kind + ']' : ''); // ★textContent
    dlgInput.style.display = (msg.kind === 'prompt') ? 'block' : 'none';
    dlgInput.value = msg.defaultValue || '';
    dlgCancel.style.display = (msg.kind === 'alert') ? 'none' : 'block';
    dlg.style.display = 'flex';
  }
  function replyDialog(action){
    if (dlgId === null) return;
    send({ type: CMSG.DIALOG_REPLY, id: dlgId, action: action, text: dlgInput.value || '' });
    dlgId = null;
    dlg.style.display = 'none';
  }
  dlgOk.addEventListener('click', function(){ replyDialog('accept'); });
  dlgCancel.addEventListener('click', function(){ replyDialog('dismiss'); });

  function connect(){
    showOverlay('연결 중...', null);
    try { ws = new WebSocket(wsUrl); } catch(e){
      showOverlay('연결 실패 — 클릭해 재연결', 'reconnect');
      postParent('cys-cast-phase', { phase: 'FAILED', errorCode: 'WS_CONNECT_FAILED' });
      return;
    }
    ws.onopen = function(){
      // ★재접속 시 seq 역전 방지: seenSeq 는 connect() 바깥 변수라 재연결에도 남는다. 서버 seq 가
      //   새 hub 에서 낮은 값부터 다시 오르면 클라이언트가 **초기 동기화 nav·tabs 를 전부 무시**해
      //   주소창·탭이 stale 로 고착한다(4-S-7 의 순서 계약이 재접속을 못 본 자리).
      seenSeq = -1;
      showOverlay('화면 대기 중...', null);
      postParent('cys-cast-phase', { phase: 'SHELL_READY' });
    };
    ws.onmessage = function(ev){
      var msg; try { msg = JSON.parse(ev.data); } catch(e){ return; }
      if (msg.type === MSG.FRAME) { onFrame(msg); return; }
      // 4-S-7 순서 계약: 늦게 도착한 옛 상태가 새 상태를 덮지 않게 한다.
      if (typeof msg.seq === 'number') {
        if (msg.seq < seenSeq) return;
        seenSeq = msg.seq;
      }
      if (msg.type === MSG.NAV) {
        // 주소창은 사람이 입력 중일 때 덮어쓰지 않는다(타이핑 중 커서 튐 방지).
        if (document.activeElement !== addr) addr.value = msg.url || '';
        if (msg.title) document.title = cap(msg.title);
        applyNav(msg);
        // 부모(cys GUI)의 pane 헤더 갱신용 — 수신·반영은 GUI 측 담당.
        postParent('cys-cast-title', { title: cap(msg.title || ''), url: msg.url || '' });
      }
      else if (msg.type === MSG.TABS) { renderTabs(msg.tabs); }
      else if (msg.type === MSG.CONTROL) {
        controlLeaseId = msg.control === 'human' && typeof msg.leaseId === 'string' ? msg.leaseId : null;
        ctrlLabel.textContent = (msg.control === 'human') ? '사람 조작 중' : '에이전트';
        handBtn.disabled = !controlLeaseId;
      }
      else if (msg.type === MSG.DIALOG) { showDialog(msg); }
      else if (msg.type === MSG.CLOSED) {
        hasFrame = false;
        postParent('cys-cast-phase', { phase: 'CLOSED' });
        showOverlay('브라우저 컨텍스트가 닫혔습니다 — 클릭해 재연결', 'reconnect');
      }
      else if (msg.type === MSG.ERR) {
        // ★전면 오버레이가 아니라 배너(4-S-4) — 정적 페이지에서 오버레이가 영구 고착하던 결함 제거.
        showBanner((msg.message || '오류') + ' [' + (msg.code || '') + ']', !!msg.retry);
        if (msg.code === 'SCREENCAST_FAILED' || msg.code === 'PAGE_CRASHED') {
          postParent('cys-cast-phase', { phase: 'FAILED', errorCode: msg.code });
        }
        if (msg.retry) { loading = false; prog.className = ''; relBtn.textContent = '⟳'; }
      }
    };
    ws.onclose = function(){
      var hadPaintedFrame = painted && hasFrame;
      hasFrame = false;
      postParent('cys-cast-phase', hadPaintedFrame
        ? { phase: 'DEGRADED' }
        : { phase: 'FAILED', errorCode: 'WS_CLOSED_BEFORE_FRAME' });
      showOverlay('연결 끊김 — 클릭해 재연결', 'reconnect');
    };
    ws.onerror = function(){ /* onclose 가 뒤따른다 */ };
  }

  // --- 마우스 입력(canvas) ---
  canvas.addEventListener('mousedown', function(e){
    proxy.focus(); // 키보드·IME 는 숨김 프록시로 라우팅
    if (!hasFrame) return;
    send({ type:CMSG.MOUSE, kind:'pressed', x:e.offsetX, y:e.offsetY, cw:canvas.clientWidth, ch:canvas.clientHeight, button:btn(e), clickCount:e.detail||1, modifiers:mods(e), deltaX:0, deltaY:0 });
  });
  canvas.addEventListener('mouseup', function(e){
    if (!hasFrame) return;
    send({ type:CMSG.MOUSE, kind:'released', x:e.offsetX, y:e.offsetY, cw:canvas.clientWidth, ch:canvas.clientHeight, button:btn(e), clickCount:e.detail||1, modifiers:mods(e), deltaX:0, deltaY:0 });
  });
  canvas.addEventListener('mousemove', function(e){
    if (!hasFrame) return;
    var now = Date.now();
    if (now - lastMoveSent < 30) return; // ~30ms 스로틀
    lastMoveSent = now;
    send({ type:CMSG.MOUSE, kind:'moved', x:e.offsetX, y:e.offsetY, cw:canvas.clientWidth, ch:canvas.clientHeight, button:'none', clickCount:0, modifiers:mods(e), deltaX:0, deltaY:0 });
  });
  canvas.addEventListener('wheel', function(e){
    if (!hasFrame) return;
    e.preventDefault();
    send({ type:CMSG.MOUSE, kind:'wheel', x:e.offsetX, y:e.offsetY, cw:canvas.clientWidth, ch:canvas.clientHeight, button:'none', clickCount:0, modifiers:mods(e), deltaX:e.deltaX, deltaY:e.deltaY });
  }, { passive: false });
  canvas.addEventListener('contextmenu', function(e){ e.preventDefault(); });

  // --- 키보드·한글 IME(숨김 프록시 input) ---
  // WKWebView 는 canvas 에 composition 이벤트를 주지 않는다 → off-screen input 을 포커스 프록시로
  // 삼아 compositionend 로 확정 텍스트를 받고, 조합 아닌 일반 키는 keydown 에서 잡아 프록시를 매번 비운다.
  proxy.addEventListener('keydown', function(e){
    if (!hasFrame) return;
    if (e.isComposing || e.keyCode === 229) return; // IME 조합 중 — 미전송(이중입력 방지)
    e.preventDefault();
    var text = (e.key && e.key.length === 1) ? e.key : ''; // 인쇄 가능 단일 문자만 text
    send({ type:CMSG.KEY, kind:'down', key:e.key, code:e.code, text:text, modifiers:mods(e) });
    proxy.value = '';
  });
  proxy.addEventListener('keyup', function(e){
    if (!hasFrame) return;
    if (e.isComposing || e.keyCode === 229) return;
    e.preventDefault();
    send({ type:CMSG.KEY, kind:'up', key:e.key, code:e.code, text:'', modifiers:mods(e) });
    proxy.value = '';
  });
  proxy.addEventListener('compositionend', function(e){
    if (!hasFrame) return;
    if (e.data) send({ type:CMSG.INSERT_TEXT, text:e.data });
    proxy.value = '';
  });

  // --- 주소창 이동 ---
  function navigate(){
    var u = (addr.value || '').trim();
    if (!u) return;
    if (u.indexOf('://') === -1) u = 'https://' + u; // 스킴 없으면 보정
    hideBanner();
    send({ type: CMSG.NAVIGATE, url:u });
  }
  goBtn.addEventListener('click', navigate);
  addr.addEventListener('keydown', function(e){ if (e.key === 'Enter') { e.preventDefault(); navigate(); } });

  // --- 뒤로·앞으로·새로고침/중지 ---
  backBtn.addEventListener('click', function(){ if (!backBtn.disabled) send({ type:CMSG.NAV_ACTION, action:'back' }); });
  fwdBtn.addEventListener('click', function(){ if (!fwdBtn.disabled) send({ type:CMSG.NAV_ACTION, action:'forward' }); });
  relBtn.addEventListener('click', function(){ send({ type:CMSG.NAV_ACTION, action: loading ? 'stop' : 'reload' }); });

  // --- control: 에이전트에게 넘기기 ---
  handBtn.addEventListener('click', function(){
    if (!controlLeaseId) return;
    send({ type:CMSG.CONTROL, action:'release', leaseId:controlLeaseId });
  });

  connect();
})();
</script>
</body>
</html>`;
