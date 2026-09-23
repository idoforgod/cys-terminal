import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import {
  classifyPendingFeed,
  CYCLE_VERIFY_NOTE,
  CYCLE_VERIFY_DISMISS_TITLE,
  feedCreatedToastTitle,
  FEED_TOAST_APPROVAL_TITLE,
  FEED_TOAST_NOTICE_TITLE,
  NOTICE_FEED_KIND_PREFIXES,
  NOTICE_FEED_KINDS,
} from "./feedclass";

describe("classifyPendingFeed — pending 조작면 분류(패널·팔레트 공용 단일 술어)", () => {
  // ★W4-B(결함 7) 회귀 핀: cycle-verify 는 어떤 부가 신호와도 무관하게 GUI 판정 불가
  // 부류다 — Allow/Deny 가 이 kind 에 되살아나면(=standard 로 새면) GUI Allow 가
  // resolver 없는 소모가 되어 cycle 을 죽이는 기만 버튼이 재도입된다(W-4 동일 계급).
  test("★cycle-verify kind 는 항상 'cycle-verify' — daemon_issued·접두와 무관", () => {
    expect(classifyPendingFeed({ kind: "cycle-verify", request_id: "r1" })).toBe("cycle-verify");
    expect(
      classifyPendingFeed({ kind: "cycle-verify", request_id: "r1", daemon_issued: false }),
    ).toBe("cycle-verify");
    // 우선순위 핀: 겹치는 신호가 있어도 '버튼을 내리는' 분류가 이긴다(안전 방향).
    expect(
      classifyPendingFeed({ kind: "cycle-verify", request_id: "daemon-x", daemon_issued: true }),
    ).toBe("cycle-verify");
  });

  test("approval + 서버 사실(daemon_issued=true) → daemon-detected", () => {
    expect(
      classifyPendingFeed({ kind: "approval", request_id: "r2", daemon_issued: true }),
    ).toBe("daemon-detected");
  });

  test("approval + 필드 부재(구 데몬 스큐) → 'daemon-' 접두 fail-closed 폴백", () => {
    // 모르면 데몬 항목으로 취급(오판 방향이 안전한 쪽 — main.ts 주석의 논증).
    expect(classifyPendingFeed({ kind: "approval", request_id: "daemon-7" })).toBe(
      "daemon-detected",
    );
    expect(classifyPendingFeed({ kind: "approval", request_id: "req-7" })).toBe("standard");
  });

  test("approval + 서버가 부정(daemon_issued=false) → 접두가 있어도 standard(서버 사실 우선)", () => {
    // 데몬이 'daemon-' 접두를 예약 네임스페이스로 거부하므로 실제로는 생기지 않는 조합 —
    // ?? 의 의미(서버 값이 있으면 폴백 미발동)를 핀한다.
    expect(
      classifyPendingFeed({ kind: "approval", request_id: "daemon-7", daemon_issued: false }),
    ).toBe("standard");
  });

  test("특례 보존: ceo-promote-request 등 다른 kind 는 standard(Allow 경로 유지)", () => {
    expect(classifyPendingFeed({ kind: "ceo-promote-request", request_id: "r3" })).toBe("standard");
    expect(classifyPendingFeed({ kind: "learn_proposal", request_id: "r4" })).toBe("standard");
  });
});

describe("cycle-verify 안내 문구 — 문자열 핀(기만·무고지 금지)", () => {
  test("안내: 판정 주체(지정 검증자)·유효 경로(cys feed reply)·GUI 승인 불가 사유를 전부 적는다", () => {
    expect(CYCLE_VERIFY_NOTE).toContain("지정 검증자");
    expect(CYCLE_VERIFY_NOTE).toContain("cys feed reply");
    expect(CYCLE_VERIFY_NOTE).toContain("clear 미실행");
  });
  test("치우기 title: '판정이 아님'과 진행 중 cycle 안전 중단 부작용을 고지한다", () => {
    expect(CYCLE_VERIFY_DISMISS_TITLE).toContain("판정이 아닙니다");
    expect(CYCLE_VERIFY_DISMISS_TITLE).toContain("dismissed");
    expect(CYCLE_VERIFY_DISMISS_TITLE).toContain("안전 중단");
  });
});

// ★U10(0.14.41) 토스트 제목 분리 — 정보성 kind 는 'ℹ 알림', 그 밖(승인·결정성·미지)은 '📥 승인 요청'.
describe("feedCreatedToastTitle — 정보성 알림 vs 승인 요청", () => {
  test("정보성 kind(허용목록·formation-* 접두) → ℹ 알림", () => {
    for (const k of ["hook-missing", "bootstrap-fail", "warn", "error", "formation", "ceo-notice",
                     "formation-complete", "formation-partial", "formation-pending", "formation-failed"]) {
      expect(feedCreatedToastTitle(k)).toBe(FEED_TOAST_NOTICE_TITLE);
    }
  });
  test("승인·결정성·미지 kind → 📥 승인 요청(모르면 승인 쪽)", () => {
    for (const k of ["permission", "approval", "first_run_gate", "cycle-verify", "learn_proposal",
                     "mission-set", "ceo-promote-request", "question", "", "formationX", "Hook-Missing",
                     undefined, null, 7]) {
      expect(feedCreatedToastTitle(k)).toBe(FEED_TOAST_APPROVAL_TITLE);
    }
  });
  test("데몬 정본(state.rs NOTICE_FEED_KINDS·PREFIXES)과 사본이 같다", () => {
    const rs = readFileSync(new URL("../../src/bin/cysd/state.rs", import.meta.url), "utf-8");
    const grab = (name: string): string[] => {
      const m = rs.match(new RegExp("pub const " + name + ": &\\[&str\\] = &\\[([^\\]]*)\\]"));
      expect(m).not.toBeNull();
      return Array.from((m as RegExpMatchArray)[1].matchAll(/"([^"]*)"/g), (x) => x[1]);
    };
    expect([...grab("NOTICE_FEED_KINDS")].sort()).toEqual([...NOTICE_FEED_KINDS].sort());
    expect([...grab("NOTICE_FEED_KIND_PREFIXES")].sort()).toEqual([...NOTICE_FEED_KIND_PREFIXES].sort());
  });
  test("main.ts 배선 — feed.item.created 토스트가 분류 함수를 쓴다(고정 '📥 승인 요청' 리터럴 제거)", () => {
    const main = readFileSync(new URL("./main.ts", import.meta.url), "utf-8");
    const at = main.indexOf('if (name === "feed.item.created") {');
    expect(at).toBeGreaterThan(0);
    const branch = main.slice(at, at + 400);
    expect(branch).toContain("feedCreatedToastTitle(payload.kind)");
    expect(branch).not.toContain('"📥 승인 요청"');
  });
});
