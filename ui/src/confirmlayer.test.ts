// 확인 창 층 보장·무음 경로 **배선** 회귀 핀(P2) — main.ts 를 데이터로 읽어 단언한다.
//
// 사건(2026-09-23 · 0.14.41): 승인 Feed 카드의 [확인 창 열기] → confirmModal 이 연 창이 Control Center 패널 뒤에
// 깔려 '무반응'으로 보였다. 층서 자체(토큰 표)는 stacking.test.ts(P1)가 못박고, 이 파일은 그 짝인 코드 쪽을 본다:
//   ① 확인 창이 붙을 때 **열린 패널과의 관계를 코드가 한 번 더 보장**한다(§5-4 — 표가 훗날 어긋나도 뒤에 깔리지 않게).
//   ② :has() 를 못 읽는 WebView 대비 폴백 표지 body.modal-open(§5-3 토스트 회피)을 DOM 존재로 켜고 끈다.
//   ③ 카드 안내문이 대화 승인 경로를 먼저 적는다(§11 R10).
//   ④ 카드 버튼이 **아무 가시 효과 없이 끝나는 경로 0** — 의도된 무음('나중에')은 주석으로 구분한다(§5-4).
// 설계 정본: 팀만들기-확인창-무반응-수정설계안-최종-20260923.md §5-3·§5-4 · §11 R10 · §13 P2.
// P1(stacking.test.ts)과 파일을 나눈 것은 의도다 — §13 롤백 지점: P1·P2 는 독립적으로 되돌릴 수 있어야 한다.
import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";

const SRC = readFileSync(new URL("./main.ts", import.meta.url), "utf-8");
const strip = (s: string) =>
  s
    .split("\n")
    .map((l) => (l.trimStart().startsWith("//") ? "" : l.replace(/\s\/\/.*$/, "")))
    .join("\n");
const CODE = strip(SRC);

/** 최상위 함수 본문(주석 제거본 또는 원문) — 앵커가 사라지면 이 핀이 아무것도 지키지 않으므로 즉시 실패. */
function fnBody(name: string, src: string = CODE): string {
  const a = src.search(new RegExp(`(async )?function ${name}\\(`));
  expect(a).toBeGreaterThanOrEqual(0);
  const b = src.indexOf("\n}\n", a);
  expect(b).toBeGreaterThan(a);
  return src.slice(a, b);
}

describe("confirmModal — 열린 패널 위에 뜨는 관계를 코드가 보장한다(§5-4)", () => {
  test("오버레이를 붙인 **직후** keepModalAboveOpenPanels(ov) 를 부른다", () => {
    const f = fnBody("confirmModal");
    const app = f.indexOf("document.body.appendChild(ov);");
    const keep = f.indexOf("keepModalAboveOpenPanels(ov);");
    expect(app).toBeGreaterThan(0);
    expect(keep).toBeGreaterThan(app);
  });

  test("판정은 열린 패널(#cc-panel·.palette-overlay)의 **계산된** 층과의 관계다 — 같거나 높으면 자기 층을 그 위로 올린다", () => {
    const g = fnBody("keepModalAboveOpenPanels");
    expect(CODE).toContain('"#cc-panel, .palette-overlay"'); // 대상 패널 선택자(OPEN_PANEL_SELECTOR)
    expect(g).toContain("OPEN_PANEL_SELECTOR");
    expect(g).toContain("getComputedStyle(ov).zIndex"); // 토큰이 풀린 실제 값(표의 숫자를 코드에 다시 적지 않는다)
    expect(g).toContain('display === "none"'); // 닫힌 패널([hidden])은 관계 밖
    expect(g).toContain("ov.style.zIndex = String(highest + 1)");
  });

  test("관계 보장은 패널을 닫지 않는다 — 사용자가 보던 카드·문맥을 잃지 않게(ⓑ 자기 층을 올린다)", () => {
    const g = fnBody("keepModalAboveOpenPanels");
    expect(g).not.toContain("setCcOpen(");
    expect(g).not.toContain(".remove()");
    expect(g).not.toContain(".hidden =");
  });
});

describe("body.modal-open — 토스트 회피(§5-3)의 폴백 표지", () => {
  test("syncModalOpenClass 는 떠 있는 .modal-overlay 의 **존재**로 켜고 끈다(계수·플래그 없음 — 고착 방지)", () => {
    const s = fnBody("syncModalOpenClass");
    expect(s).toContain('classList.toggle("modal-open", document.querySelector(".modal-overlay") != null)');
  });

  test("confirmModal 이 열 때(붙인 직후)와 닫을 때(떼어 낸 직후 · resolve 전) 표지를 맞춘다", () => {
    const f = fnBody("confirmModal");
    const app = f.indexOf("document.body.appendChild(ov);");
    const openSync = f.indexOf("syncModalOpenClass();", app);
    expect(app).toBeGreaterThan(0);
    expect(openSync).toBeGreaterThan(app);
    const rm = f.indexOf("ov.remove();");
    const closeSync = f.indexOf("syncModalOpenClass();", rm);
    const res = f.indexOf("resolve(v);", rm);
    expect(rm).toBeGreaterThan(0);
    expect(closeSync).toBeGreaterThan(rm);
    expect(res).toBeGreaterThan(closeSync);
  });

  test("확인 창 밖의 창(입력·업데이트·피드백 창 — 전부 .modal-overlay)도 같은 표지를 따르게 body 직계 자식을 지켜본다", () => {
    expect(CODE).toContain("new MutationObserver(syncModalOpenClass).observe(document.body, { childList: true })");
  });

  // ★(0.14.42 리뷰 F2) 키 큰 창 위의 토스트 클릭 가로채기 — 확인 창이 **아닌** 창이 떠 있으면 토스트를 창 밑으로.
  //   CSS 쪽 관계는 stacking.test.ts 가, 여기는 그 선택자가 기대는 표지(.confirm-overlay · body.toast-under-modal)를 본다.
  test("confirmModal 의 오버레이는 .confirm-overlay 표지를 단다(토스트가 위에 머무는 유일한 창 — 본문 + 아래 버튼 줄)", () => {
    const f = fnBody("confirmModal");
    expect(f).toContain('ov.className = "modal-overlay confirm-overlay";');
  });

  test("syncModalOpenClass 는 확인 창 밖의 창 존재로 body.toast-under-modal 을 켜고 끈다(:has() 폴백 · 계수 없음)", () => {
    const s = fnBody("syncModalOpenClass");
    expect(s).toContain(
      'classList.toggle("toast-under-modal", document.querySelector(".modal-overlay:not(.confirm-overlay)") != null)',
    );
  });
});

describe("팀 제안 카드 — 안내문(R10)·무음 경로 0(§5-4)", () => {
  const card = () => {
    const i = SRC.indexOf('classifyPendingFeed(item) === "team-create"');
    expect(i).toBeGreaterThan(0);
    return SRC.slice(i, SRC.indexOf("} else if", i + 10));
  };

  test("안내문이 대화 승인 경로를 먼저 적고, 화면 경로([확인 창 열기] → [만들기])를 뒤에 적는다", () => {
    const seg = card();
    const talk = seg.indexOf("'만들어'라고 답해 주시면 바로 만듭니다");
    const gui = seg.indexOf("화면에서 직접 하시려면 [확인 창 열기] → [만들기]");
    expect(talk).toBeGreaterThan(0);
    expect(gui).toBeGreaterThan(talk);
  });

  // ★(0.14.42 리뷰 m6) 카드는 UI 갱신과 함께 모든 설치에 즉시 뜨지만, 이미 CEO 로 승격된 기계는 MASTER_DIRECTIVE 가
  //   사용자 소유라 신본 지침이 .new 로만 도착한다(pack-merge 전까지 대표는 질문(ask)을 열지 않는다). 그래서 대화
  //   경로는 **대표가 먼저 여쭌 경우**로 조건을 걸어 적는다 — 무조건 "말씀하시면 바로 만듭니다"는 그 기계에서 거짓이다.
  test("대화 경로 안내는 대표의 질문('만들까요?')을 조건으로 건다 — 지침이 옛 판인 기계에서도 거짓이 되지 않게", () => {
    const seg = strip(card()); // 주석 속 인용(옛 문구)은 안내문이 아니다
    const cond = seg.indexOf("'만들까요?'라고 여쭈면");
    expect(cond).toBeGreaterThan(0);
    expect(cond).toBeLessThan(seg.indexOf("'만들어'라고 답해 주시면 바로 만듭니다"));
    expect(seg).not.toContain("말씀하시면 바로 만듭니다");
  });

  test("카드 버튼은 흐름의 예외를 삼키지 않는다 — 거부된 약속(Promise)은 토스트로 보인다", () => {
    const seg = strip(card());
    expect(seg).toContain("runTeamProposalFlow(item).catch(");
    expect(seg).not.toContain("void runTeamProposalFlow(item)");
  });

  test("runTeamProposalFlow 의 모든 return 은 가시 효과(토스트·busy 안내) 뒤이거나 '의도된 무음' 주석이 달린 나중에 경로다", () => {
    const lines = fnBody("runTeamProposalFlow", SRC).split("\n");
    const silent: string[] = [];
    let intended = 0;
    lines.forEach((l, i) => {
      if (!/\breturn\b/.test(strip(l))) return;
      const win = lines.slice(Math.max(0, i - 2), i + 1).join("\n");
      if (/의도된 무음/.test(l)) intended++;
      else if (!/toast\(|notifyTeamFlowBusy\(\)|daemonActionBlocked\(\)/.test(win)) silent.push(l.trim());
    });
    expect(silent).toEqual([]);
    expect(intended).toBe(1); // 확인 창의 [나중에]·바깥 클릭 — 사용자가 스스로 닫은 것(창이 사라지는 것이 가시 효과)
  });

  test("daemonActionBlocked 는 true 를 돌려주기 전에 스스로 토스트를 낸다(위 핀이 이 호출을 가시 효과로 인정하는 근거)", () => {
    const d = fnBody("daemonActionBlocked");
    expect(d.indexOf("toast(")).toBeGreaterThan(0);
    expect(d.indexOf("toast(")).toBeLessThan(d.indexOf("return true"));
  });
});
