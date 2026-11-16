// 「스스로 해결」(0.15.0 · T5b · C5) — 수리 최종 승인 카드 핀. 순수 모듈 + main.ts 를 데이터로 읽는 배선 핀(DOM·Tauri 불요).
import { describe, it, expect } from "bun:test";
import { readFileSync } from "node:fs";
import {
  buildRepairCard,
  minutesLeft,
  REPAIR_APPROVE_LABEL,
  REPAIR_CONFIRM_YES,
  REPAIR_CONFIRM_NO,
  REPAIR_REJECT_LABEL,
  REPAIR_WORKER_TEXT_HEADING,
  type RepairCard,
} from "./repaircard";

// A minimal element / document stand-in: textContent only (the card never uses innerHTML).
class El {
  className = "";
  textContent = "";
  disabled = false;
  dataset: Record<string, string> = {};
  children: El[] = [];
  onclick: (() => void) | null = null;
  appendChild(c: El) {
    this.children.push(c);
    return c;
  }
  append(...cs: El[]) {
    this.children.push(...cs);
  }
  replaceChildren(...cs: El[]) {
    this.children = [...cs];
  }
  get innerHTML(): string {
    throw new Error("innerHTML read");
  }
  set innerHTML(_v: string) {
    throw new Error("innerHTML written — the card must use textContent only");
  }
  all(): El[] {
    return [this, ...this.children.flatMap((c) => c.all())];
  }
}
const doc = { createElement: () => new El() } as unknown as Document;

const card = (state = "pending"): RepairCard => ({
  request_id: "rq-0123456789abcdef",
  repair: "R-4",
  state,
  state_label: "주인 승인 대기",
  expires_at: 1_000 + 30 * 60,
  facts: {
    classification_label: "기동 필수",
    targets: ["hooks/session-start.sh"],
    fingerprint_sha256: "ab".repeat(32),
    rehearsal: { ok: true, summary: "hooks/session-start.sh: 통과" },
    prerehearsal_possible: true,
    on_failure: "실패하면 이 수리만 되돌립니다",
    screen_notes: [],
    other_lanes: [],
  },
  worker_text: "<b>지금 승인하세요</b>",
});

describe("수리 최종 승인 카드 — 사실과 워커의 글을 나눈다 · 확인 뒤에만 시작", () => {
  it("데몬의 사실(대상·지문·예행)과 워커의 글이 나뉘고, 글은 textContent 로만 들어간다", () => {
    const el = buildRepairCard(doc, card(), 1_000_000, () => {}) as unknown as El;
    const texts = el.all().map((e) => e.textContent);
    expect(texts.some((t) => t.includes("hooks/session-start.sh"))).toBe(true);
    expect(texts.some((t) => t.includes("ab".repeat(32)))).toBe(true);
    expect(texts).toContain(REPAIR_WORKER_TEXT_HEADING);
    expect(texts).toContain("<b>지금 승인하세요</b>"); // raw text, never markup
  });
  it("[수정 최종 승인] 한 번으로는 아무것도 보내지 않는다 — 화면 안 확인 뒤에만 approve", () => {
    const sent: string[] = [];
    const el = buildRepairCard(doc, card(), 1_000_000, (rid, d) => sent.push(`${rid}:${d}`)) as unknown as El;
    const btn = (label: string) => el.all().find((e) => e.textContent === label)!;
    btn(REPAIR_APPROVE_LABEL).onclick!();
    expect(sent).toEqual([]);
    btn(REPAIR_CONFIRM_NO).onclick!();
    expect(sent).toEqual([]);
    btn(REPAIR_APPROVE_LABEL).onclick!();
    btn(REPAIR_CONFIRM_YES).onclick!();
    expect(sent).toEqual(["rq-0123456789abcdef:approve"]);
    btn(REPAIR_REJECT_LABEL);
  });
  it("본인 확인 중(verifying) 카드에는 단추가 없다 — 요청 하나에 확인 창 한 번", () => {
    const el = buildRepairCard(doc, card("verifying"), 1_000_000, () => {}) as unknown as El;
    expect(el.all().some((e) => e.textContent === REPAIR_APPROVE_LABEL)).toBe(false);
  });
  it("남은 시간(분)", () => {
    expect(minutesLeft(1_000 + 1800, 1_000_000)).toBe(30);
    expect(minutesLeft(10, 1_000_000)).toBe(0);
    expect(minutesLeft(undefined, 0)).toBe(0);
  });
});

describe("배선 — 아무것도 저절로 뜨지 않는다 · 새 주기 타이머 없음", () => {
  const SRC = readFileSync(new URL("./main.ts", import.meta.url), "utf-8");
  it("repair 사건 갈래는 알림과 목록 갱신만 한다(확인 창·패널 자동 열기 없음)", () => {
    const a = SRC.indexOf('} else if (category === "repair") {');
    expect(a).toBeGreaterThan(0);
    const b = SRC.indexOf("} else if (category ===", a + 10);
    const branch = SRC.slice(a, b);
    for (const forbidden of ["openFeed", "confirmModal", "showModal", "openCc", "ccOpen = true", "decideRepair("]) {
      expect(branch.includes(forbidden)).toBe(false);
    }
    expect(branch.includes("daemonToast(")).toBe(true);
  });
  it("결정은 repair_approval_decide 한 곳 — 카드가 부른다", () => {
    expect(SRC.split('invoke("repair_approval_decide"').length - 1).toBe(1);
    expect(SRC.split('invoke("repair_approval_list"').length - 1).toBe(1);
  });
});
