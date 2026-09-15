// ui/src/ctxpick.ts — CTX 축 선택 단일 소스(★WP6-2 · 0.14.31 감사).
//
// 같은 이름으로 불리는 CTX 값이 이 제품에 **두 개** 있다:
//   ① 실측 usage.ctx_pct — 데몬(cysd usage.rs)이 세션 파일을 tail 해 계산. 낡으면 null 로 내려간다.
//   ② 자기보고 status.context_pct — 노드가 `cys set-status --context <추정%>` 로 타이핑한 추정치.
//      데몬은 상한만 자르고(handlers.rs status.set) 값 자체에 유효기간이 없어 낡아도 그대로 남는다.
// 종전에는 60% 임계를 읽는 자리마다 축이 제각각이었고(자기보고 전용 / 자기보고 우선 / 결측을 0 으로
// 접음), 결측을 0 으로 접는 자리는 신고 없는 좌석(부팅 직후·죽은 좌석·agy)을 "0%" 로 위장해 60%
// 목록에서 **조용히 뺐다**. 여기 한 벌로 모은다.
//
// 정본 규칙은 cysjavis-pack/bin/javis_hud_bridge.py pick_ctx() 와 같다:
//   실측(usage.ctx_pct) > 자기보고(status.context_pct). 결측은 null 이지 0 이 아니다.
// 자기보고는 **신선할 때만**(마지막 set-status 후 CTX_SELF_REPORT_MAX_AGE_SECS 이내) 보조 축이다.
//
// ★실패 방향: 못 재면 목록에서 **빠진다**(isHotCtx=false) — 0% 로 위장하지 않는다. 자기보고가 있는데
//   낡음/나이 미상이면 src="stale"(pct null)로 돌려 화면이 "판정 불가"를 그리게 한다(숨기지 않는다).
// ★같은 수 300 이 세 언어에 흩어져 있다 — Rust(WP6-1 CTX_SELF_REPORT_MAX_AGE_SECS) ·
//   Python(javis_report.py CTX_SELF_REPORT_MAX_AGE_S) · 여기. 바꿀 때는 셋을 함께 고쳐라 —
//   한 자리만 고치면 화면과 경보가 다시 갈린다.
export const CTX_SELF_REPORT_MAX_AGE_SECS = 300;
// "stale" = 자기보고는 있으나 낡았거나(age>상한) 나이 미상 — pct 는 null(판정 불가)이되 화면은 그 사실을
//   **보이게** 그린다(오너 원칙: 없으면 없다고 표시하고 그 사실이 보이게 하라). isHotCtx 는 stale 을 false 로 본다.
export type CtxSource = "measured" | "self" | "stale" | "none";
export function pickCtx(n: any): { pct: number | null; src: CtxSource } {
  const m = n?.usage?.ctx_pct;
  if (typeof m === "number") return { pct: m, src: "measured" };
  const s = n?.status?.context_pct;
  if (typeof s !== "number") return { pct: null, src: "none" };
  const age = n?.status?.age_secs;
  // 나이를 모르면 "방금"이 아니라 "모른다" — 낡은 추정으로 임계를 읽지 않는다.
  if (typeof age !== "number" || age > CTX_SELF_REPORT_MAX_AGE_SECS)
    return { pct: null, src: "stale" };
  return { pct: s, src: "self" };
}
/** 60% 임계 판정 — 결측은 false 다(0 으로 접지 않는다). */
export function isHotCtx(n: any, threshold = 60): boolean {
  const { pct } = pickCtx(n);
  return pct !== null && pct >= threshold;
}
