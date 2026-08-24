// clipath.ts 순수 함수 회귀 테스트 (bun test — 신규 의존성 0).
//
// 이 버튼이 한 번 제거됐던 이유(결함 4종)를 테스트로 못박는다:
//   ① 플랫폼 분기가 없어 Windows/Linux에서도 보였다 → macOS 양성 판정만 통과.
//   ② 그림자화·검증 실패인데 "설치 완료" 성공 토스트가 떴다 → 등급을 낮춘다.
//   ③ 해제 경로가 없었다 → 라벨·확인 문구·결과 등급.
//   ④ status 미상을 성공으로 접었다 → 모르면 unverified(측정 불능은 통과가 아니다).
import { describe, it, expect } from "bun:test";
import {
  isMacUserAgent,
  normalizeInstallStatus,
  installResultToast,
  readInstallState,
  cliButtonView,
  uninstallConfirmText,
  uninstallResultToast,
} from "./clipath";

const MAC_UA =
  "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15";
const WIN_UA =
  "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36";
const LINUX_UA =
  "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36";

describe("isMacUserAgent — macOS 양성 판정만 버튼을 연다", () => {
  it("macOS UA는 참", () => {
    expect(isMacUserAgent(MAC_UA)).toBe(true);
  });
  it("Windows UA는 거짓", () => {
    expect(isMacUserAgent(WIN_UA)).toBe(false);
  });
  it("Linux UA는 거짓 — !IS_WINDOWS 로 대체하면 재현되던 결함", () => {
    expect(isMacUserAgent(LINUX_UA)).toBe(false);
  });
  it("iPhone/iPad('like Mac OS X')는 거짓", () => {
    expect(
      isMacUserAgent("Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15"),
    ).toBe(false);
  });
  it("빈 문자열·미상 UA는 거짓(모르면 열지 않는다)", () => {
    expect(isMacUserAgent("")).toBe(false);
  });
});

describe("normalizeInstallStatus — 모르는 값은 전부 unverified", () => {
  it("계약 3값은 그대로", () => {
    expect(normalizeInstallStatus("installed")).toBe("installed");
    expect(normalizeInstallStatus("installed_shadowed")).toBe("installed_shadowed");
    expect(normalizeInstallStatus("unverified")).toBe("unverified");
  });
  it("undefined(구버전 백엔드·필드 없음)는 unverified — 성공으로 둔갑 금지", () => {
    expect(normalizeInstallStatus(undefined)).toBe("unverified");
  });
  it("오타·타입 불일치도 unverified", () => {
    expect(normalizeInstallStatus("ok")).toBe("unverified");
    expect(normalizeInstallStatus(true)).toBe("unverified");
    expect(normalizeInstallStatus(null)).toBe("unverified");
  });
});

describe("installResultToast — 등급 분리(installed 만 성공)", () => {
  const base = { cys_link: "/usr/local/bin/cys", cysd_link: "/usr/local/bin/cysd" };

  it("installed → system 등급 + ✅ 접두", () => {
    const t = installResultToast({ ...base, status: "installed" });
    expect(t.category).toBe("system");
    expect(t.title).toContain("✅");
    expect(t.body).toContain("/usr/local/bin/cys");
  });

  it("installed_shadowed → watchdog 등급 + ⚠ 접두 + 가리는 경로 명시", () => {
    const t = installResultToast({
      ...base,
      status: "installed_shadowed",
      shadowed_by: "/opt/homebrew/bin/cys",
    });
    expect(t.category).toBe("watchdog");
    expect(t.title).toContain("⚠");
    expect(t.body).toContain("/opt/homebrew/bin/cys");
    expect(t.body).toContain("which -a cys"); // 다음 조치가 문장으로 있다
    expect(t.title).toContain("미완료"); // 제거 사유였던 "설치 완료" 오보고의 반대말
  });

  it("unverified → watchdog 등급 + '확인 불가' + 직접 확인 안내", () => {
    const t = installResultToast({ ...base, status: "unverified" });
    expect(t.category).toBe("watchdog");
    expect(t.title).toContain("확인 불가");
    expect(t.body).toContain("which -a cys");
  });

  it("status 필드가 아예 없으면 성공이 아니라 unverified 문구", () => {
    const t = installResultToast({ ...base, ok: true });
    expect(t.category).toBe("watchdog");
    expect(t.title).not.toContain("✅");
  });

  it("세 등급의 본문이 서로 다르다(등급 분리의 핵심)", () => {
    const a = installResultToast({ ...base, status: "installed" }).body;
    const b = installResultToast({ ...base, status: "installed_shadowed" }).body;
    const c = installResultToast({ ...base, status: "unverified" }).body;
    expect(new Set([a, b, c]).size).toBe(3);
  });

  it("warnings 는 등급과 무관하게 본문 말미에 붙는다", () => {
    const t = installResultToast({ ...base, status: "installed", warnings: ["표준 위치가 아닙니다"] });
    expect(t.body).toContain("표준 위치가 아닙니다");
  });
});

describe("readInstallState — 상태 조회 응답의 관용 판독", () => {
  it("installed:true/false 를 1순위로 읽는다", () => {
    expect(readInstallState({ installed: true })).toBe("installed");
    expect(readInstallState({ installed: false })).toBe("absent");
  });
  it("경로별 플래그(cys_installed)도 받는다 — 해제 버튼이 조용히 도달 불가가 되지 않게", () => {
    expect(readInstallState({ cys_installed: true })).toBe("installed");
    expect(readInstallState({ cys_installed: false })).toBe("absent");
  });
  it("status enum 으로 와도 받는다", () => {
    expect(readInstallState({ status: "installed_shadowed" })).toBe("installed");
    expect(readInstallState({ status: "not_installed" })).toBe("absent");
  });
  it("null·미상 응답은 unknown(해제 쪽으로 넘어가지 않는다)", () => {
    expect(readInstallState(null)).toBe("unknown");
    expect(readInstallState({})).toBe("unknown");
    expect(readInstallState("installed")).toBe("unknown");
  });
});

describe("cliButtonView — 라벨과 툴팁을 함께 산출", () => {
  it("설치됨이면 해제 라벨", () => {
    const v = cliButtonView("installed");
    expect(v.label).toBe("셸 cys 해제");
    expect(v.title).toContain("제거");
  });
  it("미설치면 설치 라벨(=index.html 초기값과 동일)", () => {
    expect(cliButtonView("absent").label).toBe("셸에 cys 설치");
  });
  it("unknown 은 설치 쪽 — 비가역 해제로 기울지 않는다", () => {
    const v = cliButtonView("unknown");
    expect(v.label).toBe("셸에 cys 설치");
    expect(v.title).toContain("확인하지 못했");
  });
  it("라벨이 바뀌면 툴팁도 함께 바뀐다(설치 안내가 해제 버튼에 남지 않는다)", () => {
    expect(cliButtonView("installed").title).not.toBe(cliButtonView("absent").title);
  });
});

describe("uninstallConfirmText — 집행 전 확인 문구", () => {
  it("무엇을 지우고 무엇을 건너뛰는지 명시한다", () => {
    const c = uninstallConfirmText();
    expect(c.body).toContain("/usr/local/bin/cys");
    expect(c.body).toContain("건너뜁니다");
    expect(c.yes).toBe("해제");
    expect(c.no).toBe("취소");
  });
  it("되돌리는 방법을 알려 준다(같은 버튼으로 재설치)", () => {
    expect(uninstallConfirmText().body).toContain("다시 필요하면");
  });
});

describe("uninstallResultToast — 부분 실패를 성공으로 감추지 않는다", () => {
  it("전부 제거 → system 등급", () => {
    const t = uninstallResultToast({ removed: ["/usr/local/bin/cys", "/usr/local/bin/cysd"] });
    expect(t.category).toBe("system");
    expect(t.title).toContain("✅");
  });
  it("건너뛴 항목이 있으면 watchdog + 사유 노출", () => {
    const t = uninstallResultToast({
      removed: ["/usr/local/bin/cys"],
      skipped: [{ path: "/usr/local/bin/cysd", reason: "심볼릭 링크가 아닌 일반 파일" }],
    });
    expect(t.category).toBe("watchdog");
    expect(t.body).toContain("/usr/local/bin/cysd");
    expect(t.body).toContain("일반 파일");
  });
  it("지운 것도 건너뛴 것도 없으면 '해제할 심링크 없음'", () => {
    const t = uninstallResultToast({});
    expect(t.category).toBe("watchdog");
    expect(t.title).toContain("없음");
  });
  it("제거 0건 + 건너뜀만 있으면 완료라고 말하지 않는다", () => {
    const t = uninstallResultToast({ skipped: [{ path: "/usr/local/bin/cys", reason: "다른 앱을 가리킴" }] });
    expect(t.category).toBe("watchdog");
    expect(t.body).toContain("제거한 항목 없음");
  });
});
