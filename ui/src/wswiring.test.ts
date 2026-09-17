// 워크스페이스 복원 **배선** 회귀 핀 — main.ts 소스를 데이터로 읽어 계약을 단언한다
// (typegate.test.ts 관례: 런타임 코드 0줄 · 본체에서 import 되지 않는다 · DOM/Tauri 불요).
//
// ★왜 판정 테스트(wsreconcile.test.ts)만으로 부족한가 (2026-09-16 성찰 1회 지적):
// 이번 실사고에서 실제로 틀린 것은 **판정이 아니라 배선**이었다. 순수 함수는 옳아도
//   · main.ts 가 그 함수를 부르지 않거나
//   · 복원 체인의 await 하나가 시간 상한을 빠뜨리거나
//   · 되살아난 옛 술어 한 줄이 남아 있으면
// 화면은 그대로 사라진다. 실제로 성찰 1회가 찾은 blocker 는 '복원 체인의 마지막 두
// newSurface 만 상한을 빠뜨린 것'이었다 — 사람이 한 줄씩 세는 방식으로는 반드시 다시 빠진다.
// 그래서 배선을 기계가 센다. 여기 실패는 "고쳐 두었다고 믿는 것이 코드에 없다"는 뜻이다.
import { describe, it, expect } from "bun:test";
import { readFileSync } from "node:fs";

const MAIN_URL = new URL("./main.ts", import.meta.url);
const src = readFileSync(MAIN_URL, "utf-8");
/** 주석을 걷어낸 코드 본문 — '코드에 있는가'를 묻는 핀은 주석에 속으면 안 된다(설명문이 핀을 깨뜨린다). */
const code = src
  .split("\n")
  .map((l) => (l.trimStart().startsWith("//") ? "" : l.replace(/\s\/\/.*$/, "")))
  .join("\n");

/** start() 의 세션 복원 블록만 잘라낸다 — 이 계약들은 그 구간에만 적용된다. */
function restoreSlice(): string {
  const a = src.indexOf("// Session restore (멀티마스터 F4)");
  const b = src.indexOf("started = true;", a);
  expect(a).toBeGreaterThan(0); // 구간 앵커가 사라졌다 = 이 핀이 무엇도 지키지 않는다
  expect(b).toBeGreaterThan(a);
  return src.slice(a, b);
}

describe("복원 배선 — 판정 모듈이 실제로 꽂혀 있다", () => {
  it("★옛 차별 술어가 되살아나지 않았다 (`ws.socket != null && lb?.ok === true`)", () => {
    // 이 한 줄이 재부팅 후 본부 5팀을 화면에서 지웠다. 되살아나면 즉시 red.
    expect(code.includes("ws.socket != null && lb?.ok === true")).toBe(false);
  });

  it("복원 필터가 keepWorkspaceOnRestore 를 쓴다(판정 인라인 복제 금지)", () => {
    expect(restoreSlice().includes("keepWorkspaceOnRestore(")).toBe(true);
  });

  it("존재 진실원 보강이 missingKnownWorkspaces 를 쓴다", () => {
    expect(restoreSlice().includes("missingKnownWorkspaces(")).toBe(true);
  });

  it("3초 틱의 유령 수렴이 advanceGhostStrikes 를 쓴다(2연속 게이트 인라인 복제 금지)", () => {
    expect(src.includes("advanceGhostStrikes(")).toBe(true);
  });
});

describe("복원 배선 — 체인의 모든 대기에 시간 상한이 있다", () => {
  // 먹통(accept 후 무응답) 데몬 하나가 start() 를 붙잡으면 **탭이 하나도 렌더되지 않는다**.
  // 상한이 빠진 await 가 한 곳만 있어도 그 결함이 통째로 되살아난다.
  it("★복원 구간의 newSurface 호출은 전부 상한 인자(T_NEW)를 넘긴다", () => {
    // 줄 단위로 본다 — `newSurface(null, current().socket, T_NEW)` 처럼 인자에 괄호가 들어가면
    // 괄호 균형을 안 맞추는 정규식이 첫 `)` 에서 끊겨 T_NEW 를 놓친다(핀이 조용히 무력해진다).
    const lines = restoreSlice().split("\n").filter((l) => l.includes("newSurface("));
    expect(lines.length).toBeGreaterThan(0); // 호출이 사라졌다면 이 핀은 무의미해진다
    for (const l of lines) expect(l).toContain("T_NEW");
  });

  it("★복원 구간의 데몬 invoke 는 전부 rpcT 를 통과한다", () => {
    const slice = restoreSlice();
    // 복원 체인이 직접 부르는 데몬 왕복만 본다(이름 목록이 계약이다 — 늘어나면 여기 추가).
    for (const cmd of ["list_depts", "dept_tombstones", "daemon_status", "launch_dept_daemon", "list_surfaces"]) {
      const re = new RegExp(`(\\w+)\\(invoke\\("${cmd}"`, "g");
      const wrappers = [...slice.matchAll(re)].map((m) => m[1]);
      expect(wrappers.length).toBeGreaterThan(0); // 그 호출이 사라졌으면 목록을 갱신하라
      for (const w of wrappers) expect(w).toBe("rpcT");
    }
  });

  it("상한 값은 명명 상수다(리터럴 흩뿌리기 금지 · Windows 배율이 한 곳에서 걸린다)", () => {
    for (const k of ["T_REG", "T_LIST", "T_STATUS", "T_LAUNCH", "T_NEW", "winScaled"]) {
      expect(src.includes(`const ${k}`)).toBe(true);
    }
  });
});

describe("복원 배선 — 무응답과 사망을 구분한다", () => {
  it("★daemon_status 타임아웃은 재기동 사유가 아니다(중복 launch 폭주 차단)", () => {
    // 살아 있는(느린·먹통인) 데몬에 rival launch 를 걸면 중복 데몬·좌석 탈취로 번진다.
    const slice = restoreSlice();
    expect(slice.includes("statusUnknown")).toBe(true);
    expect(slice.includes("if (statusUnknown) continue;")).toBe(true);
  });
});

describe("복원 배선 — 같은 소켓에 요청을 쌓지 않는다(폭주 축)", () => {
  // JS 상한은 **JS 쪽 포기**일 뿐 Rust/데몬 왕복을 취소하지 못한다. 상한만 두고 3초마다 같은
  // 소켓에 새 요청을 보내면 대기열이 자란다 — 가드의 전부는 '언제 푸느냐'다.
  it("★in-flight 해제는 원 호출이 실제로 끝났을 때다(rpcT 포기 시점 아님)", () => {
    // rpcT 가 포기하는 순간 풀면 다음 틱이 또 얹는다 = 가드가 있으나 마나.
    expect(src.includes("function releaseFlightWhenSettled(")).toBe(true);
    // 두 주기 폴러(3초 틱 list_surfaces · 10초 사이드바 org_status) 모두에 걸려 있어야 한다.
    expect((src.match(/releaseFlightWhenSettled\(/g) ?? []).length).toBeGreaterThanOrEqual(3);
    // 조기 해제(finally 에서 지우기)가 되살아나면 red.
    expect(code.includes("inFlight.delete(flightKey)")).toBe(false);
    expect(code.includes("inFlight.delete(sbKey)")).toBe(false);
  });

  it("★영구 skip 방지 — 끝나지 않는 호출도 재시도 상한이 있다(자가치유 전멸 차단)", () => {
    expect(src.includes("const INFLIGHT_RETRY_MS")).toBe(true);
    expect(src.includes("Date.now() - since < INFLIGHT_RETRY_MS")).toBe(true);
  });
});

describe("복원 배선 — 묘비는 결측이면 닫는다(fail-closed)", () => {
  it("★묘비 조회 실패(null)에 부서를 만들지 않는다 — 순수부에 그 조항이 있다", () => {
    const ws = readFileSync(new URL("./wsreconcile.ts", import.meta.url), "utf-8");
    expect(ws.includes("if (tombs === null) return out;")).toBe(true);
  });
  it("그룹 삭제도 teardown 전에 삭제 의도를 남긴다(부활 경로 차단)", () => {
    const a = src.indexOf("async function confirmDeleteGroup(");
    const b = src.indexOf("stop_dept_daemon_by_socket", a);
    expect(a).toBeGreaterThan(0);
    expect(src.slice(a, b).includes("dept_tombstone_by_socket")).toBe(true); // 묘비가 teardown 앞이다
  });
});
