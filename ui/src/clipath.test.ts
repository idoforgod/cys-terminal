// clipath.ts 순수 함수 회귀 테스트 (bun test — 신규 의존성 0).
//
// 이 버튼이 한 번 제거됐던 이유(결함 4종)를 테스트로 못박는다:
//   ① 플랫폼 분기가 없어 Windows/Linux에서도 보였다 → macOS 양성 판정만 통과.
//   ② 그림자화·검증 실패인데 "설치 완료" 성공 토스트가 떴다 → 등급을 낮춘다.
//   ③ 해제 경로가 없었다 → 라벨·확인 문구·결과 등급.
//   ④ status 미상을 성공으로 접었다 → 모르면 unverified(측정 불능은 통과가 아니다).
//
// ★2026-08-25 계약 드리프트 수리(MAJOR-5). 이 파일이 **초록인 채로 결함을 봉인하고 있었다**:
//   픽스처가 Rust 실물이 아니라 잘못된 TS 모양(skipped: {path,reason}[])을 먹였기 때문에,
//   "모든 skip 이 소멸하고 부분 실패가 성공으로 둔갑한다"는 실제 결함을 한 번도 밟지 않았다.
//   그래서 지금은 픽스처를 **Rust 실물 모양 하나**로 통일하고, 그 픽스처가 계약과 같은 모양인지를
//   먼저 검사한다(아래 "계약 드리프트 가드"). 픽스처가 계약을 벗어나면 그 테스트가 먼저 빨개진다.
import { describe, it, expect } from "bun:test";
import { readFileSync } from "node:fs";
import {
  isMacUserAgent,
  normalizeInstallStatus,
  unverifiedCause,
  toastClassName,
  toastEmitPlan,
  NONINTERACTIVE_PROBE_NOTE,
  LOGIN_SHELL_PATH_FILES,
  installResultToast,
  readInstallReport,
  readUninstallReport,
  readCliStatus,
  readInstallState,
  cliButtonView,
  statusNoticePlan,
  uninstallConfirmText,
  uninstallResultToast,
  isBenignSkip,
  FOREIGN_BACKUP_NOTICE,
  INSTALL_TOAST_ID,
  UNINSTALL_TOAST_ID,
  CLI_NOTES_TOAST_ID,
  type InstallCliReport,
  type UninstallCliReport,
  type CliInstallStatusReport,
} from "./clipath";

// ══════════════════════════════════════════════════════════════════════════════
// 계약 드리프트 가드 — src-tauri/src/main.rs 의 #[derive(serde::Serialize)] 구조체 사본
// ══════════════════════════════════════════════════════════════════════════════
// serde rename 이 없으므로 필드명은 Rust 그대로(snake_case)이고, Vec<String> → string[],
// Option<String> → string | null 이다. 이 표가 곧 "Rust 가 실제로 보내는 것"의 선언이며,
// 아래 픽스처는 전부 이 표를 만족해야 한다(만족하지 못하면 테스트가 먼저 죽는다).
const RUST_INSTALL_REPORT: Record<string, string> = {
  ok: "boolean",
  status: "string",
  target_dir: "string",
  cys_link: "string",
  cysd_link: "string",
  source_cys: "string",
  effective_cys: "string|null",
  shadowed_by: "string|null",
  // ★2026-08-25 계약 확장(N3): unverified 두 갈래의 기계 판별자.
  // status=="unverified" 일 때만 "not_on_path"|"probe_failed", 그 외에는 null.
  unverified_reason: "string|null",
  warnings: "string[]",
};
const RUST_UNINSTALL_REPORT: Record<string, string> = {
  ok: "boolean",
  removed: "string[]",
  skipped: "string[]", // ★문자열이다("경로 — 사유"). 객체 배열로 읽던 것이 MAJOR-5 결함의 본체.
  warnings: "string[]",
};
const RUST_STATUS_REPORT: Record<string, string> = {
  platform_supported: "boolean",
  installed: "boolean",
  state: "string",
  cys_link: "string",
  cysd_link: "string",
  notes: "string[]",
};

function tagOf(v: unknown): string {
  if (v === null) return "null";
  if (Array.isArray(v)) return v.every((x) => typeof x === "string") ? "string[]" : "unknown[]";
  return typeof v;
}

/** 객체가 계약 표와 **정확히 같은 필드 집합·타입**인지 단언(부족·잉여·타입 불일치 전부 실패). */
function expectShape(obj: Record<string, unknown>, spec: Record<string, string>) {
  expect(Object.keys(obj).sort()).toEqual(Object.keys(spec).sort());
  for (const [field, allowed] of Object.entries(spec)) {
    const t = tagOf(obj[field]);
    const ok = allowed.split("|").includes(t);
    expect({ field, type: ok ? allowed : t }).toEqual({ field, type: allowed });
  }
}

// ── Rust 실물 모양 픽스처(유일한 픽스처원) ─────────────────
// 실제 응답 예시를 그대로 옮긴다. 부분 객체를 만들지 않는다 — Rust 는 늘 전 필드를 보내므로
// 부분 객체를 먹이는 순간 테스트는 '있을 수 없는 입력'을 검사하게 되고, 그게 드리프트의 은신처다.
function installReport(over: Partial<InstallCliReport> = {}): InstallCliReport {
  return {
    ok: true,
    status: "installed",
    target_dir: "/usr/local/bin",
    cys_link: "/usr/local/bin/cys",
    cysd_link: "/usr/local/bin/cysd",
    source_cys: "/Applications/cys.app/Contents/MacOS/cys",
    effective_cys: "/usr/local/bin/cys",
    shadowed_by: null,
    unverified_reason: null, // installed 이므로 None — 두 갈래 판별자는 unverified 에만 붙는다
    warnings: [],
    ...over,
  };
}
function uninstallReport(over: Partial<UninstallCliReport> = {}): UninstallCliReport {
  return { ok: true, removed: [], skipped: [], warnings: [], ...over };
}
function statusReport(over: Partial<CliInstallStatusReport> = {}): CliInstallStatusReport {
  return {
    platform_supported: true,
    installed: false,
    state: "absent",
    cys_link: "/usr/local/bin/cys",
    cysd_link: "/usr/local/bin/cysd",
    notes: [],
    ...over,
  };
}

// Rust plan_cli_uninstall / cli_install_status 가 실제로 만드는 문장(main.rs 사본).
const SKIP_NOT_SYMLINK =
  "/usr/local/bin/cysd — 심볼릭이 아니라 실제 파일입니다. 다른 도구가 설치한 것일 수 있어 건드리지 않았습니다.";
const SKIP_FOREIGN =
  "/usr/local/bin/cys — cys.app 번들이 아닌 곳(/opt/other/cys)을 가리키는 링크라 건드리지 않았습니다.";
const SKIP_ABSENT = "/usr/local/bin/cysd — 없음(이미 해제된 상태)";
const WARN_LEFTOVER =
  "/usr/local/bin/cys 가 아직 남아 있습니다 — 터미널에서 'sudo rm /usr/local/bin/cys' 로 지우세요.";
const NOTE_NOT_SYMLINK =
  "/usr/local/bin/cys — 심볼릭이 아닌 실제 파일이 이미 있습니다(다른 도구 설치본일 수 있어 자동으로 제거하지 않습니다).";
// ★2026-08-25(N3): 아래 셋은 Rust 가 **실제로 보내는 문장**의 사본이다(main.rs
// classify_install_status · install_cli_to_path 의 백업 재관측). 예전 픽스처는 이 실물이 아니라
// TS 정규식에 맞춰 지어낸 문장이었고, 그래서 "TS 가 Rust 문구를 제대로 가른다"를 한 번도 검사하지
// 못했다. 지금은 문구가 **판정에 쓰이지 않는다** — 이 상수들은 오직
//   ① 본문 말미(tail)에 원문 그대로 노출되는지
//   ② 문구를 어떻게 섞어도 분기가 흔들리지 않는지
// 를 검사하는 데만 쓴다. 그래서 Rust 가 문구를 다듬어도 이 파일은 빨개지지 않는다(그것이 목적이다).
const WARN_NOT_ON_PATH =
  "PATH 확인 결과: 검증 명령은 정상 실행됐지만 로그인 셸(zsh) PATH에서 cys를 찾지 못했습니다(PATH에 /usr/local/bin/cys의 폴더가 없을 수 있습니다). 새 터미널을 열어 'which -a cys'로 확인하세요.";
const WARN_PROBE_FAILED =
  "PATH 확인 실패: 심볼릭은 만들었지만 로그인 셸(zsh)로 'which -a cys'를 실행하지 못했습니다: zsh 타임아웃(5초 초과). 새 터미널에서 'which -a cys'로 직접 확인하세요.";
/// 같은 warnings 배열에 **합류하는 남의 문장** — 예전 정규식은 이런 문장까지 함께 읽어 판정했다.
const WARN_BACKUP =
  "/usr/local/bin/cys에 심볼릭이 아닌 파일이 있어 지우지 않고 /usr/local/bin/cys.cys-backup-20260825-101112로 백업한 뒤 링크를 만들었습니다. 되돌리려면 'sudo mv /usr/local/bin/cys.cys-backup-20260825-101112 /usr/local/bin/cys', 필요 없으면 'sudo rm /usr/local/bin/cys.cys-backup-20260825-101112' 를 실행하세요.";

describe("계약 드리프트 가드 — 픽스처가 Rust 실물과 같은 모양인가", () => {
  it("InstallCliReport 픽스처는 계약 필드 집합·타입과 일치", () => {
    expectShape(installReport(), RUST_INSTALL_REPORT);
    expectShape(installReport({ effective_cys: null, shadowed_by: "/opt/homebrew/bin/cys" }), RUST_INSTALL_REPORT);
  });
  it("UninstallCliReport 픽스처는 계약과 일치 — skipped 는 문자열 배열이다", () => {
    expectShape(uninstallReport(), RUST_UNINSTALL_REPORT);
    expectShape(uninstallReport({ skipped: [SKIP_FOREIGN], warnings: [WARN_LEFTOVER] }), RUST_UNINSTALL_REPORT);
    // 예전 TS 가 상상하던 모양({path,reason}[])은 계약 위반으로 잡힌다 — 봉인이 깨졌는지의 증명.
    expect(() =>
      expectShape(
        { ok: true, removed: [], skipped: [{ path: "/usr/local/bin/cys", reason: "x" }], warnings: [] },
        RUST_UNINSTALL_REPORT,
      ),
    ).toThrow();
  });
  it("CliInstallStatusReport 픽스처는 계약과 일치 — notes 필드가 실재한다", () => {
    expectShape(statusReport(), RUST_STATUS_REPORT);
    expectShape(statusReport({ installed: true, state: "ours" }), RUST_STATUS_REPORT);
  });
  it("판독기 출력도 계약 모양을 유지한다(응답이 쓰레기여도)", () => {
    expectShape(readInstallReport(installReport()), RUST_INSTALL_REPORT);
    expectShape(readInstallReport(null), RUST_INSTALL_REPORT);
    expectShape(readUninstallReport(uninstallReport()), RUST_UNINSTALL_REPORT);
    expectShape(readUninstallReport("garbage"), RUST_UNINSTALL_REPORT);
  });
});

describe("판독기 — 응답을 있는 그대로 옮기되 미상은 안전한 쪽으로", () => {
  it("정상 응답은 값이 보존된다", () => {
    const r = readInstallReport(installReport({ shadowed_by: "/opt/homebrew/bin/cys", status: "installed_shadowed" }));
    expect(r.shadowed_by).toBe("/opt/homebrew/bin/cys");
    expect(r.status).toBe("installed_shadowed");
  });
  it("응답 없음·타입 불일치는 unverified + ok:false — 성공으로 둔갑하지 않는다", () => {
    expect(readInstallReport(undefined).status).toBe("unverified");
    expect(readInstallReport(undefined).ok).toBe(false);
    expect(readUninstallReport(undefined).ok).toBe(false);
  });
  it("해제 응답의 skipped·warnings 문자열이 그대로 살아남는다(예전엔 통째로 소실)", () => {
    const r = readUninstallReport(uninstallReport({ skipped: [SKIP_FOREIGN], warnings: [WARN_LEFTOVER] }));
    expect(r.skipped).toEqual([SKIP_FOREIGN]);
    expect(r.warnings).toEqual([WARN_LEFTOVER]);
  });
});

describe("isMacUserAgent — macOS 양성 판정만 버튼을 연다", () => {
  const MAC_UA =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15";
  const WIN_UA =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36";
  const LINUX_UA =
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36";

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
  it("undefined(필드 없음)는 unverified — 성공으로 둔갑 금지", () => {
    expect(normalizeInstallStatus(undefined)).toBe("unverified");
  });
  it("오타·타입 불일치도 unverified", () => {
    expect(normalizeInstallStatus("ok")).toBe("unverified");
    expect(normalizeInstallStatus(true)).toBe("unverified");
    expect(normalizeInstallStatus(null)).toBe("unverified");
  });
});

describe("unverifiedCause — unverified 두 갈래를 **기계 필드**로만 가른다(N3 · MINOR-9)", () => {
  it("계약값 둘은 그대로 통과한다", () => {
    expect(unverifiedCause("not_on_path")).toBe("not_on_path");
    expect(unverifiedCause("probe_failed")).toBe("probe_failed");
  });
  it("필드가 없으면(구 백엔드) unknown — 원인 불명으로 안전하게 접는다", () => {
    expect(unverifiedCause(null)).toBe("unknown");
    expect(unverifiedCause(undefined)).toBe("unknown");
  });
  it("계약 밖 값·오타·대소문자 변형은 전부 unknown — 추측하지 않는다", () => {
    expect(unverifiedCause("")).toBe("unknown");
    expect(unverifiedCause("NOT_ON_PATH")).toBe("unknown");
    expect(unverifiedCause("not-on-path")).toBe("unknown"); // 케밥은 계약이 아니다
    expect(unverifiedCause("probefailed")).toBe("unknown");
  });
  it("★경고 문장은 판별자가 아니다 — 문구를 통째로 넘겨도 unknown", () => {
    // 예전 구현이라면 이 두 줄은 not-on-path / probe-failed 로 갈렸다.
    expect(unverifiedCause(WARN_NOT_ON_PATH)).toBe("unknown");
    expect(unverifiedCause(WARN_PROBE_FAILED)).toBe("unknown");
  });
});

describe("installResultToast — 등급 분리(installed 만 성공)", () => {
  it("installed → system 등급 + ✅ 접두 + volatile(할 일이 없다)", () => {
    const t = installResultToast(installReport());
    expect(t.category).toBe("system");
    expect(t.title).toContain("✅");
    expect(t.body).toContain("/usr/local/bin/cys");
    expect(t.sticky).toBe(false);
    expect(t.id).toBe(INSTALL_TOAST_ID);
  });

  it("installed 라도 warnings 가 있으면 sticky — 백업 경로 통보가 8초에 사라지면 안 된다(BLOCK-1(c))", () => {
    const backup = "/usr/local/bin/cys 에 있던 파일을 /usr/local/bin/cys.cys-backup-20260825-101112 로 옮겼습니다.";
    const t = installResultToast(installReport({ warnings: [backup] }));
    expect(t.category).toBe("system");
    expect(t.body).toContain("cys.cys-backup-20260825-101112");
    expect(t.sticky).toBe(true);
  });

  it("installed_shadowed → watchdog + ⚠ + 가리는 경로 명시 + sticky", () => {
    const t = installResultToast(
      installReport({
        ok: false,
        status: "installed_shadowed",
        effective_cys: "/opt/homebrew/bin/cys",
        shadowed_by: "/opt/homebrew/bin/cys",
      }),
    );
    expect(t.category).toBe("watchdog");
    expect(t.title).toContain("⚠");
    expect(t.body).toContain("/opt/homebrew/bin/cys");
    expect(t.body).toContain("which -a cys"); // 다음 조치가 문장으로 있다
    expect(t.title).toContain("미완료"); // 제거 사유였던 "설치 완료" 오보고의 반대말
    expect(t.sticky).toBe(true);
  });

  it("unverified(probe_failed) → '확인 불가' + 확인 명령이 실패했다고만 말한다", () => {
    const t = installResultToast(
      installReport({
        ok: false,
        status: "unverified",
        effective_cys: null,
        unverified_reason: "probe_failed",
        warnings: [WARN_PROBE_FAILED],
      }),
    );
    expect(t.category).toBe("watchdog");
    expect(t.title).toContain("확인 불가");
    expect(t.body).toContain("which -a cys");
    expect(t.sticky).toBe(true);
  });

  it("unverified(not_on_path) → 원인을 PATH로 정확히 말한다 — '검증 명령 실패' 오단정 금지(MINOR-9)", () => {
    const t = installResultToast(
      installReport({
        ok: false,
        status: "unverified",
        effective_cys: null,
        unverified_reason: "not_on_path",
        warnings: [WARN_NOT_ON_PATH],
      }),
    );
    expect(t.title).toContain("PATH에서 cys를 찾지 못했");
    expect(t.body).toContain("/usr/local/bin 이 들어 있지 않을 수 있습니다");
    expect(t.body).not.toContain("확인 명령(which -a cys)이 실패");
    expect(t.sticky).toBe(true);
  });

  it("두 갈래의 문구가 서로 다르다(구분의 핵심)", () => {
    const a = installResultToast(installReport({ status: "unverified", unverified_reason: "not_on_path" }));
    const b = installResultToast(installReport({ status: "unverified", unverified_reason: "probe_failed" }));
    expect(a.title).not.toBe(b.title);
    expect(a.body).not.toBe(b.body);
  });

  it("판별자가 없으면(구 백엔드) 원인을 단정하지 않는다 — 둘 다 가능하다고 말한다", () => {
    const t = installResultToast(installReport({ ok: false, status: "unverified", unverified_reason: null }));
    expect(t.title).toContain("확인 불가");
    expect(t.body).toContain("단정하지 않습니다");
  });

  it("status 필드가 아예 없으면 성공이 아니라 unverified 문구(판독기 경유)", () => {
    const t = installResultToast(readInstallReport({ ok: true }));
    expect(t.category).toBe("watchdog");
    expect(t.title).not.toContain("✅");
  });

  it("세 등급의 본문이 서로 다르다(등급 분리의 핵심)", () => {
    const a = installResultToast(installReport({ status: "installed" })).body;
    const b = installResultToast(installReport({ status: "installed_shadowed" })).body;
    const c = installResultToast(installReport({ status: "unverified" })).body;
    expect(new Set([a, b, c]).size).toBe(3);
  });

  it("warnings 는 등급과 무관하게 본문 말미에 붙는다", () => {
    const t = installResultToast(installReport({ warnings: ["표준 위치가 아닙니다"] }));
    expect(t.body).toContain("표준 위치가 아닙니다");
  });
});

// ══════════════════════════════════════════════════════════════════════════════
// ★N3 / UNRESOLVED-1 — TS 는 warnings **산문**을 파싱하지 않는다(정규식 재도입 차단)
// ══════════════════════════════════════════════════════════════════════════════
// 이 블록의 목적은 하나다: 누가 편의로 문구 정규식을 되살리면 **여기가 먼저 빨개진다**.
// 근거(왜 산문이 계약이 될 수 없었나): Rust 는 "경고문 첫 구절(`PATH 확인 결과:`/`PATH 확인 실패:`)"
// 을 판별자로 선언했는데 TS 정규식은 문장 속 어절('찾지 못했'·'타임아웃')을 봤고, 같은 warnings
// 배열에 백업 통보문(WARN_BACKUP)까지 합류해 판정 대상 문자열이 오염됐다.
const CLIPATH_SRC = readFileSync(new URL("./clipath.ts", import.meta.url), "utf8");
/// 주석을 걷어낸 **코드만**. 옛 결함을 설명하는 주석("… '검증 명령 실패 또는 응답 없음' 으로
/// 단정해 …")까지 금칙어로 잡으면, 결함의 내력을 기록하지 못하게 만드는 잘못된 가드가 된다.
/// 우리가 막으려는 것은 문구를 다시 **읽는 코드**다. (전줄 주석만 걷는다 — 문자열 속 `//` 오탐 방지.)
const CLIPATH_CODE = CLIPATH_SRC.replace(/\/\*[\s\S]*?\*\//g, "").replace(/^[ \t]*\/\/.*$/gm, "");

describe("산문 파싱 금지 가드 — 분기의 유일 근거는 unverified_reason 필드다", () => {
  it("주석 제거가 코드를 먹지 않았다(가드가 빈 문자열을 검사하는 사고 방지)", () => {
    expect(CLIPATH_CODE).toContain("export function unverifiedCause");
    expect(CLIPATH_CODE).toContain("export function installResultToast");
  });

  it("삭제된 문구 정규식이 코드에 되살아나지 않았다", () => {
    for (const gone of [
      "NOT_ON_PATH_MARK",
      "PROBE_FAILED_MARK",
      "찾지 못했|not found",
      "timed? ?out",
      "상태 확인 실패",
      "응답 없음",
    ]) {
      expect({ 되살아난_패턴: gone, 코드에_존재: CLIPATH_CODE.includes(gone) }).toEqual({
        되살아난_패턴: gone,
        코드에_존재: false,
      });
    }
  });

  it("설치 판정 코드가 warnings 를 정규식·문자열 검색으로 읽지 않는다", () => {
    // installResultToast 본문에서 warnings 에 닿는 연산은 filter(Boolean)·join 둘뿐이어야 한다.
    const body = CLIPATH_CODE.slice(CLIPATH_CODE.indexOf("export function installResultToast"));
    const upto = body.slice(0, body.indexOf("\n}\n") + 3);
    for (const banned of [".test(", ".match(", ".includes(", ".search(", "RegExp"]) {
      expect({ 금지연산: banned, 사용됨: upto.includes(banned) }).toEqual({ 금지연산: banned, 사용됨: false });
    }
  });

  it("★판별자와 문구가 충돌하면 **판별자가 이긴다**(문구는 판정에 관여하지 않는다)", () => {
    // not_on_path 인데 warnings 는 probe 실패 문구만 담고 있다 → 그래도 not_on_path 문구가 나가야 한다.
    const a = installResultToast(
      installReport({ status: "unverified", unverified_reason: "not_on_path", warnings: [WARN_PROBE_FAILED] }),
    );
    expect(a.title).toContain("PATH에서 cys를 찾지 못했");
    // 반대 방향도 같다.
    const b = installResultToast(
      installReport({ status: "unverified", unverified_reason: "probe_failed", warnings: [WARN_NOT_ON_PATH] }),
    );
    expect(b.title).toContain("확인 불가");
  });

  it("★warnings 를 어떻게 섞어도 분기가 흔들리지 않는다 — 판별자 하나가 등급을 정한다", () => {
    const proseSets: string[][] = [
      [],
      [WARN_NOT_ON_PATH],
      [WARN_PROBE_FAILED],
      [WARN_BACKUP],
      [WARN_NOT_ON_PATH, WARN_PROBE_FAILED], // 예전 구현이 "unknown" 으로 떨어지던 조합
      [WARN_BACKUP, WARN_PROBE_FAILED], // 남의 문장이 합류해 판정 대상을 오염시키던 조합
    ];
    for (const reason of ["not_on_path", "probe_failed", null]) {
      const titles = new Set(
        proseSets.map(
          (w) =>
            installResultToast(installReport({ status: "unverified", unverified_reason: reason, warnings: w })).title,
        ),
      );
      expect({ reason, 서로다른_제목수: titles.size }).toEqual({ reason, 서로다른_제목수: 1 });
    }
  });

  it("판별자는 문자열 하나다 — warnings 배열을 넘기던 옛 호출 모양은 unknown 으로 떨어진다", () => {
    expect(unverifiedCause([WARN_NOT_ON_PATH] as unknown as string)).toBe("unknown");
  });

  it("문구는 그래도 사용자에게 **그대로** 도달한다(판정에서 뺀 것이지 숨긴 것이 아니다)", () => {
    const t = installResultToast(
      installReport({ status: "unverified", unverified_reason: "not_on_path", warnings: [WARN_BACKUP] }),
    );
    expect(t.body).toContain("cys.cys-backup-20260825-101112");
    expect(t.body).toContain("sudo mv");
  });
});

// ══════════════════════════════════════════════════════════════════════════════
// ★MINOR-N7 — 안내가 실행 가능한가(비대화형 로그인 셸은 ~/.zshrc 를 읽지 않는다)
// ══════════════════════════════════════════════════════════════════════════════
// 실측(2026-08-25): `zsh -lc` 는 .zshenv·.zprofile·.zlogin 만 읽고 .zshrc 는 건너뛴다.
// `bash -lc` 도 .bash_profile 만 읽는다. 그런데 예전 안내는 ~/.zshrc 를 고치라고 했다 —
// 시키는 대로 해도 경고가 사라지지 않는 **실행 불가능한 지시**였다.
describe("MINOR-N7 — PATH 안내는 실제로 읽히는 파일을 지목한다", () => {
  const notOnPath = () =>
    installResultToast(installReport({ status: "unverified", unverified_reason: "not_on_path" }));

  it("실제로 읽히는 파일 넷을 이름으로 지목한다", () => {
    const body = notOnPath().body;
    for (const f of ["~/.zshenv", "~/.zprofile", "~/.zlogin", "~/.bash_profile"]) {
      expect({ 파일: f, 언급됨: body.includes(f) }).toEqual({ 파일: f, 언급됨: true });
    }
  });

  it("~/.zshrc 는 '고치라'가 아니라 '읽히지 않는다'로만 등장한다", () => {
    expect(notOnPath().body).toContain("~/.zshrc 는 이 확인에서 읽히지 않습니다");
  });

  it("측정 조건(비대화형 로그인 셸)을 밝혀 거짓 경고에 헛수고하지 않게 한다", () => {
    expect(NONINTERACTIVE_PROBE_NOTE).toContain("비대화형 로그인 셸");
    expect(NONINTERACTIVE_PROBE_NOTE).toContain("무시해도 됩니다");
  });

  it("unverified 세 갈래 **전부**에 그 단서가 붙는다(어느 쪽도 거짓 경고일 수 있다)", () => {
    for (const reason of ["not_on_path", "probe_failed", null]) {
      const t = installResultToast(installReport({ status: "unverified", unverified_reason: reason }));
      expect({ reason, 단서: t.body.includes(NONINTERACTIVE_PROBE_NOTE) }).toEqual({ reason, 단서: true });
    }
  });

  it("성공(installed)에는 그 단서를 붙이지 않는다 — 할 일이 없는 결과에 잡음을 넣지 않는다", () => {
    expect(installResultToast(installReport()).body).not.toContain(NONINTERACTIVE_PROBE_NOTE);
  });

  it("파일 목록 상수에 ~/.zshrc·~/.bashrc 가 들어가 있지 않다(재발 차단)", () => {
    expect(LOGIN_SHELL_PATH_FILES).not.toContain("~/.zshrc");
    expect(LOGIN_SHELL_PATH_FILES).not.toContain("~/.bashrc");
  });
});

// ══════════════════════════════════════════════════════════════════════════════
// ★MINOR-N4 · N6 — 토스트 등급색 갱신 / 모순 알림 공존 금지
// ══════════════════════════════════════════════════════════════════════════════
describe("toastClassName · toastEmitPlan — 등급이 거짓말하지 않게 하는 순수 계획", () => {
  it("className 은 등급을 그대로 반영한다(등급색의 단일 진실)", () => {
    expect(toastClassName("system")).toBe("toast system");
    expect(toastClassName("watchdog")).toBe("toast watchdog");
  });

  it("(N4) 낼 때마다 className 이 **현재 등급**으로 나온다 — 재사용 엘리먼트의 낡은 색을 덮는다", () => {
    const fail = installResultToast(installReport({ ok: false, status: "installed_shadowed" }));
    const ok = installResultToast(installReport());
    // 같은 id 로 실패 뒤 성공이 와도 계획의 className 은 각각의 등급을 따른다.
    expect(fail.id).toBe(ok.id);
    expect(toastEmitPlan(fail).className).toBe("toast watchdog");
    expect(toastEmitPlan(ok).className).toBe("toast system");
    expect(toastEmitPlan(fail).className).not.toBe(toastEmitPlan(ok).className);
  });

  it("(N6) volatile 은 같은 id 의 살아 있는 sticky 를 먼저 내린다", () => {
    const ok = installResultToast(installReport()); // 성공 = volatile
    const emit = toastEmitPlan(ok);
    expect(emit.sticky).toBe(false);
    expect(emit.dismissStickyId).toBe(INSTALL_TOAST_ID);
  });

  it("sticky 로 낼 때는 내리지 않는다(stickyToast 가 같은 id 를 갱신하므로 깜빡임만 생긴다)", () => {
    const warn = installResultToast(installReport({ ok: false, status: "installed_shadowed" }));
    const emit = toastEmitPlan(warn);
    expect(emit.sticky).toBe(true);
    expect(emit.dismissStickyId).toBeNull();
  });

  it("해제 쪽도 같은 규약이다 — 성공 volatile 은 남은 해제 sticky 를 내린다", () => {
    const ok = uninstallResultToast(uninstallReport({ removed: ["/usr/local/bin/cys"] }));
    expect(toastEmitPlan(ok).dismissStickyId).toBe(UNINSTALL_TOAST_ID);
    const part = uninstallResultToast(uninstallReport({ ok: false, warnings: [WARN_LEFTOVER] }));
    expect(toastEmitPlan(part).dismissStickyId).toBeNull();
    expect(toastEmitPlan(part).className).toBe("toast watchdog");
  });
});

describe("readCliStatus — 상태 조회 응답 판독(계약 필드만 읽는다)", () => {
  it("installed:true/false 가 라벨을 가른다", () => {
    expect(readCliStatus(statusReport({ installed: true, state: "ours" })).button).toBe("installed");
    expect(readCliStatus(statusReport({ installed: false })).button).toBe("absent");
    expect(readInstallState(statusReport({ installed: true, state: "partial" }))).toBe("installed");
  });
  it("state 5값을 그대로 읽고, 모르는 값은 unknown", () => {
    expect(readCliStatus(statusReport({ state: "foreign" })).linkState).toBe("foreign");
    expect(readCliStatus(statusReport({ state: "partial" })).linkState).toBe("partial");
    expect(readCliStatus(statusReport({ state: "wat" })).linkState).toBe("unknown");
  });
  it("notes 를 판독한다 — 예전 타입엔 필드 자체가 없어 통째로 소실됐다(BLOCK-1(d))", () => {
    expect(readCliStatus(statusReport({ state: "foreign", notes: [NOTE_NOT_SYMLINK] })).notes).toEqual([
      NOTE_NOT_SYMLINK,
    ]);
  });
  it("platform_supported=false 만 미지원 — 판독 실패로 버튼을 숨기지 않는다", () => {
    expect(readCliStatus(statusReport({ platform_supported: false, state: "unsupported" })).supported).toBe(false);
    expect(readCliStatus(null).supported).toBe(true);
  });
  it("null·미상 응답은 unknown(해제 쪽으로 넘어가지 않는다)", () => {
    expect(readInstallState(null)).toBe("unknown");
    expect(readInstallState({})).toBe("unknown");
    expect(readInstallState("installed")).toBe("unknown");
  });
});

describe("cliButtonView — 라벨과 툴팁을 함께 산출 + notes 노출(BLOCK-1(d))", () => {
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
  it("남의 실체 파일이 있으면 툴팁에 사유 + '백업된다'는 사실이 함께 붙는다", () => {
    const v = cliButtonView("absent", [NOTE_NOT_SYMLINK]);
    expect(v.title).toContain(NOTE_NOT_SYMLINK);
    expect(v.title).toContain("cys-backup");
    expect(v.title).toContain("지우지 않고");
  });
  it("이미 설치된 상태(해제 라벨)에서는 설치 시 백업 안내를 붙이지 않는다(누르면 해제니까)", () => {
    const v = cliButtonView("installed", [NOTE_NOT_SYMLINK]);
    expect(v.title).toContain(NOTE_NOT_SYMLINK);
    expect(v.title).not.toContain(FOREIGN_BACKUP_NOTICE);
  });
});

describe("statusNoticePlan — notes 를 토스트로도 낸다(BLOCK-1(d))", () => {
  it("notes 가 없으면 null(정상은 무음)", () => {
    expect(statusNoticePlan(readCliStatus(statusReport()))).toBeNull();
    expect(statusNoticePlan(readCliStatus(statusReport({ installed: true, state: "ours" })))).toBeNull();
  });
  it("foreign 이면 사유 + 백업 예고를 sticky 로 낸다", () => {
    const p = statusNoticePlan(readCliStatus(statusReport({ state: "foreign", notes: [NOTE_NOT_SYMLINK] })));
    expect(p).not.toBeNull();
    expect(p!.body).toContain(NOTE_NOT_SYMLINK);
    expect(p!.body).toContain(FOREIGN_BACKUP_NOTICE);
    expect(p!.sticky).toBe(true);
    expect(p!.category).toBe("watchdog");
    expect(p!.id).toBe(CLI_NOTES_TOAST_ID);
  });
  it("partial(설치됨 + 남의 파일 1개)은 사유만 — 버튼이 해제라 백업 예고는 부적절", () => {
    const p = statusNoticePlan(
      readCliStatus(statusReport({ installed: true, state: "partial", notes: [NOTE_NOT_SYMLINK] })),
    );
    expect(p!.body).toContain(NOTE_NOT_SYMLINK);
    expect(p!.body).not.toContain(FOREIGN_BACKUP_NOTICE);
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

describe("isBenignSkip — '이미 없었다'만 무해로 본다", () => {
  it("부재 skip 은 무해", () => {
    expect(isBenignSkip(SKIP_ABSENT)).toBe(true);
  });
  it("실체 파일·타 대상 링크는 무해가 아니다(조치 필요)", () => {
    expect(isBenignSkip(SKIP_NOT_SYMLINK)).toBe(false);
    expect(isBenignSkip(SKIP_FOREIGN)).toBe(false);
  });
  it("판정이 안 서는 문장은 조치 필요 쪽으로 남긴다", () => {
    expect(isBenignSkip("/usr/local/bin/cys — 알 수 없는 사유")).toBe(false);
  });
});

describe("uninstallResultToast — 부분 실패를 성공으로 감추지 않는다(MAJOR-5)", () => {
  it("전부 제거 → system 등급 + volatile", () => {
    const t = uninstallResultToast(uninstallReport({ removed: ["/usr/local/bin/cys", "/usr/local/bin/cysd"] }));
    expect(t.category).toBe("system");
    expect(t.title).toContain("✅");
    expect(t.sticky).toBe(false);
    expect(t.id).toBe(UNINSTALL_TOAST_ID);
  });

  it("★건너뛴 항목(문자열)이 본문에 그대로 나온다 — 예전 TS 는 전부 소멸시켰다", () => {
    const t = uninstallResultToast(
      uninstallReport({ ok: true, removed: ["/usr/local/bin/cys"], skipped: [SKIP_NOT_SYMLINK] }),
    );
    expect(t.category).toBe("watchdog");
    expect(t.title).toContain("부분 완료");
    expect(t.body).toContain(SKIP_NOT_SYMLINK); // 경로도 사유도 통째로
    expect(t.body).toContain("실제 파일");
    expect(t.sticky).toBe(true);
  });

  it("★ok=false + warnings 의 복구 명령('sudo rm')이 사용자에게 도달한다", () => {
    const t = uninstallResultToast(
      uninstallReport({ ok: false, removed: ["/usr/local/bin/cysd"], warnings: [WARN_LEFTOVER] }),
    );
    expect(t.category).toBe("watchdog");
    expect(t.body).toContain("sudo rm /usr/local/bin/cys");
    expect(t.sticky).toBe(true);
  });

  it("ok=false 는 skipped 가 비어 있어도 성공 토스트가 되지 않는다", () => {
    const t = uninstallResultToast(uninstallReport({ ok: false, removed: ["/usr/local/bin/cys"] }));
    expect(t.category).toBe("watchdog");
    expect(t.title).not.toContain("✅");
  });

  it("지운 것도 건너뛴 것도 없으면 '해제할 심링크 없음'", () => {
    const t = uninstallResultToast(uninstallReport());
    expect(t.category).toBe("watchdog");
    expect(t.title).toContain("없음");
    expect(t.sticky).toBe(false);
  });

  it("제거 0건 + 건너뜀만 있으면 완료라고 말하지 않는다", () => {
    const t = uninstallResultToast(uninstallReport({ skipped: [SKIP_FOREIGN] }));
    expect(t.category).toBe("watchdog");
    expect(t.body).toContain("제거한 항목 없음");
    expect(t.body).toContain(SKIP_FOREIGN);
    expect(t.sticky).toBe(true);
  });

  it("한쪽만 있던 정상 해제(나머지는 '이미 해제된 상태')는 성공이다 — 오보고 금지", () => {
    const t = uninstallResultToast(uninstallReport({ removed: ["/usr/local/bin/cys"], skipped: [SKIP_ABSENT] }));
    expect(t.category).toBe("system");
    expect(t.title).toContain("✅");
    expect(t.body).toContain("이미 없던 항목");
    expect(t.sticky).toBe(false);
  });

  it("판독기를 거친 쓰레기 응답은 성공이 아니다(측정 불능은 통과가 아니다)", () => {
    const t = uninstallResultToast(readUninstallReport(null));
    expect(t.category).toBe("watchdog");
    expect(t.title).not.toContain("✅");
  });
});
