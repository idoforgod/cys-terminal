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
    // ★(0.14.43 · C5) 계수가 남았는데 입력줄이 비어 보이면 유령 계수다(단독 Esc·Backspace·글자를 치고 전부 지운 줄 등) — 사람이 그 창에서
    //   Ctrl-U 한 번이면 풀린다. 기계가 키를 보내는 길은 없다(사람이 누른다).
    return {
      mode: "refuse",
      reason:
        "사람이 작성 중인 초안이 있습니다. 초안을 제출하거나 삭제한 뒤 재시도하세요. " +
        "입력줄이 비어 보이면 그 창을 클릭하고 Ctrl-U 를 한 번 누르세요.",
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
// 【장치】 데몬이 내는 **오류 코드와 문면**을 그 좌석에서 사람이 할 수 있는 다음 행동으로 번역한다.
//   근거: `src/bin/cysd/handlers.rs` 의 `clear_first_unsupported`(launch-agent 등록 pane 한정 ·
//   Ctrl-U 의미가 TUI 마다 달라서 둔 정당한 제한) · 초안 게이트 `draft_gate`/`pending_input` ·
//   타이핑 가드 `typing_guard`. 코드가 문자열화되어 `"{code}: {message}"` 로 올라온다.
//   모르는 오류는 **삼키지 않고 원문을 그대로 싣는다** — 번역표에 없다는 이유로 조용해지면
//   이 함수가 고치려던 결함이 그대로 재발한다.
export function restartInvokeFailureReason(err: unknown): string {
  const raw =
    err instanceof Error ? err.message : typeof err === "string" ? err : String(err ?? "");
  const text = raw.trim();
  // ★(0.14.39 · 성찰2 major) 코드 축 + **문면 축**. send_input 이 `"{code}: {message}"` 를 올리도록
  //   고쳤지만(src-tauri/src/main.rs send_input), 코드가 유실되는 경로에서도 발화해야 한다.
  //   문면 상수의 정의처는 `src/lib.rs` 의 MSG_TYPING_GUARD 와 handlers.rs 의 clear_first_unsupported 문면이다.
  //   draft_gate 거부도 code 는 typing_guard 이고 message 에 [draft_gate:…] 가 붙는다 — 초안 게이트를
  //   먼저 보아야 더 정확한 제출·삭제 처방이 일반 타이핑 가드의 잠시 뒤 재시도로 덮이지 않는다.
  // ★(0.14.43 · RQFIX F12 · I6 감사 N1) 초안 게이트 분기 **앞**에 세 갈래 — 이 셋도 문면에 `draft_gate` 가 붙어 있어 아래 일반 분기가 먼저 잡으면 더 정확한 처방이
  //   '초안을 제출·삭제' 로 덮인다. 번역 조건 구절은 데몬 소스의 상수·리터럴에서 왔다(검체가 소스를 읽어 핀한다 — 상수가 바뀌면 검체가 적색):
  //   · 모달 태그 `[draft_gate:modal]` — governance.rs `DraftGateDenied::Modal` 의 `as_str()` 와 lib.rs `DRAFT_GATE_TAG`
  //   · 정착 창 태그 ` [settle:<ms>]` — lib.rs `SEND_SETTLE_TAG`(기계 제출이 진행 중이라 줄이 곧 빈다) / 사유 `submit_settling`
  //   · 유령 계수 처방 접미 — lib.rs `GHOST_CTRL_U_SUFFIX`(입력줄이 비어 보이는데 미제출 계수가 남았다)
  if (text.includes("draft_gate:modal")) {
    return "질문·선택 창이 떠 있어 보류했습니다 — 그 창을 먼저 처리한 뒤 재시도";
  }
  if (text.includes("[settle:") || text.includes("draft_gate:submit_settling")) {
    return "직전 제출이 처리되는 중이라 보류했습니다 — 잠시 뒤 다시 시도";
  }
  if (text.includes("입력줄이 비어 보이면 유령 계수다")) {
    return (
      "입력줄에 미제출 입력이 있다고 계수돼 보류했습니다 — 입력줄이 비어 보이면 그 창을 클릭하고 Ctrl-U 를 한 번 누른 뒤 재시도" +
      "(글이 있으면 제출하거나 지운다)"
    );
  }
  if (text.includes("draft_gate") || text.includes("pending_input")) {
    return "대상 입력줄에 미제출 입력이 있어 보류했습니다 — 해당 pane 에서 초안을 제출·삭제한 뒤 재시도";
  }
  if (text.includes("clear_first_unsupported") || text.includes("launch-agent-registered pane")) {
    return "이 좌석은 launch-agent 등록이 없어 자동 정리를 못 합니다 — 해당 pane 에서 Ctrl-U 후 재시도";
  }
  if (text.includes("typing_guard") || text.includes("human is typing")) {
    return "대상 pane 에 사람 입력이 감지돼 보류했습니다 — 잠시 뒤 재시도";
  }
  return text === "" ? "알 수 없는 오류" : text;
}
