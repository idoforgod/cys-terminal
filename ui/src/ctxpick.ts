// ui/src/ctxpick.ts — CTX 축 선택 단일 소스(★WP6-2 · 0.14.31 감사).
//
// 같은 이름으로 불리는 CTX 값이 이 제품에 **두 개** 있다:
//   ① 실측 usage.ctx_pct — 데몬(cysd usage.rs)이 세션 파일을 tail 해 계산. 낡으면 null 로 내려가는 것은
//      **휴리스틱 매핑뿐**이다(아래 ★실측 축의 낡음).
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
// ★실측 축의 낡음(0.14.31 감사 정정) — 데몬이 낡은 실측을 null 로 지우는 범위는 **휴리스틱 매핑뿐**이다:
//   cysd/usage.rs mapping_is_fresh 는 등록 매핑(heuristic=false · SessionStart 등록 = 통상의 claude 경로)의
//   나이를 보지 않고, idle_stale_transition 은 source=="statusline" 을 건드리지 않으며, collect_tick 은
//   exited·agent_meta 없는 좌석을 건너뛴다 → 등록·statusline·종료 좌석의 usage.ctx_pct 는 마지막 값에
//   **동결**된 채 org.status 에 실린다(`exited: true` 와 함께 · usage.updated_at 도 실리지만 읽지 않는다).
//   실측에 300s 나이 게이트를 걸지 **않는다** — idle 이어도 산 좌석의 실측은 정확하고, 걸면 조용한 좌석
//   전부가 `?` 가 돼 오경보가 된다(master 결정). 대신 페이로드의 사망 신호 두 축으로 막는다 —
//   `exited`(pane 종료 · state.rs reader 가 EOF 에서 세운다) ∨ `agent_alive === false`(governance 워치독 3상 ·
//   false = 관측된 사망 확정만 · main.ts 도 `agent_alive !== false` 로 소비). 워치독은 exited 좌석을 건너뛰어
//   agent_alive 가 동결되므로 한 축만 보면 반쪽이다(0.14.31 후속 · Rust ctx_cell·Py pick_node_ctx 와 대칭).
// ★실패 방향: 못 재면 목록에서 **빠진다**(isHotCtx=false) — 0% 로 위장하지 않는다. 자기보고가 있는데
//   낡음/나이 미상이면 src="stale"(pct null)로 돌려 화면이 "판정 불가"를 그리게 한다(숨기지 않는다).
//   `exited === true` 또는 `agent_alive === false` 면 실측·자기보고가 있어도 src="stale"(pct null) — 동결값을
//   산 값으로 읽지 않는다. 부재·null·(exited=false / agent_alive=true) 는 게이트를 열지 않는다(구버전 데몬
//   페이로드 무해 · null 은 "모른다"이지 "죽었다"가 아니다). 잔여 한계: 한 번도 관측되지 않은 채
//   (agent_alive null) 죽은 좌석과 사망 뒤 워치독 틱 전의 창은 잡히지 않는다(의도).
// ★같은 수 300 이 네 언어에 흩어져 있다 — Rust(WP6-1 cys.rs CTX_SELF_REPORT_MAX_AGE_SECS) ·
//   Python 보고(javis_report.py CTX_SELF_REPORT_MAX_AGE_S) · Python HUD(javis_hud_bridge.py
//   CTX_SELF_REPORT_MAX_AGE_S) · 여기. 바꿀 때는 넷을 함께 고쳐라 — 한 자리만 고치면 화면과 경보가 다시 갈린다.
export const CTX_SELF_REPORT_MAX_AGE_SECS = 300;
// "stale" = 자기보고는 있으나 낡았거나(age>상한) 나이 미상 — pct 는 null(판정 불가)이되 화면은 그 사실을
//   **보이게** 그린다(오너 원칙: 없으면 없다고 표시하고 그 사실이 보이게 하라). isHotCtx 는 stale 을 false 로 본다.
export type CtxSource = "measured" | "self" | "stale" | "none";
export function pickCtx(n: any): { pct: number | null; src: CtxSource } {
  // 사망 게이트 — 데몬 수집기가 건너뛰는 좌석의 동결 실측을 산 값으로 읽지 않는다(머리말 ★실측 축의 낡음).
  if (n?.exited === true || n?.agent_alive === false) return { pct: null, src: "stale" };
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
