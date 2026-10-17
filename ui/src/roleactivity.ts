// ★F-01(0.14.52 · U) 역할 점 활동 표시 — 순수 판정 + 바뀐 것만 쓰는 칠하기. 타이머·IPC·전역 상태 0(칠하기 캐시 WeakMap 하나뿐).
//
// 무엇을 하나: 데몬이 `surface.list` 의 각 줄에 싣는 `display_activity`(src/bin/cysd/activity.rs `wire_at`)를 믿을 수 있는 만큼만 받아,
//   점을 다섯 가지 중 하나로 보인다 — 작업 중(깜박임) · 대기(고정) · 입력 대기(구별되는 고정 주의 표지) · 상태 확인 지연(U4-5 · 빈 원) · 모름(빈 원).
//   역할색은 그대로다(역할색은 role 이 정하고 이 모듈은 상태 표지만 정한다). 알림·토스트·배지·feed 는 만들지 않는다.
//
// 시간 계약(DESIGN-U-FROZEN-R3 R3-4 · 헤드 판정 U4-5) — 전부 **단조 시계(ms)** 위의 값이다(벽시계 빼기 금지):
//   P=3 s(목록 주기) · D=T_LIST(8 s mac / 16 s Windows — 호출부가 넘긴다) · E=6 s(전경 지연 허용) · L=3 s(국소 만료 쓸기 = 기존 3초 틱)
//   T = 3P + 2D + E = 31 s / 47 s — 그 응답을 낳은 **요청의 시작** 시각에 묶인다(수신 시각 아님 — 늦은 응답이 새 수명을 사지 못한다).
//   U4-5: 작업 중인데 응답 나이(지금 − 요청 시작)가 생산자 리스(6 s) + 목록 한 주기(3 s) = 9 s 를 넘으면 깜박임을 멈추고 '모름' 꼴의 고정 표지로
//   낮추며 접근 문구에 그 나이를 적는다(알림 없음). T 는 그와 별개인 하드 만료다. UI 는 데몬의 K(일시 거부 허용)를 다시 적용하지 않고
//   어떤 리스도 연장하지 않는다 — 데몬이 직렬화 때 자기 6초 리스를 이미 집행한다.

export const ACTIVITY_P_MS = 3_000;
export const ACTIVITY_E_MS = 6_000;
export const ACTIVITY_L_MS = 3_000;
export const PRODUCER_LEASE_MS = 6_000;
/** U4-5 — 두 OS 공통 9 s(생산자 리스 + 목록 한 주기). */
export const STALE_WORKING_MS = PRODUCER_LEASE_MS + ACTIVITY_P_MS;
/** T = 3P + 2D + E (소켓 수 N 과 무관). D = 기존 T_LIST. */
export const activityExpiryMs = (dMs: number): number => 3 * ACTIVITY_P_MS + 2 * dMs + ACTIVITY_E_MS;

export type ActivityState = "working" | "waiting" | "waiting_for_input" | "unknown";
export type ActivityRaw = "active" | "idle" | "permission" | "question" | "unknown";
export type ActivityFamily = "claude" | "codex" | "gemini";

export interface ActivityObservation {
  kind: "observation";
  state: ActivityState;
  raw: ActivityRaw;
  reason: string;
  family: ActivityFamily | null;
  grammarRevision: string | null;
  observedAgeSecs: number | null;
  lastPositiveAgeSecs: number | null;
  stableAgeSecs: number | null;
  seq: number;
  epoch: string;
  retained: boolean;
  consecutiveRejections: number;
}
/** 관측 없음·무효화·종료·관측기 꺼짐(데몬의 퇴화형) · 구버전 데몬(키 없음) · 형식 오류 — 셋 다 Unknown 이다. */
export interface ActivityUnknown {
  kind: "unknown";
  reason: string;
  malformed: boolean;
}
export type ParsedActivity = ActivityObservation | ActivityUnknown;

const STATES: readonly string[] = ["working", "waiting", "waiting_for_input", "unknown"];
const RAWS: readonly string[] = ["active", "idle", "permission", "question", "unknown"];
const FAMILIES: readonly string[] = ["claude", "codex", "gemini"];
const MAX_TEXT = 200; // 사유·세대 문자열 상한(IPC 데이터라 전부 의심한다)

const MISSING: ActivityUnknown = { kind: "unknown", reason: "활동 정보 없음(구버전 데몬)", malformed: false };
const MALFORMED: ActivityUnknown = { kind: "unknown", reason: "활동 정보 형식 오류", malformed: true };

const isAge = (v: unknown): v is number | null => v === null || (typeof v === "number" && Number.isFinite(v) && v >= 0);
const isCount = (v: unknown): v is number => typeof v === "number" && Number.isSafeInteger(v) && v >= 0;
const isText = (v: unknown): v is string => typeof v === "string" && v.length <= MAX_TEXT;

/** 데몬 `display_activity` 한 개 → 판정. 빠짐 = 구버전 데몬(Unknown) · 꼴이 하나라도 어긋나면 형식 오류(Unknown). */
export function parseDisplayActivity(v: unknown): ParsedActivity {
  if (v === undefined) return MISSING;
  if (v === null || typeof v !== "object" || Array.isArray(v)) return MALFORMED;
  const o = v as Record<string, unknown>;
  if (typeof o.state !== "string" || !STATES.includes(o.state)) return MALFORMED;
  if (typeof o.raw !== "string" || !RAWS.includes(o.raw)) return MALFORMED;
  if (!isText(o.reason) || typeof o.retained !== "boolean") return MALFORMED;
  // 퇴화형 — 신원(세대·순번) 없는 Unknown 만 정상이다. 신원 없는 Working/대기는 믿을 근거가 없다.
  if (!("display_epoch" in o) && !("observation_seq" in o)) {
    return o.state === "unknown" ? { kind: "unknown", reason: o.reason, malformed: false } : MALFORMED;
  }
  if (!isText(o.display_epoch) || o.display_epoch.length === 0 || !isCount(o.observation_seq)) return MALFORMED;
  if (!isAge(o.observed_age_secs) || !isAge(o.last_positive_age_secs) || !isAge(o.stable_age_secs)) return MALFORMED;
  if (!(o.family === null || (typeof o.family === "string" && FAMILIES.includes(o.family)))) return MALFORMED;
  if (!(o.grammar_revision === null || isText(o.grammar_revision))) return MALFORMED;
  if (!isCount(o.consecutive_rejections)) return MALFORMED;
  if (o.state === "working" && o.observed_age_secs === null) return MALFORMED; // 나이를 모르는 Working 은 보수적으로 늙힐 수 없다
  return {
    kind: "observation",
    state: o.state as ActivityState,
    raw: o.raw as ActivityRaw,
    reason: o.reason,
    family: o.family as ActivityFamily | null,
    grammarRevision: o.grammar_revision,
    observedAgeSecs: o.observed_age_secs,
    lastPositiveAgeSecs: o.last_positive_age_secs,
    stableAgeSecs: o.stable_age_secs,
    seq: o.observation_seq,
    epoch: o.display_epoch,
    retained: o.retained,
    consecutiveRejections: o.consecutive_rejections,
  };
}

export type AcceptVerdict =
  | "accepted"
  | "expired"
  | "wrong-lifetime"
  | "out-of-order"
  | "epoch-regression"
  | "sequence-regression"
  | "duplicate-sequence";

export interface ActivityRecord {
  /** 그 응답을 받은 끝점 수명(surfacelist.ts 가 매긴 전역 증가 번호). */
  readonly lifetime: number;
  /** 그 응답을 낳은 요청의 **시작** 단조 시각 — 만료(T)·U4-5·보수적 나이의 유일한 앵커. */
  readonly startMs: number;
  readonly receivedMs: number;
  readonly parsed: ParsedActivity;
  /** 이 키에서 마지막으로 받아들인 관측 세대·순번(퇴화형 Unknown 은 바꾸지 않는다). */
  readonly epoch: string | null;
  readonly seq: number;
  /** 물러난 세대(되돌아오면 거부) — 상한 4. */
  readonly retired: readonly string[];
}
const RETIRED_CAP = 4;

/**
 * 표시 전용 저장소 — 키 = pane 런타임 키(paneKey(sid, socket)). 응답 하나를 **그 응답의 수명·요청 시작**과 함께 받아,
 * 늦은 만료 응답 · 옛 끝점 수명 · 되돌아온 세대 · 줄지 않는 순번을 거부한다. 같은 순번이라도 생산자 Unknown(데몬이 직렬화 때 리스를 끊은 것)은
 * 즉시 받아들인다 — 생산자 Unknown 이 UI 의 Working 보다 언제나 이긴다(R3-4).
 */
export class RoleActivityStore {
  private readonly recs = new Map<string, ActivityRecord>();
  constructor(private readonly expiryMs: number) {}

  accept(key: string, lifetime: number, startMs: number, receivedMs: number, wire: unknown): AcceptVerdict {
    if (receivedMs - startMs >= this.expiryMs) return "expired"; // 받는 순간 이미 start+T — 새 수명을 사지 못한다
    let rec = this.recs.get(key);
    if (rec && lifetime < rec.lifetime) return "wrong-lifetime";
    if (rec && lifetime > rec.lifetime) rec = undefined; // 새 끝점 수명 — 처음부터
    if (rec && startMs < rec.startMs) return "out-of-order";
    const parsed = parseDisplayActivity(wire);
    let epoch = rec?.epoch ?? null;
    let seq = rec?.seq ?? -1;
    let retired = rec?.retired ?? [];
    if (parsed.kind === "observation") {
      if (rec && rec.epoch === parsed.epoch) {
        if (parsed.seq < rec.seq) return "sequence-regression";
        if (parsed.seq === rec.seq && parsed.state !== "unknown") return "duplicate-sequence";
      } else if (rec && rec.epoch !== null) {
        if (rec.retired.includes(parsed.epoch)) return "epoch-regression";
        retired = [...rec.retired, rec.epoch].slice(-RETIRED_CAP);
      }
      epoch = parsed.epoch;
      seq = parsed.seq;
    }
    this.recs.set(key, { lifetime, startMs, receivedMs, parsed, epoch, seq, retired });
    return "accepted";
  }

  get(key: string): ActivityRecord | undefined {
    return this.recs.get(key);
  }

  /** keep 이 false 인 키를 버린다(사라진 좌석 · 로스터에서 빠진 끝점). */
  prune(keep: (key: string) => boolean): void {
    for (const k of [...this.recs.keys()]) if (!keep(k)) this.recs.delete(k);
  }

  get size(): number {
    return this.recs.size;
  }
}

export type DotView = "working" | "waiting" | "input" | "stale" | "unknown";
export interface DotProjection {
  view: DotView;
  text: string;
}

/** 보수적 나이(초, 올림) — 데몬이 직렬화한 단조 나이 + 요청 시작 이후 흐른 시간(직렬화는 요청 시작 이후에 일어났다). */
const ageText = (wireSecs: number | null, elapsedMs: number): string | null =>
  wireSecs === null ? null : `${Math.ceil(wireSecs + elapsedMs / 1000)}초 전`;

/** 기록 + 지금(단조 ms) → 점 상태와 접근 문구(상태 · 사유 · 나이). 순수 함수. */
export function projectActivity(rec: ActivityRecord | undefined, nowMs: number, expiryMs: number): DotProjection {
  if (!rec) return { view: "unknown", text: "상태 모름 · 관측 없음" };
  const elapsedMs = Math.max(0, nowMs - rec.startMs);
  if (elapsedMs >= expiryMs) return { view: "unknown", text: `상태 모름 · 응답 만료 — 마지막 응답 ${Math.ceil(elapsedMs / 1000)}초 전` };
  const p = rec.parsed;
  if (p.kind === "unknown") return { view: "unknown", text: `상태 모름 · ${p.reason}` };
  const seen = ageText(p.observedAgeSecs, elapsedMs);
  const tail = seen === null ? "" : ` · 관측 ${seen}`;
  switch (p.state) {
    case "working": {
      // U4-5: conservative response age = daemon-serialized observation age + elapsed since request start.
      const ageMs = (p.observedAgeSecs ?? 0) * 1000 + elapsedMs;
      if (ageMs > STALE_WORKING_MS) return { view: "stale", text: `상태 확인 지연 — 작업 중 표시를 멈춤 · 마지막 관측 ${Math.ceil(ageMs / 1000)}초 전` };
      const lp = ageText(p.lastPositiveAgeSecs, elapsedMs);
      const kept = p.retained ? ` · 일시 유지(연속 거부 ${p.consecutiveRejections}회${lp === null ? "" : ` · 마지막 활동 ${lp}`})` : "";
      return { view: "working", text: `작업 중 · ${p.reason}${tail}${kept}` };
    }
    case "waiting":
      return { view: "waiting", text: `대기 · ${p.reason}${tail}` };
    case "waiting_for_input":
      return { view: "input", text: `입력 대기 · ${p.reason}(${p.raw})${tail}` };
    default:
      return { view: "unknown", text: `상태 모름 · ${p.reason}${tail}` };
  }
}

/** 상태 → CSS 클래스(style.css `.pane .pane-role-dot.act-*`). */
export const DOT_CLASS: Readonly<Record<DotView, string>> = {
  working: "act-working",
  waiting: "act-waiting",
  input: "act-input",
  stale: "act-stale",
  unknown: "act-unknown",
};

export interface DotPaint {
  visible: boolean;
  /** 역할색 — `--role-color` 로만 싣는다(인라인 background 금지: 빈 원 클래스가 이겨야 한다). */
  color: string;
  view: DotView;
  /** 접근 문구 — title 과 aria-label 에 같은 값. */
  label: string;
}
/** 무역할 셸·종료 좌석 — 종전처럼 숨긴다. */
export const HIDDEN_DOT: DotPaint = { visible: false, color: "", view: "unknown", label: "" };

/** 칠하기 대상의 최소 꼴(HTMLElement 가 만족한다 — 시험은 대역을 쓴다). */
export interface DotEl {
  title: string;
  style: { display: string; setProperty(name: string, value: string): void };
  classList: { add(c: string): void; remove(c: string): void };
  setAttribute(name: string, value: string): void;
}
interface Applied {
  display: string | null;
  color: string | null;
  cls: string | null;
  label: string | null;
  role: boolean;
}
const applied = new WeakMap<object, Applied>();

/** 바뀐 클래스·속성만 쓴다(같은 칠하기는 쓰기 0). 전체 렌더·호스트 교체·포커스·스크롤과 무관하다. 반환 = 쓴 항목 수. */
export function applyDotPaint(el: DotEl, next: DotPaint): number {
  const a: Applied = { ...(applied.get(el) ?? { display: null, color: null, cls: null, label: null, role: false }) };
  let w = 0;
  const display = next.visible ? "" : "none";
  if (a.display !== display) {
    el.style.display = display;
    a.display = display;
    w++;
  }
  if (next.visible) {
    if (a.color !== next.color) {
      el.style.setProperty("--role-color", next.color);
      a.color = next.color;
      w++;
    }
    const cls = DOT_CLASS[next.view];
    if (a.cls !== cls) {
      if (a.cls) el.classList.remove(a.cls);
      el.classList.add(cls);
      a.cls = cls;
      w++;
    }
    if (!a.role) {
      el.setAttribute("role", "img");
      a.role = true;
      w++;
    }
    if (a.label !== next.label) {
      el.title = next.label;
      el.setAttribute("aria-label", next.label);
      a.label = next.label;
      w++;
    }
  }
  applied.set(el, a);
  return w;
}
