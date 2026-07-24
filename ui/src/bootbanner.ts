// 팀 기동 경고 배너(boot-warn) 수명 판정 — 순수 함수 (능력파리티 W3/T3 · 2026-07-24).
//
// 배경: "팀 기동 경고" 배너는 Tauri app.emit("boot-warning")(src-tauri spawn_orchestra_boot)로
// 뜨지만 자동 소멸이 없었다(불멸 배너). 편성(javis_formation.py)이 설치 CLI 기준으로 팀을 완결하면
// 데몬 feed 버스로 formation-* 상태가 흘러온다(feed.item.created · payload.kind). 그 신호로 배너의
// 수명을 결정한다 — complete=소멸, partial=문구 갱신(아직 미완이므로 제거 아님), 그 외=무시.
//
// ★소켓 스코프(FIX-1): 배너는 부서(소켓)별로 독립이어야 한다 — 다른 부서 데몬의 formation-complete
// 가 base 의 유효한 경고를 오소멸하면 C6(침묵 금지) 위반이다. boot-warning payload 에 소켓 slug 를
// 동봉(Rust)하고, UI 는 `boot-warn:<slug>` 스코프 id 로 배너를 만든다. formation feed 이벤트는
// event.socket_slug 로 자기 스코프 배너만 건드린다(각 소켓 = 독립 배너 id → 교차 오소멸 구조적 차단).
//
// ★엣지 경합 백스톱(FIX-2): complete 가 배너 생성보다 먼저 오면 dismiss 가 no-op → 직후 배너 생성 →
// 영구 잔존(재방출 없음). UI 가 소켓별 최신 formation 상태(lastFormationKind)를 들고, 배너 생성 시
// 그 소켓이 이미 complete 면 만들지 않는다(순서 무관 수렴 = 레벨 기반 재조정 · 신규 IPC 0).
//
// 전달 경로(실측): 배너 트리거=Tauri app.emit("boot-warning")(cysd 버스 아님). 편성 소멸 신호는
// javis_formation._surface → `cys feed push --kind formation-*` → 데몬 EventBus.publish(
// "feed.item.created") → spawn_event_forwarder(socket_slug 주입) → app.emit("daemon-event") →
// onDaemonEvent. 수신 코드가 앱 빌드에 포함되므로 버전 스큐는 "신팩+구앱"만 문제(→ min_binary_version
// 게이트, T6). 계약이 산문 아닌 kind/slug 문자열이라 회귀 0 을 테스트로 못박는다.

export type BootBannerDecision =
  | { action: "dismiss" } // 편성 완결 → 배너 제거
  | { action: "update"; text: string } // 부분 편성 → 문구 갱신(배너 유지)
  | { action: "ignore" }; // 관련 없음/미완/실패 → 배너 무변경

// 부분 편성 시 배너에 노출할 문구(제거하지 않고 갱신 — 아직 편성 미완).
export const PARTIAL_BANNER_TEXT =
  "일부 노드만 편성됐습니다 — 나머지 CLI 설치 시 자동으로 편성이 완결됩니다.";

// 구 payload(식별 부재) 하위호환용 전역 배너 id.
export const GLOBAL_BOOT_WARN_ID = "boot-warn";

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

/** 소켓 스코프 배너 id — slug 있으면 `boot-warn:<slug>`, 없으면(구 payload) 전역 `boot-warn`. */
export function bannerScopeId(slug: string | null | undefined): string {
  return slug ? `${GLOBAL_BOOT_WARN_ID}:${slug}` : GLOBAL_BOOT_WARN_ID;
}

export type BootWarning = { slug: string | null; message: string };

/** boot-warning payload 정규화 — 신형 {slug,message} · 구형 string 하위호환. */
export function normalizeBootWarning(payload: unknown): BootWarning {
  const fallback = "팀 기동에 실패했습니다.";
  if (typeof payload === "string") return { slug: null, message: payload || fallback };
  if (payload && typeof payload === "object") {
    const o = payload as Record<string, unknown>;
    const slug = typeof o.slug === "string" && o.slug ? o.slug : null;
    const message = typeof o.message === "string" && o.message ? o.message : fallback;
    return { slug, message };
  }
  return { slug: null, message: fallback };
}

export type FormationBannerAction =
  | { op: "dismiss"; bannerId: string }
  | { op: "update"; bannerId: string; text: string }
  | { op: "none" };

/**
 * formation feed 이벤트 → 소켓 스코프 배너 조치. slug 없으면 op:none(스코프 불명 → 오소멸 방지).
 * 각 소켓이 자기 배너 id 만 대상으로 하므로 교차소켓 오소멸이 구조적으로 차단된다(FIX-1).
 */
export function formationFeedAction(kind: string, slug: string | null): FormationBannerAction {
  if (!slug) return { op: "none" }; // 스코프 불명(socket_slug 부재) → 보수적 무조치(오소멸 방지)
  const d = bootBannerDecision(kind);
  const bannerId = bannerScopeId(slug);
  if (d.action === "dismiss") return { op: "dismiss", bannerId };
  if (d.action === "update") return { op: "update", bannerId, text: d.text };
  return { op: "none" };
}

/**
 * boot-warning 수신 시 배너 생성 계획(백스톱·FIX-2). 해당 소켓의 최신 상태가 complete 면 생성하지
 * 않는다(complete 선도착 경합에서 순서 무관 수렴). lastKind = lastFormationKind.get(slug).
 */
export function bootWarningPlan(
  slug: string | null,
  lastKind: string | undefined,
): { create: boolean; bannerId: string } {
  const bannerId = bannerScopeId(slug);
  if (slug && lastKind === "formation-complete") return { create: false, bannerId };
  return { create: true, bannerId };
}
