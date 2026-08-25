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
  cliButtonIntent,
  shadowTarget,
  backupOrigin,
  backupNoticeLine,
  cliNoticeLines,
  withCliNotice,
  CLI_NOTICE_HEADING,
  partitionSkips,
  statusNoticePlan,
  uninstallConfirmText,
  uninstallResultToast,
  uninstallLeftovers,
  NOTICE_TITLE_FOREIGN,
  NOTICE_TITLE_BACKUP,
  NOTICE_TITLE_INFO,
  FOREIGN_BACKUP_NOTICE,
  INSTALL_TOAST_ID,
  UNINSTALL_TOAST_ID,
  CLI_NOTES_TOAST_ID,
  type InstallCliReport,
  type UninstallCliReport,
  type CliInstallStatusReport,
} from "./clipath";

// ══════════════════════════════════════════════════════════════════════════════
// ★I6 — 계약의 기계화: 기준표는 **Rust 가 덤프한 파일**이지 이 파일의 손글씨가 아니다
// ══════════════════════════════════════════════════════════════════════════════
// 예전에는 여기에 `RUST_INSTALL_REPORT = { ok: "boolean", ... }` 같은 표를 **손으로** 적어 두고
// 그것을 "Rust 가 실제로 보내는 것"이라 불렀다. 그러나 그 표는 실물과 **연결되어 있지 않아서**,
// Rust 가 필드를 더하거나 `#[serde(rename = "...")]` 로 이름을 바꿔도 이 파일은 초록인 채였다.
// 계약을 지키는 문서가 계약이 깨져도 빨개지지 않는다면 그것은 계약이 아니라 장식이다.
//
// 그래서 기준을 뒤집는다: **Rust 의 mod tests 가** 세 리포트의 키 집합·타입 태그를
// `ui/src/__contract__.json` 으로 덤프하고, 이 파일은 그 생성물을 읽어 기준으로 삼는다.
// 파일이 없으면 검사를 건너뛰지 않고 **실패**한다 — 측정 불능은 통과가 아니다(헌장).
//
// ★받아들이는 JSON 모양(생성 측 자유도를 넓게 잡는다 — 형식 합의 실패로 게이트가 죽지 않게):
//   { "InstallCliReport": { "ok": "boolean", "effective_cys": "string|null", ... }, ... }
//   · 리포트 이름은 대소문자·구분자 무시하고 맞춘다(`install_cli_report`·`InstallCliReport` 동일).
//   · 최상위에 `reports` 래퍼가 있으면 벗긴다.
//   · 값은 태그 문자열("string"·"bool"·"Vec<String>"·"Option<String>"·"string|null"),
//     태그 배열(["string","null"]), `{type,nullable}` 객체, **또는 표본값 그대로**(예: true·
//     "/usr/local/bin"·[])를 받는다. 표본값이면 그 값의 태그로 환원한다.
const CONTRACT_URL = new URL("./__contract__.json", import.meta.url);
const CONTRACT_HINT =
  "ui/src/__contract__.json 이 없습니다. 이 파일은 src-tauri 의 계약 덤프 테스트가 만듭니다 — " +
  "`cd src-tauri && cargo test contract` 로 생성한 뒤 다시 실행하세요. " +
  "(손으로 쓴 표로 되돌리지 마세요 — 그것이 I6 가 없앤 결함입니다.)";

/** 표기 흔들림을 우리 어휘 4종(boolean·string·string[]·null)으로 환원한다. */
function normTag(raw: string): string[] {
  const s = raw.trim().toLowerCase().replace(/\s+/g, "");
  if (s.includes("|")) return s.split("|").flatMap(normTag);
  if (s === "option<string>" || s === "optionstring") return ["string", "null"];
  if (s === "bool" || s === "boolean") return ["boolean"];
  if (s === "str" || s === "string") return ["string"];
  if (s === "none" || s === "null" || s === "nil") return ["null"];
  if (s === "vec<string>" || s === "array<string>" || s === "string[]" || s === "liststring")
    return ["string[]"];
  if (s === "number" || s === "u32" || s === "u64" || s === "i32" || s === "i64" || s === "f64")
    return ["number"];
  return [s];
}
const TAG_WORDS = new Set([
  "boolean", "bool", "string", "str", "null", "none", "nil", "number",
  "string[]", "vec<string>", "array<string>", "option<string>",
  "u32", "u64", "i32", "i64", "f64",
]);
function isTagWord(v: string): boolean {
  return v
    .trim()
    .toLowerCase()
    .replace(/\s+/g, "")
    .split("|")
    .every((x) => TAG_WORDS.has(x));
}

function tagOf(v: unknown): string {
  if (v === null) return "null";
  if (Array.isArray(v)) return v.every((x) => typeof x === "string") ? "string[]" : "unknown[]";
  return typeof v;
}

/** 계약 파일의 한 필드 값 → 허용 태그 집합. 태그 표기든 표본값이든 같은 곳으로 모은다. */
function fieldSpec(v: unknown): Set<string> {
  if (typeof v === "string") return new Set(isTagWord(v) ? normTag(v) : ["string"]);
  if (Array.isArray(v) && v.length > 0 && v.every((x) => typeof x === "string" && isTagWord(x)))
    return new Set(v.flatMap((x) => normTag(x as string)));
  if (v && typeof v === "object" && !Array.isArray(v)) {
    const o = v as Record<string, unknown>;
    const t = o.type ?? o.tag ?? o.tags;
    if (typeof t === "string" || Array.isArray(t)) {
      const base = fieldSpec(t);
      if (o.nullable === true || o.optional === true) base.add("null");
      return base;
    }
  }
  return new Set([tagOf(v)]);
}

type ReportSpec = Record<string, Set<string>>;
const REPORT_ALIASES: Record<string, string[]> = {
  InstallCliReport: ["installclireport", "installclitopath", "installclitopathreport", "installcli", "install"],
  UninstallCliReport: ["uninstallclireport", "uninstallclifrompath", "uninstallcli", "uninstall"],
  CliInstallStatusReport: ["cliinstallstatusreport", "cliinstallstatus", "installstatusreport", "installstatus", "status"],
};
const norm = (k: string) => k.toLowerCase().replace(/[^a-z0-9]/g, "");

function loadContract(): Record<string, ReportSpec> | null {
  let raw: unknown;
  try {
    raw = JSON.parse(readFileSync(CONTRACT_URL, "utf8"));
  } catch {
    return null;
  }
  if (!raw || typeof raw !== "object") return null;
  let top = raw as Record<string, unknown>;
  const wrapper = top.reports ?? top.contract ?? top.schema;
  if (wrapper && typeof wrapper === "object" && !Array.isArray(wrapper))
    top = wrapper as Record<string, unknown>;
  const byNorm = new Map<string, Record<string, unknown>>();
  for (const [k, v] of Object.entries(top)) {
    if (v && typeof v === "object" && !Array.isArray(v)) byNorm.set(norm(k), v as Record<string, unknown>);
  }
  const out: Record<string, ReportSpec> = {};
  for (const [name, aliases] of Object.entries(REPORT_ALIASES)) {
    const hit = aliases.map((a) => byNorm.get(a)).find(Boolean);
    if (!hit) continue;
    const spec: ReportSpec = {};
    for (const [f, v] of Object.entries(hit)) spec[f] = fieldSpec(v);
    out[name] = spec;
  }
  return Object.keys(out).length === Object.keys(REPORT_ALIASES).length ? out : null;
}

const CONTRACT = loadContract();

function specOf(report: string): ReportSpec {
  if (!CONTRACT) throw new Error(CONTRACT_HINT);
  const spec = CONTRACT[report];
  if (!spec) throw new Error(`${CONTRACT_HINT} (${report} 항목을 찾지 못했습니다)`);
  return spec;
}

const tagsText = (s: Set<string>) => [...s].sort().join("|");

/// 태그 비교의 유일한 관용: **문자열 계열의 null**. JSON 은 `String` 과 `Option<String>` 을 구분하지
/// 못하므로, 덤프가 한쪽 팔만 표본으로 잡았을 수 있다(Some 만 봤거나 None 만 봤거나). 그래서 계약이
/// 문자열 계열이라고만 말한 필드에는 null 도 허용한다. **키 집합에는 이런 관용이 없다** — rename·
/// 필드 추가·삭제를 잡는 것은 그쪽이고, 그것이 이 게이트의 본체다.
function allows(spec: Set<string>, tag: string): boolean {
  if (spec.has(tag)) return true;
  const stringFamily = [...spec].every((t) => t === "string" || t === "null");
  return stringFamily && (tag === "string" || tag === "null");
}

/** 객체가 **생성된 계약**과 정확히 같은 필드 집합인지 + 타입이 호환되는지 단언. */
function expectShape(obj: Record<string, unknown>, report: string) {
  const spec = specOf(report);
  expect({ report, fields: Object.keys(obj).sort() }).toEqual({ report, fields: Object.keys(spec).sort() });
  for (const [field, allowed] of Object.entries(spec)) {
    const t = tagOf(obj[field]);
    const ok = allows(allowed, t);
    expect({ report, field, type: ok ? tagsText(allowed) : t }).toEqual({
      report,
      field,
      type: tagsText(allowed),
    });
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
  // skipped_benign 기본값은 false = "무해하다고 말할 수 없다"(C3). 건너뜀이 없으면 등급에 무관.
  return {
    ok: true,
    removed: [],
    skipped: [],
    skipped_reasons: [],
    skipped_benign: false,
    restored: [],
    warnings: [],
    ...over,
  };
}
function statusReport(over: Partial<CliInstallStatusReport> = {}): CliInstallStatusReport {
  return {
    platform_supported: true,
    installed: false,
    state: "absent",
    cys_link: "/usr/local/bin/cys",
    cysd_link: "/usr/local/bin/cysd",
    notes: [],
    backups: [],
    ...over,
  };
}

// Rust plan_cli_uninstall / cli_install_status 가 실제로 만드는 문장(main.rs 사본).
const SKIP_NOT_SYMLINK =
  "/usr/local/bin/cysd — 심볼릭이 아니라 실제 파일입니다. 다른 도구가 설치한 것일 수 있어 건드리지 않았습니다.";
const SKIP_FOREIGN =
  "/usr/local/bin/cys — cys.app 번들이 아닌 곳(/opt/other/cys)을 가리키는 링크라 건드리지 않았습니다.";
const SKIP_ABSENT = "/usr/local/bin/cysd — 없음(이미 해제된 상태)";
/// ★(R1 · 6R) **픽스처 드리프트 수리.** 예전 값은 `… — 터미널에서 'sudo rm …' 로 지우세요.` 였는데,
/// Rust 는 5R(G2)에서 복구 명령 산문을 뺐다(main.rs: "…가 아직 남아 있습니다 — 자동으로 제거하지
/// 못했습니다."). 낡은 픽스처가 백엔드 대신 'sudo rm' 을 넣어 주는 바람에, **UI 가 복구 명령을
/// 조립하지 않는데도** 테스트는 초록이었다 — 손으로 지어낸 픽스처가 실물과 달라 결함을 봉인하는
/// 계열(I6)의 재발이다. 실물 문장으로 되돌리고, 명령은 UI 조립(uninstallLeftovers)이 낸다.
const WARN_LEFTOVER = "/usr/local/bin/cys 가 아직 남아 있습니다 — 자동으로 제거하지 못했습니다.";
/// (R1) `cli_install_status` 가 주는 두 링크 경로 — 해제 결과 토스트가 복구 명령을 조립할 후보.
const CLI_LINKS = ["/usr/local/bin/cys", "/usr/local/bin/cysd"] as const;
/// ★(MINOR-N13 · 2026-08-25 5R) **실물 형식은 epoch 초다.** 예전 값
/// `…cys-backup-20260825-101112`(사람이 읽는 날짜)는 코드가 만들지 않는 이름이고, Rust
/// `is_our_backup_name`(스탬프 전부 숫자)이 **거부**한다 — 그런 이름의 파일은 앱이 복원 후보로도
/// 잡지 않는다. 손으로 지어낸 픽스처가 실물과 달라 초록인 채 봉인되는 계열(I6)의 한 사례였다.
/// 실물: main.rs `backup_stamp`(UNIX epoch 초) + `backup_path_for`(`<경로>.cys-backup-<초>`).
const BACKUP_PATH = "/usr/local/bin/cys.cys-backup-1756089600";
// ★I3① Rust 는 백업본 **경로만** 사실로 보낸다(`backups`) — 문구는 UI 소유다.
// Rust uninstall_cli_from_path(I3③)가 복원 통보로 warnings 에 싣는 문장의 사본 — **실패가 아니다**.
const WARN_RESTORED =
  "/usr/local/bin/cys — 설치 때 백업해 둔 원본을 그 자리에 되돌렸습니다(해제 전 상태로 복구).";
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
///
/// ★(G2 · 5R) 이 픽스처는 **사실 문장만** 담는다. 복구 명령('sudo mv …')은 백엔드가 아니라 UI 가
/// 조립하기 때문이다(backupNoticeLine). 그래서 이 상수를 쓰는 단언도 "특정 낱말이 있는가"가 아니라
/// "받은 문장이 **그대로** 사용자에게 도달하는가"만 본다 — Rust 가 문구를 어떻게 다듬어도
/// 이 파일이 빨개지지 않아야 한다(그것이 산문을 계약에서 뺀 이유다).
const WARN_BACKUP =
  "/usr/local/bin/cys에 우리 것이 아닌 파일/링크가 있어 지우지 않고 /usr/local/bin/cys.cys-backup-1756089600로 백업한 뒤 링크를 만들었습니다.";

describe("★I6 계약 드리프트 가드 — 기준은 Rust 가 덤프한 __contract__.json 이다", () => {
  it("생성된 계약 파일이 실재하고 세 리포트를 모두 담고 있다(없으면 계약을 검증할 수 없다)", () => {
    expect({ 계약파일: "ui/src/__contract__.json", 읽힘: CONTRACT !== null, 안내: CONTRACT_HINT }).toEqual({
      계약파일: "ui/src/__contract__.json",
      읽힘: true,
      안내: CONTRACT_HINT,
    });
  });
  it("InstallCliReport 픽스처는 계약 필드 집합·타입과 일치", () => {
    expectShape(installReport(), "InstallCliReport");
    expectShape(installReport({ effective_cys: null, shadowed_by: "/opt/homebrew/bin/cys" }), "InstallCliReport");
  });
  it("UninstallCliReport 픽스처는 계약과 일치 — skipped 는 문자열 배열이다", () => {
    expectShape(uninstallReport(), "UninstallCliReport");
    expectShape(uninstallReport({ skipped: [SKIP_FOREIGN], warnings: [WARN_LEFTOVER] }), "UninstallCliReport");
    // 예전 TS 가 상상하던 모양({path,reason}[])은 계약 위반으로 잡힌다 — 봉인이 깨졌는지의 증명.
    expect(() =>
      expectShape(
        { ok: true, removed: [], skipped: [{ path: "/usr/local/bin/cys", reason: "x" }], warnings: [] },
        "UninstallCliReport",
      ),
    ).toThrow();
  });
  it("CliInstallStatusReport 픽스처는 계약과 일치 — notes 필드가 실재한다", () => {
    expectShape(statusReport(), "CliInstallStatusReport");
    expectShape(statusReport({ installed: true, state: "ours" }), "CliInstallStatusReport");
  });
  it("판독기 출력도 계약 모양을 유지한다(응답이 쓰레기여도)", () => {
    expectShape(readInstallReport(installReport()), "InstallCliReport");
    expectShape(readInstallReport(null), "InstallCliReport");
    expectShape(readUninstallReport(uninstallReport()), "UninstallCliReport");
    expectShape(readUninstallReport("garbage"), "UninstallCliReport");
  });
  it("★필드 이름이 하나만 달라져도(rename) 잡힌다 — 이 게이트의 존재 이유", () => {
    // #[serde(rename = "...")] 를 붙이면 계약 파일의 키가 바뀌고, 우리 판독기 출력의 키 집합과
    // 어긋난다. 그 상황을 계약 쪽이 아니라 **객체 쪽**을 비틀어 재현한다(파일은 건드리지 않는다).
    const renamed = { ...readUninstallReport(uninstallReport()) } as Record<string, unknown>;
    renamed.skippedBenign = renamed.skipped_benign;
    delete renamed.skipped_benign;
    expect(() => expectShape(renamed, "UninstallCliReport")).toThrow();
  });
  // ── 계약 확장 의존 ─────────────────
  // 이 라운드가 새로 요구하는 **기계 판별자 둘**. TS 의 분기는 이 필드 하나씩에 걸려 있고,
  // 백엔드가 아직 싣지 않았다면 그 사실이 **이 이름으로** 드러나야 한다(다른 테스트를 덮지 않고).
  it("(C3) 계약이 UninstallCliReport.skipped_benign 을 선언한다 — 해제 등급 분기의 유일 근거", () => {
    expect({ 필드: "UninstallCliReport.skipped_benign", 계약에_선언됨: !!CONTRACT?.UninstallCliReport?.skipped_benign }).toEqual(
      { 필드: "UninstallCliReport.skipped_benign", 계약에_선언됨: true },
    );
  });
  it("(C3) 계약이 UninstallCliReport.skipped_reasons 를 선언한다 — 줄별 분류의 유일 근거", () => {
    expect({ 필드: "UninstallCliReport.skipped_reasons", 계약에_선언됨: !!CONTRACT?.UninstallCliReport?.skipped_reasons }).toEqual(
      { 필드: "UninstallCliReport.skipped_reasons", 계약에_선언됨: true },
    );
  });
  it("(I3①) 계약이 CliInstallStatusReport.backups 를 선언한다 — 잔존 백업 노출의 유일 근거", () => {
    expect({ 필드: "CliInstallStatusReport.backups", 계약에_선언됨: !!CONTRACT?.CliInstallStatusReport?.backups }).toEqual(
      { 필드: "CliInstallStatusReport.backups", 계약에_선언됨: true },
    );
  });
  it("(I3③) 계약이 UninstallCliReport.restored 를 선언한다 — '되돌린 원본'은 실패가 아니라 사실이다", () => {
    expect({ 필드: "UninstallCliReport.restored", 계약에_선언됨: !!CONTRACT?.UninstallCliReport?.restored }).toEqual(
      { 필드: "UninstallCliReport.restored", 계약에_선언됨: true },
    );
  });

  it("★타입이 달라져도 잡힌다(string[] 자리에 string)", () => {
    const wrong = { ...readUninstallReport(uninstallReport()), removed: "/usr/local/bin/cys" } as Record<
      string,
      unknown
    >;
    expect(() => expectShape(wrong, "UninstallCliReport")).toThrow();
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
    const t = installResultToast(installReport({ warnings: [WARN_BACKUP] }));
    expect(t.body).toContain(BACKUP_PATH);
    expect(t.sticky).toBe(true);
  });

  // ══════════════════════════════════════════════════════════════════════════
  // ★G14(5R) — ⚠ 는 ✅ 안에 숨을 수 없다(등급 분리)
  // ══════════════════════════════════════════════════════════════════════════
  // 실사고 형태: cysd 그림자 경고(C5)가 "✅ 셸 설치 완료" 토스트 **본문의 ⚠ 한 줄**로 나갔다.
  // 제목이 ✅ 면 사람은 본문을 읽지 않으므로, 그 경고는 사실상 전달되지 않는다.
  // 분기의 근거는 기계 신호 하나(warnings.length)이고 문구는 읽지 않는다.
  it("(G14) 깨끗한 성공만 ✅ + system + volatile 이다", () => {
    const t = installResultToast(installReport({ warnings: [] }));
    expect({ category: t.category, ok표시: t.title.includes("✅"), sticky: t.sticky }).toEqual({
      category: "system",
      ok표시: true,
      sticky: false,
    });
  });
  it("★(G14) installed 라도 warnings 가 있으면 등급이 내려간다 — ✅ 가 ⚠ 를 덮지 않는다", () => {
    const t = installResultToast(installReport({ warnings: [WARN_BACKUP] }));
    expect({ category: t.category, ok표시: t.title.includes("✅"), warn표시: t.title.includes("⚠") }).toEqual({
      category: "watchdog",
      ok표시: false,
      warn표시: true,
    });
  });
  it("★(G14) cysd 그림자 경고 하나만 있어도 성공 등급이 아니다(adv9 계열이 ✅ 안에 숨지 않는다)", () => {
    const cysdWarn =
      "cysd 확인 결과: 로그인 셸(zsh) PATH 앞쪽의 다른 cysd가 우선합니다: /opt/homebrew/bin/cysd. " +
      "/usr/local/bin/cysd가 아니라 그쪽이 실행되므로 데몬 버전이 어긋날 수 있습니다.";
    const t = installResultToast(installReport({ warnings: [cysdWarn] }));
    expect(t.category).toBe("watchdog");
    expect(t.body).toContain(cysdWarn); // 등급만 내리고 문장은 한 글자도 줄이지 않는다
  });
  it("(G14) 등급을 내려도 '링크는 만들어졌다'는 사실은 그대로 말한다(과잉 경보 금지)", () => {
    const t = installResultToast(installReport({ warnings: [WARN_BACKUP] }));
    expect(t.body).toContain("/usr/local/bin/cys");
    expect(t.body).toContain("설치 자체가 실패한 것은 아닙니다");
  });
  it("★(R3 배치) 확인 항목이 **본문 맨 앞**에 온다 — cysd 경고가 성공 서술 뒤에 묻히지 않는다", () => {
    const cysdWarn =
      "cysd 확인 결과: 로그인 셸(zsh) PATH 앞쪽의 다른 cysd가 우선합니다: /opt/homebrew/bin/cysd.";
    const t = installResultToast(installReport({ warnings: [cysdWarn] }));
    // 등급은 그대로다(master 결정: cysd 경고가 성공 등급을 낮추는지는 이 라운드에서 바꾸지 않는다).
    expect(t.category).toBe("watchdog");
    expect(t.body.startsWith("확인이 필요한 항목:")).toBe(true);
    expect(t.body.indexOf(cysdWarn)).toBeLessThan(t.body.indexOf("바로 쓸 수 있습니다"));
  });
  it("★(G14 비대칭 근거) 해제는 같은 규칙을 쓰지 않는다 — warnings 에 '되돌렸습니다'가 섞이기 때문", () => {
    // C3 가 없앤 결함의 재발 방지: 정상 해제 + 복원 통보를 ⚠ 로 오보고하지 않는다.
    const t = uninstallResultToast(
      uninstallReport({ removed: ["/usr/local/bin/cys"], restored: ["/usr/local/bin/cys"], warnings: [WARN_RESTORED] }),
    );
    expect(t.category).toBe("system");
    expect(t.title).toContain("✅");
    // 다만 ✅ 본문 안에 ⚠ 글리프는 두지 않는다(한 알림이 두 등급을 주장하지 않는다).
    expect(t.body).toContain(WARN_RESTORED);
    expect(t.body).not.toContain("⚠");
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
    // ★(G2) '특정 낱말이 있는가'가 아니라 **받은 문장이 그대로 도달하는가**를 본다.
    // 복구 명령('sudo mv …')은 백엔드 문장이 아니라 UI 조립물(backupNoticeLine)이므로,
    // 그것을 여기서 요구하면 백엔드가 산문을 뺀 순간 이 테스트가 잘못된 이유로 빨개진다.
    expect(t.body).toContain(WARN_BACKUP);
    expect(t.body).toContain(BACKUP_PATH);
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

  it("(N4) 같은 id 로 실패 뒤 성공이 와도 등급색은 각각의 등급을 따른다", () => {
    const fail = installResultToast(installReport({ ok: false, status: "installed_shadowed" }));
    const ok = installResultToast(installReport());
    expect(fail.id).toBe(ok.id);
    expect(toastClassName(fail.category)).toBe("toast watchdog");
    expect(toastClassName(ok.category)).toBe("toast system");
    expect(toastClassName(fail.category)).not.toBe(toastClassName(ok.category));
  });

  // ★(I7) 예전 ToastEmit 에는 className 필드가 있었지만 **아무도 읽지 않았다**(main.ts 의
  // toast()·stickyToast() 가 각자 toastClassName 을 부른다). 값이 늘 같으니 버그로 드러나지도
  // 않았고, "계획이 등급색을 정한다"는 거짓 계약만 남았다 — 두 번째 진실원이다. 필드를 지웠고,
  // 실제 수리(재사용 엘리먼트에 등급색 재적용)는 **main.ts 배선 핀**이 아래에서 못박는다.
  it("(I7) ToastEmit 은 소비되는 두 필드뿐이다 — 죽은 className 이 되살아나지 않았다", () => {
    const plan = installResultToast(installReport());
    expect(Object.keys(toastEmitPlan(plan)).sort()).toEqual(["dismissStickyId", "sticky"]);
    expect(CLIPATH_CODE).not.toContain("className");
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
    expect(toastClassName(part.category)).toBe("toast watchdog");
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
  it("(I3①) 잔존 백업 경로를 **기계 필드**로 판독한다 — 산문(notes)에서 캐내지 않는다", () => {
    const v = readCliStatus(statusReport({ notes: [NOTE_NOT_SYMLINK], backups: [BACKUP_PATH] }));
    expect(v.notes).toEqual([NOTE_NOT_SYMLINK]);
    expect(v.backups).toEqual([BACKUP_PATH]);
  });
  it("(I3①) 필드가 없는 응답은 빈 배열 — 없다고 단언하는 문장을 만들지 않는다", () => {
    expect(readCliStatus(null).backups).toEqual([]);
    expect(readCliStatus(statusReport()).backups).toEqual([]);
  });
});

// ══════════════════════════════════════════════════════════════════════════════
// ★I3① — 백업본 문구는 UI 소유(백엔드는 사실만). 원래 경로는 **우리 이름 규칙**에서 되읽는다
// ══════════════════════════════════════════════════════════════════════════════
describe("backupOrigin · backupNoticeLine — 되돌리기 명령을 UI 가 만든다", () => {
  it("우리 규칙(<원래 경로>.cys-backup-<epoch초>)이면 원래 경로를 되찾는다", () => {
    expect(backupOrigin(BACKUP_PATH)).toBe("/usr/local/bin/cys");
    expect(backupOrigin("/usr/local/bin/cysd.cys-backup-1")).toBe("/usr/local/bin/cysd");
  });
  it("규칙에 맞지 않으면 null — 추측한 경로로 mv 를 시키지 않는다", () => {
    expect(backupOrigin("/usr/local/bin/cys")).toBeNull();
    expect(backupOrigin("/usr/local/bin/cys.cys-backup-")).toBeNull(); // 스탬프가 비었다
    expect(backupOrigin(".cys-backup-1")).toBeNull(); // 원래 경로가 없다
    expect(backupOrigin("")).toBeNull();
  });
  it("스탬프 앞에 마커가 또 들어 있어도 **마지막** 마커에서 자른다(가장 짧은 원래 경로가 아니라)", () => {
    expect(backupOrigin("/a/cys.cys-backup-1.cys-backup-2")).toBe("/a/cys.cys-backup-1");
  });
  it("원래 경로를 알면 복원 명령을, 모르면 삭제 안내만 준다", () => {
    const known = backupNoticeLine(BACKUP_PATH);
    expect(known).toContain(`sudo mv ${BACKUP_PATH} /usr/local/bin/cys`);
    expect(known).toContain("sudo rm");
    const unknown = backupNoticeLine("/usr/local/bin/weird");
    expect(unknown).not.toContain("sudo mv");
    expect(unknown).toContain("sudo rm /usr/local/bin/weird");
  });

  // ★MINOR-N13(5R) — 스탬프는 **epoch 초(숫자)** 다. Rust is_our_backup_name 이 숫자가 아닌 스탬프를
  // 거부하므로, 그런 이름에 UI 가 'sudo mv' 를 제시하면 **앱이 되돌리지 않을 파일**에 대해 복원
  // 명령을 만들어 주는 셈이다(판정=Rust / 안내=UI 가 갈리는 MAJOR-6 과 같은 형태의 격차).
  it("★(N13) 날짜 형식 스탬프는 우리 규칙이 아니다 — Rust 가 거부하는 이름에 mv 를 시키지 않는다", () => {
    const dated = "/usr/local/bin/cys.cys-backup-20260825-101112"; // 문서 예시가 쓰던 가짜 형식
    expect(backupOrigin(dated)).toBeNull();
    expect(backupNoticeLine(dated)).not.toContain("sudo mv");
    expect(backupNoticeLine(dated)).toContain("sudo rm"); // 지우는 안내는 남는다(정보 소실 금지)
  });
  it("(N13) 숫자 스탬프만 통과한다", () => {
    expect(backupOrigin("/usr/local/bin/cys.cys-backup-0")).toBe("/usr/local/bin/cys");
    expect(backupOrigin("/usr/local/bin/cys.cys-backup-1756089600")).toBe("/usr/local/bin/cys");
    expect(backupOrigin("/usr/local/bin/cys.cys-backup-17560896x")).toBeNull();
    expect(backupOrigin("/usr/local/bin/cys.cys-backup-old")).toBeNull();
  });
});

// ══════════════════════════════════════════════════════════════════════════════
// ★MAJOR-7(5R) — 동의 화면이 실제 집행과 같은 말을 하는가
// ══════════════════════════════════════════════════════════════════════════════
// 결함: C1 수리로 **남의 심볼릭 링크도 백업**되는데, 누르기 전에 보여 주는 동의 문구는 여전히
// "다른 곳을 가리키던 링크는 (백업 없이) 새 링크로 바뀝니다" 였다. 사용자가 승인한 것이 실제로
// 일어나는 일이 아니었다.
//
// 실제 집행(main.rs build_install_script 실측): 자리에 무엇이든 있으면 백업 대상으로 잡고,
// **우리 번들을 가리키는 심볼릭일 때만** 백업을 뺀다. 즉 실체 파일과 남의 링크가 같은 취급이다.
describe("FOREIGN_BACKUP_NOTICE — 동의 문구가 실제 집행과 같은 말을 한다(MAJOR-7)", () => {
  it("★'링크는 백업 없이 바뀐다'는 거짓 고지가 되살아나지 않았다", () => {
    for (const lie of ["백업 없이", "링크는 새 링크로 바뀝니다", "새 링크로 바뀝니다"]) {
      expect({ 되살아난_거짓고지: lie, 문구에_존재: FOREIGN_BACKUP_NOTICE.includes(lie) }).toEqual({
        되살아난_거짓고지: lie,
        문구에_존재: false,
      });
    }
  });
  it("실체 파일과 심볼릭 링크 **둘 다** 백업된다고 말한다", () => {
    expect(FOREIGN_BACKUP_NOTICE).toContain("실제 파일");
    expect(FOREIGN_BACKUP_NOTICE).toContain("심볼릭 링크");
    expect(FOREIGN_BACKUP_NOTICE).toContain("지우지 않고");
  });
  it("백업 이름 규칙과 되돌리는 방법을 함께 말한다(이름을 모르면 되찾을 수 없다)", () => {
    expect(FOREIGN_BACKUP_NOTICE).toContain("cys-backup");
    expect(FOREIGN_BACKUP_NOTICE).toContain("epoch");
    expect(FOREIGN_BACKUP_NOTICE).toContain("sudo mv");
  });
  it("해제가 자동으로 되돌린다는 사실도 미리 말한다(승인 대상이 '파괴'만이 아니다)", () => {
    expect(FOREIGN_BACKUP_NOTICE).toContain("되돌립니다");
  });
});

describe("cliButtonView — 라벨과 툴팁을 함께 산출 + notes 노출(BLOCK-1(d))", () => {
  it("설치됨이면 해제 라벨", () => {
    const v = cliButtonView("installed");
    expect(v.label).toBe("셸 cys 해제");
    expect(v.title).toContain("제거");
    expect(v.intent).toBe("uninstall");
  });
  it("미설치면 설치 라벨(=index.html 초기값과 동일)", () => {
    expect(cliButtonView("absent").label).toBe("셸에 cys 설치");
    expect(cliButtonView("absent").intent).toBe("install");
  });
  it("unknown 은 설치 쪽 — 비가역 해제로 기울지 않는다", () => {
    const v = cliButtonView("unknown");
    expect(v.label).toBe("셸에 cys 설치");
    expect(v.title).toContain("확인하지 못했");
    expect(v.intent).toBe("install");
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

// ══════════════════════════════════════════════════════════════════════════════
// ★I2 / adv8 — "설치 미완료" 라고 알린 직후 버튼이 '해제'로 뒤집히지 않는다
// ══════════════════════════════════════════════════════════════════════════════
// 두 진실원이 서로 다른 것을 잰다: 설치 판정은 **PATH 에서 잡히는가**, 상태 조회는 **링크가
// 있는가**. 그림자화된 설치에서는 둘 다 참이라, 상태만 보면 라벨이 '해제'가 된다 — 방금
// "미완료니 다시 시도하라"는 안내를 읽은 사용자가 같은 자리를 누르면 정반대(비가역 해제)가 나간다.
describe("cliButtonIntent — 직전 설치 결과와 라벨이 어긋나지 않는다(I2 · adv8)", () => {
  it("직전 설치가 그림자화면 링크가 있어도 '다시 설치'다", () => {
    expect(cliButtonIntent("installed", "installed_shadowed")).toBe("reinstall");
    const v = cliButtonView("installed", [], "installed_shadowed");
    expect(v.label).toBe("셸에 cys 다시 설치");
    expect(v.label).not.toContain("해제");
    expect(v.intent).toBe("reinstall");
  });
  it("직전 설치가 확인 불가여도 '다시 설치'다(측정 불능은 완료가 아니다)", () => {
    expect(cliButtonIntent("installed", "unverified")).toBe("reinstall");
    expect(cliButtonView("installed", [], "unverified").label).toBe("셸에 cys 다시 설치");
  });
  it("직전 설치가 완료면 정상대로 '해제'다", () => {
    expect(cliButtonIntent("installed", "installed")).toBe("uninstall");
    expect(cliButtonView("installed", [], "installed").label).toBe("셸 cys 해제");
  });
  it("래치가 비어 있으면(패널 재열기·앱 기동 직후) 상태 그대로 판정한다", () => {
    expect(cliButtonIntent("installed", null)).toBe("uninstall");
    expect(cliButtonIntent("absent", null)).toBe("install");
    expect(cliButtonIntent("unknown", null)).toBe("install");
  });
  it("링크가 없는데 직전 설치가 미완료면 그래도 '다시 설치'(해제할 것이 없다)", () => {
    expect(cliButtonIntent("absent", "unverified")).toBe("reinstall");
    expect(cliButtonIntent("unknown", "installed_shadowed")).toBe("reinstall");
  });
  it("★툴팁이 '이 버튼은 해제가 아니다'와 '해제하는 법'을 함께 말한다(막다른 길 금지)", () => {
    const t = cliButtonView("installed", [], "installed_shadowed").title;
    expect(t).toContain("다시 시도");
    expect(t).toContain("해제하려면");
    expect(t).toContain("Control Center");
  });
  it("★'다시 설치' 는 설치 계열이므로 백업 예고가 함께 붙는다(누르면 설치가 일어난다)", () => {
    const t = cliButtonView("installed", [NOTE_NOT_SYMLINK], "unverified").title;
    expect(t).toContain(FOREIGN_BACKUP_NOTICE);
  });
  it("★라벨 3종이 서로 다르다 — 사용자가 무엇이 일어날지 라벨만 보고 안다", () => {
    const labels = new Set([
      cliButtonView("absent", [], null).label,
      cliButtonView("installed", [], null).label,
      cliButtonView("installed", [], "installed_shadowed").label,
    ]);
    expect(labels.size).toBe(3);
  });
});

// ══════════════════════════════════════════════════════════════════════════════
// ★G9(5R) — adv8 의 판정 단언을 Rust 에서 **이관**받는다. TS 가 유일 진실원이다.
// ══════════════════════════════════════════════════════════════════════════════
// 이관 전 상태(결함): 같은 판정이 두 곳에 있었다 —
//   · Rust  src-tauri/src/main.rs `cli_button_label` / `enum CliButtonLabel`
//   · TS    ui/src/clipath.ts     `cliButtonIntent`
// 그런데 **프로덕션에서 Rust 쪽을 부르는 곳이 없었다**(라벨은 TS 가 만든다). 죽은 판정이었고,
// 게다가 규칙이 서로 달랐다: 링크가 없는 상태(Absent)에서 직전 설치가 `unverified` 일 때
//   Rust cli_button_label(Absent, Some("unverified")) → Install
//   TS   cliButtonIntent("absent", "unverified")      → Reinstall
// Rust 의 adv8 테스트는 **TS 와 반대인 규칙**을 초록으로 못박고 있었다 — 두 번째 진실원이 서로
// 다른 것을 지키며 둘 다 통과하는, 이 라운드가 닫는 계열 결함의 전형이다.
//
// master 확정(5R): Rust 의 cli_button_label·CliButtonLabel·adv8 을 **삭제**하고, 그 단언을
// 여기로 옮긴다. 아래 표가 이제 이 판정의 유일한 규범이다.
//
// ★TS 규칙이 옳은 이유(반대 규칙을 버린 근거): 설치를 눌러 `unverified` 로 끝났다면 링크가
// 생겼는지조차 확신할 수 없다(측정 불능). 그때 다음 동작으로 옳은 것은 '설치'가 아니라 **'다시
// 설치'** 다 — 라벨이 "다시"라고 말해야 사용자가 방금 읽은 "미완료" 안내와 이어진다. 링크 유무로
// 라벨을 되돌리면(Rust 규칙) 방금 실패했다는 맥락이 화면에서 지워진다.
describe("★G9 — Rust adv8 판정 단언 이관본(TS 가 유일 진실원)", () => {
  // 매핑: Rust CliLinkState → TS CliButtonState
  //   Ours·Partial → installed(=cli_install_status.installed 가 true 인 두 값)
  //   Absent·Foreign → absent
  const OURS = "installed" as const;
  const ABSENT = "absent" as const;

  it("[adv8 본체] 그림자화된 설치 직후에도 재시도 경로가 남는다", () => {
    // Rust: cli_button_label(Ours, Some("installed_shadowed")) == Reinstall
    expect(cliButtonIntent(OURS, "installed_shadowed")).toBe("reinstall");
  });
  it("[adv8] 반대 방향 — 정상 설치 직후·래치 없음은 해제다(과잉 잠금 금지)", () => {
    // Rust: cli_button_label(Ours, Some("installed")) == Uninstall
    expect(cliButtonIntent(OURS, "installed")).toBe("uninstall");
    // Rust: cli_button_label(Ours, None) == Uninstall
    expect(cliButtonIntent(OURS, null)).toBe("uninstall");
  });
  it("[adv8] 링크가 없으면 해제로 기울지 않는다", () => {
    // Rust: cli_button_label(Foreign, None) == Install
    expect(cliButtonIntent(ABSENT, null)).toBe("install");
    expect(cliButtonIntent("unknown", null)).toBe("install");
  });
  it("★[adv8 규칙 충돌 지점] 링크가 없어도 직전 설치가 미완료면 '다시 설치'다 — Rust 는 Install 이라 했다", () => {
    // Rust: cli_button_label(Absent, Some("unverified")) == Install  ← 이 단언을 **버린다**.
    // 측정 불능으로 끝난 직후에는 '다시'가 사실이다(해제로 기울지도 않는다 — 둘 다 설치 계열).
    expect(cliButtonIntent(ABSENT, "unverified")).toBe("reinstall");
    expect(cliButtonIntent("unknown", "installed_shadowed")).toBe("reinstall");
    // 어느 쪽이든 **비가역 해제로는 절대 가지 않는다**(그것이 두 규칙의 공통 안전선이다).
    for (const s of [ABSENT, "unknown"] as const)
      for (const last of ["unverified", "installed_shadowed"] as const)
        expect(cliButtonIntent(s, last)).not.toBe("uninstall");
  });
  it("[adv8] 판정 전수표 — 3(state) × 4(last) 전 조합이 계약대로다", () => {
    const table: Record<string, string> = {
      "installed|null": "uninstall",
      "installed|installed": "uninstall",
      "installed|installed_shadowed": "reinstall",
      "installed|unverified": "reinstall",
      "absent|null": "install",
      "absent|installed": "install",
      "absent|installed_shadowed": "reinstall",
      "absent|unverified": "reinstall",
      "unknown|null": "install",
      "unknown|installed": "install",
      "unknown|installed_shadowed": "reinstall",
      "unknown|unverified": "reinstall",
    };
    const got: Record<string, string> = {};
    for (const s of ["installed", "absent", "unknown"] as const)
      for (const last of [null, "installed", "installed_shadowed", "unverified"] as const)
        got[`${s}|${last === null ? "null" : last}`] = cliButtonIntent(s, last);
    expect(got).toEqual(table);
  });
});

// ══════════════════════════════════════════════════════════════════════════════
// ★G2(5R) — 한 사건에 한 알림. 복구 명령 문장은 **UI 가 조립**한다.
// ══════════════════════════════════════════════════════════════════════════════
describe("withCliNotice — 결과 토스트에 상시 고지를 접어 넣는다(두 토스트 금지)", () => {
  it("고지할 것이 없으면 계획을 그대로 둔다(무음 규약 유지)", () => {
    const base = installResultToast(installReport());
    expect(withCliNotice(base, [])).toEqual(base);
    expect(withCliNotice(base, ["", ""])).toEqual(base);
  });
  it("고지 줄이 있으면 같은 토스트 **하나**에 실린다(id 가 늘지 않는다)", () => {
    const base = installResultToast(installReport());
    const merged = withCliNotice(base, cliNoticeLines({ notes: [], backups: [BACKUP_PATH] }));
    expect(merged.id).toBe(base.id);
    expect(merged.body).toContain(base.body);
    expect(merged.body).toContain(BACKUP_PATH);
    expect(merged.body).toContain(`sudo mv ${BACKUP_PATH} /usr/local/bin/cys`); // UI 조립물
  });
  it("고지가 붙으면 sticky 로 올린다 — 8초에 사라지면 다음 CC 열기까지 아무도 말해 주지 않는다", () => {
    const base = installResultToast(installReport());
    expect(base.sticky).toBe(false);
    expect(withCliNotice(base, [NOTE_NOT_SYMLINK]).sticky).toBe(true);
  });
  it("등급은 건드리지 않는다(등급의 진실원은 결과 계획이다)", () => {
    const ok = uninstallResultToast(uninstallReport({ removed: ["/usr/local/bin/cys"] }));
    expect(withCliNotice(ok, [NOTE_NOT_SYMLINK]).category).toBe(ok.category);
  });
  it("★해제 실패 경로에도 같은 고지가 붙는다(G1 대칭 — 실패만 덜 말하지 않는다)", () => {
    const failed = {
      category: "watchdog",
      title: "셸 cys 해제 실패",
      body: "Error: User canceled.",
      sticky: true,
      id: UNINSTALL_TOAST_ID,
    };
    const merged = withCliNotice(failed, cliNoticeLines({ notes: [], backups: [BACKUP_PATH] }));
    expect(merged.body).toContain("Error: User canceled.");
    expect(merged.body).toContain(BACKUP_PATH);
  });
  it("고지 절에는 그 줄들이 무엇인지 알리는 제목이 붙는다(맥락 없는 경로 나열 금지)", () => {
    const merged = withCliNotice(installResultToast(installReport()), [NOTE_NOT_SYMLINK]);
    expect(merged.body).toContain(CLI_NOTICE_HEADING);
  });
  it("같은 백업 줄을 두 번 싣지 않는다", () => {
    const base = installResultToast(installReport());
    const line = backupNoticeLine(BACKUP_PATH);
    const merged = withCliNotice(base, [line]);
    expect(merged.body.split(line).length - 1).toBe(1);
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

  // ★I3① — 백업 재발견. 설치 직후의 sticky 는 60초 뒤 사라지고 알람 이력은 메모리 전용이라,
  // 앱을 껐다 켜면 "당신의 원본을 어디로 옮겼는지"를 말해 주는 곳이 하나도 없었다.
  // 지금은 상태 조회의 notes 에 상시 실리고, 그 문장이 여기(토스트)와 버튼 툴팁 양쪽에 도달한다.
  it("고지할 남의 파일은 없고 백업만 남아 있어도 알린다(무음이 아니다)", () => {
    const p = statusNoticePlan(readCliStatus(statusReport({ backups: [BACKUP_PATH] })));
    expect(p).not.toBeNull();
    expect(p!.title).toContain("백업");
    expect(p!.body).toContain(BACKUP_PATH);
    expect(p!.body).toContain("sudo mv"); // 되돌리는 방법이 함께 온다
    expect(p!.sticky).toBe(true);
    expect(p!.id).toBe(CLI_NOTES_TOAST_ID);
  });
  it("남의 파일 고지와 백업 고지는 한 토스트에 함께 실린다(둘 다 잃지 않는다)", () => {
    const p = statusNoticePlan(
      readCliStatus(statusReport({ state: "foreign", notes: [NOTE_NOT_SYMLINK], backups: [BACKUP_PATH] })),
    );
    expect(p!.body).toContain(NOTE_NOT_SYMLINK);
    expect(p!.body).toContain(BACKUP_PATH);
    expect(p!.body).toContain(FOREIGN_BACKUP_NOTICE);
  });
  it("백업 줄을 두 번 싣지 않는다", () => {
    const p = statusNoticePlan(readCliStatus(statusReport({ backups: [BACKUP_PATH] })));
    expect(p!.body.split(backupNoticeLine(BACKUP_PATH)).length - 1).toBe(1);
  });
  it("아무것도 없으면 여전히 무음이다", () => {
    expect(statusNoticePlan(readCliStatus(statusReport({ backups: [] })))).toBeNull();
  });
  it("★툴팁과 토스트가 **같은 고지 줄**을 쓴다 — 남는 표면(툴팁)이 덜 말하지 않는다", () => {
    const view = readCliStatus(statusReport({ state: "foreign", notes: [NOTE_NOT_SYMLINK], backups: [BACKUP_PATH] }));
    const lines = cliNoticeLines(view);
    expect(lines).toContain(NOTE_NOT_SYMLINK);
    expect(lines.some((l) => l.includes(BACKUP_PATH))).toBe(true);
    const tip = cliButtonView(view.button, lines).title;
    for (const l of lines) expect(tip).toContain(l);
    for (const l of lines) expect(statusNoticePlan(view)!.body).toContain(l);
  });
});

// ══════════════════════════════════════════════════════════════════════════════
// ★MAJOR-D(6R) — 정상 사용자에게 **매번 거짓 경고**를 내지 않는다(제목은 기계 필드가 정한다)
// ══════════════════════════════════════════════════════════════════════════════
// 실사고: G4 가 `cli_install_status.notes` 에 PATH-그림자·프로브실패 문장을 새로 실었는데, 제목은
// 여전히 `notes.length > 0` 하나로 "⚠ /usr/local/bin 에 이 앱의 것이 아닌 cys 파일이 있습니다" 로
// 고정돼 있었다 — 남의 파일이 **하나도 없는** 정상 설치 사용자가 Control Center 를 열 때마다
// 거짓 경고를 봤다. notes 는 성격이 다른 문장이 합류하는 채널이라 '들어 있음'이 종류를 말해 주지
// 않는다(채널의 내용물 유무로 판정을 추정하는, 이 계열이 반복해 온 형태).
// 제목은 Rust 가 이미 주는 기계 필드(state·backups)로만 정한다 — 산문은 본문에만 싣는다.
describe("statusNoticePlan 제목 — notes 의 '있음'이 종류를 말해 주지 않는다(MAJOR-D)", () => {
  // Rust path_shadow_note / cysd_shadow_warning 이 실제로 만드는 문장의 사본(main.rs).
  const NOTE_SHADOW =
    "cysd 확인 결과: 로그인 셸(zsh) PATH 앞쪽의 다른 cysd가 우선합니다: /opt/homebrew/bin/cysd. " +
    "새 터미널에서 'cysd'를 치면 /usr/local/bin/cysd가 아니라 그쪽이 실행됩니다.";
  const NOTE_PROBE_FAILED =
    "PATH 확인 실패: 로그인 셸(zsh)로 'which -a cys'를 실행하지 못했습니다: timeout.";

  it("★남의 파일이 없는 정상 설치(state=ours) + 그림자 안내 → ⚠ 도, '남의 파일' 도 아니다", () => {
    const p = statusNoticePlan(
      readCliStatus(statusReport({ installed: true, state: "ours", notes: [NOTE_SHADOW] })),
    );
    expect(p).not.toBeNull();
    expect(p!.title).toBe(NOTICE_TITLE_INFO);
    expect(p!.title).not.toContain("⚠");
    expect(p!.title).not.toContain("아닌 cys 파일");
    // 테두리색도 중립이어야 한다 — 제목만 중립이면 색이 등급을 거짓말한다(N4 와 같은 원리).
    expect(p!.category).toBe("system");
    expect(p!.body).toContain(NOTE_SHADOW); // 등급만 내리고 문장은 한 글자도 줄이지 않는다
  });

  it("프로브 실패 안내도 같다 — 확인을 못 했다는 사실이 '남의 파일이 있다'가 되지 않는다", () => {
    const p = statusNoticePlan(
      readCliStatus(statusReport({ installed: true, state: "ours", notes: [NOTE_PROBE_FAILED] })),
    );
    expect(p!.title).toBe(NOTICE_TITLE_INFO);
    expect(p!.body).toContain(NOTE_PROBE_FAILED);
  });

  it("state=foreign 일 때만 남의 파일 경고 제목이다(⚠ + watchdog)", () => {
    const p = statusNoticePlan(readCliStatus(statusReport({ state: "foreign", notes: [NOTE_NOT_SYMLINK] })));
    expect(p!.title).toBe(NOTICE_TITLE_FOREIGN);
    expect(p!.category).toBe("watchdog");
  });

  it("백업본이 남아 있으면 백업 고지 제목이다(그림자 안내가 함께 있어도 · 둘 다 본문에 남는다)", () => {
    const p = statusNoticePlan(
      readCliStatus(
        statusReport({ installed: true, state: "ours", notes: [NOTE_SHADOW], backups: [BACKUP_PATH] }),
      ),
    );
    expect(p!.title).toBe(NOTICE_TITLE_BACKUP);
    expect(p!.body).toContain(NOTE_SHADOW);
    expect(p!.body).toContain(BACKUP_PATH);
  });

  it("★제목이 notes 의 개수·문구에 흔들리지 않는다 — 기계 필드 하나가 정한다", () => {
    const proseSets = [
      [NOTE_SHADOW],
      [NOTE_PROBE_FAILED],
      [NOTE_SHADOW, NOTE_PROBE_FAILED],
      ["미래의 Rust 가 다듬은 전혀 다른 문장"],
    ];
    const titles = new Set(
      proseSets.map(
        (notes) =>
          statusNoticePlan(readCliStatus(statusReport({ installed: true, state: "ours", notes })))!.title,
      ),
    );
    expect({ 서로다른_제목수: titles.size }).toEqual({ 서로다른_제목수: 1 });
  });

  it("고지할 것이 아무것도 없으면 여전히 무음이다(정상은 말이 없다)", () => {
    expect(statusNoticePlan(readCliStatus(statusReport({ installed: true, state: "ours" })))).toBeNull();
  });

  it("statusNoticePlan 이 notes **문구**를 읽어 제목을 정하지 않는다(산문 파싱 금지 가드)", () => {
    const body = CLIPATH_CODE.slice(CLIPATH_CODE.indexOf("export function statusNoticePlan"));
    const upto = body.slice(0, body.indexOf("\n}\n") + 3);
    expect(upto.length).toBeGreaterThan(200); // 빈 슬라이스를 검사하는 사고 방지
    for (const banned of [".test(", ".match(", ".includes(", ".search(", ".startsWith(", ".indexOf(", "RegExp"]) {
      expect({ 금지연산: banned, 사용됨: upto.includes(banned) }).toEqual({ 금지연산: banned, 사용됨: false });
    }
  });
});

// ══════════════════════════════════════════════════════════════════════════════
// ★R1(6R) — 해제 복구 명령('sudo rm')은 **UI 가 조립한다**(백엔드는 사실만 보낸다)
// ══════════════════════════════════════════════════════════════════════════════
// 5R 에서 Rust 가 복구 명령 산문을 뺐는데 UI 는 조립하지 않아, 해제가 부분 실패했을 때 사용자가
// 손으로 정리할 방법을 어디에서도 듣지 못했다(문서는 여전히 보여준다고 약속하고 있었다).
// 근거는 전부 기존 기계 값이다: 후보 = cys_link·cysd_link, 빼기 = removed(+건너뛴 줄이 가리키는 경로).
describe("uninstallLeftovers — 남은 링크에만 복구 명령을 붙인다(R1)", () => {
  it("제거되지 않은 우리 링크를 지목한다", () => {
    expect(
      uninstallLeftovers(uninstallReport({ ok: false, removed: ["/usr/local/bin/cysd"] }), CLI_LINKS),
    ).toEqual(["/usr/local/bin/cys"]);
  });

  it("전부 제거됐으면 지목할 것이 없다", () => {
    expect(uninstallLeftovers(uninstallReport({ removed: [...CLI_LINKS] }), CLI_LINKS)).toEqual([]);
  });

  it("★건너뛴 남의 파일은 절대 지목하지 않는다 — 지키기로 한 파일을 사용자 손으로 지우게 하지 않는다", () => {
    const r = uninstallReport({
      ok: true,
      removed: ["/usr/local/bin/cys"],
      skipped: [SKIP_NOT_SYMLINK],
      skipped_reasons: ["not_symlink"],
    });
    expect(uninstallLeftovers(r, CLI_LINKS)).toEqual([]);
    expect(uninstallResultToast(r, CLI_LINKS).body).not.toContain("sudo rm /usr/local/bin/cysd");
  });

  it("경로 접두가 다른 링크를 삼키지 않는다(cys 가 cysd 줄을 먹지 않는다)", () => {
    const r = uninstallReport({ ok: false, skipped: [SKIP_ABSENT], skipped_reasons: ["absent"] });
    expect(uninstallLeftovers(r, CLI_LINKS)).toEqual(["/usr/local/bin/cys"]);
  });

  it("★건너뜀 줄의 형식이 바뀌면 아무 것도 지목하지 않는다(fail-closed — 파괴적 지시가 새지 않는다)", () => {
    const r = uninstallReport({ ok: false, skipped: ["건너뜀: /usr/local/bin/cysd (실제 파일)"] });
    expect(uninstallLeftovers(r, CLI_LINKS)).toEqual([]);
  });

  it("링크 경로를 못 읽었으면(상태 조회 실패) 없는 경로를 지목하지 않는다", () => {
    expect(uninstallLeftovers(uninstallReport({ ok: false }), ["", ""])).toEqual([]);
    expect(uninstallResultToast(uninstallReport({ ok: false }), ["", ""]).body).not.toContain("sudo rm");
  });

  it("결과 토스트가 그 경로에 복구 명령을 붙여 낸다", () => {
    const t = uninstallResultToast(
      uninstallReport({ ok: false, removed: ["/usr/local/bin/cysd"] }),
      CLI_LINKS,
    );
    expect(t.body).toContain("아직 남아 있는 링크");
    expect(t.body).toContain("'sudo rm /usr/local/bin/cys'");
  });

  it("성공한 해제에는 복구 명령을 붙이지 않는다(없는 문제를 만들지 않는다)", () => {
    const t = uninstallResultToast(uninstallReport({ removed: [...CLI_LINKS] }), CLI_LINKS);
    expect(t.body).not.toContain("sudo rm");
  });

  it("links 를 넘기지 않는 옛 호출부도 깨지지 않는다(기본값 = 지목 없음)", () => {
    expect(uninstallResultToast(uninstallReport({ ok: false })).body).not.toContain("sudo rm");
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

  // ★I3③ — 해제는 지우기만 하는 것이 아니라 **되돌린다**. 사용자가 승인하는 행위가 무엇인지
  // 확인 창이 먼저 말해야 한다(승인 대상이 '삭제'만이 아니기 때문이다).
  it("백업본 복원이 일어난다는 사실을 미리 말한다", () => {
    const b = uninstallConfirmText().body;
    expect(b).toContain("cys-backup");
    expect(b).toContain("되돌립니다");
  });

  // ★I3② — 결정의 순간에 현재 상태(notes)를 그대로 보여준다. 문구를 읽어 분류하지 않는다 —
  // 분기의 근거는 notes 의 **길이** 하나뿐이다(C3 와 같은 원리).
  it("고지할 것이 없으면 상태·백업 절을 만들지 않는다(정상은 무음)", () => {
    const b = uninstallConfirmText().body;
    expect(b).not.toContain("현재 /usr/local/bin 상태");
    expect(b).not.toContain("남아 있습니다");
    expect(uninstallConfirmText([], []).body).toBe(b);
  });
  it("★잔존 백업이 있으면 경로와 되돌리는 명령이 확인 창에 나온다(60초 뒤 사라지지 않는 경로)", () => {
    const b = uninstallConfirmText([], [BACKUP_PATH]).body;
    expect(b).toContain(BACKUP_PATH);
    expect(b).toContain(`sudo mv ${BACKUP_PATH} /usr/local/bin/cys`);
    expect(b).toContain("제자리에 되돌립니다");
  });
  it("남의 실체 파일 고지(notes)는 그대로 옮겨 실린다(문장을 가르지 않는다)", () => {
    const b = uninstallConfirmText([NOTE_NOT_SYMLINK], []).body;
    expect(b).toContain("현재 /usr/local/bin 상태");
    expect(b).toContain(NOTE_NOT_SYMLINK);
  });
  it("둘 다 있으면 둘 다 실린다(하나가 다른 하나를 덮지 않는다)", () => {
    const b = uninstallConfirmText([NOTE_NOT_SYMLINK], [BACKUP_PATH]).body;
    expect(b).toContain(NOTE_NOT_SYMLINK);
    expect(b).toContain(BACKUP_PATH);
  });
  it("빈 문자열만 든 목록은 절을 열지 않는다", () => {
    expect(uninstallConfirmText([""], [""]).body).toBe(uninstallConfirmText().body);
  });
  it("고지가 있어도 기존 문장은 그대로 남는다(정보를 대체하지 않고 더한다)", () => {
    const b = uninstallConfirmText([NOTE_NOT_SYMLINK], [BACKUP_PATH]).body;
    expect(b).toContain("건너뜁니다");
    expect(b).toContain("다시 필요하면");
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

  it("★ok=false 면 복구 명령('sudo rm')이 사용자에게 도달한다 — 이제 UI 가 조립한다(R1)", () => {
    const t = uninstallResultToast(
      uninstallReport({ ok: false, removed: ["/usr/local/bin/cysd"], warnings: [WARN_LEFTOVER] }),
      CLI_LINKS,
    );
    expect(t.category).toBe("watchdog");
    expect(t.body).toContain(WARN_LEFTOVER); // 백엔드의 사실 문장도 그대로 도달한다
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
    const t = uninstallResultToast(uninstallReport({ skipped: [SKIP_FOREIGN], skipped_benign: false }));
    expect(t.category).toBe("watchdog");
    expect(t.body).toContain("제거한 항목 없음");
    expect(t.body).toContain(SKIP_FOREIGN);
    expect(t.sticky).toBe(true);
  });

  it("한쪽만 있던 정상 해제(나머지는 '이미 해제된 상태')는 성공이다 — 오보고 금지", () => {
    const t = uninstallResultToast(
      uninstallReport({ removed: ["/usr/local/bin/cys"], skipped: [SKIP_ABSENT], skipped_benign: true }),
    );
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

  it("건너뜀이 전부 무해여도 제거 0건이면 '해제할 심링크 없음'이다(완료라고 말하지 않는다)", () => {
    const t = uninstallResultToast(uninstallReport({ skipped: [SKIP_ABSENT], skipped_benign: true }));
    expect(t.category).toBe("watchdog");
    expect(t.title).toContain("없음");
    expect(t.body).toContain(SKIP_ABSENT);
    expect(t.sticky).toBe(false);
  });
});

// ══════════════════════════════════════════════════════════════════════════════
// ★C3 — 해제 등급의 유일 근거는 skipped_benign 필드다(산문 정규식 재도입 차단)
// ══════════════════════════════════════════════════════════════════════════════
// 설치 경로는 N3 에서 산문 파싱을 이미 걷어냈는데(warnings → unverified_reason) **해제 경로에는
// 같은 수리가 적용되지 않아** `/이미 해제/` 정규식이 그대로 살아 있었다. 같은 결함의 거울쌍이다.
// Rust 가 "없음(이미 해제된 상태)" 를 한 단어만 다듬으면 정상 해제가 조용히 '부분 완료'로
// 오보고된다 — 문구는 계약이 될 수 없다. 이 블록은 그 정규식이 되살아나면 먼저 빨개진다.
describe("해제 경로 산문 파싱 금지 가드(C3)", () => {
  it("isBenignSkip·'이미 해제' 정규식이 코드에 되살아나지 않았다", () => {
    for (const gone of ["isBenignSkip", "이미 해제/", "/이미 해제", "없음(이미"]) {
      expect({ 되살아난_패턴: gone, 코드에_존재: CLIPATH_CODE.includes(gone) }).toEqual({
        되살아난_패턴: gone,
        코드에_존재: false,
      });
    }
  });

  it("uninstallResultToast 가 skipped·warnings 를 정규식·문자열 검색으로 읽지 않는다", () => {
    const body = CLIPATH_CODE.slice(CLIPATH_CODE.indexOf("export function uninstallResultToast"));
    const upto = body.slice(0, body.indexOf("\n}\n") + 3);
    expect(upto.length).toBeGreaterThan(200); // 슬라이스가 빈 문자열을 검사하는 사고 방지
    for (const banned of [".test(", ".match(", ".includes(", ".search(", ".startsWith(", ".indexOf(", "RegExp"]) {
      expect({ 금지연산: banned, 사용됨: upto.includes(banned) }).toEqual({ 금지연산: banned, 사용됨: false });
    }
  });

  it("★판별자와 문구가 충돌하면 판별자가 이긴다 — 문구는 판정에 관여하지 않는다", () => {
    // 문장은 '이미 해제된 상태'인데 판별자는 false → 무해로 접지 않는다(성공 둔갑 금지).
    const a = uninstallResultToast(
      uninstallReport({ removed: ["/usr/local/bin/cys"], skipped: [SKIP_ABSENT], skipped_benign: false }),
    );
    expect(a.category).toBe("watchdog");
    expect(a.title).toContain("부분 완료");
    // 반대 방향: 문장은 '실제 파일'인데 판별자가 true → Rust 가 무해라 했으면 무해다.
    const b = uninstallResultToast(
      uninstallReport({ removed: ["/usr/local/bin/cys"], skipped: [SKIP_NOT_SYMLINK], skipped_benign: true }),
    );
    expect(b.category).toBe("system");
  });

  it("★건너뜀 문구를 어떻게 바꿔도 등급이 흔들리지 않는다 — 판별자 하나가 등급을 정한다", () => {
    const proseSets = [
      [SKIP_ABSENT],
      [SKIP_FOREIGN],
      [SKIP_NOT_SYMLINK],
      ["/usr/local/bin/cysd — 지울 것이 없었습니다"], // 문구를 다듬은 미래의 Rust
      [SKIP_ABSENT, SKIP_FOREIGN],
    ];
    for (const benign of [true, false]) {
      const cats = new Set(
        proseSets.map(
          (sk) =>
            uninstallResultToast(
              uninstallReport({ removed: ["/usr/local/bin/cys"], skipped: sk, skipped_benign: benign }),
            ).category,
        ),
      );
      expect({ benign, 서로다른_등급수: cats.size }).toEqual({ benign, 서로다른_등급수: 1 });
    }
  });

  it("판별자가 없는 응답(구 백엔드)은 무해로 접히지 않는다 — 판정 불가는 조치 필요 쪽이다", () => {
    const rep = readUninstallReport({ ok: true, removed: ["/usr/local/bin/cys"], skipped: [SKIP_ABSENT] });
    expect(rep.skipped_benign).toBe(false);
    const t = uninstallResultToast(rep);
    expect(t.category).toBe("watchdog");
    expect(t.title).not.toContain("✅");
  });

  it("판별자는 bool 이다 — 문자열·숫자 같은 참 같은 값은 false 로 접힌다", () => {
    for (const v of ["true", 1, "이미 해제", {}]) {
      expect(readUninstallReport({ ok: true, removed: [], skipped: [], skipped_benign: v }).skipped_benign).toBe(
        false,
      );
    }
  });

  it("건너뜀 문장은 그래도 사용자에게 **그대로** 도달한다(판정에서 뺀 것이지 숨긴 것이 아니다)", () => {
    const t = uninstallResultToast(
      uninstallReport({ removed: ["/usr/local/bin/cys"], skipped: [SKIP_FOREIGN], skipped_benign: false }),
    );
    expect(t.body).toContain(SKIP_FOREIGN);
  });
});

// ══════════════════════════════════════════════════════════════════════════════
// ★C3 — 줄별 분류도 기계 태그로(partitionSkips) · ★I3③ 복원은 실패가 아니다
// ══════════════════════════════════════════════════════════════════════════════
describe("partitionSkips — 건너뛴 줄을 태그로 가른다(문구를 읽지 않는다)", () => {
  it("태그가 정렬돼 있으면 줄별로 정확히 가른다", () => {
    const parts = partitionSkips([SKIP_ABSENT, SKIP_FOREIGN], ["absent", "foreign_target"], false);
    expect(parts.benign).toEqual([SKIP_ABSENT]);
    expect(parts.blocking).toEqual([SKIP_FOREIGN]);
  });
  it("★문구와 태그가 어긋나면 **태그가 이긴다**", () => {
    // 문장은 '이미 해제된 상태'인데 태그는 foreign_target → 조치 필요 쪽이다.
    const parts = partitionSkips([SKIP_ABSENT], ["foreign_target"], true);
    expect(parts.benign).toEqual([]);
    expect(parts.blocking).toEqual([SKIP_ABSENT]);
  });
  it("길이가 어긋나면(계약 위반·구 백엔드) 줄별 분류를 포기하고 skipped_benign 에 맡긴다", () => {
    expect(partitionSkips([SKIP_ABSENT, SKIP_FOREIGN], [], true).benign.length).toBe(2);
    expect(partitionSkips([SKIP_ABSENT, SKIP_FOREIGN], [], false).blocking.length).toBe(2);
  });
  it("건너뜀이 없으면 양쪽 다 빈 배열", () => {
    expect(partitionSkips([], [], true)).toEqual({ benign: [], blocking: [] });
  });
});

describe("uninstallResultToast — 복원·고지를 실패로 오보고하지 않는다(I3③)", () => {
  it("★복원 통보가 warnings 에 있어도 등급은 성공이다 — warnings 유무로 실패를 추정하지 않는다", () => {
    const t = uninstallResultToast(
      uninstallReport({
        ok: true,
        removed: ["/usr/local/bin/cys"],
        restored: ["/usr/local/bin/cys"],
        warnings: [WARN_RESTORED],
      }),
    );
    expect(t.category).toBe("system");
    expect(t.title).toContain("✅");
    expect(t.body).toContain("되돌렸습니다");
    expect(t.body).toContain(WARN_RESTORED); // 문장도 그대로 도달한다
  });
  it("성공이라도 warnings 가 있으면 sticky 다(8초에 사라지면 못 읽는다 — 설치 쪽과 같은 규약)", () => {
    const withWarn = uninstallResultToast(
      uninstallReport({ removed: ["/usr/local/bin/cys"], warnings: [WARN_RESTORED] }),
    );
    const clean = uninstallResultToast(uninstallReport({ removed: ["/usr/local/bin/cys"] }));
    expect(withWarn.sticky).toBe(true);
    expect(clean.sticky).toBe(false);
  });
  it("★진짜 실패(ok=false)는 여전히 ⚠ 다 — 복구 명령이 그대로 온다", () => {
    const t = uninstallResultToast(
      uninstallReport({ ok: false, removed: ["/usr/local/bin/cysd"], warnings: [WARN_LEFTOVER] }),
      CLI_LINKS,
    );
    expect(t.category).toBe("watchdog");
    expect(t.body).toContain("sudo rm /usr/local/bin/cys");
  });
  it("⚠ 등급에서도 되돌린 원본을 먼저 밝힌다(무엇이 살아났는지 사용자가 알아야 한다)", () => {
    const t = uninstallResultToast(
      uninstallReport({
        ok: false,
        removed: ["/usr/local/bin/cys"],
        restored: ["/usr/local/bin/cys"],
        warnings: [WARN_LEFTOVER],
      }),
    );
    expect(t.body).toContain("되돌린 원본: /usr/local/bin/cys");
  });
  it("줄별 태그가 있으면 무해한 줄만 '이미 없던 항목'으로 접힌다", () => {
    const t = uninstallResultToast(
      uninstallReport({
        removed: ["/usr/local/bin/cys"],
        skipped: [SKIP_ABSENT],
        skipped_reasons: ["absent"],
        skipped_benign: true,
      }),
    );
    expect(t.category).toBe("system");
    expect(t.body).toContain("이미 없던 항목");
  });
  it("★태그 하나라도 무해가 아니면 skipped_benign 이 true 여도 ⚠ 다(안전한 방향)", () => {
    const t = uninstallResultToast(
      uninstallReport({
        removed: ["/usr/local/bin/cys"],
        skipped: [SKIP_ABSENT, SKIP_FOREIGN],
        skipped_reasons: ["absent", "foreign_target"],
        skipped_benign: true, // 두 신호가 어긋난다 — 덜 알리는 쪽으로 접지 않는다
      }),
    );
    expect(t.category).toBe("watchdog");
    expect(t.body).toContain(SKIP_FOREIGN);
  });
});

// ══════════════════════════════════════════════════════════════════════════════
// ★C3 대칭 점검 — shadowed_by 는 **셸이 뱉은 한 줄**이다(검증 없이 rm 을 시키지 않는다)
// ══════════════════════════════════════════════════════════════════════════════
describe("shadowTarget — 경로로 읽히는 값일 때만 파괴적 지시에 넣는다", () => {
  it("절대경로 한 개는 그대로 채택한다", () => {
    expect(shadowTarget("/opt/homebrew/bin/cys")).toEqual({
      path: "/opt/homebrew/bin/cys",
      label: "/opt/homebrew/bin/cys",
    });
  });
  it("공백이 든 줄(zsh 함수 래퍼 본문·프로필 배너)은 경로로 채택하지 않는다(adv1 · adv7 계열)", () => {
    expect(shadowTarget("  /opt/foo/cys --wrap \"$@\"").path).toBeNull();
    expect(shadowTarget("/opt/corp/toolchain/env loaded").path).toBeNull();
  });
  it("상대경로·빈 값·null 도 채택하지 않는다", () => {
    expect(shadowTarget("cys").path).toBeNull();
    expect(shadowTarget("").path).toBeNull();
    expect(shadowTarget(null).path).toBeNull();
    expect(shadowTarget(undefined).label).toBe("(경로 미상)");
  });
  it("★경로가 특정되면 'rm 대상'을 지목하고, 아니면 지목하지 않는다", () => {
    const named = installResultToast(
      installReport({ status: "installed_shadowed", shadowed_by: "/opt/homebrew/bin/cys" }),
    );
    expect(named.body).toContain("그 파일(/opt/homebrew/bin/cys)을 지우거나");

    const noisy = installResultToast(
      installReport({ status: "installed_shadowed", shadowed_by: "/opt/corp/toolchain/env loaded" }),
    );
    expect(noisy.body).not.toContain("을 지우거나");
    expect(noisy.body).toContain("단정하지 않습니다");
    expect(noisy.body).toContain("which -a cys"); // 다음 조치는 여전히 있다
  });
  it("경로 미상이어도 등급은 그대로 watchdog + sticky 다(등급은 status 가 정한다)", () => {
    const t = installResultToast(installReport({ status: "installed_shadowed", shadowed_by: null }));
    expect(t.category).toBe("watchdog");
    expect(t.sticky).toBe(true);
    expect(t.body).toContain("(경로 미상)");
  });
});

// ══════════════════════════════════════════════════════════════════════════════
// ★I7 / N4 — main.ts 배선 핀: 등급색을 **낼 때마다** 다시 못박는가
// ══════════════════════════════════════════════════════════════════════════════
// N4 의 수리 본체는 순수 함수가 아니라 main.ts 의 한 줄이다(재사용 엘리먼트에 className 재적용).
// 예전에는 그 한 줄을 아무도 지키지 않았고, 대신 소비되지 않는 ToastEmit.className 이 "계획이
// 등급색을 정한다"는 거짓 계약만 세워 두었다. 필드를 지운 자리에 **진짜 핀**을 박는다.
const MAIN_SRC = readFileSync(new URL("./main.ts", import.meta.url), "utf8");
function mainFnBody(name: string): string {
  const i = MAIN_SRC.indexOf(`function ${name}(`);
  expect({ 함수: name, 존재: i >= 0 }).toEqual({ 함수: name, 존재: true });
  const end = MAIN_SRC.indexOf("\n}\n", i);
  return MAIN_SRC.slice(i, end > i ? end + 3 : undefined);
}

/// 거대 소스(383KB)를 단언에 **직접** 넣으면 실패 출력이 그 전체를 토해 낸다(실측: 389KB).
/// 판정은 여기서 boolean 으로 접고, 어디를 봤는지는 라벨로 남긴다.
function pinHas(where: string, hay: string, needle: string) {
  expect({ 위치: where, 찾는_배선: needle, 있음: hay.includes(needle) }).toEqual({
    위치: where,
    찾는_배선: needle,
    있음: true,
  });
}
function pinLacks(where: string, hay: string, needle: string) {
  expect({ 위치: where, 없어야_할_배선: needle, 있음: hay.includes(needle) }).toEqual({
    위치: where,
    없어야_할_배선: needle,
    있음: false,
  });
}

describe("main.ts 배선 핀 — 토스트 등급색·CLI 클릭 분기", () => {
  it("(N4) stickyToast 는 낼 때마다 className 을 현재 등급으로 다시 못박는다(재사용 엘리먼트)", () => {
    pinHas("stickyToast", mainFnBody("stickyToast"), "el.className = toastClassName(category)");
  });
  it("volatile toast() 도 같은 함수로 등급색을 정한다(단일 진실)", () => {
    pinHas("toast", mainFnBody("toast"), "toastClassName(category)");
  });
  it("(I7) main.ts 가 사라진 emit.className 을 참조하지 않는다", () => {
    pinLacks("main.ts", MAIN_SRC, "emit.className");
  });
  it("(I2) 클릭 분기가 cliStatus.button 이 아니라 cliButtonIntent 를 쓴다(라벨과 행동 일치)", () => {
    pinHas("main.ts", MAIN_SRC, 'cliButtonIntent(cliStatus.button, cliLastInstall) === "uninstall"');
    pinLacks("main.ts", MAIN_SRC, 'const wantUninstall = cliStatus.button === "installed"');
  });
  it("(I2) 라벨 산출도 같은 래치를 먹는다(툴팁·라벨과 클릭 분기가 한 판정에서 나온다)", () => {
    pinHas("applyCliButtonView", mainFnBody("applyCliButtonView"), "cliButtonView(cliStatus.button, cliNoticeLines(cliStatus), cliLastInstall)");
  });
  it("(I2) 래치는 Control Center 를 열 때 풀린다(해제 경로가 영구히 막히지 않는다)", () => {
    pinHas("setCcOpen", mainFnBody("setCcOpen"), "cliLastInstall = null");
  });
  it("(I3②) 해제 확인 문구에 현재 상태(notes)와 잔존 백업(backups)을 함께 넘긴다", () => {
    pinHas("main.ts", MAIN_SRC, "uninstallConfirmText(cliStatus.notes, cliStatus.backups)");
  });

  // ★G2 — 한 액션에 한 알림. 배선이 본체라 여기에 핀을 박는다(순수 함수만으로는 못 지킨다).
  it("(G2) 액션 직후 재조회는 상시 고지 토스트를 내지 않는다", () => {
    pinHas("main.ts", MAIN_SRC, "refreshCliInstallState({ notice: false })");
  });
  it("(G2) 결과 토스트 하나에 고지 줄을 접어 넣는다(cli-install 과 cli-status-notes 가 겹치지 않는다)", () => {
    pinHas("main.ts", MAIN_SRC, "showCliToast(withCliNotice(plan, cliNoticeLines(cliStatus)))");
  });
  it("(G2) 결과 토스트를 재조회 **전에** 따로 내던 옛 배선이 남아 있지 않다", () => {
    pinLacks("main.ts", MAIN_SRC, "showCliToast(installResultToast(rep))");
    pinLacks("main.ts", MAIN_SRC, "showCliToast(uninstallResultToast(rep))");
  });
  it("(G2) 상시 경로(CC 열기)는 그대로 고지를 낸다 — 억제한 것은 중복뿐이다", () => {
    pinHas("refreshCliInstallState", mainFnBody("refreshCliInstallState"), "statusNoticePlan(cliStatus)");
  });

  // ★R1 — 복구 명령의 조립 후보(링크 경로)를 넘기는 것은 배선이라 순수 함수만으로는 못 지킨다.
  it("(R1) 해제 결과 토스트에 링크 경로를 넘긴다(그래야 'sudo rm <경로>' 가 조립된다)", () => {
    pinHas("main.ts", MAIN_SRC, "uninstallResultToast(rep, [cliStatus.cysLink, cliStatus.cysdLink])");
  });
});
