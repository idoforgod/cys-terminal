import { describe, it, expect } from "bun:test";
import {
  RESET_PHRASE,
  resetPhraseMatches,
  resetMismatchHint,
  resetSizeHuman,
  resetNoticeLines,
  normalizePhrase,
  resetOutsideItems,
  resetTopItems,
  resetResultTitle,
  resetResultBody,
  type ResetPreview,
} from "./resetconfirm";

const preview = (over: Partial<ResetPreview> = {}): ResetPreview => ({
  quarantineCount: 34,
  totalBytes: 2_200_000_000,
  keptCount: 3,
  stripProfiles: 5,
  trashDir: "~/.local/state/cys-trash/factory-reset-X",
  ...over,
});

describe("resetPhraseMatches", () => {
  it("정확 일치·공백 관용만 통과", () => {
    expect(resetPhraseMatches("완전 초기화")).toBe(true);
    expect(resetPhraseMatches("  완전 초기화  ")).toBe(true);
    expect(resetPhraseMatches("완전초기화")).toBe(false); // 공백 소실은 여전히 불일치
    expect(resetPhraseMatches("")).toBe(false);
  });

  // ★P2-1: 육안상 같은 입력은 같게 받는다(CLI normalize_confirm_phrase 와 동일 규칙).
  it("따옴표 복사·NFD 자모·NBSP/전각/ZWSP 를 관용한다", () => {
    for (const s of [
      '"완전 초기화"',
      "'완전 초기화'",
      "\u201C완전 초기화\u201D",
      "완전\u00A0초기화",
      "완전\u3000초기화",
      "완전 \u200B초기화",
      "완전  초기화",
      "완전 초기화".normalize("NFD"),
    ]) {
      expect(resetPhraseMatches(s)).toBe(true);
    }
    // 관용이 정확성을 삼키지 않는다.
    for (const s of ["완전 삭제", "초기화", "완전초기화"]) {
      expect(resetPhraseMatches(s)).toBe(false);
    }
  });

  it("문구 상수는 CLI(cys factory-reset FACTORY_RESET_PHRASE)와 동일 계약", () => {
    expect(RESET_PHRASE).toBe("완전 초기화");
  });
});

describe("resetMismatchHint", () => {
  it("빈 입력·일치는 무힌트, 불일치는 사유를 말한다(무반응 오인 방지)", () => {
    expect(resetMismatchHint("")).toBe("");
    expect(resetMismatchHint("완전 초기화")).toBe("");
    const h = resetMismatchHint("완전삭제");
    expect(h).toContain(RESET_PHRASE);
    expect(h.length).toBeGreaterThan(0);
  });
});

describe("resetSizeHuman", () => {
  it("purgeDept sizeHuman과 동일 임계", () => {
    expect(resetSizeHuman(0)).toBe("0 B");
    expect(resetSizeHuman(999)).toBe("999 B");
    expect(resetSizeHuman(1500)).toBe("1.5 KB");
    expect(resetSizeHuman(2_400_000)).toBe("2.4 MB");
    expect(resetSizeHuman(2_200_000_000)).toBe("2.2 GB");
  });
});

describe("resetNoticeLines", () => {
  it("정직 고지 계약: 즉시 종료·격리 규모·재로그인·보존·복구·온보딩·입력 문구를 전부 담는다", () => {
    const all = resetNoticeLines(preview({ liveSessions: 4, deptCount: 2 })).join("\n");
    expect(all).toContain("34건");
    expect(all).toContain("2.2 GB");
    expect(all).toContain("재로그인");
    expect(all).toContain("3건은 삭제되지 않습니다");
    expect(all).toContain("직접 만든 설정"); // ~/.cys/local 기본 보존 계약(A8)
    expect(all).toContain("--undo"); // 복구 수단이 제품에 있다는 사실을 명시(P0)
    expect(all).toContain("14일");
    expect(all).toContain("온보딩");
    expect(all).toContain(RESET_PHRASE);
    // ★P0-2: 세션 즉사 고지가 **첫 줄**이어야 한다 — 가장 되돌릴 수 없는 피해다.
    expect(resetNoticeLines(preview({ liveSessions: 4, deptCount: 2 }))[0]).toContain("즉시 종료");
    expect(all).toContain("세션 4개");
    expect(all).toContain("부서 2개");
  });

  it("★P0-2: 사용자 폴더 안에서 사라지는 항목은 경로를 반드시 노출한다", () => {
    const info = preview({
      items: [
        { path: "/dummy-home/.cys/pack", label: "팩", size_bytes: 17_000_000 },
        {
          path: "/dummy-home/Desktop/CYSjavis/cys-homepage/_round",
          label: "작업기억(_round)",
          size_bytes: 279_000_000,
          outside_state: true,
        },
      ],
    });
    const outside = resetOutsideItems(info);
    expect(outside).toHaveLength(1);
    const all = resetNoticeLines(info).join("\n");
    expect(all).toContain("내 폴더 안에서 사라지는 항목");
    expect(all).toContain("cys-homepage/_round");
    expect(all).toContain("279.0 MB");
  });

  it("★P0-2: report_only(자동 정리 안 함)·이전 중단 흔적을 사용자에게 전달한다", () => {
    const all = resetNoticeLines(
      preview({
        reportOnly: ["~/.zshrc 에 cys 관련 줄이 있다 — 자동 수정하지 않음"],
        interruptedPrior: ["~/.local/state/cys-trash/factory-reset-20260101T000000Z"],
      }),
    ).join("\n");
    expect(all).toContain("자동 정리하지 않음");
    expect(all).toContain(".zshrc");
    expect(all).toContain("이전에 중단된 초기화 흔적");
    expect(all).toContain("journal.ndjson");
  });

  it("주요 항목은 크기 내림차순 상위 N건 + 나머지 접기", () => {
    const items = Array.from({ length: 10 }, (_, i) => ({
      path: `/p/${i}`,
      label: "x",
      size_bytes: i * 1000,
    }));
    const top = resetTopItems(preview({ items }), 3);
    expect(top.map((t) => t.path)).toEqual(["/p/9", "/p/8", "/p/7"]);
    expect(resetNoticeLines(preview({ items })).join("\n")).toContain("…외 4건");
  });
});

describe("resetResultTitle / resetResultBody", () => {
  it("★P0-4: 부분 실패는 제목부터 '부분 완료'이고 실패 경로가 본문 정면에 온다", () => {
    const rep = {
      ok: false,
      moved: 31,
      skipped_absent: 2,
      trash_dir: "/t/factory-reset-X",
      failed: [{ path: "/dummy-home/.claude/settings.json", error: "is a symlink — refusing" }],
      report_path: "/t/factory-reset-X/REPORT.txt",
    };
    expect(resetResultTitle(rep)).toContain("부분 완료");
    const body = resetResultBody(rep);
    expect(body).toContain("정리되지 않은 항목");
    expect(body).toContain(".claude/settings.json");
    expect(body).toContain("REPORT.txt"); // 화면이 사라져도 남는 증거
    expect(body).toContain("--undo");
    // 예고와 완료 건수 차이를 분해해 설명한다.
    expect(body).toContain("이동 31건");
    expect(body).toContain("이미 없음 2건");
  });

  it("성공이면 '완료'이고, 데몬 부활 경고가 있으면 부분 완료로 승격된다", () => {
    expect(resetResultTitle({ ok: true, moved: 34 })).toBe("완전 초기화 완료");
    expect(resetResultTitle({ ok: true, moved: 34, revived_warning: "부활" })).toContain("부분 완료");
  });

  it("복구 지도 기록 실패는 침묵하지 않는다", () => {
    const body = resetResultBody({ ok: true, moved: 3, manifest_written: false });
    expect(body).toContain("journal.ndjson");
  });

  it("이연 안내는 실제로 가능한 행동을 지시한다(껐다 켜면 알아서 되는 것처럼 말하지 않는다)", () => {
    const body = resetResultBody({
      ok: true,
      moved: 3,
      deferred: [{ path: "/L/com.cysjavis.terminal", error: "in use" }],
    });
    expect(body).toContain("cys factory-reset");
    expect(body).not.toContain("종료 후 다시 실행하면 정리됩니다");
  });
});
