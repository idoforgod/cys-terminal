// REVISE-3a/R-6 — `context.threshold` 알림 문구를 **발화 출처(source)** 에 따라 결정하는 순수 함수.
// main.ts 의 onDaemonEvent 가 이 판정에 배선만 하고, toast/osBanner 호출은 호출측이 한다.
//
// 왜 순수 분리인가: 이 문구는 수신자(오너·master·CSO)가 "저 pane 을 지금 /clear 해도 되는가"를
// 판단하는 근거다. 데몬은 귀속이 확실한 관측(`observed`)과 **확실하지 않은 채로 고위험 임계에서
// 어쩔 수 없이 발화한 관측**(`observed-uncertain`)을 payload.source 로 이미 구분해 보내는데
// (usage.rs `observed_fire_source` · UNCERTAIN_FIRE_PCT=85), UI 가 둘을 같은 확신 어조로 찍으면
// 그 구분이 화면에서 소멸한다 → 엉뚱한 pane 을 순환시켜 그 pane 의 작업이 날아간다.
// 오탐의 대가가 큰 판정이라 DOM 없이 결정론으로 회귀 테스트한다(ctxthreshold.test.ts) —
// deadagent.ts · deptlabel.ts 와 동일 관용구.
//
// 계약:
//   · `source === "observed-uncertain"` 일 때만 불확실 표기를 병기한다. 나머지 출처
//     (`self-report`·`observed`·`statusline`·필드 부재[구 데몬])는 **종전 문구 그대로** —
//     additive 스큐 안전(모르는 값은 확신 어조가 아니라 종전 어조로 떨어진다).
//   · 표기는 문구 병기일 뿐이다. 이벤트를 삼키거나 OS 배너 임계(≥80)를 바꾸지 않는다 —
//     고위험 폴백 발화가 UI 단에서 조용히 사라지면 C1 폴백을 도입한 취지가 죽는다.

export interface ContextThresholdPayload {
  // 데몬 `context.threshold` payload 의 부분집합(추가 필드는 무시 — additive 스큐 안전).
  context_pct?: number | string | null;
  threshold?: number | string | null;
  role?: string | null;
  surface_ref?: string | null;
  action?: string | null;
  source?: string | null;
  attribution?: string | null;
}

/** 데몬이 '귀속 불확실'로 선언한 발화인가. 이 문자열 하나만 참이다(usage.rs 와 동일 리터럴). */
export const UNCERTAIN_SOURCE = "observed-uncertain";

/** 확신 어조와 구별하기 위해 병기하는 꼬리표. */
export const UNCERTAIN_NOTE = "(귀속 불확실 — pane 확인 필요)";

export function isUncertainAttribution(source: unknown): boolean {
  return source === UNCERTAIN_SOURCE;
}

/** 토스트·OS 배너 제목. 출처와 무관하게 동일하다(퍼센트가 제목의 전부다). */
export function contextThresholdTitle(p: ContextThresholdPayload | null | undefined): string {
  return `🔋 컨텍스트 ${p?.context_pct}%`;
}

/**
 * 토스트·OS 배너 본문. 기존 문구(`role surface ≥ N% — action`)를 그대로 두고, 귀속 불확실일
 * 때만 꼬리표를 **덧붙인다**(치환이 아니라 병기 — 원 정보 손실 0).
 */
export function contextThresholdBody(p: ContextThresholdPayload | null | undefined): string {
  const base = `${p?.role ?? ""} ${p?.surface_ref ?? ""} ≥ ${p?.threshold}% — ${p?.action ?? ""}`;
  return isUncertainAttribution(p?.source) ? `${base} ${UNCERTAIN_NOTE}` : base;
}

/** OS 배너(고우선)로 올릴 것인가 — 종전 계약 그대로 ≥80%. 출처는 이 판정을 바꾸지 않는다. */
export function shouldOsBanner(p: ContextThresholdPayload | null | undefined): boolean {
  return Number(p?.context_pct ?? 0) >= 80;
}
