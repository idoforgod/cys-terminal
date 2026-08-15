// selfdiag.ts — 자가진단·CEO 승격 팔레트 노출 결정(D4 · 결정 D4)의 순수 로직.
//
// 팔레트에는 CEO 관련 항목이 둘 있다: 'CEO 승격 진행'(PENDING 티켓 해소=promote_pending_ceo)과
// 'CEO 승격 재실행(템플릿 전진 적용)'(드리프트 해소=approve_ceo_promotion). 두 신호가 동시에 참인
// 비정형 창(승격 뒤 새 PENDING 생성 등)에서 둘을 같이 띄우면 오너가 순서를 고를 수 없고, '재실행'이
// 먼저 눌리면 최초 승격 절차(부트 게이트 경유)가 역전된다 — 그래서 **상호 배타·pending 우선**을
// 여기 순수 함수 하나로 고정한다(main.ts 는 이 결과를 그대로 배선만 한다).

/// 데몬/파일 실측 신호(각 invoke 는 실패 시 false 로 접힌다 — 노출 억제가 안전 기본값).
export interface CeoGateSignals {
  /// ~/.cys/state/ceo-pending 존재 — 최초 승격이 부트 게이트로 보류된 상태(ceo_pending).
  pending: boolean;
  /// .pre-ceo 존재 ∧ md≠라이브 CEO_TEMPLATE — 승격본이 릴리스 전진으로 구본화(ceo_promotion_drift).
  drift: boolean;
}

export type CeoPaletteEntry = "pending" | "repromote";

/// 노출할 CEO 팔레트 항목(0~1개). pending 이 항상 우선한다(최초 승격 미완에 '재실행' 권유 금지).
export function ceoPaletteEntries(g: CeoGateSignals): CeoPaletteEntry[] {
  if (g.pending) return ["pending"];
  if (g.drift) return ["repromote"];
  return [];
}
