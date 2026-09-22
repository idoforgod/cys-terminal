// GUI 재기동 주입의 순수 판단 로직 (DOM 무접촉 — main.ts가 전송·토스트에 배선한다).
// D-12 Text 게이트는 전체 pending을 보므로, 에이전트가 죽은 셸에 기계 잔여만 있어도
// 재기동을 막는다. 마커가 없는 셸은 stale 계수 리셋도 못 하므로 ClearFirst로 선정리한다.
// ClearFirst 팔은 사람 계수를 다시 검사하므로 사람 초안 보호는 유지된다.
// 데몬 Inject가 Ctrl-U → 본문 → CR을 원자로 보내므로 clear_first 본문에는 개행을
// 붙이지 않는다 — 개행까지 보내면 빈 Enter가 한 번 더 제출된다.
// 구 데몬의 계수 부재는 '모름'이다. 전체 잔여가 확인되지 않으면 plain을 유지해
// launch-agent 미등록 pane도 재기동할 수 있게 한다. 전체 잔여가 확인되고 사람 축만
// 모르면 clear_first로 보내되 데몬의 사람 축 재검사에 맡긴다. 확인된 사람 초안은 우선 보류한다.

export type RestartInjectPlan =
  | { mode: "plain" | "clear_first"; data: string; clearFirst: boolean }
  | { mode: "refuse"; reason: string };

function pendingCount(value: unknown): number {
  return typeof value === "number" && Number.isFinite(value) && value > 0 ? value : 0;
}

export function planRestartInject(cmd: string, obs: {
  pending_input_bytes?: number | null;
  pending_input_human_bytes?: number | null;
}): RestartInjectPlan {
  if (pendingCount(obs.pending_input_human_bytes) > 0) {
    return {
      mode: "refuse",
      reason: "사람이 작성 중인 초안이 있습니다. 초안을 제출하거나 삭제한 뒤 재시도하세요.",
    };
  }
  if (pendingCount(obs.pending_input_bytes) > 0) {
    return { mode: "clear_first", data: cmd, clearFirst: true };
  }
  return { mode: "plain", data: cmd + "\n", clearFirst: false };
}
