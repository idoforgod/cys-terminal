// 0.14.51 TOAST 축 — 사람이 읽는 **자발적 배경 경보**를 끈다 (오너 2026-10-15 「전부 보이지 않게 하라」 · 2026-10-12 「내부 신호는 없애지말라, 내 눈에서 보이지 않게만」).
//
// ★판별자는 **출처**다(부서장 판정 2026-10-15): 사용자가 아무것도 하지 않았는데 **데몬 이벤트가 밀어 올린** 박스·배너·배지·카드·화면 전환만 끈다.
//   사용자 행동의 응답(전송 실패·전출 실패·복사됨)과 업데이트 경로·앱 백엔드 알림은 toast/stickyToast/osBanner 를 **직접** 불러 그대로 남는다.
//   출처는 **호출 경로**로 판정한다 — onDaemonEvent 본문은 daemonToast/daemonStickyToast/daemonOsBanner 래퍼를 부르고, 직접 호출은 approval.stalled 의 셋뿐이다(R5 복원 · 시험이 소스에서 센다),
//   문구의 낱말로 추정하지 않는다.
//
// 이 모듈은 **끄는 자리의 단일 진실**이다. 순수 — DOM·타이머·invoke 를 모른다.
//   · 호출부 200+ 곳과 모듈(starvednotice.ts 등)은 지우지 않는다(고아 8배 금지). 되돌리는 것도 이 파일의 한 줄이다.
//   · 끄는 것은 **보이는 층**뿐이다: 데몬 이벤트·배달 원장·노드 수신·`--json` 필드·exit code 는 이 축이 **0 바이트** 건드린다.
//   · ⛔`onDaemonEvent` 에서 early return 하지 않는다 — 그 함수는 토스트 말고도 refreshSidebarStatus·upsertTaskCell·마우스 소등 정합기를 부른다(AT-10).
//
// 동결 재고(부서장 판정 2026-10-15, 출처 판별자로 개정):
//   끄는 것 = 데몬 이벤트 출처의 모든 사람 싱크 — toast·stickyToast·osBanner(래퍼 경유) · recordAlarm(그 래퍼가 닿지 않아 기록 0) · renderAlerts · 사이드바 ⚠ ·
//             탭 부제 상태 점 · 승인 대기 건수 배지 · Feed 자동 화면 전환 · Control Center 실패 배지  (= UI 12)  +  CLI 경보 산문 호출부 7곳 (= 채널 1) ⇒ 채널 13
//   남기는 것 = 사용자 행동 응답 토스트 · 업데이트 경로 전부 · 앱 백엔드 알림 · 데이터 뷰 · `--json` 필드·exit code · 명령 오류·사용법·진단 stderr
// 런마다 coverageDeclaration() 이 이 재고를 출력하고 시험이 소스의 실제 가드 수와 대조한다(B-216).

/** 가드를 직접 갖는 UI 싱크 — 이름이 곧 main.ts 가드 호출의 인자다(alertsQuiet("<sink>")). */
export const QUIET_SINKS = [
  "toast", // daemonToast() — 우하단 박스(데몬 출처)
  "stickyToast", // daemonStickyToast() — 지속형 박스(데몬 출처)
  "osBanner", // daemonOsBanner() — OS 네이티브 배너(sendNotification, 데몬 출처)
  "renderAlerts", // Control Center 경보 배지(#cc-alertbadge) + 스트립(#cc-alerts)
  "sidebarBadge", // 사이드바 승인 대기 ⚠ 배지(ws-approve-badge)
  "ccFailBadge", // Control Center 부서 데몬 실패 배지(cc-fail-badge)
  "wsDot", // 워크스페이스 탭 부제 상태 점(ws-dot)·사망/CTX/유휴 비트·심각도 색 — 탭에는 pane 수·대표 제목만 남는다
  "pendingBadges", // Control Center 버튼(cc-pending-badge)·승인 Feed 탭(cc-feed-tabbadge) 건수 배지
  "feedSwitch", // 데몬 출처 승인 Feed 화면 자동 전환(feed.item.created 호출부 인라인 가드의 유예 후 전환 예약 — 포커스 강탈 방지. approval.stalled 의 즉시 전환은 R5 로 복원되어 래퍼 daemonOpenFeed 는 제거됨)
  "ccFooter", // Control Center 푸터 「⚠ 데몬 응답 없음」(updateCcStale)
  "ccTaskError", // Tasks 부서 오류 안내 행(renderTasks d.error)
] as const;
export type QuietSink = (typeof QUIET_SINKS)[number];

/** 소스에 있어야 하는 가드 호출 수(시험이 main.ts 를 세어 대조한다). */
export const EXPECTED_GUARDS: Readonly<Record<QuietSink, number>> = {
  toast: 1,
  stickyToast: 1,
  osBanner: 1,
  renderAlerts: 1,
  sidebarBadge: 1,
  ccFailBadge: 1,
  wsDot: 1,
  pendingBadges: 1,
  feedSwitch: 1, // feed.item.created 호출부 인라인 가드 (daemonOpenFeed 래퍼는 R5 복원 뒤 호출자 0 이라 제거)
  ccFooter: 1,
  ccTaskError: 1,
};

/** 가드 호출이 없고 **경로로** 덮이는 채널 — recordAlarm 은 toast/stickyToast 만 부르고, 데몬 출처 호출은 래퍼에서 돌아가므로 닿지 않는다(시험: 알람 이력 0). */
export const ROUTE_COVERED = ["recordAlarm"] as const;

/** CLI 경보 산문 호출부(src/bin/cys.rs) — 프로세스가 달라 이 파일이 끄지는 않지만 재고 숫자는 한 곳에 적는다(Rust 쪽 alert_prose 와 짝, 시험이 대조). */
export const CLI_PROSE_CALLSITES = 7; // stderr 6(큐 durable=false·pause 안내·막힌 좌석 안내·유령 계수 처방 2·구데몬 --queued 경고) + stdout 1(org status 도달불가)

/** 끄는 판정. 유일한 상수 — true 면 그 싱크는 **아무것도 그리지 않고 돌아간다**. */
const ALERTS_QUIET = true;

export function alertsQuiet(_sink: QuietSink): boolean {
  return ALERTS_QUIET;
}

/** 남기는 것(재고) — 선언에 같이 찍는다. */
export const KEPT = [
  "사용자 행동 응답 토스트(전송 실패·전출 실패·복사됨 등)",
  "업데이트 경로 전부",
  "앱 백엔드 알림(안전모드·번들 손상 등 — 데몬 이벤트 출처 아님)",
  "승인 Feed 의 상호작용 면(카드·버튼·묶음 확인) — 대기 중인 승인 요청은 요청이지 경고가 아니다(부서장 판정 2026-10-15 R1 §3). Feed 발 토스트·배지만 끈다",
  "데이터 뷰(render* 중 경보 아닌 것)",
  "--json 필드·exit code",
  "명령 오류·사용법·진단 stderr",
] as const;

/** 채널 수(재고): 가드 보유 싱크 + 경로 덮임 + CLI 군. */
export const CHANNEL_COUNT = QUIET_SINKS.length + ROUTE_COVERED.length + 1;

/** 런마다 출력하는 커버리지 선언 — 세는 모집단이 재고와 일치하는지 기계가 말하게 한다. */
export function coverageDeclaration(guardCounts?: Record<string, number>): string {
  const lines = [
    `TOAST-QUIET coverage: channels=${CHANNEL_COUNT} (guarded_ui=${QUIET_SINKS.length} route_covered=${ROUTE_COVERED.length} cli_prose_group=1[callsites=${CLI_PROSE_CALLSITES}]) quiet=${ALERTS_QUIET} discriminator=origin(daemon event call path)`,
    `  guarded ui sinks: ${QUIET_SINKS.join(", ")}`,
    `  route-covered: ${ROUTE_COVERED.join(", ")}`,
    `  kept: ${KEPT.join(" | ")}`,
  ];
  if (guardCounts) lines.push(`  guards found in source: ${JSON.stringify(guardCounts)}`);
  return lines.join("\n");
}
