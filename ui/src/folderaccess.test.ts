// folderaccess.ts 순수 로직 + 이름 SOT 파리티 핀 (bun test — 신규 의존성 0).
//
// ★(0.14.41 · U14/U18) 무엇을 고정하나
//   ① 목록에 실제로 보이는 이름은 「cys」 하나다(tccd 로그 164/164 · 반박 재확인) — 그 이름은
//      tauri.conf.json productName 에서 온다. 문구가 이름을 하드코딩하지 않고 SOT 한 곳을 쓴다.
//   ② 토스트는 **두 문장**(무엇이 막혔나 + 누르면 설정 열림). 전체 디스크 접근 ＋추가 권고는
//      토스트에서 뺀다(자율 에이전트 앱에 과한 권한 · 거의 닿지 않는 경우 — 반박 D2). 상세는 매뉴얼.
//   ③ 모르는 폴더를 "데스크탑"으로 부르지 않는다(조사 R4 회귀 핀).
//   ④ 좌석 막힘 안내(U18)는 폴더별 1장 · 같은 좌석을 3초마다 다시 알리지 않는다.
import { describe, it, expect } from "bun:test";
import { readFileSync } from "node:fs";
import {
  PRIVACY_APP_NAME,
  SIGNER_DISPLAY_NAME,
  PERM_TOAST_PREFIX,
  folderLabel,
  permWarningToast,
  cwdBlockedNotices,
  loginItemsGuide,
  isMacFolderPermissionError,
} from "./folderaccess";

describe("이름 SOT — 목록 이름은 번들 표시 이름(productName) 하나", () => {
  it("PRIVACY_APP_NAME == tauri.conf.json productName (측정 불능은 실패)", () => {
    const conf = JSON.parse(
      readFileSync(new URL("../../src-tauri/tauri.conf.json", import.meta.url), "utf-8"),
    ) as { productName?: string };
    expect(typeof conf.productName).toBe("string");
    expect(PRIVACY_APP_NAME).toBe(conf.productName as string);
  });
  it("앱 이름을 바꾸면 문구가 따라간다(하드코딩 금지 핀)", () => {
    const t = permWarningToast("Desktop", "cys 2");
    expect(t).not.toBeNull();
    expect(t!.detail).toContain("「cys 2」");
    expect(t!.detail).not.toContain("「cys」");
  });
});

describe("folderLabel — 명시 매핑 · 모르는 값은 null", () => {
  it("Desktop·Documents·Downloads", () => {
    expect(folderLabel("Desktop")).toBe("데스크탑");
    expect(folderLabel("Documents")).toBe("문서");
    expect(folderLabel("Downloads")).toBe("다운로드");
  });
  it("모르는 값·비문자열은 null(거짓 '데스크탑' 금지)", () => {
    expect(folderLabel("/Volumes/X")).toBeNull();
    expect(folderLabel("Pictures")).toBeNull();
    expect(folderLabel(undefined)).toBeNull();
    expect(folderLabel(3)).toBeNull();
  });
});

describe("permWarningToast — GUI 자체 점검(앱 기동 시 데스크탑·문서 읽기) 안내", () => {
  it("두 문장: 무엇이 막혔나 + 누르면 설정이 열린다", () => {
    const t = permWarningToast("Desktop")!;
    expect(t.id).toBe(`${PERM_TOAST_PREFIX}Desktop`);
    expect(t.target).toBe("files");
    expect(t.title).toContain("데스크탑");
    expect(t.detail).toContain("「파일 및 폴더」");
    expect(t.detail).toContain(`「${PRIVACY_APP_NAME}」`);
    expect(t.detail).toContain("데스크탑");
    expect(t.detail).toContain("누르면");
    // 문장 수 = 마침표로 끝나는 문장 2개
    expect(t.detail.split(/[.]\s*/).filter((x) => x.trim()).length).toBe(2);
  });
  it("문서 폴더", () => {
    const t = permWarningToast("Documents")!;
    expect(t.id).toBe(`${PERM_TOAST_PREFIX}Documents`);
    expect(t.title).toContain("문서");
    expect(t.detail).toContain("문서");
  });
  it("전체 디스크 접근 ＋추가 권고·개발 용어(EPERM)·'cysd' 보험 문구는 토스트에 없다", () => {
    for (const f of ["Desktop", "Documents"]) {
      const t = permWarningToast(f)!;
      const all = t.title + t.detail;
      expect(all).not.toContain("전체 디스크");
      expect(all).not.toContain("＋");
      expect(all).not.toContain("EPERM");
      expect(all).not.toContain("cysd");
    }
  });
  it("모르는 폴더는 그 이름을 그대로 쓰고 '데스크탑'이라 부르지 않는다(R4 회귀 핀)", () => {
    const t = permWarningToast("/Volumes/X")!;
    expect(t.title + t.detail).toContain("/Volumes/X");
    expect(t.title + t.detail).not.toContain("데스크탑");
  });
  it("폴더 값이 없거나 문자열이 아니면 null(렌더하지 않는다)", () => {
    expect(permWarningToast(undefined)).toBeNull();
    expect(permWarningToast("")).toBeNull();
    expect(permWarningToast({})).toBeNull();
  });
});

describe("cwdBlockedNotices — 좌석 작업 폴더 막힘(U18) 폴더별 1장 · 재알림 억제", () => {
  const desk = { path: "/Users/u/Desktop/proj", folder: "Desktop" };
  it("같은 폴더의 좌석들은 한 장으로 묶이고 역할이 나열된다", () => {
    const r = cwdBlockedNotices(new Set(), [
      { scope: "hq", role: "worker-2", cwd_blocked: desk },
      { scope: "hq", role: "worker-3", cwd_blocked: { path: "/Users/u/Desktop/other", folder: "Desktop" } },
      { scope: "hq", role: "cso", cwd_blocked: null },
    ]);
    expect(r.notices.length).toBe(1);
    const n = r.notices[0];
    expect(n.id).toBe(`${PERM_TOAST_PREFIX}seat-Desktop`);
    expect(n.target).toBe("files");
    expect(n.detail).toContain("worker-2");
    expect(n.detail).toContain("worker-3");
    expect(n.detail).toContain("데스크탑");
    expect(n.detail).toContain(`「${PRIVACY_APP_NAME}」`);
    expect(n.detail).toContain("누르면");
    expect(n.detail).not.toContain("전체 디스크");
  });
  it("이미 알린 좌석은 다음 틱에 다시 알리지 않는다(3초 루프 소음·TTL 무한 연장 방지)", () => {
    const first = cwdBlockedNotices(new Set(), [{ scope: "hq", role: "worker-2", cwd_blocked: desk }]);
    const again = cwdBlockedNotices(first.seen, [{ scope: "hq", role: "worker-2", cwd_blocked: desk }]);
    expect(again.notices.length).toBe(0);
    // 같은 폴더에 새 좌석이 오면 그 폴더 한 장을 갱신한다(두 역할 모두 나열).
    const more = cwdBlockedNotices(first.seen, [
      { scope: "hq", role: "worker-2", cwd_blocked: desk },
      { scope: "hq", role: "worker-4", cwd_blocked: desk },
    ]);
    expect(more.notices.length).toBe(1);
    expect(more.notices[0].detail).toContain("worker-2");
    expect(more.notices[0].detail).toContain("worker-4");
  });
  it("모르는 위치는 경로를 그대로 쓰고 '데스크탑'이라 부르지 않는다", () => {
    const r = cwdBlockedNotices(new Set(), [
      { scope: "hq", role: "worker", cwd_blocked: { path: "/Volumes/Ext/job", folder: null } },
    ]);
    expect(r.notices.length).toBe(1);
    expect(r.notices[0].title + r.notices[0].detail).toContain("/Volumes/Ext/job");
    expect(r.notices[0].title + r.notices[0].detail).not.toContain("데스크탑");
  });
  it("역할 없는 항목·형식이 깨진 값은 건너뛴다(throw 0)", () => {
    const r = cwdBlockedNotices(new Set(), [
      { scope: "hq", role: null, cwd_blocked: desk },
      { scope: "hq", role: "w", cwd_blocked: "x" },
      { scope: "hq", role: "w", cwd_blocked: { folder: "Desktop" } },
      { scope: "hq", role: "w", cwd_blocked: { path: 3 } },
    ]);
    expect(r.notices.length).toBe(0);
  });
  it("부서가 달라도(같은 역할명) 각각 좌석으로 센다", () => {
    const r = cwdBlockedNotices(new Set(), [
      { scope: "hq", role: "master", cwd_blocked: desk },
      { scope: "dept-1", role: "master", cwd_blocked: desk },
    ]);
    expect(r.seen.size).toBe(2);
    expect(r.notices.length).toBe(1);
  });
  it("입력 집합을 변형하지 않는다(순수)", () => {
    const seen = new Set<string>();
    cwdBlockedNotices(seen, [{ scope: "hq", role: "worker-2", cwd_blocked: desk }]);
    expect(seen.size).toBe(0);
  });
});

describe("loginItemsGuide — 로그인 항목 목록에 실제로 보이는 두 줄", () => {
  it("「cys」와 개발자 이름 줄을 모두 말한다", () => {
    const g = loginItemsGuide();
    expect(g).toContain(`「${PRIVACY_APP_NAME}」`);
    expect(g).toContain(SIGNER_DISPLAY_NAME);
    expect(g).toContain("백그라운드에서 허용");
    expect(g).not.toContain("관련 항목");
  });
  it("Rust 데몬 실패 문구도 같은 두 이름을 쓴다(교차 파리티 · 측정 불능은 실패)", () => {
    const rs = readFileSync(new URL("../../src-tauri/src/main.rs", import.meta.url), "utf-8");
    const i = rs.indexOf("데몬을 시작하지 못했습니다");
    expect(i).toBeGreaterThan(0);
    const line = rs.slice(i, rs.indexOf("\n", i));
    expect(line).toContain(`「${PRIVACY_APP_NAME}」`);
    expect(line).toContain(SIGNER_DISPLAY_NAME);
  });
});

describe("isMacFolderPermissionError — EPERM(1)만 · EACCES(13)는 아니다", () => {
  it("경계", () => {
    expect(isMacFolderPermissionError("Operation not permitted (os error 1)")).toBe(true);
    expect(isMacFolderPermissionError(new Error("Operation not permitted (os error 1)"))).toBe(true);
    expect(isMacFolderPermissionError("Permission denied (os error 13)")).toBe(false);
    expect(isMacFolderPermissionError("No such file or directory (os error 2)")).toBe(false);
    expect(isMacFolderPermissionError("Access is denied. (os error 5)")).toBe(false);
    expect(isMacFolderPermissionError(undefined)).toBe(false);
  });
});
