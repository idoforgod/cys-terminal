// ui/src/starvednotice.ts — 큐 기아 경보(`queue.starved`)를 사람이 보는 토스트·OS 배너 문구로 바꾸는 순수 도우미(0.14.43 · UI2).
//
// 왜 필요한가: 데몬은 좌석 큐 머리가 임계(기본 600초) 이상 막히면 `queue.starved` 를 낸다(좌석당 5분 쿨다운). 그런데 GUI 는 이 이벤트를
// 소비하지 않았다 — 이벤트 category 가 "queue" 라 onDaemonEvent 의 폴백 3종(health·watchdog·feed) 어디에도 걸리지 않아 **화면에 아무것도 안
// 떴다**. 조치는 대부분 사람이 해야 하는데(그 창을 클릭하고 Ctrl-U · 질문 창에 답하기 …) 알리는 표면이 없어 좌석 큐가 몇 시간씩 막혀도
// 아무도 몰랐다. CSO 좌석 자신의 경보는 라우터가 폐기하므로(자기 제외) 더더욱 보이지 않는다.
//
// 이 모듈은 payload → 문구·분류·id 변환과 '토스트를 누르면 갈 좌석 확정'만 한다. DOM·Tauri·저장소는 모른다(main.ts 가 배선·렌더를 한다).
// ★payload 는 데몬에서 온 **신뢰할 수 없는 데이터**다 — 타입부터 의심하고, 화면에 올릴 문자열은 제어문자를 걷고 길이를 자른다.
//   렌더는 main.ts 가 stickyToast(textContent) 로만 한다(HTML 삽입 없음).
// ★이 문구는 사람을 위한 것이다 — 데몬이 remedy 끝에 붙이는 "LLM 에이전트는 자동 조치(강제 배달·드레인·키 주입) 금지" 꼬리는 LLM 소비자용이라
//   여기서 떼어 낸다. 이 모듈은 **어떤 조치도 하지 않는다**(자동 키 주입·강제 배달·화면 자동 전환 없음).
//
// ★이 모듈의 불변식(starvednotice.test.ts 가 핀으로 고정 — usagebar.ts 와 같다):
//   · 최상위 부수효과 0 — 선언(export/const/function/interface/type)만. 브라우저 저장소(local·session)·문서 객체·창 객체·타이머 접근 0
//     (이 파일에는 그 낱말 자체가 한 번도 나오지 않는다 — 핀이 원문 문자열로 센다).
//     main.js 는 번들 하나라 여기서 평가 중 예외가 나면 앱 전체가 백지가 된다.
//   · 구형 WKWebView 가 파싱하지 못하는 문법 0 — 정규식 뒤돌아보기(lookbehind)·배열 끝 인덱스 접근·뒤에서 찾기·구조 복제·소유 판정 정적 메서드·전체 치환 계열.
//     `bun build --target browser` 는 다운레벨하지 않으므로 파싱 실패가 곧 백지다.

export type StarvedLevel = "health" | "idle";

/** 화면에 올릴 알림 한 건. */
export interface StarvedNotice {
  /** 토스트 id — `starved:<소켓 slug>:surface:<n>`. 같은 좌석은 같은 id 라 갱신된다(중첩 없음). */
  id: string;
  /** 사람 손이면 `⏳ 큐 막힘 —`, 기다림이면 종류별 대기 제목 + `<역할> surface:<n>`. */
  title: string;
  /** `<N분째> · <조치 문장>`(200자 상한). */
  detail: string;
  /** 사람이 조치해야 하는가 — true 면 OS 배너를 겸한다(false 는 토스트만). */
  humanNeeded: boolean;
  /** 사람 손은 health, 기다림은 idle. */
  level: StarvedLevel;
}

/** 토스트 본문(detail)의 글자 수 상한(코드 포인트) — 넘으면 앞 199자 + `…` = 200자(usagebar.ts 의 말줄임과 같은 관례: 총 길이가 상한). */
export const STARVED_DETAIL_MAX = 200;
/** 데몬이 모든 remedy 끝에 붙이는 LLM 소비자용 꼬리의 **접두**(governance.rs REMEDY_LLM_SUFFIX 의 앞부분) — 사람에게는 불필요해 떼어 낸다.
 *  뒤따르는 괄호 안 금지 목록(강제 배달·드레인·키 주입·동결 해제·항목 삭제 …)은 데몬 판마다 늘 수 있어 문면 전체가 아니라 이 접두로 찾는다. */
export const STARVED_LLM_TAIL = " · LLM 에이전트는 자동 조치";
/** 사람이 조치해야 하는 remedy_code — 처방 code 14종(데몬 리뷰 확정 · governance.rs QUEUE_REMEDY_CODES) 가운데 calm 3종을 뺀 11종.
 *  ★(0.14.45 성찰 2회차 M3) `stale_screen` — 바쁨 표지가 60초 넘게 정적인데 cys 가 다시 그리기를 약속할 수 없어(노브 끔·대상 아님·창 너무 작음·요청했으나 미복구)
 *  처방이 사람에게 창 크기 변경·확인을 요구한다. 종전에는 같은 문장이 calm `wait` 로 와 OS 배너가 나가지 않았다. */
export const STARVED_HUMAN_CODES: readonly string[] = [
  "machine_residue",
  "phantom_count",
  "after_cursor_text",
  "human_draft",
  "input_pending_unknown",
  "answer_modal",
  "alt_screen",
  "empty_seat",
  "prompt_unknown",
  "stale_screen",
  "unknown",
];
/** 구버전 데몬이 보낼 수 있는 옛 code 이름 — `phantom_count_ctrl_u` 는 `phantom_count` 로 개명됐다(코드 이름에서 키 이름을 뺐다). 같은 뜻이라 똑같이 사람 조치 필요다. */
export const STARVED_LEGACY_HUMAN_CODES: readonly string[] = ["phantom_count_ctrl_u"];
/** 사람 조치가 필요 없는 remedy_code — 승인(approval)은 기존 승인 알림 경로가 맡고, 동결(paused)·일시 보류(wait)는 스스로 풀린다. */
export const STARVED_CALM_CODES: readonly string[] = ["approval", "paused", "wait"];
/**
 * ★(성찰 2회차 R2F-UI · A3 m1) 구버전 데몬(0.14.42 — payload 에 `remedy_code` 가 **아예 없다**)의 막힘 사유(`blocked_by`) 가운데 **스스로 풀리거나 사람 조치가 아닌** 것의 접두.
 * 코드가 없던 시절에는 모든 막힘이 '모르면 알린다'로 OS 배너까지 나가, 업데이트 직후 세션을 살려 둔 사용자가 워커가 10분 넘게 일할 때마다(좌석당 5분 간격) 배너를 받았다 —
 * 새 데몬은 같은 사유를 calm 코드(wait·approval·paused)로 내 배너를 내지 않는다. 이 접두에 걸리면 토스트만 낸다(새 데몬의 calm 3종과 같은 뜻).
 * 구성 = 데몬 `REMEDY_WAIT_PREFIXES`(wait 7종 — busy·delivery_interval·settle·quiescing·prompt_not_ready·human_typing·queue_paused) + `approval_pending`(approval) + `paused`(일시정지 — kill-switch 동결).
 * 데몬 소스를 읽는 어휘 핀(starvednotice.test.ts)이 이 목록을 묶는다 — 데몬이 wait 접두를 더하면 이 목록도 따라가야 한다. 코드가 **있는** payload 에는 쓰지 않는다(코드가 있으면 코드로만 가른다).
 */
export const STARVED_LEGACY_CALM_PREFIXES: readonly string[] = [
  "busy",
  "delivery_interval",
  "settle",
  "quiescing",
  "prompt_not_ready",
  "human_typing",
  "queue_paused",
  "approval_pending",
  "paused",
];

// ★(0.14.48 · DESIGN-D-v3 §2~4) 사람 손 경보는 그대로 두고, 기다림 3종은 상태 제목·idle 로 구분한다.
const STARVED_TITLE_HEAD_HUMAN = "⏳ 큐 막힘 —";
const STARVED_CALM_TITLE_HEADS = {
  wait: "⏳ 배달 대기 중 —",
  approval: "⏳ 승인 대기 중 —",
  paused: "⏳ 일시 정지 중 —",
};
const STARVED_REASON_HEAD_HUMAN = "막힘 사유:";
const STARVED_REASON_HEAD_CALM = "대기 사유:";

const ROLE_MAX = 40;
const BLOCKED_BY_MAX = 80;
const SLUG_MAX = 64;
/** 대기 분 표기의 상한(≈69일) — 이상한 값이 지수 표기로 화면을 어지럽히지 않게. */
const WAIT_MINUTES_MAX = 99_999;
/** `surface:<숫자>` — 데몬 `cys::surface_ref`(`format!("surface:{id}")`)의 모양. u64 최대가 20자리. */
const SURFACE_REF = /^surface:(\d{1,20})$/;
/** 제어문자·줄바꿈·양방향 제어·제로폭 문자 — 알림 한 줄을 속이거나 깨뜨릴 수 있는 것들. */
const INVISIBLE = /[\u{0}-\u{1f}\u{7f}-\u{9f}\u{200b}-\u{200f}\u{2028}\u{2029}\u{202a}-\u{202e}\u{2066}-\u{2069}\u{feff}]/gu;

/** 문자열만 받아 제어문자를 공백으로 바꾸고 공백을 접어 앞뒤를 다듬는다. 문자열이 아니면 빈 문자열. */
function clean(v: unknown): string {
  if (typeof v !== "string") return "";
  return v.replace(INVISIBLE, " ").replace(/\s+/g, " ").trim();
}

/** 코드 포인트 단위 절단 — 넘으면 앞 `max-1` 자 + `…`(총 `max` 자). 이모지 같은 서로게이트 쌍을 반으로 자르지 않는다. */
function clip(s: string, max: number): string {
  const cs = Array.from(s);
  return cs.length > max ? cs.slice(0, max - 1).join("") + "…" : s;
}

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);

/**
 * remedy_code → 사람이 조치해야 하는가. 코드가 **있으면** 코드로만 가른다 — calm 3종(approval·paused·wait)만 false, 모르는 값·문자열이 아닌 값은 **true**(모르면 알린다).
 * ★R2F-UI(A3 m1): 코드가 **아예 없으면**(`undefined` — 0.14.42 데몬의 payload) 막힘 사유(`blockedBy`)의 접두(STARVED_LEGACY_CALM_PREFIXES)로 가른다 — 스스로 풀리거나 사람 조치가 아닌 사유면 false(토스트만),
 * 그 밖·사유가 없거나 문자열이 아니면 true(모르면 알린다). `null` 같은 '있는데 이상한' 값은 코드가 없는 것이 아니다 — 알린다.
 */
export function starvedHumanNeeded(code: unknown, blockedBy?: unknown): boolean {
  if (code === undefined) {
    const b = clean(blockedBy);
    return !(b !== "" && STARVED_LEGACY_CALM_PREFIXES.some((p) => b.startsWith(p)));
  }
  return !(typeof code === "string" && STARVED_CALM_CODES.indexOf(code) >= 0);
}

/** 기다림의 종류 — 코드가 없는 구판은 기존 접두 판정을 따르며, queue_paused 는 공통 wait 로 둔다. */
export function starvedCalmKind(code: unknown, blockedBy?: unknown): "wait" | "approval" | "paused" | null {
  if (starvedHumanNeeded(code, blockedBy)) return null;
  if (code === "approval" || code === "paused" || code === "wait") return code;
  const b = clean(blockedBy);
  if (b.startsWith("approval_pending")) return "approval";
  if (b.startsWith("paused")) return "paused";
  return "wait";
}

/** DESIGN-D-v3 §4(B): 사람 손은 매번, 기다림은 구간의 마지막 제목이 달라질 때 팝업한다. */
export function starvedShouldPop(lastTitle: string | undefined, n: StarvedNotice): boolean {
  return n.humanNeeded || lastTitle !== n.title;
}

/** 이 UI 가 아는 remedy_code 인가(사람 조치 필요 ∪ 옛 이름 ∪ calm). 모르는 값이면 조치 문장을 믿지 않고 `blocked_by` 만 보인다. */
function knownCode(code: unknown): boolean {
  if (typeof code !== "string") return false;
  return STARVED_HUMAN_CODES.indexOf(code) >= 0 || STARVED_LEGACY_HUMAN_CODES.indexOf(code) >= 0 || STARVED_CALM_CODES.indexOf(code) >= 0;
}

/**
 * 막힌 시간의 분 표기 — `N분째`. **올림**이다: 59초 → "1분째" · 60초 → "1분째" · 61초 → "2분째" · 600초 → "10분째".
 * (막힌 지 1초라도 지났는데 "0분째"라고 말하지 않는다.) 유한한 양수가 아니면(0·음수·NaN·문자열·결측) 빈 문자열 — 지어내지 않는다.
 */
export function starvedWaitText(waitedSecs: unknown): string {
  if (typeof waitedSecs !== "number" || !Number.isFinite(waitedSecs) || waitedSecs <= 0) return "";
  return `${Math.min(WAIT_MINUTES_MAX, Math.max(1, Math.ceil(waitedSecs / 60)))}분째`;
}

/** 소켓 구분자(이벤트의 socket_slug) → id 조각. 없거나 문자열이 아니면 `"base"`(본부 데몬 — 구분 필드가 없던 시절의 단일 데몬 호환). */
export function starvedSlug(socketSlug: unknown): string {
  return clean(socketSlug).slice(0, SLUG_MAX) || "base";
}

const toastId = (socketSlug: unknown, surfaceRef: string): string => `starved:${starvedSlug(socketSlug)}:${surfaceRef}`;

/** `surface:<숫자>` 문자열에서 좌석 번호 — 그 꼴이 아니거나 안전한 정수가 아니면 null. */
export function surfaceIdOfRef(surfaceRef: unknown): number | null {
  const m = typeof surfaceRef === "string" ? SURFACE_REF.exec(surfaceRef) : null;
  if (!m) return null;
  const n = Number(m[1]);
  return Number.isSafeInteger(n) ? n : null;
}

/**
 * 좌석 번호(이벤트의 surface_id)로 그 좌석의 막힘·대기 토스트 id 를 만든다 — 막힘이 풀리거나(queue.delivered) 좌석이 끝났을 때(surface.exited·closed)
 * 토스트를 거두는 데 쓴다. id 의 모양은 starvedNotice().id 와 같은 함수 하나(toastId)가 만든다. 번호가 0 이상의 안전한 정수가 아니면 null.
 */
export function starvedDismissId(socketSlug: unknown, surfaceId: unknown): string | null {
  if (typeof surfaceId !== "number" || !Number.isSafeInteger(surfaceId) || surfaceId < 0) return null;
  return toastId(socketSlug, `surface:${surfaceId}`);
}

/** remedy 문장에서 LLM 소비자용 꼬리(" · LLM 에이전트는 자동 조치"로 시작해 끝까지)를 뗀다. 꼬리가 없으면 원문 그대로.
 *  앞뒤를 다듬은 문장이라 맨 앞에 붙은 꼬리의 앞 공백이 없을 수 있다 — 공백 하나를 앞에 붙여 찾는다(그 경우 조치 문장은 빈 문자열이 된다). */
function stripLlmTail(remedy: string): string {
  const i = (" " + remedy).indexOf(STARVED_LLM_TAIL);
  return (i >= 0 ? remedy.slice(0, Math.max(0, i - 1)) : remedy).trim();
}

/**
 * `queue.starved` payload → 알림 한 건. 화면에 올릴 수 없는 payload(객체가 아님·`surface_ref` 가 `surface:<숫자>` 꼴이 아님)는 null.
 * `socketSlug` = 이벤트를 낸 데몬의 구분자(Tauri 가 이벤트에 싣는 socket_slug) — 본부·부서 데몬의 같은 surface 번호가 id 에서 겹치지 않게 한다. 없으면 "base".
 *
 *  · humanNeeded — remedy_code 가 approval·paused·wait 가 아니면 true(모르는 값이면 true). 승인은 기존 승인 알림 경로가, 동결·일시 보류는
 *    스스로 풀리는 것이라 사람이 할 일이 없다. 코드가 **아예 없는** payload(구버전 데몬)는 `blocked_by` 의 접두로 가린다(starvedHumanNeeded · R2F-UI A3 m1) — 스스로 풀리는 사유면 토스트만.
 *  · detail — `<N분째> · <조치 문장>`. 조치 문장 = remedy 에서 LLM 꼬리를 뗀 것. remedy 가 없거나 비었거나 remedy_code 를 모르면(구버전 데몬·새 코드)
 *    사람 손은 `막힘 사유: <blocked_by>`, 기다림은 `대기 사유: <blocked_by>`(없으면 "알 수 없음"). 대기 시간이 유한한 양수가 아니면 분 표기만 생략한다. 총 길이는 200자 이내.
 */
export function starvedNotice(payload: unknown, socketSlug?: unknown): StarvedNotice | null {
  if (!isObj(payload)) return null;
  const ref = payload.surface_ref;
  if (typeof ref !== "string" || !SURFACE_REF.test(ref)) return null;

  const code = payload.remedy_code;
  const humanNeeded = starvedHumanNeeded(code, payload.blocked_by);
  const calmKind = starvedCalmKind(code, payload.blocked_by);
  const titleHead = calmKind === null ? STARVED_TITLE_HEAD_HUMAN : STARVED_CALM_TITLE_HEADS[calmKind];
  const remedy = knownCode(code) ? stripLlmTail(clean(payload.remedy)) : "";
  const blockedBy = clip(clean(payload.blocked_by), BLOCKED_BY_MAX);
  const sentence = remedy || `${humanNeeded ? STARVED_REASON_HEAD_HUMAN : STARVED_REASON_HEAD_CALM} ${blockedBy || "알 수 없음"}`;
  const wait = starvedWaitText(payload.waited_secs);

  return {
    id: toastId(socketSlug, ref),
    title: [titleHead, clip(clean(payload.role), ROLE_MAX), ref].filter((x) => x !== "").join(" "),
    detail: clip(wait ? `${wait} · ${sentence}` : sentence, STARVED_DETAIL_MAX),
    humanNeeded,
    level: humanNeeded ? "health" : "idle",
  };
}

/** 토스트를 눌렀을 때 갈 좌석을 확정하는 데 필요한 바깥 사실 — main.ts 가 주입한다(이 모듈은 전역·IPC 를 보지 않는다). */
export interface StarvedSeatLookup {
  /** slug → 부서 데몬 소켓(main.ts socketForSlug). 부서가 아니거나 미등록이면 undefined. */
  deptSocket(slug: string): string | undefined;
  /** 이 slug 가 본부(기본) 데몬의 것인가 — 본부 slug 는 deptSocket 에 없다(부서만 등록). 백엔드가 본부 소켓·이 번호로 만든 attach 이벤트 이름에 slug 가 들어 있는지로 판정한다. */
  isBaseSlug(slug: string, sid: number): Promise<boolean>;
  /** 그 데몬(소켓 · undefined=본부)의 워크스페이스 가운데 이 번호의 좌석을 가진 탭이 있는가(기동 중 자리표시 탭은 제외). */
  hasSeat(socket: string | undefined, sid: number): boolean;
}

/**
 * 막힘·대기 토스트를 **눌렀을 때** 갈 좌석의 소켓을 확정한다 — 확정하지 못하면 null(호출부는 무동작).
 * 좌석 번호(sid)는 데몬마다 독립 발급이라 번호만으로 고르면 **다른 데몬의 같은 번호 좌석**이 열린다(onDaemonEvent 의 surface.exited 가
 * slug 미해석을 기본 데몬으로 폴백하지 않는 이유와 같다). 그래서 데몬을 먼저 확정한다:
 *   · slug 가 비었으면(구분 필드 없음) 본부 · slug 가 부서 소켓으로 풀리면 그 부서 · 풀리지 않으면 본부인지 확인(isBaseSlug) — 아니면 null
 *     (예: 탭이 없는 부서의 경보 — 같은 번호의 본부 좌석을 열면 안 된다)
 *   · 그 데몬에 그 번호의 좌석을 가진 탭이 없으면 null
 * 확인 중 오류도 null — 눌러도 아무 일이 없는 쪽이 엉뚱한 좌석을 여는 쪽보다 안전하다.
 */
export async function locateStarvedSeat(
  socketSlug: string,
  sid: number,
  look: StarvedSeatLookup,
): Promise<{ socket: string | undefined } | null> {
  try {
    let socket: string | undefined;
    if (socketSlug !== "") {
      socket = look.deptSocket(socketSlug);
      if (socket === undefined && !(await look.isBaseSlug(socketSlug, sid))) return null;
    }
    return look.hasSeat(socket, sid) ? { socket } : null;
  } catch {
    return null;
  }
}
