// 업데이트 배지 **배선** 핀 (U9 · 0.14.41) — main.ts 소스를 데이터로 읽어 계약을 단언한다
// (wswiring.test.ts 관례: 런타임 코드 0줄 · 본체에서 import 되지 않는다 · DOM/Tauri 불요).
//
// 판정 순수 함수(updatestate.ts)가 옳아도, main.ts 가 배지를 다른 곳에서 직접 쓰거나 팩 확인 실패를
// 삼키면 오너 증상("숫자가 떠 있는데 눌러 보면 최신")이 되살아난다. 종전 결함 목록(보고서 R2~R5):
//   R2 팩 확인 실패를 catch {} 로 삼킴 · 실패 시 낡은 배지 보존   R3 ↻ 배지인데 클릭은 본체 창부터
//   R4 판독 실패를 '최신'으로 접음                            R5 no-op 팩 설치를 "완료"로 보고
import { describe, it, expect } from "bun:test";
import { readFileSync } from "node:fs";

const src = readFileSync(new URL("./main.ts", import.meta.url), "utf-8");
const stripComments = (s: string): string =>
  s
    .split("\n")
    .map((l) => (l.trimStart().startsWith("//") ? "" : l.replace(/\s\/\/.*$/, "")))
    .join("\n");
const code = stripComments(src);

/** `function name(` 부터 다음 최상위 `\n}\n` 까지(최상위 함수 본문). */
function fnBody(name: string): string {
  const a = code.search(new RegExp(`(async )?function ${name}\\(`));
  expect(a).toBeGreaterThanOrEqual(0); // 앵커가 사라졌다 = 이 핀이 무엇도 지키지 않는다
  const b = code.indexOf("\n}\n", a);
  expect(b).toBeGreaterThan(a);
  return code.slice(a, b);
}

describe("업데이트 배지 배선 — 단일 상태 · 단일 보기", () => {
  // ★성찰 A(major): renderUpdateBadge **정의**의 body(위 두 it)는 실제로 옳아도, renderUpdateAll
  //   본체가 그 함수를 부르는 줄 자체가 지워지면(호출 제거 뮤턴트) 배지가 다시는 갱신되지 않는다
  //   — 오너 증상(초록 점·배지 불일치)의 재발 형태다. 기존 핀은 "update-badge 를 쓰는 곳은
  //   renderUpdateBadge 하나"만 세므로 호출 제거에는 반응하지 않는다(정의 안의 "update-badge"
  //   문자열은 여전히 1건). 호출부를 직접 죈다.
  it("★(U16-A5-4) renderUpdateAll 은 실제로 renderUpdateBadge(deriveUpdateView(...)) 를 부른다(호출 제거 뮤턴트 차단)", () => {
    const b = fnBody("renderUpdateAll");
    expect(b).toContain("renderUpdateBadge(deriveUpdateView(updState, updClock));");
  });

  it("★update-badge 를 쓰는 곳은 renderUpdateBadge 하나뿐", () => {
    const hits = code.split('"update-badge"').length - 1;
    expect(hits).toBe(1);
    const body = fnBody("renderUpdateBadge");
    expect(body).toContain('"update-badge"');
    // 리뷰1 F2(M-U7/M-U8 공허): hidden 대입 줄 자체를 핀 — 삭제되거나 `badge.hidden = false` 로
    // 고정돼도(짝 규칙이 아무리 옳아도) 배지가 안 뜨거나 확인 전에도 보이는 회귀를 여기서 잡는다.
    expect(body).toContain("badge.hidden = v.badge.hidden");
  });

  it("종전 이중 원천(updateAvailable · packUpdateAvailable 전역)이 되살아나지 않았다", () => {
    expect(/\blet updateAvailable\b/.test(code)).toBe(false);
    expect(/\blet packUpdateAvailable\b/.test(code)).toBe(false);
    expect(code.includes("updatePlan(")).toBe(false); // 판정은 deriveUpdateView 안에서만
  });

  it("'최신' 문구는 main.ts 에 없다 — 파생 보기(updatestate.ts)만 말한다", () => {
    expect(code.includes("최신 버전입니다")).toBe(false);
    expect(code.includes("✅ 최신 버전")).toBe(false);
  });

  it("팩 확인 실패를 삼키지 않는다(R2) — 두 invoke 결과가 모두 상태 전이 함수로 간다", () => {
    const b = fnBody("refreshUpdateState");
    expect(b).toContain('invoke("check_update")');
    expect(b).toContain('invoke("check_pack_update")');
    // 리뷰1 F2(M-U9 공허): 식별자 존재만으로는 `packFromCheck(updState.pack, {status:"none"}, now)`
    // 처럼 packR 을 버리고 늘 '없음' 으로 고정해도 살아남는다. 실제로 이 확인의 결과(binR·packR)가
    // 이전 상태(updState.bin·updState.pack)와 함께 들어가는지 인자까지 핀.
    expect(b).toContain("binFromCheck(updState.bin, binR");
    expect(b).toContain("packFromCheck(updState.pack, packR");
    expect(code.includes("packCheckFailed = true")).toBe(false);
  });

  it("확인은 단일 비행(동시 클릭·폴링이 curl 을 겹쳐 띄우지 않는다) + 확인마다 상한(멈춘 연결이 비행을 영구히 붙잡지 않게)", () => {
    const b = fnBody("refreshUpdateState");
    expect(b).toContain("updRefreshInFlight");
    expect(b).toContain('rpcT(invoke("check_update"), T_UPD_CHECK)');
    expect(b).toContain('rpcT(invoke("check_pack_update"), T_UPD_CHECK)');
    expect(b).toContain("updRefreshInFlight = null"); // finally 에서 반드시 풀린다
    // 리뷰1 F4: 본체·팩 확인을 동시에 띄운다 — 순차로 되돌리면(둘 다 멈춘 연결일 때) 최악
    // 2×T_UPD_CHECK 를 기다리는 회귀가 되살아난다.
    expect(b).toContain("Promise.all([binCheck(), packCheck()])");
  });

  it("Update 클릭 = 상태 창(R3) — 캐시로 본체 설치 창부터 여는 분기 금지", () => {
    const b = fnBody("onUpdateButton");
    expect(b).toContain("openUpdatePanel(");
    expect(b).toContain("refreshUpdateState(false)");
    expect(b.includes("promptBinaryPatch(")).toBe(false);
    expect(b.includes("promptPackInstall(")).toBe(false);
  });

  it("no-op 팩 설치는 pack-uptodate 로 받는다(R5) · pack-updated 는 배지를 직접 만지지 않는다", () => {
    const u = code.indexOf('listen("pack-uptodate"');
    expect(u).toBeGreaterThan(0);
    const upSeg = code.slice(u, code.indexOf("});", u));
    // 리뷰1 F2(M-U6 공허): 리스너 실재만으로는 `packAfterUpToDate(` 를 `packAfterInstalled(` 로
    // 바꿔치기해도(디스크를 못 읽은 no-op 까지 '최신'으로 접는다 — D3 무력화) 살아남는다. 이
    // 세그먼트가 실제로 packAfterUpToDate 를 부르는지 핀.
    expect(upSeg).toContain("packAfterUpToDate(");
    expect(upSeg.includes("packAfterInstalled(")).toBe(false);

    const a = code.indexOf('listen("pack-updated"');
    expect(a).toBeGreaterThan(0);
    const seg = code.slice(a, code.indexOf("});", a));
    expect(seg.includes(".hidden")).toBe(false);
    expect(seg).toContain("packAfterInstalled(");
  });

  it("창 제목 색은 isLatest 일 때만 ok — 미확인·확인 중을 최신과 같은 초록으로 그리지 않는다(리뷰1 F3)", () => {
    const b = fnBody("renderUpdatePanel");
    // v.badge.tone 을 그대로 클래스에 쓰면(중립 '…' 상태의 tone 이 "ok") '확인 중…'이 초록으로
    // 보인다 — isLatest 가 아닌 "ok" 톤은 muted 로 낮추는 분기가 있어야 한다.
    expect(b).toContain('v.isLatest ? "ok"');
    expect(b).toContain('v.badge.tone === "ok" ? "muted"');
  });

  it("폴링 주기·silent 불변식 유지 — 시작 1회 + 6시간(새 타이머 0) · silent 경로는 창을 열지 않는다", () => {
    expect(code.includes("refreshUpdateState(true);")).toBe(true);
    expect(code.includes("setInterval(() => refreshUpdateState(true), 6 * 3600 * 1000)")).toBe(true);
    expect(fnBody("refreshUpdateState").includes("openUpdatePanel(")).toBe(false);
  });
});
