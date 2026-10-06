// 0.14.44 B5·B6 — 오피스 탭 안내의 순수 판정 모듈(DOM·Tauri·저장소·타이머 무관).
//
// 앱 백엔드 명령 `office_health`(브리지의 실제 응답 확인 — 설계 B3 의 탐침과 같은 꼴)의 답을 받아 "지금 탭에 무엇을 보일까 · 화면을 실을까 ·
// 자산 복구를 시작할까 · 다음 확인은 언제"를 정한다. main.ts 는 조회·DOM 배선만 한다(officetab.test.ts 가 판정을 표로, usagewiring 류 핀이 배선을 못박는다).
//
// ★초보 기준(설계 §0-2): 화면 문구에는 터미널 명령을 쓰지 않는다 — 명령 문자열 0 은 시험이 기계로 센다.
// ★이 모듈의 불변식(main.js 는 번들 하나라 여기서 평가 중 예외가 나면 앱 전체가 백지가 된다):
//   · 최상위 부수효과 0 — 선언(export/const/function/interface/type)만. 브라우저 저장소·document·window·타이머 접근 0.
//   · 구형 WKWebView 가 파싱하지 못하는 문법 0 — 정규식 lookbehind, `.at(`, findLast, structuredClone, Object.hasOwn, replaceAll.

/** `office_health` 의 답(src-tauri `office_health_value` + `repair`). IPC 데이터라 전부 의심한다. */
export interface OfficeHealth {
  ok?: boolean;
  reachable?: boolean;
  reason?: string;
  legacy?: boolean;
  boot_id?: string | null;
  pack_version?: string | null;
  /** 없는 화면 자산 — 팩 상대경로 목록(`/health` 의 `assets` 가 false 인 것). 모르면 빈 배열. */
  assets_missing?: string[];
  /** 자산 복구 실행 방식 — auto(자동 + 단추) · button(단추만) · none(안내 문구만). */
  repair?: string;
}

export type OfficeStage =
  | "loaded" // 화면을 싣는다 — 탭 안내는 숨기고 확인을 멈춘다(그 뒤의 문제는 화면 안 배너가 맡는다)
  | "preparing" // 3분 안 — 준비 중
  | "long" // 3분이 지나도 안 열림
  | "repairing" // 자산 복구 중
  | "repair_failed" // 복구 실패 — 다시 시도 단추
  | "assets_button" // 자산이 없고 단추로만 복구(맥에서 정책이 자동을 끔 · 윈도우에서 단추가 켜진 경우)
  | "assets_none"; // 자산이 없고 앱이 할 수 있는 것이 없음 — 피드백 안내만

export interface OfficePlan {
  stage: OfficeStage;
  /** 탭 안내 문구(빈 문자열이면 안내를 숨긴다). */
  text: string;
  /** 단추 글자(빈 문자열이면 단추 없음). 누르면 사람이 직접 복구를 시작한다(manual). */
  buttonLabel: string;
  /** 지금 자동 복구를 시작해야 하는가(앱 세션당 1회 상한은 백엔드가 지킨다). */
  autoRepair: boolean;
  /** 화면(iframe)을 싣는가. */
  loadFrame: boolean;
  /** 다음 확인까지의 ms — 0 이면 더 확인하지 않는다. */
  nextPollMs: number;
}

export interface OfficeCtx {
  /** 이 탭 확인을 시작한 뒤 지난 ms. */
  elapsedMs: number;
  /** 자산 복구 명령이 지금 돌고 있는가. */
  repairing: boolean;
  /** 마지막 복구 시도의 결과 — none(시도 전) · failed(실패·상한) · blocked(원장 항목·팩 버전 불일치 — 앱이 손대지 않는 상태). */
  repairOutcome: "none" | "failed" | "blocked";
}

/** 화면이 아직 실리지 않았을 때의 확인 간격 — 3분 동안 3초, 그 뒤 15초(설계 B5). */
export const OFFICE_POLL_FAST_MS = 3000;
export const OFFICE_POLL_SLOW_MS = 15000;
export const OFFICE_POLL_FAST_WINDOW_MS = 180000;

export const OFFICE_TEXT_PREPARING = "오피스를 준비하고 있습니다… 자동으로 다시 확인합니다.";
export const OFFICE_TEXT_LONG =
  "오피스가 아직 열리지 않습니다. 자동으로 계속 다시 시도합니다. 오래 열리지 않으면 컴퓨터를 다시 시작해 보시고, 그래도 안 되면 왼쪽 아래 '피드백'으로 알려 주세요.";
export const OFFICE_TEXT_REPAIRING = "오피스 화면 파일을 복구하고 있습니다…";
export const OFFICE_TEXT_REPAIR_FAILED = "복구하지 못했습니다.";
export const OFFICE_TEXT_ASSETS_BUTTON = "오피스 화면 파일이 없습니다.";
export const OFFICE_TEXT_ASSETS_NONE = "오피스 화면 파일을 찾지 못했습니다. 왼쪽 아래 '피드백'으로 알려 주세요.";
export const OFFICE_BUTTON_REPAIR = "복구";
export const OFFICE_BUTTON_RETRY = "다시 시도";

/** 다음 확인까지의 ms(3분 동안 3초 · 그 뒤 15초). 이상한 경과 값은 빠른 간격으로. */
export function officePollDelay(elapsedMs: number): number {
  return Number.isFinite(elapsedMs) && elapsedMs >= OFFICE_POLL_FAST_WINDOW_MS ? OFFICE_POLL_SLOW_MS : OFFICE_POLL_FAST_MS;
}

function asHealth(h: unknown): OfficeHealth {
  return typeof h === "object" && h !== null ? (h as OfficeHealth) : {};
}
function missingList(h: OfficeHealth): string[] {
  const m = h.assets_missing;
  if (!Array.isArray(m)) return [];
  return m.filter((x) => typeof x === "string" && x !== "");
}

/** 탭에 무엇을 보일까. h 는 `office_health` 의 답(호출이 실패했으면 null — 준비 중으로 본다). */
export function planOfficeTab(h: unknown, ctx: OfficeCtx): OfficePlan {
  const hh = asHealth(h);
  const poll = officePollDelay(ctx.elapsedMs);
  const ok = hh.ok === true;
  const missing = ok ? missingList(hh) : [];
  if (ok && missing.length === 0) {
    return { stage: "loaded", text: "", buttonLabel: "", autoRepair: false, loadFrame: true, nextPollMs: 0 };
  }
  if (ok) {
    // 브리지는 떠 있는데 화면 자산이 없다 — B6. 앱이 할 수 있는 일은 실행 방식(repair)이 정한다.
    const mode = hh.repair === "auto" || hh.repair === "button" ? hh.repair : "none";
    if (ctx.repairing) return { stage: "repairing", text: OFFICE_TEXT_REPAIRING, buttonLabel: "", autoRepair: false, loadFrame: false, nextPollMs: OFFICE_POLL_FAST_MS };
    if (ctx.repairOutcome === "blocked" || mode === "none")
      return { stage: "assets_none", text: OFFICE_TEXT_ASSETS_NONE, buttonLabel: "", autoRepair: false, loadFrame: false, nextPollMs: poll };
    if (ctx.repairOutcome === "failed")
      return { stage: "repair_failed", text: OFFICE_TEXT_REPAIR_FAILED, buttonLabel: OFFICE_BUTTON_RETRY, autoRepair: false, loadFrame: false, nextPollMs: poll };
    if (mode === "auto")
      return { stage: "repairing", text: OFFICE_TEXT_REPAIRING, buttonLabel: "", autoRepair: true, loadFrame: false, nextPollMs: OFFICE_POLL_FAST_MS };
    return { stage: "assets_button", text: OFFICE_TEXT_ASSETS_BUTTON, buttonLabel: OFFICE_BUTTON_REPAIR, autoRepair: false, loadFrame: false, nextPollMs: poll };
  }
  // 브리지가 아직 응답하지 않는다(없음 · 느림 · 표식 상태) — 3분 동안은 준비 중, 그 뒤는 긴 안내. 터미널 명령은 어디에도 없다.
  const long = Number.isFinite(ctx.elapsedMs) && ctx.elapsedMs >= OFFICE_POLL_FAST_WINDOW_MS;
  return {
    stage: long ? "long" : "preparing",
    text: long ? OFFICE_TEXT_LONG : OFFICE_TEXT_PREPARING,
    buttonLabel: "",
    autoRepair: false,
    loadFrame: false,
    nextPollMs: poll,
  };
}

/** 복구 명령의 답(status) → 다음 상태. repaired·partial 은 바로 다시 확인(성공 여부는 건강 확인이 판정한다) · 앱이 손대지 않는 상태는 blocked · 나머지는 failed. */
export function repairOutcomeOf(status: unknown): "retry" | "blocked" | "failed" {
  if (status === "repaired" || status === "partial") return "retry";
  if (status === "ledger_entry" || status === "version_mismatch" || status === "unavailable" || status === "nothing_missing") return "blocked";
  return "failed";
}
