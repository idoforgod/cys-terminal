// drain_verify 폴백 원인 분류 — 순수 함수(main.ts의 재시작 흐름이 배선만 한다).
//
// [F5] tauri drain_verify 커맨드는 JSON을 못 내면 Err 문자열 접두로 원인을 신호한다:
//   "unsupported:"  = 구버전 cys 바이너리(--verify 미지원, clap unknown-flag)
//   "verify_failed:" = 실행은 됐으나 크래시/하드캡 백스톱으로 결과 미산출
// 두 경우 모두 거동은 plain drain 폴백으로 동일하고, UI 문구만 정직하게 분기한다("무손실" 표현 없음).
// 그 외(알 수 없는 에러)는 null → 호출측이 rethrow(폴백 아님).

export type DrainVerifyFallback = "unsupported" | "verify_failed" | null;

export function classifyDrainVerifyFallback(errMsg: string): DrainVerifyFallback {
  if (errMsg.includes("unsupported")) return "unsupported";
  if (errMsg.includes("verify_failed")) return "verify_failed";
  return null;
}

// 폴백 사유별 사용자 토스트 문구(제목·본문). 거동은 동일하나 원인을 정직하게 알린다.
export function drainVerifyFallbackToast(reason: "unsupported" | "verify_failed"): {
  title: string;
  body: string;
} {
  if (reason === "unsupported") {
    return {
      title: "⚠ 저장 검증 미지원",
      body: "현재 cys 버전은 저장 검증을 지원하지 않습니다 — 기존 방식(best-effort 저장)으로 재시작합니다.",
    };
  }
  return {
    title: "⚠ 저장 검증 실패",
    body: "저장 검증 실행에 실패했습니다(원인 미상) — 기존 방식(best-effort 저장)으로 재시작합니다. 재시작 후 노드 상태를 점검하세요.",
  };
}

// ★U4-B2②(0.14.41): drain --verify 보고서의 도달 불가 데몬(`unreachable`).
//   kind="down"         = 연결 자체 실패(데몬 없음 — 저장할 노드도 없다) → 사유로 나열하지 않는다(정보성).
//   kind="unresponsive" = 연결은 됐는데 응답 없음 — 그 안의 노드는 저장 신호를 못 받았다 → 사유로 나열.
// 구버전 cys(필드 부재)는 빈 목록 — 종전 문구 그대로.
export type DrainUnreachable = {
  dept: string;
  department?: string;
  socket?: string;
  kind: string;
  detail?: string;
};

export function drainVerifyUnresponsiveLines(_unreachable: DrainUnreachable[] | undefined): string[] {
  return []; // RED 골격 — GREEN 커밋에서 구현
}
