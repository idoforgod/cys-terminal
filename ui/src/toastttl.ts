// 토스트 TTL 정책 + 알람 이력 링버퍼 — 순수 로직(main.ts는 DOM 배선만 한다).
//
// ★T-0147-3(GUI 에러 알람 자동 소멸 2026-07-30): 오너 요구 = "모든 에러 알람은 종류 불문
// 일정 시간이 지나면 자연적으로 꺼진다". 구 구현은 sticky 토스트에 타이머가 아예 없어
// boot-warn·safe-mode·perm-*·purge-fail-* 가 화면에 영구 잔존했고(dismiss 호출부 없음),
// #toasts에 높이 상한도 없어 무한 스택이 가능했다.
//
// 정보 소실 방지 대칭 장치: 소멸은 '표시'만 끝내고 내용은 이 파일의 링버퍼(알람 이력)에 남는다.
// main.ts:4878 주석의 실사고(실패를 8초 토스트로 내면 사용자가 실패 자체를 인지 못함)는
// ①이력 탭 조회 ②수동 × 닫기 ③고위험 id의 만료 시 OS 배너 1회 보강으로 해소한다.

/** 일회성(volatile) 토스트 기본 수명 — 구 하드코딩 8초를 그대로 승계(회귀 0). */
export const VOLATILE_TTL_MS = 8_000;

/** 지속형(sticky) 토스트 기본 수명 — 경고·실패류는 60초면 읽힌다(이후 이력 탭에 남음). */
export const STICKY_TTL_MS = 60_000;

/**
 * 진행형(progress) sticky 수명 — 갱신 없이 60초를 넘길 수 있는 장기 작업 전용.
 *
 * 왜 필요한가: 갱신마다 타이머가 리셋되므로 대부분의 진행 토스트는 60초로 안전하지만,
 * `restore`(5204)·`rotate-daemon`(4132)·`transfer`(2289→2331)는 시작 시 1회만 띄우고
 * 완료/실패까지 중간 갱신이 없다. 노드 복원·데몬 교대가 60초를 넘기면 "진행 중인데 토스트가
 * 사라지는" 회귀가 나므로 3분을 준다. 3분 뒤에는 이것도 사라진다(오너 요구 = 종류 불문 소멸).
 */
export const PROGRESS_TTL_MS = 180_000;

/**
 * ★(0.14.41 · U14) 폴더 접근 안내(sticky id 접두 `perm-`) 수명 — 10분.
 *
 * 왜 필요한가: 이 안내는 사람이 시스템 설정의 여러 단계를 따라가는 동안 **보고 있어야 하는** 것이다.
 * 60초면 설정 화면을 여는 사이에 사라졌다(phase1 U14 반박 M4). 여전히 유한하다(오너 요구 =
 * 종류 불문 소멸 — 무한 불변식 테스트가 이 id 도 함께 잰다). 접두는 folderaccess.ts 의
 * PERM_TOAST_PREFIX 와 같다(모듈 결합을 피하려 값만 같게 둔다 — 테스트가 두 값을 함께 잰다).
 *
 * ★(0.14.43 · J2) '업데이트가 설치되지 않았습니다'(sticky id `update-not-installed`)도 이 수명을 받는다 — 본문이 이벤트 뷰어 경로를
 * 따라가며 읽는 안내이고, 앱을 다시 연 직후 1분 안에 못 보면 사용자가 실패 자체를 모르고 지나간다(이 알림은 시도 기록을 지우며 **1회만**
 * 나온다 — 놓치면 다시 오지 않는다). 여전히 유한하다(10분). 이 id 는 updatenotice.ts 의 UPDATE_FAILED_TOAST_ID 와 같다(역시 모듈
 * 결합을 피하려 값만 같게 둔다 — 테스트가 두 값을 함께 잰다).
 */
export const GUIDE_TTL_MS = 600_000;
/** GUIDE_TTL_MS 를 받는 sticky id **접두**(폴더 접근 안내) — 접두가 우연히 겹치는 다른 id 는 연장하지 않는다(정확히 `perm-` 접두만). */
const GUIDE_STICKY_PREFIXES = ["perm-"] as const;
/** GUIDE_TTL_MS 를 받는 sticky id **정확 일치**(1회만 나오는 실패 안내) — 접두 일치가 아니다(`update-not-installed-x` 는 연장하지 않는다). */
const GUIDE_STICKY_IDS = ["update-not-installed"] as const;

/** 알람 이력 링버퍼 보관 건수. */
export const ALARM_HISTORY_CAP = 200;

/** 위 PROGRESS_TTL_MS를 적용받는 sticky id(진행 상태를 스스로 갱신하지 않거나 장기인 것). */
const PROGRESS_STICKY_IDS = [
  "upd-bin", // 다운로드 진행률로 자주 갱신되지만 '세션 정리 중' 단계에서 정지 구간이 있다
  "upd-pack",
  "restore",
  "rotate-daemon",
  "restart-daemon",
  "transfer",
  "daemon-hint", // daemon-ready 수신까지 대기(중간 갱신 없음)
] as const;

/**
 * TTL 만료 시 OS 네이티브 배너로 1회 보강할 sticky id 접두 —
 * D2b(purge-safety) 계열처럼 "놓치면 사용자가 실패 자체를 모르는" 고위험 실패 알람.
 */
const BANNER_ON_EXPIRY_PREFIXES = ["purge-fail-"] as const;
/**
 * TTL 만료 시 OS 네이티브 배너로 1회 보강할 sticky id **정확 일치** — ★(0.14.43 · J2) 업데이트 미설치 알림은 시도 기록을 지우며 1회만
 * 나오므로 10분 안에 못 본 채 만료되면 사용자가 '업데이트가 안 깔렸다'는 사실 자체를 모른다(값은 updatenotice.ts 의 UPDATE_FAILED_TOAST_ID).
 */
const BANNER_ON_EXPIRY_IDS = ["update-not-installed"] as const;

export type ToastKind = "volatile" | "sticky";

/** 종류·id별 수명. id는 sticky에만 의미가 있다(volatile은 익명). */
export function toastTtl(kind: ToastKind, id?: string): { ttlMs: number } {
  if (kind === "volatile") return { ttlMs: VOLATILE_TTL_MS };
  if (id && (PROGRESS_STICKY_IDS as readonly string[]).includes(id)) return { ttlMs: PROGRESS_TTL_MS };
  if (id && (GUIDE_STICKY_IDS as readonly string[]).includes(id)) return { ttlMs: GUIDE_TTL_MS };
  if (id && GUIDE_STICKY_PREFIXES.some((p) => id.startsWith(p))) return { ttlMs: GUIDE_TTL_MS };
  return { ttlMs: STICKY_TTL_MS };
}

/**
 * 타이머 배선 계획 — 같은 id로 다시 호출(갱신)되면 이전 타이머를 걷고 새로 건다(debounce).
 * 진행 중 작업이 갱신을 계속 밀어넣는 동안 소멸하지 않게 하는 규칙의 단일 진실.
 */
export function toastTimerPlan(
  kind: ToastKind,
  id?: string,
  hadExisting = false,
): { ttlMs: number; clearPrevious: boolean } {
  const { ttlMs } = toastTtl(kind, id);
  // volatile은 매 호출이 새 엘리먼트라 걷을 이전 타이머가 없다.
  return { ttlMs, clearPrevious: kind === "sticky" && hadExisting };
}

/** 이 sticky가 조용히 만료될 때 OS 배너로 한 번 더 알려야 하는가(접두 일치 또는 정확 일치). */
export function needsExpiryBanner(id: string): boolean {
  return (BANNER_ON_EXPIRY_IDS as readonly string[]).includes(id) || BANNER_ON_EXPIRY_PREFIXES.some((p) => id.startsWith(p));
}

/** 만료 보강 배너 문구 — 토스트는 사라졌고 내용은 이력 탭에 있음을 명시(정직성). */
export function expiryBannerText(name: string, detail: string): { title: string; body: string } {
  return {
    title: `⚠ ${name}`,
    body: `${detail}\n(알람은 자동으로 닫혔습니다 — Control Center의 알람 탭에서 다시 볼 수 있습니다.)`,
  };
}

export interface AlarmRecord {
  /** epoch ms */
  ts: number;
  category: string;
  name: string;
  detail: string;
  /** sticky 토스트의 id(있으면 같은 id는 최신 1건으로 합쳐진다) */
  id?: string;
}

/**
 * 링버퍼 적재 — 최신순(index 0 = 최신)으로 유지하고 cap을 넘으면 오래된 것부터 버린다.
 *
 * 합침(coalesce) 규칙: `id`가 있는 레코드는 같은 id의 기존 항목을 제거하고 최신 상태만 남긴다.
 * upd-bin 다운로드 진행률처럼 초당 여러 번 갱신되는 sticky가 200칸을 다 먹어치워
 * 정작 봐야 할 실패 알람을 밀어내는 것을 막는다(진행형은 '최종 상태'만 의미가 있다).
 *
 * 순수 함수 — 입력 배열을 변형하지 않고 새 배열을 돌려준다.
 */
export function pushAlarm(
  ring: readonly AlarmRecord[],
  rec: AlarmRecord,
  cap: number = ALARM_HISTORY_CAP,
): AlarmRecord[] {
  const kept = rec.id ? ring.filter((r) => r.id !== rec.id) : ring.slice();
  return [rec, ...kept].slice(0, Math.max(1, cap));
}

/** 이력 목록의 시각 표기(HH:MM:SS) — 렌더 전용 헬퍼(테스트 가능하게 여기 둔다). */
export function formatAlarmTime(ts: number): string {
  const d = new Date(ts);
  const p = (x: number) => String(x).padStart(2, "0");
  return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
}
