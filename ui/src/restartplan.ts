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

// ★(0.14.39 · 적대 major ②ⓑ) 데몬 거부를 사람이 읽을 처방으로 옮긴다.
//
// 【무엇이 틀렸었나】 `restartNode` 는 `await invoke("send_input", …)` 를 try/catch 없이 불렀고,
//   호출자(팔레트 `run`)도 감싸지 않았으며 반환 프로미스를 아무도 받지 않았다. 그래서 데몬 거부가
//   **토스트 한 줄 없이** 사라졌다(unhandled rejection). 보류(`plan.mode === "refuse"`) 팔에만
//   토스트가 있어, 정작 기계가 막힌 경우에만 화면이 조용한 비대칭이었다 — 같은 파일
//   `injectRawToPane` 이 지키는 '무음 실패 금지' 관례와 어긋난다.
//
// 【장치】 데몬이 내는 **오류 코드**를 그 좌석에서 사람이 할 수 있는 다음 행동으로 번역한다.
//   근거: `src/bin/cysd/handlers.rs` 의 `clear_first_unsupported`(launch-agent 등록 pane 한정 ·
//   Ctrl-U 의미가 TUI 마다 달라서 둔 정당한 제한) · 초안 게이트 `draft_gate`/`pending_input` ·
//   타이핑 가드 `typing_guard`. 코드가 문자열화되어 `"{code}: {message}"` 로 올라온다.
//   모르는 오류는 **삼키지 않고 원문을 그대로 싣는다** — 번역표에 없다는 이유로 조용해지면
//   이 함수가 고치려던 결함이 그대로 재발한다.
export function restartInvokeFailureReason(err: unknown): string {
  const raw =
    err instanceof Error ? err.message : typeof err === "string" ? err : String(err ?? "");
  const text = raw.trim();
  if (text.includes("clear_first_unsupported")) {
    return "이 좌석은 launch-agent 등록이 없어 자동 정리를 못 합니다 — 해당 pane 에서 Ctrl-U 후 재시도";
  }
  if (text.includes("draft_gate") || text.includes("pending_input")) {
    return "대상 입력줄에 미제출 입력이 있어 보류했습니다 — 해당 pane 에서 초안을 제출·삭제한 뒤 재시도";
  }
  if (text.includes("typing_guard")) {
    return "대상 pane 에 사람 입력이 감지돼 보류했습니다 — 잠시 뒤 재시도";
  }
  return text === "" ? "알 수 없는 오류" : text;
}
