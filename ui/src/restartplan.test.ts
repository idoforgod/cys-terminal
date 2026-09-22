// GUI 재기동 주입의 초안 보호·기계 잔여 정리·배선 계약 (bun test — DOM/Tauri 불요).
import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { planRestartInject, restartInvokeFailureReason } from "./restartplan";

const cmd = "cys launch-agent --role worker --agent claude";
const plain = { mode: "plain", data: cmd + "\n", clearFirst: false };
const clearFirst = { mode: "clear_first", data: cmd, clearFirst: true };

describe("재기동 주입 — 계수에 따라 제출 방식을 고르고 사람 초안을 보호한다", () => {
  test("전체·사람 계수가 0이면 종전처럼 개행을 붙여 보낸다", () => {
    expect(planRestartInject(cmd, {
      pending_input_bytes: 0, pending_input_human_bytes: 0,
    })).toEqual(plain);
  });

  test("구 데몬이 두 키를 보고하지 않으면 종전 방식으로 보낸다", () => {
    expect(planRestartInject(cmd, {})).toEqual(plain);
  });

  test("전체 계수가 null이거나 부재하고 사람 계수가 0이면 종전 방식으로 보낸다", () => {
    for (const pending of [null, undefined]) {
      expect(planRestartInject(cmd, {
        pending_input_bytes: pending, pending_input_human_bytes: 0,
      })).toEqual(plain);
      expect(planRestartInject(cmd, {
        pending_input_bytes: pending, pending_input_human_bytes: null,
      })).toEqual(plain);
    }
  });

  test("기계 잔여만 있으면 먼저 정리하고 본문에 개행을 붙이지 않는다", () => {
    expect(planRestartInject(cmd, {
      pending_input_bytes: 24, pending_input_human_bytes: 0,
    })).toEqual(clearFirst);
  });

  for (const human of [null, undefined]) {
    test(`전체 잔여가 있고 사람 계수가 ${String(human)}이면 정리 후 데몬의 사람 축 재검사를 받는다`, () => {
      expect(planRestartInject(cmd, {
        pending_input_bytes: 24, pending_input_human_bytes: human,
      })).toEqual(clearFirst);
    });
  }

  for (const pending of [24, 0, null, undefined]) {
    test(`사람 초안이 있으면 전체 계수 ${String(pending)}보다 우선해 보류하고 재시도를 안내한다`, () => {
      const plan = planRestartInject(cmd, {
        pending_input_bytes: pending, pending_input_human_bytes: 3,
      });
      expect(plan.mode).toBe("refuse");
      if (plan.mode !== "refuse") throw new Error("사람 초안이 있으면 재기동을 보류해야 한다");
      for (const word of ["초안", "제출", "삭제", "재시도"]) {
        expect(plan.reason).toContain(word);
      }
    });
  }

  const invalidCounts = [
    { label: "음수", value: -1 },
    { label: "숫자가 아닌 값", value: NaN },
    { label: "양의 무한대", value: Infinity },
    { label: "음의 무한대", value: -Infinity },
    { label: "양수 문자열", value: "24" },
    { label: "영 문자열", value: "0" },
    { label: "빈 문자열", value: "" },
    { label: "불리언", value: true },
    { label: "객체", value: {} },
    { label: "배열", value: [24] },
  ];
  for (const { label, value } of invalidCounts) {
    test(`${label} 계수는 숫자로 강제 변환하지 않고 0으로 접는다`, () => {
      // org.status 런타임 응답이 타입 계약을 어긴 경우도 순수 함수에서 방어한다.
      for (const obs of [
        { pending_input_bytes: value, pending_input_human_bytes: 0 },
        { pending_input_bytes: 0, pending_input_human_bytes: value },
        { pending_input_bytes: value, pending_input_human_bytes: value },
      ]) {
        expect(planRestartInject(cmd, obs as unknown as Parameters<typeof planRestartInject>[1])).toEqual(plain);
      }
    });
  }
});

const src = readFileSync(new URL("./main.ts", import.meta.url), "utf-8");
// droppoint.test.ts 관례: 주석에만 적힌 함수명으로 배선 검체가 통과하지 않게 한다.
const code = src
  .replace(/\/\*[\s\S]*?\*\//g, "")
  .split("\n")
  .map((line) => line.trimStart().startsWith("//") ? "" : line.replace(/\s\/\/.*$/, ""))
  .join("\n");

function functionSlice(name: string): string {
  const start = code.indexOf(`function ${name}(`);
  expect(start).toBeGreaterThanOrEqual(0);
  const end = code.indexOf("\n}\n", start);
  expect(end).toBeGreaterThan(start);
  return code.slice(start, end + 3);
}

describe("재기동 배선 — 순수 주입 계획이 실제 UI 전송을 결정한다", () => {
  test("재기동은 순수 모듈의 계획을 쓰고 명령에 개행을 직접 붙여 넘기지 않는다", () => {
    const body = functionSlice("restartNode");
    expect(/import\s*\{[^}]*\bplanRestartInject\b[^}]*\}\s*from\s*["']\.\/restartplan["']/.test(code)).toBe(true);
    expect(/planRestartInject\(\s*cmd\s*,\s*target\s*\)/.test(body)).toBe(true);
    expect(/data\s*:\s*cmd\s*\+\s*["']\\n["']/.test(body)).toBe(false);
  });

  test("보류 계획은 watchdog 토스트를 표시하고 전송 전에 반환한다", () => {
    const body = functionSlice("restartNode");
    const refusal = body.match(/if\s*\(plan\.mode\s*===\s*"refuse"\)\s*\{([\s\S]*?)\}/);
    expect(refusal !== null).toBe(true);
    if (!refusal) throw new Error("보류 분기가 있어야 한다");
    expect(refusal[1]).toContain('toast("watchdog", "재기동 보류", plan.reason)');
    expect(refusal[1]).toContain("return;");
    expect(body.indexOf(refusal[0])).toBeLessThan(body.indexOf('invoke("send_input"'));
  });

  test("계획의 본문과 정리 여부를 같은 대상에 기계 문안으로 보낸다", () => {
    const body = functionSlice("restartNode");
    const send = body.slice(body.indexOf('invoke("send_input"'));
    for (const field of [
      "socket: socket ?? null", "surfaceId: target.surface_id",
      "data: plan.data", "clearFirst: plan.clearFirst", "machineOrigin: true",
    ]) {
      expect(send).toContain(field);
    }
  });

  test("대상 좌석 점프는 기존처럼 전송을 준비하기 전에 수행한다", () => {
    const body = functionSlice("restartNode");
    const jump = body.indexOf("jumpToSurface(target.surface_id, socket)");
    expect(jump).toBeGreaterThanOrEqual(0);
    expect(jump).toBeLessThan(body.indexOf("planRestartInject("));
  });
});

// ★(0.14.39 · 적대 major ②ⓑ) 데몬 거부의 무음 실패 금지 — 사유 번역과 배선.
// 프로덕션 문면(정의처): src/bin/cysd/handlers.rs:4303-4308 (code=clear_first_unsupported)
const PROD_CLEAR_FIRST_UNSUPPORTED =
  "clear_first_unsupported: clear_first requires a launch-agent-registered pane (Ctrl-U semantics vary by TUI)";
// 정의처: src/lib.rs:633-634 — ERR_TYPING_GUARD + MSG_TYPING_GUARD
const PROD_TYPING_GUARD = "typing_guard: human is typing in this pane; retry later or use --queued";
// 정의처: src/bin/cysd/handlers.rs:2241-2246 draft_gate_denied_response — code 는 typing_guard, message 에 [draft_gate:…] 태그
const PROD_DRAFT_GATE =
  "typing_guard: human is typing in this pane; retry later or use --queued [draft_gate:pending_input]";

describe("재기동 실패 — 데몬 거부를 한국어 처방으로 낸다", () => {
  for (const { label, error, reason } of [
    {
      label: "clear_first 미지원 좌석은 Ctrl-U 후 재시도를 안내한다",
      error: PROD_CLEAR_FIRST_UNSUPPORTED,
      reason: "이 좌석은 launch-agent 등록이 없어 자동 정리를 못 합니다 — 해당 pane 에서 Ctrl-U 후 재시도",
    },
    {
      label: "초안 게이트 거부는 typing_guard보다 우선해 초안 제출·삭제를 안내한다",
      error: PROD_DRAFT_GATE,
      reason: "대상 입력줄에 미제출 입력이 있어 보류했습니다 — 해당 pane 에서 초안을 제출·삭제한 뒤 재시도",
    },
    {
      label: "타이핑 가드 거부는 잠시 뒤 재시도를 안내한다",
      error: PROD_TYPING_GUARD,
      reason: "대상 pane 에 사람 입력이 감지돼 보류했습니다 — 잠시 뒤 재시도",
    },
  ]) {
    for (const withCode of [true, false]) {
      test(`${label} — ${withCode ? "코드 포함" : "코드가 유실된 message만"}`, () => {
        const text = withCode ? error : error.slice(error.indexOf(": ") + 2);
        for (const err of [text, new Error(text)]) {
          expect(restartInvokeFailureReason(err)).toBe(reason);
        }
      });
    }
  }

  test("send_input은 rpc_full로 데몬 오류 코드를 UI까지 보존한다", () => {
    const rust = readFileSync(new URL("../../src-tauri/src/main.rs", import.meta.url), "utf-8");
    const start = rust.indexOf("async fn send_input(");
    expect(start).toBeGreaterThanOrEqual(0);
    const end = rust.indexOf("\n}", start);
    expect(end).toBeGreaterThan(start);
    const body = rust.slice(start, end + 2)
      .replace(/\/\*[\s\S]*?\*\//g, "")
      .split("\n")
      .map((line) => line.trimStart().startsWith("//") ? "" : line.replace(/\s\/\/.*$/, ""))
      .join("\n");
    if (!body.includes("rpc_full(") || body.includes("rpc_on(")) {
      throw new Error("send_input 이 rpc_on 으로 되돌아갔다 — error.code 가 유실돼 번역이 다시 vacuous 가 된다");
    }
  });

  test("번역표에 없는 오류는 삼키지 않고 원문을 싣는다", () => {
    expect(restartInvokeFailureReason(new Error("daemon unreachable"))).toBe("daemon unreachable");
    expect(restartInvokeFailureReason("  socket closed  ")).toBe("socket closed");
    for (const empty of ["", "   ", null, undefined]) {
      expect(restartInvokeFailureReason(empty)).toBe("알 수 없는 오류");
    }
  });

  test("재기동 전송은 try/catch 안에 있고 실패를 토스트로 낸다", () => {
    const body = functionSlice("restartNode");
    const tryAt = body.indexOf("try {");
    const sendAt = body.indexOf('invoke("send_input"');
    const catchAt = body.indexOf("} catch (");
    expect(tryAt).toBeGreaterThanOrEqual(0);
    expect(tryAt).toBeLessThan(sendAt);
    expect(catchAt).toBeGreaterThan(sendAt);
    const handler = body.slice(catchAt);
    expect(handler).toContain("restartInvokeFailureReason(");
    expect(handler).toContain('toast("watchdog", "재기동 실패"');
  });

  test("팔레트 액션 실행도 감싸여 무음으로 죽지 않는다", () => {
    const runAt = code.indexOf("const run = async (it: PaletteItem)");
    expect(runAt).toBeGreaterThanOrEqual(0);
    const body = code.slice(runAt, code.indexOf("\n  };\n", runAt) + 5);
    expect(body).toContain("try {");
    expect(body).toContain("await it.action();");
    expect(body).toContain("catch");
    expect(body).toContain("restartInvokeFailureReason(");
  });
});
