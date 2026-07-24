// 팀 기동 경고 배너(boot-warn) 수명 판정 — 순수 함수 (능력파리티 W3/T3 · 2026-07-24).
//
// 배경: "팀 기동 경고" 배너는 Tauri app.emit("boot-warning")(src-tauri spawn_orchestra_boot)로
// 뜨지만 자동 소멸이 없었다(불멸 배너). 편성(javis_formation.py)이 설치 CLI 기준으로 팀을 완결하면
// 데몬 feed 버스로 formation-* 상태가 흘러온다(feed.item.created · payload.kind). 그 신호로 배너의
// 수명을 결정한다 — complete=소멸, partial=문구 갱신(아직 미완이므로 제거 아님), 그 외=무시.
//
// 전달 경로(실측): javis_formation._surface → `cys feed push --kind formation-*` → 데몬 EventBus
// publish("feed.item.created","feed",…) → spawn_event_forwarder → app.emit("daemon-event") →
// onDaemonEvent. 이 수신 코드는 앱 빌드에 포함되므로 버전 스큐는 "신팩+구앱"만 문제다(→ min_binary
// _version 게이트, T6). 계약이 산문 아닌 kind 문자열이라 회귀 0 을 테스트로 못박는다.

export type BootBannerDecision =
  | { action: "dismiss" } // 편성 완결 → 배너 제거
  | { action: "update"; text: string } // 부분 편성 → 문구 갱신(배너 유지)
  | { action: "ignore" }; // 관련 없음/미완/실패 → 배너 무변경

// 부분 편성 시 배너에 노출할 문구(제거하지 않고 갱신 — 아직 편성 미완).
export const PARTIAL_BANNER_TEXT =
  "일부 노드만 편성됐습니다 — 나머지 CLI 설치 시 자동으로 편성이 완결됩니다.";

/**
 * feed.item.created 의 payload.kind → 배너 조치.
 * formation-complete=dismiss · formation-partial=update · 그 외(pending/failed/비-formation)=ignore.
 */
export function bootBannerDecision(feedKind: string): BootBannerDecision {
  switch (feedKind) {
    case "formation-complete":
      return { action: "dismiss" };
    case "formation-partial":
      return { action: "update", text: PARTIAL_BANNER_TEXT };
    default:
      // formation-pending·formation-failed·formation(구팩 호환)·기타 approval kind → 배너 무변경.
      return { action: "ignore" };
  }
}

/** payload.kind 가 편성 상태 신호인지(=승인 요청이 아닌 상태 표면화) 판정 — onDaemonEvent 라우팅용. */
export function isFormationKind(feedKind: string): boolean {
  return feedKind.startsWith("formation");
}
