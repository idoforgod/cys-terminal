// ctxrender.ts — CTX 칸의 **그리기 규칙**만 담은 순수 모듈 (P11 / C-30).
//
// ★왜 별 모듈인가: 세 화면(좌석 배지 · CC 함대 행 · Tasks 막대)이 같은 "값이 없다"를 서로 다르게
//   그려서, 사람이 "문제 없음"과 "아무도 재지 않음"을 구별할 수 없었다(C-30). 규칙을 한 곳에 모아
//   **행동으로** 시험한다(문자열 매칭 핀이 아니라 반환값 단언).
// ★선택기(ctxpick.ts pickCtx)는 건드리지 않는다 — 여기 있는 것은 이미 정해진 결과의 **표현**뿐이다.
// ★절대 규칙(설계 D1-R2/D3-R3): 각 화면은 **자기 payload 가 싣는 것만** 말한다. surface.list 에는
//   status 가 없고 control.dashboard 에는 status/exited/agent_alive 가 없으므로, 그 두 화면의 글에는
//   자기보고·가드·보호에 대한 어떤 문장도 넣지 않는다.

export type BadgeCtxPart = { text: string; cls: string; tip: string | null };

/** 좌석 배지(surface.list payload)의 CTX 조각. null = 아무것도 그리지 않는다. */
export function badgeCtxPart(u: { ctx_pct?: number | null; source?: string } | null | undefined): BadgeCtxPart | null {
  if (!u) return null;
  const p = u.ctx_pct;
  if (typeof p === "number") {
    return { text: `CTX ${p}%`, cls: p >= 80 ? "crit" : p >= 60 ? "warn" : "", tip: null };
  }
  // ★D1-R2: 실측값이 없으면 **조용히 빠지지 않는다**. 중립 회색 `unmeasured`(warn/crit 아님)로
  //   "없다"를 그리고, 툴팁에는 이 행이 실제로 싣고 있는 것만 적는다 — surface.list 에는 status 가
  //   없으므로 자기보고·가드·보호를 말할 근거가 이 화면에 없다.
  const src = typeof u.source === "string" ? u.source : "";
  const stale = src.endsWith(":stale") ? " · 낡은 매핑" : "";
  return {
    text: "CTX —",
    cls: "unmeasured",
    tip: `컨텍스트 실측값 없음 (이 행에 실린 usage.ctx_pct = null · source ${src})${stale}`,
  };
}

export type FleetCtxCell = { text: string; title: string };

/**
 * CC 함대 행(control.dashboard payload)의 CTX 칸. 실측값만 싣는 화면이다.
 * ★항상 칸을 돌려준다 — 종전처럼 빈 문자열로 사라지는 갈래는 없다(그것이 C-30 결함이었다).
 */
export function fleetCtxCell(f: { usage?: { ctx_pct?: number | null } | null } | null | undefined): FleetCtxCell {
  const p = f?.usage?.ctx_pct;
  if (typeof p === "number") {
    return { text: `CTX ${p}%`, title: "컨텍스트 사용률 — 모델 컨텍스트 창 대비" };
  }
  // ★D3-R3: control.dashboard 행에는 status·exited·agent_alive 가 **없다**. 그래서 pickCtx 로
  //   올리지 않고(그렇게 하면 신선한 자기보고 64 를 "없음"이라 말하게 된다 — codex r1 재현),
  //   이 화면이 아는 사실 하나만 적는다: 실측값이 없다.
  return { text: "CTX —", title: "컨텍스트 실측값 없음 (이 화면은 실측값만 싣는다)" };
}

/**
 * Tasks 막대(org.status payload)에서 pickCtx 결과가 값 없음일 때의 표시.
 * ★두 갈래 모두 그린다 — `stale`=`?`(종전), `none`=`—`(신설). 빈 칸으로 돌려주는 갈래는 없다.
 */
export function barCtxNoValue(src: "stale" | "none"): { value: string; title: string } {
  if (src === "stale") {
    return {
      value: "?",
      title: "컨텍스트 판정 불가 — 좌석 종료/에이전트 사망(동결값) 또는 실측 없음 · 자기보고 낡음/나이 미상",
    };
  }
  // ★D2-R2: `none`(실측도 자기보고도 없음)은 종전에 **빈 칸**이었다 — "문제 없음"과 구별되지 않는다.
  //   `stale`(?) 과는 다른 낱말(—)을 쓴다. 두 상태를 한 글자로 합치지 않는다.
  return { value: "—", title: "현재 쓸 수 있는 컨텍스트 값 없음" };
}
