// 0.14.43 J2(추가 과제) — 송신 실패를 사용자에게 알리는 장치의 소스 핀(I6 감사 N7) + 전출 실패 토스트의 번역기 경유 핀(I6 감사 N2).
//
// 배경: GUI 가 데몬에 직접 보내는 6곳은 거부를 전부 토스트로 알린다(무음 삼킴·무한 재시도 0건 — 결함 아님). 그런데 그 장치를 지키는 소스
//   핀은 재기동 경로(restartplan.test.ts)에만 있었다 — 누가 `.catch(() => {})` 로 되돌려도 스위트가 초록이었다. 이 파일이 나머지 셋을 고정한다:
//   · sendRaw(사람 실키) 의 `.catch(noteSendFail)` → flushSendFail 의 '입력 전송 실패' 토스트
//   · injectRawToPane(경로 삽입) 의 '삽입 실패' 토스트
//   · transferCrossDept(부서 전출) 바깥 catch 의 '전출 실패' 토스트
//   그리고 N2: 전출 실패 토스트의 사유는 데몬 원문('retry later or use --queued' 같은 GUI 에서 실행할 수 없는 처방)이 아니라 재기동 경로와 같은
//   번역기 restartInvokeFailureReason 을 거친다(모르는 오류는 원문 그대로 — 정보 손실 없음).
// 이 파일은 런타임 코드 0줄이다 — main.ts 소스를 데이터로 읽어 계약을 단언한다(wswiring.test.ts·restartplan.test.ts 관례).
// 슬라이스 경계는 함수 이름으로 잡는다 — 이름이 바뀌면 앵커 소실로 빨개진다(무엇도 지키지 못하는 초록 금지).
import { describe, it, expect } from "bun:test";
import { readFileSync } from "node:fs";

const src = readFileSync(new URL("./main.ts", import.meta.url), "utf-8");
// 주석에만 적힌 장치로 핀이 통과하지 않게 한다(restartplan.test.ts 관례).
const code = src
  .replace(/\/\*[\s\S]*?\*\//g, "")
  .split("\n")
  .map((line) => (line.trimStart().startsWith("//") ? "" : line.replace(/\s\/\/.*$/, "")))
  .join("\n");

/** `start` 로 시작해 그 뒤 첫 `end` 까지(끝 포함). 앵커가 없으면 실패한다. */
function slice(start: string, end: string): string {
  const a = code.indexOf(start);
  expect({ 앵커: start, 존재: a >= 0 }).toEqual({ 앵커: start, 존재: true });
  expect({ 앵커: start, 유일: code.indexOf(start, a + 1) < 0 }).toEqual({ 앵커: start, 유일: true });
  const b = code.indexOf(end, a);
  expect({ 앵커: start, 끝: b > a }).toEqual({ 앵커: start, 끝: true });
  return code.slice(a, b + end.length);
}
const count = (hay: string, needle: string): number => hay.split(needle).length - 1;

describe("N7 핀 ① sendRaw(사람 실키) — 실패는 .catch(noteSendFail) 로 화면에 나온다(체인은 유지)", () => {
  const sendRaw = slice("const sendRaw = (data: string) => {", "\n  };\n");

  it("체인은 send_input → .catch(noteSendFail) 로 끝나고 빈 catch 로 삼키지 않는다", () => {
    expect(sendRaw.includes("sendChain = sendChain")).toBe(true);
    const sendAt = sendRaw.indexOf('invoke("send_input"');
    const catchAt = sendRaw.indexOf(".catch(noteSendFail)");
    expect(sendAt).toBeGreaterThanOrEqual(0);
    expect(catchAt).toBeGreaterThan(sendAt);
    expect(count(sendRaw, ".catch(")).toBe(1);
    // 무음 삼킴 형태(빈 화살표·빈 블록·undefined 반환·noop 대입) 전부 금지
    expect(/\.catch\(\s*(\(\s*\w*\s*\)|\w+)?\s*=>\s*(\{\s*\}|undefined|void 0|null)\s*\)/.test(sendRaw)).toBe(false);
    expect(sendRaw.includes("return sendChain;")).toBe(true);
  });

  it("noteSendFail 은 사유를 기록하고 flushSendFail 로 이어지며, flushSendFail 은 '입력 전송 실패' sticky 토스트를 낸다", () => {
    const note = slice("const noteSendFail = (e: unknown) => {", "\n  };\n");
    expect(note.includes("sendFailReason = String(e)")).toBe(true);
    expect(note.includes("flushSendFail()")).toBe(true);
    expect(note.includes("window.setTimeout(flushSendFail, wait)")).toBe(true);
    const flush = slice("const flushSendFail = () => {", "\n  };\n");
    expect(flush.includes('stickyToast(sendFailToastId, "health", `입력 전송 실패${n}`')).toBe(true);
    expect(flush.includes("${label} — ${reason}")).toBe(true);
  });
});

describe("N7 핀 ② injectRawToPane(경로 삽입) — 실패는 '삽입 실패' 토스트로 나온다", () => {
  const body = slice("async function injectRawToPane(", "\n}\n");

  it("send_input 은 try 안에 있고 catch 가 '삽입 실패' 토스트를 낸다", () => {
    const tryAt = body.indexOf("try {");
    const sendAt = body.indexOf('invoke("send_input"');
    const catchAt = body.indexOf("} catch (e) {");
    expect(tryAt).toBeGreaterThanOrEqual(0);
    expect(tryAt).toBeLessThan(sendAt);
    expect(catchAt).toBeGreaterThan(sendAt);
    const handler = body.slice(catchAt);
    expect(handler.includes('toast("watchdog", "삽입 실패", String(e));')).toBe(true);
    // UI 가 조립한 문안이다 — 기계 문안 표식은 그대로(R5 불변식)
    expect(body.includes("machineOrigin: true")).toBe(true);
  });
});

describe("N7 핀 ③ transferCrossDept(부서 전출) — 바깥 catch 는 '전출 실패' 토스트를 낸다 · N2 사유는 번역기를 거친다", () => {
  const body = slice("async function transferCrossDept(", "\n}\n");
  const fin = body.lastIndexOf("} finally {");
  const outerCatch = body.lastIndexOf("} catch (e) {", fin);
  const outer = body.slice(outerCatch, fin);

  it("바깥 try 는 catch 에서 '전출 실패' watchdog 토스트를 낸다(삼키지 않는다) — finally 가 뒤따른다", () => {
    expect(fin).toBeGreaterThan(0);
    expect(outerCatch).toBeGreaterThan(0);
    expect(outerCatch).toBeLessThan(fin);
    expect(outer.includes('toast("watchdog", "전출 실패"')).toBe(true);
    // 제목이 있는 토스트가 이 함수에 정확히 둘(안쪽 보상 롤백 catch · 바깥 catch) — 하나가 사라지거나 셋째가 번역기 없이 생기면 이 핀이 알린다
    expect(count(body, '"전출 실패"')).toBe(2);
  });

  it("★N2: 두 '전출 실패' 토스트 모두 사유가 restartInvokeFailureReason 을 거친다 — 데몬 원문(${e})을 그대로 싣지 않는다", () => {
    expect(/import\s*\{[^}]*\brestartInvokeFailureReason\b[^}]*\}\s*from\s*["']\.\/restartplan["']/.test(code)).toBe(true);
    // 함수 전체에 원문 보간이 없다
    expect(body.includes("${e}")).toBe(false);
    // 바깥 catch: 번역기를 직접 거친다 · 토스트 제목·꼬리 문구는 그대로
    expect(outer.includes("${restartInvokeFailureReason(e)} — 원본 pane은 보존됩니다")).toBe(true);
    // 안쪽(보상 롤백) catch: 한 번 번역해 세 분기가 같은 값을 쓴다
    const innerCatch = body.indexOf("} catch (e) {");
    expect(innerCatch).toBeGreaterThan(0);
    expect(innerCatch).toBeLessThan(outerCatch);
    const inner = body.slice(innerCatch, outerCatch);
    expect(inner.includes("const why = restartInvokeFailureReason(e);")).toBe(true);
    expect(count(inner, "${why}")).toBe(3);
    // 번역은 토스트 앞이다
    expect(inner.indexOf("const why = restartInvokeFailureReason(e);")).toBeLessThan(inner.indexOf('"전출 실패"'));
    // 분기 문구(원본 보존·런처 셸 안내)는 종전 그대로다 — 번역기만 끼웠다
    for (const piece of [
      "${why} — 원본 pane은 보존되고 새 pane은 회수했습니다",
      "${why} — 원본 pane은 보존됩니다. 목적지 런처 셸 surface:${newSid} 는 기동이 진행 중일 수 있어 남겨 두었습니다(확인 후 정리하세요)",
      "${why} — 원본 pane은 보존됩니다. 목적지에 기동된 surface:${agentSid} 와 런처 셸 surface:${newSid} 는 살아 있으니 확인 후 정리하세요",
    ])
      expect({ 조각: piece, 있음: inner.includes(piece) }).toEqual({ 조각: piece, 있음: true });
  });
});
