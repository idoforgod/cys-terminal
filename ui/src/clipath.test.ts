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
//
// ══════════════════════════════════════════════════════════════════════════════
// ★MINOR-6(2026-08-25 9R) — 이 파일은 **배포 문서를 읽는다**. 새 결합이 생겼다.
// ══════════════════════════════════════════════════════════════════════════════
// 무엇이 무엇을 읽는가 (전부 이 파일 안에서 실행 시각에 읽는다 · 생성물 아님):
//
//   이 파일 ──읽음──▶ ../../docs/INSTALL.md      (거울쌍 배터리 · mirrorBattery)
//           ──읽음──▶ ../../USER-MANUAL.md       (거울쌍 배터리 · mirrorBattery)
//           ──읽음──▶ ../../docs/**/*.md + 리포 루트 *.md
//                                                 (코드블록 회귀핀 · scanDocCommands)
//           ──읽음──▶ ./main.ts                   (배선 핀 · MAIN_CODE)
//
// 그래서 **문서만 고쳐도 `bun test` 가 깨질 수 있고, `bun test` 는 릴리스 태그 빌드의 게이트다.**
// 오탈자 교정 한 줄로 배포가 멈출 수 있다는 뜻이다. 그 대가를 알고 받아들였다 — 화면과 문서가
// 갈라지는 것이 이 라운드들에서 반복해 사용자 파일을 파괴한 결함이었고, 사람 눈으로 두 곳을
// 맞추는 방식은 이미 세 번 실패했다(제목 '세 갈래 vs 네 갈래' 드리프트, `mv -n` 뜻풀이의 화면·문서
// 불일치, 해제 표면 세 곳 중 하나만 수리). 다만 **깨졌을 때 어디를 볼지**는 여기 적어 둔다:
//
//   · `★거울쌍 …` 실패      → 화면 상수(clipath.ts)와 그 문서가 다른 말을 한다.
//                              실패 출력의 `문서:` 가 **어느 문서**인지, `제목:`/`낱말:` 이 **어느
//                              문자열**이 빠졌는지 그대로 찍는다. 고칠 곳은 둘 중 하나이지, 테스트가
//                              아니다 — 화면이 옳으면 문서를, 문서가 옳으면 화면 상수를 고친다.
//                              ★상수를 고치면 **두 문서를 같은 커밋에서** 고쳐야 한다(한쪽만 고치면
//                              다른 쪽이 빨개진다 — 그것이 이 핀의 목적이다).
//   · `★전수 — 맨 ln -sf …` 실패 → 어느 문서 코드블록이 "따라 치면 파일이 사라지는" 명령을 얻었다.
//                              출력이 `파일:줄 [갈래] 명령` 을 그대로 찍는다. 고치는 정본은
//                              docs/INSTALL.md §B 의 가드 블록이고, 의도된 비가역이라면 그 절의
//                              `DOC_CMD_ALLOW` 에 **사유와 함께** 올린다(사유 없는 예외는 금지).
//   · `main.ts 배선 핀 …` 실패 → 문서가 아니라 배선이 바뀌었다. 핀은 주석을 걷은 코드만 본다.
//
// 같은 결합이 설계정본(docs/plans/2026-08-25-shell-cli-restore-design.md)에도 적혀 있다 —
// 문서 쪽에서 이 파일로 오는 길과, 이 파일에서 문서로 가는 길을 **양쪽 다** 열어 둔다.
import { describe, it, expect } from "bun:test";
import { readFileSync, readdirSync } from "node:fs";
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
  SKIP_REASON_ABSENT,
  statusNoticePlan,
  statusNoticeKind,
  backupRestoreSpot,
  MV_EMPTY_CAVEAT,
  BACKUP_LAST_COPY_WARNING,
  uninstallConfirmText,
  uninstallResultToast,
  uninstallLeftovers,
  NOTICE_TITLE_FOREIGN,
  NOTICE_TITLE_BACKUP,
  NOTICE_TITLE_INFO,
  NOTICE_TITLE_PARTIAL,
  FOREIGN_BACKUP_NOTICE,
  INSTALL_TOAST_ID,
  UNINSTALL_TOAST_ID,
  CLI_NOTES_TOAST_ID,
  type InstallCliReport,
  type UninstallCliReport,
  type CliInstallStatusReport,
  type CliLinkState,
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


// ══════════════════════════════════════════════════════════════════════════════
// ★MAJOR-2(2026-08-25 8R) — 판독기 **관통** 시험: 기계 필드가 사용자 문구까지 살아 도착하는가
// ══════════════════════════════════════════════════════════════════════════════
// 전 라운드 반증자 실측: `readInstallReport` 의 `unverified_reason` 을 상수 `null` 로,
// `readUninstallReport` 의 `restored` 를 상수 `[]` 로 개악해도 **201/201 초록**이었다. 즉
// MINOR-9(unverified 두 갈래)·I3③(해제 시 원본 복원)을 "닫았다"고 선언한 수리가 회귀로부터
// 전혀 보호되지 않았다.
//
// 원인은 셋이 겹친 것이다:
//   ① 모든 문구 테스트가 픽스처(**이미 타입이 맞는 객체**)를 UI 함수에 직접 먹여 판독기를 우회했다.
//   ② 위 "판독기" describe 는 shadowed_by·status·ok·skipped·warnings 몇 개만 덮었다.
//   ③ expectShape 는 **키 존재와 타입 태그**만 보므로 `null`·`[]` 도 계약을 만족해 통과한다.
//
// 그래서 이 블록의 규칙은 하나다 — **입력은 raw(unknown), 출력은 사용자에게 나가는 문자열**이다.
// 중간에 픽스처를 UI 함수로 직접 넘기지 않는다. 판독기의 한 줄을 죽이면 여기가 먼저 빨개진다.
//
// ★전 필드 전수표(이 diff 가 만든 기계 필드 19종 · (리포트, 필드) 쌍으로는 24개):
//
//   InstallCliReport      status ✔관통 / unverified_reason ✔관통 / shadowed_by ✔관통 /
//                         cys_link ✔관통 / cysd_link ✔관통 / warnings ✔관통 /
//                         ok ▲판독기보존만(UI 소비처 없음 — 등급은 status 가 정한다) /
//                         target_dir ▲ / source_cys ▲ / effective_cys ▲ (셋 다 UI 소비처 0)
//   UninstallCliReport    ok ✔ / removed ✔ / skipped ✔ / skipped_benign ✔ / restored ✔ /
//                         warnings ✔ / skipped_reasons ✔(단 **두 기계 신호가 어긋날 때만**
//                         화면이 달라진다 — 그 갈래를 그대로 못박는다)
//   CliInstallStatusReport platform_supported ✔ / installed ✔ / state ✔ / notes ✔ /
//                         backups ✔ / cys_link ✔(해제 결과의 'sudo rm' 조립) / cysd_link ✔
//
//   ▲ 표시 넷은 **화면에 닿지 않는다**(ui/src 전역 grep 0건). 그 사실을 숨기지 않고, 대신
//     '판독기가 값을 옮긴다'를 양방향으로 못박는다 — 화면 문구를 지어내 관통을 흉내 내면 그것이
//     곧 장식 테스트이기 때문이다. 소비처가 생기는 날 이 자리에 관통 단언을 더한다.

/// raw(unknown) → **판독기** → 설치 결과 토스트의 사용자 노출면 전체. 픽스처를 UI 에 직접 주지 않는다.
const installText = (raw: unknown) => {
  const p = installResultToast(readInstallReport(raw));
  return [p.title, p.body, `category=${p.category}`, `sticky=${p.sticky}`].join("\n");
};
/// raw(unknown) → **판독기** → 해제 결과 토스트의 사용자 노출면 전체.
const uninstallText = (raw: unknown, links: readonly string[] = CLI_LINKS) => {
  const p = uninstallResultToast(readUninstallReport(raw), links);
  return [p.title, p.body, `category=${p.category}`, `sticky=${p.sticky}`].join("\n");
};
/// raw(unknown) → **판독기** → 상태 조회가 만드는 표면 셋(상시 고지 토스트 · 버튼 라벨 · 툴팁).
/// main.ts 의 배선 그대로다: statusNoticePlan(cliStatus) · cliButtonView(button, cliNoticeLines(...)).
const statusText = (raw: unknown) => {
  const v = readCliStatus(raw);
  const n = statusNoticePlan(v);
  const b = cliButtonView(v.button, cliNoticeLines(v), null);
  return [
    n ? [n.title, n.body, `category=${n.category}`].join("\n") : "(무음)",
    `label=${b.label}`,
    b.title,
    `supported=${v.supported}`,
  ].join("\n");
};

describe("★MAJOR-2 관통 — InstallCliReport 의 기계 필드가 설치 토스트까지 도착한다", () => {
  it("status — 세 종단이 각자의 제목으로 갈린다(판독기가 status 를 접으면 전부 '확인 불가'로 무너진다)", () => {
    expect(installText(installReport())).toContain("✅ 셸 설치 완료");
    expect(
      installText(installReport({ ok: false, status: "installed_shadowed", shadowed_by: "/opt/homebrew/bin/cys" })),
    ).toContain("다른 cys가 앞을 가립니다");
    expect(installText(installReport({ ok: false, status: "unverified", unverified_reason: "probe_failed" }))).toContain(
      "⚠ 셸 설치 확인 불가",
    );
  });

  it("★unverified_reason — 두 갈래가 서로 다른 문구로 나간다(MINOR-9/N3 의 회귀 자물쇠)", () => {
    const notOnPath = installText(
      installReport({ ok: false, status: "unverified", unverified_reason: "not_on_path", effective_cys: null, warnings: [WARN_NOT_ON_PATH] }),
    );
    // 원인이 특정된 갈래 — 제목이 원인을 말하고, 본문이 **읽히는 프로필 파일**을 지목한다.
    expect(notOnPath).toContain("PATH에서 cys를 찾지 못했습니다");
    expect(notOnPath).toContain(LOGIN_SHELL_PATH_FILES);
    // 원인 불명 갈래로 떨어지면 이 문장이 나온다 — 그것이 곧 판독기가 필드를 버렸다는 증거다.
    expect(notOnPath).not.toContain("어느 쪽인지는 단정하지 않습니다");

    const probeFailed = installText(
      installReport({ ok: false, status: "unverified", unverified_reason: "probe_failed", effective_cys: null, warnings: [WARN_PROBE_FAILED] }),
    );
    expect(probeFailed).toContain("확인 명령(which -a cys)이 실패했거나 비정상 종료·무응답이라");
    expect(probeFailed).not.toContain("어느 쪽인지는 단정하지 않습니다");

    // 그리고 **정말로 없을 때만** 원인 불명 문구가 나온다(구 백엔드 = 필드 부재).
    expect(installText(installReport({ ok: false, status: "unverified", unverified_reason: null }))).toContain(
      "어느 쪽인지는 단정하지 않습니다",
    );
  });

  it("shadowed_by — 경로가 본문의 파괴적 지시에 들어간다(판독기가 버리면 '(경로 미상)' 로 무너진다)", () => {
    const t = installText(
      installReport({ ok: false, status: "installed_shadowed", shadowed_by: "/opt/homebrew/bin/cys", effective_cys: "/opt/homebrew/bin/cys" }),
    );
    expect(t).toContain("그 파일(/opt/homebrew/bin/cys)을 지우거나");
    expect(t).not.toContain("(경로 미상)");
  });

  it("cys_link · cysd_link — 만들어진 링크 두 경로가 본문에 그대로 나온다", () => {
    const t = installText(installReport({ cys_link: "/usr/local/bin/cys", cysd_link: "/usr/local/bin/cysd" }));
    expect(t).toContain("/usr/local/bin/cys · /usr/local/bin/cysd");
    // 한쪽만 버려도 이 단언이 깨진다(빈 문자열은 filter(Boolean) 에서 사라져 ' · ' 가 없어진다).
  });

  it("warnings — 백엔드 문장이 한 글자도 줄지 않고 도착하고, 등급까지 낮춘다(G14)", () => {
    const t = installText(installReport({ warnings: [WARN_BACKUP] }));
    expect(t).toContain(WARN_BACKUP);
    expect(t).toContain("⚠ 셸 설치 완료 — 확인할 항목이 있습니다"); // ✅ 안에 ⚠ 가 숨지 않는다
    expect(t).toContain("sticky=true");
    // 판독기가 warnings 를 버리면 여기로 무너진다.
    expect(t).not.toContain("✅ 셸 설치 완료\n");
  });

  it("▲ok — UI 소비처가 없다(등급은 status 가 정한다). 그래서 관통 대신 **양방향 보존**을 못박는다", () => {
    // 판독기의 계약은 '있는 그대로 옮긴다' 다 — status 로 다시 계산해 덮지 않는다(둘이 어긋나면
    // 그 사실이 보여야 한다). 상수로 접는 개악은 둘 중 하나에서 반드시 빨개진다.
    expect(readInstallReport(installReport({ ok: true, status: "installed" })).ok).toBe(true);
    expect(readInstallReport(installReport({ ok: false, status: "installed" })).ok).toBe(false);
  });

  it("▲target_dir · source_cys · effective_cys — UI 소비처 0. 판독기 보존만 못박는다(관통 문구 없음)", () => {
    const r = readInstallReport(
      installReport({ target_dir: "/usr/local/bin", source_cys: "/Applications/cys.app/Contents/MacOS/cys", effective_cys: "/usr/local/bin/cys" }),
    );
    expect({ target_dir: r.target_dir, source_cys: r.source_cys, effective_cys: r.effective_cys }).toEqual({
      target_dir: "/usr/local/bin",
      source_cys: "/Applications/cys.app/Contents/MacOS/cys",
      effective_cys: "/usr/local/bin/cys",
    });
    // Option<String> 의 None 쪽도 옮긴다(빈 문자열로 뭉개지 않는다 — 없음과 ''는 다른 사실이다).
    expect(readInstallReport(installReport({ effective_cys: null })).effective_cys).toBe(null);
  });
});

describe("★MAJOR-2 관통 — UninstallCliReport 의 기계 필드가 해제 토스트까지 도착한다", () => {
  const CYS = "/usr/local/bin/cys";
  const CYSD = "/usr/local/bin/cysd";

  it("ok — 등급의 근본 신호. 참/거짓이 서로 다른 제목으로 나간다", () => {
    expect(uninstallText(uninstallReport({ ok: true, removed: [CYS, CYSD] }))).toContain("✅ 셸 cys 해제 완료");
    expect(uninstallText(uninstallReport({ ok: false, removed: [CYS] }))).toContain("⚠ 셸 cys 해제 부분 완료");
  });

  it("removed — 지운 경로가 본문에 나온다(버리면 '지운 것이 없습니다' 로 무너진다)", () => {
    const t = uninstallText(uninstallReport({ ok: true, removed: [CYS, CYSD] }));
    expect(t).toContain(`${CYS} · ${CYSD} 를 제거했습니다`);
    expect(t).not.toContain("해제할 심링크 없음");
  });

  it("skipped — 건너뛴 줄이 **원문 그대로** 나오고 등급을 낮춘다", () => {
    const t = uninstallText(
      uninstallReport({ ok: true, removed: [CYS], skipped: [SKIP_NOT_SYMLINK], skipped_reasons: ["not_symlink"], skipped_benign: false }),
    );
    expect(t).toContain("건너뜀 1건 — 직접 확인하세요:");
    expect(t).toContain(SKIP_NOT_SYMLINK);
    expect(t).toContain("category=watchdog");
  });

  it("★skipped_benign — 같은 skip 이 이 bool 하나로 성공/부분완료를 가른다(양방향)", () => {
    const base = { ok: true, removed: [CYS], skipped: [SKIP_ABSENT], skipped_reasons: [SKIP_REASON_ABSENT] };
    // true = '애초에 지울 게 없었다' → 정상 해제(✅) + 괄호 안내
    const benign = uninstallText(uninstallReport({ ...base, skipped_benign: true }));
    expect(benign).toContain("✅ 셸 cys 해제 완료");
    expect(benign).toContain("(이미 없던 항목:");
    // false = '무해하다고 말할 수 없다' → 조치 필요(⚠). 판독기가 bool 을 상수로 접으면 한쪽이 깨진다.
    const attention = uninstallText(uninstallReport({ ...base, skipped_benign: false }));
    expect(attention).toContain("⚠ 셸 cys 해제 부분 완료");
    expect(attention).toContain("건너뜀 1건");
  });

  it("★skipped_reasons — 줄별 태그. **두 기계 신호가 어긋날 때** 화면이 갈린다(fail-closed 갈래)", () => {
    // 계약대로면 benign=true 는 '모든 skip 이 absent' 다. 그 둘이 어긋난 응답(= 계약 위반·구
    // 백엔드)에서, 줄별 태그가 살아 있으면 foreign_target 한 줄이 blocking 으로 잡혀 ⚠ 가 된다.
    // 태그를 버리면 partitionSkips 가 bool 하나에 맡기고 **정상 해제(✅)로 둔갑**한다 — 그 회귀를
    // 여기서 잡는다. (태그가 성립하는 정상 응답에서는 두 신호가 같은 답을 내므로 화면이 같다.)
    const t = uninstallText(
      uninstallReport({
        ok: true,
        removed: [CYS],
        skipped: [SKIP_ABSENT, SKIP_FOREIGN],
        skipped_reasons: [SKIP_REASON_ABSENT, "foreign_target"],
        skipped_benign: true,
      }),
    );
    expect(t).toContain("⚠ 셸 cys 해제 부분 완료");
    expect(t).toContain(SKIP_FOREIGN);
    expect(t).not.toContain("✅ 셸 cys 해제 완료");
  });

  it("★restored — '되돌린 원본'이 문구까지 도착한다(I3③ 의 회귀 자물쇠)", () => {
    const ok = uninstallText(uninstallReport({ ok: true, removed: [CYS], restored: [CYS], warnings: [WARN_RESTORED] }));
    expect(ok).toContain("설치 때 백업해 둔 원본 1건을 그 자리에 되돌렸습니다");
    // 부분 완료 갈래에도 별도 줄로 나간다(실패 알림만 덜 말하지 않는다 — G1 대칭).
    const partial = uninstallText(
      uninstallReport({ ok: false, removed: [CYS], restored: [CYS], skipped: [SKIP_FOREIGN], skipped_reasons: ["foreign_target"], skipped_benign: false }),
    );
    expect(partial).toContain(`되돌린 원본: ${CYS}`);
  });

  it("warnings — 성공 등급이라도 문장이 그대로 나가고 수명이 sticky 로 올라간다", () => {
    const t = uninstallText(uninstallReport({ ok: true, removed: [CYS], restored: [CYS], warnings: [WARN_RESTORED] }));
    expect(t).toContain("남은 조치·안내:");
    expect(t).toContain(WARN_RESTORED);
    expect(t).toContain("sticky=true");
  });
});

describe("★MAJOR-2 관통 — CliInstallStatusReport 의 기계 필드가 고지·라벨까지 도착한다", () => {
  it("platform_supported — Rust 의 명시 부정만 버튼을 되숨긴다(main.ts `if (!cliStatus.supported)`)", () => {
    expect(statusText(statusReport({ platform_supported: false }))).toContain("supported=false");
    expect(statusText(statusReport())).toContain("supported=true");
    // 판독 실패(응답 없음)로는 숨기지 않는다 — 기능이 조용히 사라지는 쪽이 더 나쁘다.
    expect(readCliStatus(undefined).supported).toBe(true);
  });

  it("installed — 버튼 라벨이 이 bool 하나로 뒤집힌다", () => {
    expect(statusText(statusReport({ installed: true, state: "ours" }))).toContain("label=셸 cys 해제");
    expect(statusText(statusReport({ installed: false, state: "absent" }))).toContain("label=셸에 cys 설치");
  });

  it("★state — 상시 고지의 **제목(=등급 표시)** 이 이 문자열 하나로 갈린다(MAJOR-D · MAJOR-1)", () => {
    const foreign = statusText(statusReport({ state: "foreign", notes: [NOTE_NOT_SYMLINK] }));
    expect(foreign).toContain(NOTICE_TITLE_FOREIGN);
    const partial = statusText(statusReport({ state: "partial", installed: true, notes: [NOTE_NOT_SYMLINK] }));
    expect(partial).toContain(NOTICE_TITLE_PARTIAL);
    expect(partial).toContain("category=watchdog");
    // 판독기가 state 를 "unknown" 으로 접으면 둘 다 중립 제목으로 강등된다 — 그 회귀를 잡는다.
    expect(foreign).not.toContain(NOTICE_TITLE_INFO);
    expect(partial).not.toContain(NOTICE_TITLE_INFO);
  });

  it("★state — 백업 되돌리기 명령을 **낼지 말지**도 이 필드가 정한다(파괴적 지시 게이트)", () => {
    const free = statusText(statusReport({ state: "absent", backups: [BACKUP_PATH] }));
    expect(free).toContain(`되돌리려면 'sudo mv -n ${BACKUP_PATH} /usr/local/bin/cys'`);
    // ★(MAJOR-1 · 10R) 예전 단언은 `not.toContain("ls -l")` 였다. 이제 **버리기** 안내가 어느
    // 갈래에서든 `ls -l <백업본>` 눈확인을 동반하므로 그 단언은 이 절이 보려던 것을 넘어선다.
    // 여기서 보려는 것은 하나다: 자리가 비었다고 아는 상태에서 **원래 자리를 먼저 확인하라는
    // 조건부 안내**(unknown 갈래의 형태)가 섞여 나오지 않는가.
    expect(free).not.toContain("비어 있는지 확인한 뒤");
    expect(free).not.toContain("ls -l /usr/local/bin/cys'");
    const occupied = statusText(statusReport({ state: "ours", installed: true, backups: [BACKUP_PATH] }));
    expect(occupied).toContain("손으로 옮길 수 없습니다");
    expect(occupied).not.toContain("sudo mv");
  });

  it("notes — Rust 산문이 고지 토스트와 버튼 툴팁 **양쪽**에 그대로 도착한다(BLOCK-1(d))", () => {
    const t = statusText(statusReport({ notes: [NOTE_NOT_SYMLINK] }));
    expect(t).not.toContain("(무음)");
    // 두 표면이 같은 문장을 말한다 — 토스트는 60초 뒤 사라지고 툴팁은 상주하므로 둘 다 필요하다.
    // 개수를 못박지 않는다(표면이 늘어도 거짓 실패를 내지 않게) — '둘 다 도달했는가'만 본다.
    expect({ 두_표면_모두_도달: t.split(NOTE_NOT_SYMLINK).length - 1 >= 2 }).toEqual({ 두_표면_모두_도달: true });
  });

  it("backups — 잔존 백업 경로가 UI 조립 문구로 도착한다(I3① · 무음 규약을 깬다)", () => {
    const t = statusText(statusReport({ state: "absent", backups: [BACKUP_PATH] }));
    expect(t).toContain(NOTICE_TITLE_BACKUP);
    expect(t).toContain(BACKUP_PATH);
    // 버리면 notes 도 없으니 무음(null) 이 되어 사용자는 자기 원본의 행방을 영영 듣지 못한다.
    expect(statusText(statusReport())).toContain("(무음)");
  });

  it("★cys_link · cysd_link — 해제 결과의 'sudo rm <경로>' 조립 후보가 여기서 온다(R1 배선)", () => {
    // main.ts: uninstallResultToast(rep, [cliStatus.cysLink, cliStatus.cysdLink]) — 두 값이 판독기를
    // 통과해야 복구 명령이 만들어진다. 판독기가 버리면 후보가 사라져 안내가 통째로 없어진다.
    const v = readCliStatus(statusReport({ installed: true, state: "ours" }));
    const t = uninstallText(uninstallReport({ ok: false, skipped_benign: true, warnings: [WARN_LEFTOVER] }), [v.cysLink, v.cysdLink]);
    expect(t).toContain("아직 남아 있는 링크 2건");
    expect(t).toContain("'sudo rm /usr/local/bin/cys'");
    expect(t).toContain("'sudo rm /usr/local/bin/cysd'");
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
  it("원래 경로를 알고 그 자리가 비어 있으면 복원 명령을, 모르면 확인 안내만 준다", () => {
    // (MAJOR-2) 자리 판정이 free 인 상태(= state "absent")에서만 명령을 그대로 제시한다.
    const known = backupNoticeLine(BACKUP_PATH, "absent");
    expect(known).toContain(`sudo mv -n ${BACKUP_PATH} /usr/local/bin/cys`);
    expect(known).toContain(`sudo rm ${BACKUP_PATH}`);
    // ★(MAJOR-1 · 10R) 여기 단언이 예전에는 `toContain("sudo rm /usr/local/bin/weird")` 였다 —
    // 이름이 우리 백업 규칙과 맞지 않아 **이 앱이 만든 사본이라고 확정할 수 없는** 파일에 대해
    // 앱이 삭제 명령을 조립하는 것을, 테스트가 오히려 **요구**하고 있었다. 같은 함수가 `mv`
    // 쪽에서는 이미 "추측한 경로로는 시키지 않는다"(N13)를 지키고 있었으므로 반쪽 계약이었다.
    const unknown = backupNoticeLine("/usr/local/bin/weird", "absent");
    expect(unknown).not.toContain("sudo mv");
    expect(unknown).not.toContain("sudo rm");
    expect(unknown).toContain("/usr/local/bin/weird"); // 경로는 그대로 보여 준다(정보 소실 금지)
    expect(unknown).toContain("ls -l /usr/local/bin/weird"); // 대신 눈으로 확인하는 길을 준다
  });

  // ★MINOR-N13(5R) — 스탬프는 **epoch 초(숫자)** 다. Rust is_our_backup_name 이 숫자가 아닌 스탬프를
  // 거부하므로, 그런 이름에 UI 가 'sudo mv' 를 제시하면 **앱이 되돌리지 않을 파일**에 대해 복원
  // 명령을 만들어 주는 셈이다(판정=Rust / 안내=UI 가 갈리는 MAJOR-6 과 같은 형태의 격차).
  it("★(N13) 날짜 형식 스탬프는 우리 규칙이 아니다 — Rust 가 거부하는 이름에 mv 를 시키지 않는다", () => {
    const dated = "/usr/local/bin/cys.cys-backup-20260825-101112"; // 문서 예시가 쓰던 가짜 형식
    expect(backupOrigin(dated)).toBeNull();
    expect(backupNoticeLine(dated)).not.toContain("sudo mv");
    // ★(MAJOR-1 · 10R) rm 도 같은 이유로 내지 않는다 — Rust 가 되돌리지 않을 이름에 대해 앱이
    // "지우라"고 말하면, mv 쪽에서 닫은 격차(판정=Rust / 안내=UI)가 rm 쪽으로 그대로 옮겨 간다.
    expect(backupNoticeLine(dated)).not.toContain("sudo rm");
    expect(backupNoticeLine(dated)).toContain(`ls -l ${dated}`); // 정보는 남는다(눈으로 확인)
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
    const merged = withCliNotice(
      base,
      cliNoticeLines({ notes: [], backups: [BACKUP_PATH], linkState: "absent" }),
    );
    expect(merged.id).toBe(base.id);
    expect(merged.body).toContain(base.body);
    expect(merged.body).toContain(BACKUP_PATH);
    expect(merged.body).toContain(`sudo mv -n ${BACKUP_PATH} /usr/local/bin/cys`); // UI 조립물
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
    // ★MAJOR-1(7R) 이 자리가 중립 등급으로 강등돼 있었다 — 경고 등급이어야 한다.
    expect(p!.category).toBe("watchdog");
    expect(p!.title).toBe(NOTICE_TITLE_PARTIAL);
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
    expect(p!.body.split(backupNoticeLine(BACKUP_PATH, "absent")).length - 1).toBe(1);
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
// ★MAJOR-1(7R) — MAJOR-D 수리가 경고 방향을 **반대로 뒤집은** 자리
// ══════════════════════════════════════════════════════════════════════════════
// 6R 의 MAJOR-D 는 "notes 가 있으면 무조건 ⚠ 남의 파일" 이라는 거짓 경고를 없앴다. 그런데 경고를
// `state=="foreign"` 하나로만 좁히는 바람에, **진짜 남의 파일이 있는 다른 종착 상태**(partial)가
// ⚠ 도 경고 테두리도 없는 중립 안내로 강등됐다.
//
// 도달 경로는 가설이 아니다: 설치 스크립트가 `… && {cys링크} && {cysd링크}` 체인이라 cysd 의
// 백업 mv 가 거부되면 "우리 cys 심볼릭 + 남의 실체 cysd 파일" 로 끝난다
// → decide_cli_uninstall = Remove / SkipNotSymlink → classify_cli_links(ours=1, foreign=1) = Partial.
//
// ★master 가 제안한 판정("partial 이면 notes 가 비어 있지 않을 때 남의 것이 있다")은 Rust 실물을
// 읽어 보니 **성립하지 않는다**(main.rs `cli_install_status` notes 조립부): state 가 Ours 또는
// **Partial** 이면 `probe_path_shadows` 가 돌고 `path_shadow_note`·`cysd_shadow_warning` 이 PATH
// 그림자·프로브 실패 문장을 같은 배열에 밀어 넣는다. 그리고 partial 에는 '한쪽이 그냥 없는
// (SkipAbsent)' 갈래가 있어, 남의 파일이 하나도 없이도 notes 가 찰 수 있다. 그 관계로 판정하면
// MAJOR-D 가 없앤 거짓 경고를 partial 자리에 그대로 다시 심는다.
//
// 그래서 등급만 올리고 **제목은 단정하지 않는다**: partial 은 남의 파일이 있든 한쪽이 비었든
// 어느 쪽이든 정상이 아니다. 무엇이 있는지는 본문의 notes 원문이 말한다.
describe("★MAJOR-1 — partial 상태의 등급 강등(제목은 기계 필드가 정한다)", () => {
  // Rust cli_install_status 가 실제로 만드는 문장의 사본(main.rs).
  const NOTE_CYSD_REAL_FILE =
    "/usr/local/bin/cysd — 심볼릭이 아닌 실제 파일이 이미 있습니다(다른 도구 설치본일 수 있어 자동으로 제거하지 않습니다).";
  const NOTE_SHADOW_ONLY =
    "cys 확인 결과: 로그인 셸(zsh) PATH 앞쪽의 다른 cys가 우선합니다: /opt/homebrew/bin/cys. " +
    "새 터미널에서 'cys'를 치면 /usr/local/bin/cys가 아니라 그쪽이 실행됩니다.";

  it("★재현: 우리 cys 심볼릭 + 남의 실체 cysd 파일(partial)이 중립 등급으로 나가지 않는다", () => {
    const p = statusNoticePlan(
      readCliStatus(
        statusReport({ installed: true, state: "partial", notes: [NOTE_CYSD_REAL_FILE] }),
      ),
    );
    expect(p).not.toBeNull();
    // 강등의 세 표식(제목 ⚠ 없음 · 중립 제목 · 중립 테두리색)이 모두 사라졌는가.
    expect({
      등급: p!.category,
      제목: p!.title,
      경고글리프: p!.title.startsWith("⚠"),
    }).toEqual({ 등급: "watchdog", 제목: NOTICE_TITLE_PARTIAL, 경고글리프: true });
    expect(p!.title).not.toBe(NOTICE_TITLE_INFO);
    expect(p!.body).toContain(NOTE_CYSD_REAL_FILE); // 문장은 한 글자도 줄이지 않는다
  });

  it("★그래도 '남의 파일이 있다' 고 단정하지 않는다 — partial 에는 그림자만 있는 갈래가 있다", () => {
    const p = statusNoticePlan(
      readCliStatus(statusReport({ installed: true, state: "partial", notes: [NOTE_SHADOW_ONLY] })),
    );
    // 등급은 올라가되(반쪽 상태 자체가 확인이 필요한 사실이다) 제목은 거짓을 말하지 않는다.
    expect(p!.category).toBe("watchdog");
    expect(p!.title).toBe(NOTICE_TITLE_PARTIAL);
    expect(p!.title).not.toBe(NOTICE_TITLE_FOREIGN);
    expect(p!.title).not.toContain("아닌 cys 파일");
    expect(p!.body).toContain(NOTE_SHADOW_ONLY);
  });

  it("★정상 설치(ours)는 그대로 중립이다 — MAJOR-D 의 수리를 되돌리지 않았다", () => {
    const p = statusNoticePlan(
      readCliStatus(statusReport({ installed: true, state: "ours", notes: [NOTE_SHADOW_ONLY] })),
    );
    expect(p!.title).toBe(NOTICE_TITLE_INFO);
    expect(p!.category).toBe("system");
  });

  // ★전수표 — 4상태(+판독 실패 2종) × notes 유무 × backups 유무. 판정에 들어가는 것은 기계 값
  // 셋뿐이고, 표를 여기 통째로 적어 두면 어느 칸이 바뀌든 diff 로 드러난다.
  it("★statusNoticeKind 전수표: 6상태 × notes 유무 × backups 유무 = 24칸", () => {
    const STATES: CliLinkState[] = ["absent", "ours", "partial", "foreign", "unsupported", "unknown"];
    const got: Record<string, string> = {};
    for (const st of STATES)
      for (const n of [false, true])
        for (const b of [false, true]) got[`${st}|notes=${n}|backups=${b}`] = statusNoticeKind(st, n, b);
    expect(got).toEqual({
      // 알릴 것이 하나도 없으면 어느 상태에서도 무음이다(정상은 말이 없다).
      "absent|notes=false|backups=false": "silent",
      "absent|notes=false|backups=true": "backup",
      "absent|notes=true|backups=false": "info",
      "absent|notes=true|backups=true": "backup",
      "ours|notes=false|backups=false": "silent",
      "ours|notes=false|backups=true": "backup",
      "ours|notes=true|backups=false": "info",
      "ours|notes=true|backups=true": "backup",
      // ★partial 은 알릴 것이 있으면 **무엇이 있든** 경고 갈래다(강등 결함이 닫힌 칸들).
      "partial|notes=false|backups=false": "silent",
      "partial|notes=false|backups=true": "partial",
      "partial|notes=true|backups=false": "partial",
      "partial|notes=true|backups=true": "partial",
      // foreign 은 정의상 우리 것이 하나도 없고 남의 것이 있다 — 단정해도 되는 유일한 칸.
      "foreign|notes=false|backups=false": "silent",
      "foreign|notes=false|backups=true": "foreign",
      "foreign|notes=true|backups=false": "foreign",
      "foreign|notes=true|backups=true": "foreign",
      "unsupported|notes=false|backups=false": "silent",
      "unsupported|notes=false|backups=true": "backup",
      "unsupported|notes=true|backups=false": "info",
      "unsupported|notes=true|backups=true": "backup",
      "unknown|notes=false|backups=false": "silent",
      "unknown|notes=false|backups=true": "backup",
      "unknown|notes=true|backups=false": "info",
      "unknown|notes=true|backups=true": "backup",
    });
  });

  it("★등급 판정이 notes **문구**를 읽지 않는다(산문 파싱 금지 가드 — statusNoticeKind)", () => {
    const body = CLIPATH_CODE.slice(CLIPATH_CODE.indexOf("export function statusNoticeKind"));
    const upto = body.slice(0, body.indexOf("\n}\n") + 3);
    expect(upto.length).toBeGreaterThan(120); // 빈 슬라이스를 검사하는 사고 방지
    for (const banned of [".test(", ".match(", ".includes(", ".search(", ".startsWith(", ".indexOf(", "RegExp"])
      expect({ 금지연산: banned, 사용됨: upto.includes(banned) }).toEqual({ 금지연산: banned, 사용됨: false });
  });

  it("제목은 그 자체로 등급을 말한다 — 경고 셋만 ⚠ 를 단다", () => {
    expect({
      foreign: NOTICE_TITLE_FOREIGN.startsWith("⚠"),
      partial: NOTICE_TITLE_PARTIAL.startsWith("⚠"),
      info: NOTICE_TITLE_INFO.startsWith("⚠"),
    }).toEqual({ foreign: true, partial: true, info: false });
  });
});

// ══════════════════════════════════════════════════════════════════════════════
// ★MAJOR-2(7R) — 앱이 스스로 **파괴적 명령**을 출력하지 않는다
// ══════════════════════════════════════════════════════════════════════════════
// 실행 재현: 잔존 백업본이 있으면 backupNoticeLine 이 무조건 "되돌리려면 'sudo mv <백업본>
// <원래 경로>'" 를 냈다. `<원래 경로>` 가 비어 있는지 검사하지 않았고 `-i`/`-n` 도 없었다.
// 그래서 같은 토스트 본문에 '그 자리에 남의 실체 파일이 있습니다'(notes)와 '거기로 mv 하라'가
// 동시에 실렸고, 지시대로 따른 사용자는 자기 파일을 덮어썼다.
//
// 수리는 두 겹이다: ①명령을 `mv -n` + 뜻풀이로 바꾸고 ②앱이 자리가 차 있다고 **아는** 경우에는
// 명령을 아예 내지 않는다(먼저 정리하라고만 말한다).
/// 어떤 문자열에서든 'sudo mv' 를 찾아 `-n`(덮어쓰기 금지) 없이 나온 자리를 돌려준다.
/// 화면 문구와 문서 예시가 **같은 규칙**으로 검사되도록 모듈 스코프에 둔다.
function unguardedMv(text: string): string[] {
  const bad: string[] = [];
  let i = text.indexOf("sudo mv");
  while (i >= 0) {
    if (!text.startsWith("sudo mv -n ", i)) bad.push(text.slice(i, i + 70));
    i = text.indexOf("sudo mv", i + 1);
  }
  return bad;
}

describe("★MAJOR-2 — 되돌리기 명령이 남의 파일을 덮어쓰게 하지 않는다", () => {
  it("★backupRestoreSpot 전수표 — 자리 판정은 state 하나로만 정해진다", () => {
    const STATES: CliLinkState[] = ["absent", "ours", "partial", "foreign", "unsupported", "unknown"];
    const got: Record<string, string> = {};
    for (const st of STATES) got[st] = backupRestoreSpot(st);
    expect(got).toEqual({
      // Rust classify_cli_links 실측: 두 자리가 모두 없을 때만 Absent, 모두 우리 링크일 때만 Ours.
      absent: "free",
      ours: "occupied",
      // partial·foreign 은 '어느 자리가 비었는지' 를 기계 필드로 가릴 수 없다 — 모르면 모른다고 한다.
      partial: "unknown",
      foreign: "unknown",
      unsupported: "unknown",
      unknown: "unknown",
    });
  });

  it("자리가 비어 있을 때(absent)만 명령을 그대로 제시하고, 그 mv 는 반드시 -n 이다", () => {
    const line = backupNoticeLine(BACKUP_PATH, "absent");
    expect(line).toContain(`sudo mv -n ${BACKUP_PATH} /usr/local/bin/cys`);
    expect(line).toContain(MV_EMPTY_CAVEAT);
    expect(unguardedMv(line)).toEqual([]);
  });

  it("★우리 링크가 자리를 차지한 상태(ours)에서는 mv 를 **아예 내지 않는다**", () => {
    const line = backupNoticeLine(BACKUP_PATH, "ours");
    expect(line).not.toContain("sudo mv");
    expect(line).toContain("해제"); // 대신 앱이 되돌린다는 사실을 말한다(Rust I3③ restored)
    expect(line).toContain("sudo rm"); // 버리는 선택지는 남는다(정보 소실 금지)
  });

  it("★자리를 확정 못 하면(partial·foreign) 먼저 확인하라고 말한다 — 명령은 조건부이고 -n 이다", () => {
    for (const st of ["partial", "foreign", "unknown"] as CliLinkState[]) {
      const line = backupNoticeLine(BACKUP_PATH, st);
      expect({ 상태: st, ls확인: line.includes(`ls -l /usr/local/bin/cys`) }).toEqual({
        상태: st,
        ls확인: true,
      });
      expect({ 상태: st, 무방비mv: unguardedMv(line) }).toEqual({ 상태: st, 무방비mv: [] });
      expect(line).toContain(MV_EMPTY_CAVEAT);
    }
  });

  it("★기본값(인자 없음)은 unknown 이다 — 옛 호출부가 파괴적 기본값을 얻지 않는다(fail-closed)", () => {
    expect(backupNoticeLine(BACKUP_PATH)).toBe(backupNoticeLine(BACKUP_PATH, "unknown"));
    expect(unguardedMv(backupNoticeLine(BACKUP_PATH))).toEqual([]);
  });

  it("★재현: '남의 실체 파일이 있습니다' 와 '거기로 mv 하라' 가 한 본문에 공존하지 않는다", () => {
    // 우리 cys 심볼릭 + 남의 실체 cysd 파일 + cysd 자리의 백업본 — MAJOR-1 과 같은 종착 상태.
    const NOTE_CYSD =
      "/usr/local/bin/cysd — 심볼릭이 아닌 실제 파일이 이미 있습니다(다른 도구 설치본일 수 있어 자동으로 제거하지 않습니다).";
    const CYSD_BACKUP = "/usr/local/bin/cysd.cys-backup-1756089600";
    const p = statusNoticePlan(
      readCliStatus(
        statusReport({
          installed: true,
          state: "partial",
          notes: [NOTE_CYSD],
          backups: [CYSD_BACKUP],
        }),
      ),
    );
    expect(p!.body).toContain(NOTE_CYSD); // 사실은 그대로 남고
    expect(p!.body).toContain(CYSD_BACKUP); // 백업 경로도 그대로 남지만
    expect(unguardedMv(p!.body)).toEqual([]); // 무방비 mv 는 없다
    expect(p!.body).toContain("ls -l /usr/local/bin/cysd"); // 먼저 확인하라고 말한다
  });

  it("★전수: UI 가 만드는 어떤 화면 문자열에도 -n 없는 'sudo mv' 가 없다", () => {
    const STATES: CliLinkState[] = ["absent", "ours", "partial", "foreign", "unsupported", "unknown"];
    const texts: string[] = [FOREIGN_BACKUP_NOTICE];
    for (const st of STATES)
      for (const notes of [[] as string[], [NOTE_NOT_SYMLINK]])
        for (const backups of [[] as string[], [BACKUP_PATH], ["/usr/local/bin/weird"]]) {
          const view = readCliStatus(
            statusReport({ state: st, installed: st === "ours" || st === "partial", notes, backups }),
          );
          const lines = cliNoticeLines(view);
          texts.push(...lines);
          texts.push(cliButtonView(view.button, lines).title);
          texts.push(uninstallConfirmText(notes, backups, st).body);
          const plan = statusNoticePlan(view);
          if (plan) texts.push(plan.title, plan.body);
          texts.push(withCliNotice(installResultToast(installReport()), lines).body);
        }
    texts.push(
      uninstallResultToast(
        uninstallReport({ ok: false, removed: [], warnings: [WARN_RESTORED] }),
        CLI_LINKS,
      ).body,
    );
    const offenders = texts.flatMap(unguardedMv);
    expect({ 무방비_mv_건수: offenders.length, 예시: offenders.slice(0, 3) }).toEqual({
      무방비_mv_건수: 0,
      예시: [],
    });
  });

  it("★해제 잔존 안내의 'sudo rm' 에도 확인 절차가 함께 붙는다(계열 점검)", () => {
    const t = uninstallResultToast(
      uninstallReport({ ok: false, removed: ["/usr/local/bin/cysd"] }),
      CLI_LINKS,
    );
    expect(t.body).toContain("'sudo rm /usr/local/bin/cys'");
    // 지우기 전에 무엇인지 확인하라는 절차가 화면에도 있다(문서에만 있으면 화면이 갈라진다).
    expect(t.body).toContain("ls -l /usr/local/bin/cys");
    expect(t.body).toContain("심볼릭 링크인지");
  });

  it("설치 동의 문구도 같은 안전 명령을 말한다(누르기 전과 누른 뒤가 같은 말)", () => {
    expect(FOREIGN_BACKUP_NOTICE).toContain("sudo mv -n");
    expect(FOREIGN_BACKUP_NOTICE).toContain(MV_EMPTY_CAVEAT);
  });
});

// ══════════════════════════════════════════════════════════════════════════════
// ★MAJOR-1(10R) — 앱이 **마지막 사본을 지우라고** 상시 안내하지 않는다
// ══════════════════════════════════════════════════════════════════════════════
// 9R 까지 backupNoticeLine 의 네 갈래가 전부 맨 `sudo rm <절대경로>` 꼬리를 달고 있었다. 위 MAJOR-2
// 절이 `mv` 쪽에서 닫은 것과 **정확히 같은 결함**이 `rm` 쪽에 그대로 남아 있었던 것이다 — 그리고
// 이 문장은 상시 상태 토스트·버튼 툴팁·해제 확인 창 세 표면 모두에 상주하므로, 한 번 스치는
// 안내가 아니라 **상시 권유**였다.
//
// 검사 규칙은 mv 와 대칭이다: 우리가 화면에 내는 어떤 `sudo rm <경로>` 도, **같은 문자열 안에**
// 그 경로에 대한 `ls -l <경로>`(지우기 전 눈확인)가 함께 있어야 한다. 이 규칙은 이미 존재하던
// 해제 잔존 안내(uninstallResultToast)의 형태와 같으므로, 새 규약을 발명한 것이 아니라 **한 곳에만
// 적용돼 있던 규약을 계열 전체로 넓힌 것**이다.
/// 어떤 문자열에서든 `sudo rm <경로>` 를 찾아, 같은 문자열에 `ls -l <경로>` 가 없는 자리를 돌려준다.
/// (화면 문구는 셸 스크립트가 아니라 한 줄 문장이므로, 가드는 `[ -e ]` 같은 문법이 아니라
///  "지우기 전에 그 자리를 눈으로 보게 하는가" 라는 **절차**로 잰다.)
function unguardedRm(text: string): string[] {
  const bad: string[] = [];
  const NEEDLE = "sudo rm ";
  let i = text.indexOf(NEEDLE);
  while (i >= 0) {
    const rest = text.slice(i + NEEDLE.length);
    // 첫 인자만 읽는다 — 따옴표·공백·줄바꿈에서 끊는다(`'sudo rm /a/b'.` 형태를 그대로 다룬다).
    const path = (rest.split(/['"\s]/)[0] ?? "").replace(/[.,)]+$/, "");
    if (!path || !text.includes(`ls -l ${path}`)) bad.push(text.slice(i, i + 90));
    i = text.indexOf(NEEDLE, i + 1);
  }
  return bad;
}

describe("★MAJOR-1 — 버리기 안내에 가드와 비가역 경고가 함께 붙는다", () => {
  it("★검사기 자체의 변이 — 맨 rm 은 잡고, ls -l 이 함께 있는 형태는 통과한다", () => {
    // 이 헬퍼가 장식이 아님을 먼저 증명한다(9R 이 반복해 지적한 '초록인 채 봉인' 방지).
    expect(unguardedRm("필요 없으면 'sudo rm /usr/local/bin/cys.cys-backup-1'.").length).toBe(1);
    expect(
      unguardedRm("'ls -l /usr/local/bin/cys.cys-backup-1' 로 본 뒤 'sudo rm /usr/local/bin/cys.cys-backup-1'"),
    ).toEqual([]);
    expect(unguardedRm("sudo rm -f /usr/local/bin/cys").length).toBe(1); // 플래그만 있고 눈확인 없음
    expect(unguardedRm("지우는 명령은 드리지 않습니다")).toEqual([]);
  });

  it("★네 갈래 전부 — 비가역 경고가 붙고, 맨 rm 이 없다", () => {
    const STATES: CliLinkState[] = ["absent", "ours", "partial", "foreign"];
    for (const st of STATES) {
      const line = backupNoticeLine(BACKUP_PATH, st);
      expect({
        상태: st,
        마지막사본_경고: line.includes(BACKUP_LAST_COPY_WARNING),
        되돌릴수없음: line.includes("되돌릴 수 없습니다"),
        눈확인: line.includes(`ls -l ${BACKUP_PATH}`),
        무방비rm: unguardedRm(line),
      }).toEqual({ 상태: st, 마지막사본_경고: true, 되돌릴수없음: true, 눈확인: true, 무방비rm: [] });
    }
  });

  it("★이름이 우리 규칙이 아니면 삭제 명령을 **아예 내지 않는다**(fail-closed)", () => {
    for (const bad of ["/usr/local/bin/weird", "/usr/local/bin/cys.cys-backup-20260825-101112", "/usr/local/bin/cys.cys-backup-"]) {
      const line = backupNoticeLine(bad, "absent");
      expect({
        경로: bad,
        rm: line.includes("sudo rm"),
        mv: line.includes("sudo mv"),
        눈확인: line.includes(`ls -l ${bad}`),
        // 비가역이라는 사실은 여기서도 말한다. 다만 '이것은 마지막 사본이다' 라고 **단정하지는
        // 않는다** — 이름을 확정하지 못한 파일에 대해 아는 척하지 않는 것이 이 갈래의 요지다.
        비가역_고지: line.includes("지우면 되돌릴 수 없습니다"),
        단정하지_않음: !line.includes(BACKUP_LAST_COPY_WARNING),
      }).toEqual({ 경로: bad, rm: false, mv: false, 눈확인: true, 비가역_고지: true, 단정하지_않음: true });
    }
  });

  it("★전수: UI 가 만드는 어떤 화면 문자열에도 눈확인 없는 'sudo rm' 이 없다", () => {
    // MAJOR-2 의 mv 전수 시험과 **같은 표면 집합**을 훑는다(설치 동의문구·상시 고지·툴팁·해제
    // 확인 창·결과 토스트). 계열을 지점으로 닫지 않기 위해서다.
    const STATES: CliLinkState[] = ["absent", "ours", "partial", "foreign", "unsupported", "unknown"];
    const texts: string[] = [FOREIGN_BACKUP_NOTICE];
    for (const st of STATES)
      for (const notes of [[] as string[], [NOTE_NOT_SYMLINK]])
        for (const backups of [[] as string[], [BACKUP_PATH], ["/usr/local/bin/weird"], [BACKUP_PATH, "/usr/local/bin/cysd.cys-backup-2"]]) {
          const view = readCliStatus(
            statusReport({ state: st, installed: st === "ours" || st === "partial", notes, backups }),
          );
          const lines = cliNoticeLines(view);
          texts.push(...lines);
          texts.push(cliButtonView(view.button, lines).title);
          texts.push(uninstallConfirmText(notes, backups, st).body);
          const plan = statusNoticePlan(view);
          if (plan) texts.push(plan.title, plan.body);
          texts.push(withCliNotice(installResultToast(installReport()), lines).body);
        }
    for (const ok of [true, false])
      for (const removed of [[], ["/usr/local/bin/cys"]])
        texts.push(uninstallResultToast(uninstallReport({ ok, removed }), CLI_LINKS).body);
    const offenders = texts.flatMap(unguardedRm);
    expect({ 무방비_rm_건수: offenders.length, 예시: offenders.slice(0, 3) }).toEqual({
      무방비_rm_건수: 0,
      예시: [],
    });
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
    const b = uninstallConfirmText([], [BACKUP_PATH], "absent").body;
    expect(b).toContain(BACKUP_PATH);
    expect(b).toContain(`sudo mv -n ${BACKUP_PATH} /usr/local/bin/cys`);
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
// ★거울쌍(7R MINOR-5) — 문서가 **존재하지 않는 테스트**를 보증으로 내세우고 있었다
// ══════════════════════════════════════════════════════════════════════════════
// docs/INSTALL.md 는 "화면과 문서가 같은 말을 하는지 자동 테스트가 지킵니다" 라고 적어 두었는데,
// 리포 전체에 docs 를 **읽는** 테스트가 ts·rs·py 어디에도 없었다(grep 0건). 문서의 합격선은
// '코드와 일치'가 아니라 **'지시대로 하면 문제가 해결된다'** 이고, 화면과 문서가 갈라지는 것이
// 이 라운드들이 반복해 온 결함이다 — 그래서 문장을 지우는 (a) 대신 약속한 테스트를 실제로
// 만드는 (b) 를 택했다. 이제 그 문장은 이 절을 가리킨다.
//
// ★MINOR-5(9R) — 그 대조가 **반쪽**이었다. 7R 판은 `../../docs/INSTALL.md` 하나만 읽었고, 같은
// 사실을 적는 리포 루트 `USER-MANUAL.md` 를 읽는 테스트는 리포 전체에 0건이었다. 가정이 아니다:
// 상시 고지 제목이 '세 갈래'에서 '네 갈래'로 늘었을 때 INSTALL.md 만 고쳐지고 USER-MANUAL.md 는
// '세 갈래'로 남아, 손으로 발견될 때까지 살아 있었다(그 사고 기록이 USER-MANUAL.md 그 문단에 그대로
// 적혀 있다). 한쪽에만 핀을 박으면 정확히 핀이 없는 쪽이 드리프트한다 — 그래서 **같은 배터리**를
// 두 문서에 건다.
//
// 대조는 **낱말 단위**로 한다(문장 전체 일치가 아니라): 문서는 사람이 읽는 산문이라 줄바꿈·강조
// 표기가 자유롭게 바뀌어야 하고, 그때마다 게이트가 빨개지면 아무도 문서를 손보지 않게 된다.
// 대신 "같은 말인지"를 결정하는 **핵심 주장**과 **알림 제목 원문**은 반드시 양쪽에 있어야 한다.

/// 리포 루트(= 이 파일 기준 ../../). 문서 경로는 전부 이 뿌리에서 **리포 상대 경로**로 적는다 —
/// 실패 메시지에 그 경로가 그대로 찍혀야 "어느 파일을 보라"가 사람에게 전달된다.
/// cwd 에 기대지 않는다(`bun test` 를 어느 폴더에서 돌리든 같은 파일을 읽어야 한다).
const REPO_ROOT = new URL("../../", import.meta.url);
const repoUrl = (rel: string) => new URL(rel, REPO_ROOT);

/// 문서는 줄바꿈·들여쓰기로 접히므로 공백을 하나로 눌러 비교한다(제목이 두 줄에 걸쳐도 잡힌다).
/// 인용문(`> `) 안에서 접힌 문장도 같은 문장이다 — 줄머리 인용 표식을 먼저 걷어 낸다.
const flatDoc = (t: string) => t.replace(/^[ \t]*>[ \t]?/gm, "").replace(/\s+/g, " ").trim();

const readRepoDoc = (rel: string) => readFileSync(repoUrl(rel), "utf8");

/// 배포 문서 한 편에 **같은** 거울쌍 배터리를 건다(문서명은 실패 출력에 남는다).
function mirrorBattery(rel: string) {
  const DOC = readRepoDoc(rel);
  const DOC_FLAT = flatDoc(DOC);
  const docHas = (t: string) => DOC_FLAT.includes(flatDoc(t));

  describe(`★거울쌍 — 화면과 ${rel} 가 같은 말을 하는지 실제로 대조한다`, () => {
    it("문서 파일이 실재한다 — 없으면 건너뛰지 않고 실패한다(측정 불능은 통과가 아니다)", () => {
      expect({ 문서: rel, 읽힘: DOC.length > 2000 }).toEqual({ 문서: rel, 읽힘: true });
    });

    it("★상시 고지 제목 네 개가 화면 문자열 그대로 문서에도 적혀 있다", () => {
      const titles: [string, string][] = [
        ["foreign", NOTICE_TITLE_FOREIGN],
        ["partial", NOTICE_TITLE_PARTIAL],
        ["backup", NOTICE_TITLE_BACKUP],
        ["info", NOTICE_TITLE_INFO],
      ];
      for (const [kind, t] of titles)
        expect({ 문서: rel, 갈래: kind, 제목: t, 문서에_있음: docHas(t) }).toEqual({
          문서: rel,
          갈래: kind,
          제목: t,
          문서에_있음: true,
        });
    });

    it("★설치 동의 문구(FOREIGN_BACKUP_NOTICE)의 핵심 주장이 문서에도 있다", () => {
      // [무엇을 약속하는가, 그 약속을 담은 낱말]. 낱말은 화면·문서 **양쪽**에 있어야 한다.
      const claims: [string, string][] = [
        ["남의 실체 파일도 백업한다", "실제 파일"],
        ["남의 심볼릭 링크도 백업한다", "심볼릭 링크"],
        ["지우지 않고 옮긴다", "지우지 않고"],
        ["백업 이름 규칙을 밝힌다", "cys-backup"],
        ["스탬프는 epoch 초다", "epoch"],
        ["되돌리는 명령은 덮어쓰기 금지형이다", "sudo mv -n"],
        ["해제가 자동으로 되돌린다", "되돌립니다"],
      ];
      for (const [why, token] of claims)
        expect({
          문서: rel,
          주장: why,
          낱말: token,
          화면: FOREIGN_BACKUP_NOTICE.includes(token),
          문서에_있음: docHas(token),
        }).toEqual({ 문서: rel, 주장: why, 낱말: token, 화면: true, 문서에_있음: true });
    });

    it("★`-n` 의 뜻풀이가 화면과 문서 양쪽에 있다(가드를 사용자가 스스로 뜯지 않게)", () => {
      expect({ 문서: rel, 화면: FOREIGN_BACKUP_NOTICE.includes(MV_EMPTY_CAVEAT), 문서에_있음: docHas(MV_EMPTY_CAVEAT) }).toEqual(
        { 문서: rel, 화면: true, 문서에_있음: true },
      );
    });

    it("★문서가 화면의 '언제 명령을 내고 언제 내지 않는가' 규칙을 그대로 적고 있다", () => {
      // 화면 쪽 실제 동작(backupRestoreSpot)의 세 갈래가 문서에도 그대로 있어야 한다.
      expect({
        문서: rel,
        비었을때: docHas("sudo mv -n"),
        차있을때: docHas("아예 내지 않"),
        모를때: docHas("ls -l <원래 경로>"),
      }).toEqual({ 문서: rel, 비었을때: true, 차있을때: true, 모를때: true });
    });
  });
}

mirrorBattery("docs/INSTALL.md");
mirrorBattery("USER-MANUAL.md");

describe("★거울쌍 — 화면이 정말 그렇게 동작한다(문서만 맞고 화면이 다르면 대조는 무의미하다)", () => {
  it("backupNoticeLine 의 세 갈래가 문서가 약속한 그대로다", () => {
    expect(backupNoticeLine(BACKUP_PATH, "absent")).toContain("sudo mv -n");
    expect(backupNoticeLine(BACKUP_PATH, "ours")).not.toContain("sudo mv");
    expect(backupNoticeLine(BACKUP_PATH, "partial")).toContain("ls -l /usr/local/bin/cys");
  });

  it("★docs/INSTALL.md 의 실행 예시(코드블록)에 -n 없는 'sudo mv' 가 없다", () => {
    // 산문에는 "예전에는 `-n` 없는 sudo mv 를 안내했다" 는 과거 서술이 남아 있어야 하므로,
    // 검사 대상은 **따라 치게 되어 있는 코드블록**뿐이다.
    const blocks = readRepoDoc("docs/INSTALL.md").split("```").filter((_, i) => i % 2 === 1);
    expect(blocks.length).toBeGreaterThan(3); // 코드블록을 하나도 못 찾는 사고 방지
    expect({ 무방비_mv: blocks.flatMap(unguardedMv) }).toEqual({ 무방비_mv: [] });
  });
});

// ══════════════════════════════════════════════════════════════════════════════
// ★MINOR-5(9R) — 배포 문서 코드블록 회귀핀(docs/ 전체 + 리포 루트 .md)
// ══════════════════════════════════════════════════════════════════════════════
// 위 거울쌍은 "문서가 화면과 같은 말을 하는가" 를 본다. 이 절은 다른 것을 본다 —
// **문서가 따라 치면 파일이 사라지는 명령을 시키는가.**
//
// 이 라운드들에서 실제로 일어난 일이다. docs/INSTALL.md 의 수동 폴백이 맨 `sudo ln -sfn …` 를
// 주고 있었고(그 자리에 있던 남의 실체 `cys` 가 백업 없이 소멸), docs/GUIDE-clean-reset-KR.md 의
// '초보자용 완전 초기화' 블록이 맨 `sudo rm -f /usr/local/bin/cys …` 를 주고 있었다(같은 순간
// GUI 해제 버튼은 '남의 파일일 수 있다'며 거부했다 — **세 해제 표면 중 하나만 지웠다**).
// 두 자리 모두 손으로 찾아 손으로 고쳤고, 그때마다 **다른 문서에 같은 형태가 또 있었다.**
//
// 그래서 사람 눈이 아니라 기계가 전수로 본다. 판정 규칙은 셋이고, 전부 "그 자리에 있는 것이
// 우리 것인지 보지 않고 파괴하는가" 하나를 묻는다:
//   ① `ln -sf`/`ln -sfn` — `-f` 는 그 자리에 **실제 파일**이 있어도 묻지 않고 지우고 링크로 갈아
//      끼운다. 문서 코드블록에 맨 채로 있으면 실패.
//   ② 절대경로를 그대로 적은 `rm` — `/usr/local/bin/cys` 라는 **이름**은 그것이 우리 것이라는
//      보장이 되지 못한다(Homebrew·직접 빌드본이 같은 자리에 있다).
//   ③ `-n` 없는 절대경로 `mv` — 목적지가 차 있으면 말없이 덮어쓴다.
//
// **가드 인정 기준은 버튼과 같은 두 검사다**: 같은 코드블록 안에 `[ -L ` 와 `readlink` 가 함께
// 있으면(=①심볼릭인가 ②그 링크가 우리 번들을 가리키는가) 그 블록은 통과한다. docs/INSTALL.md §B
// 의 설치·해제 블록이 정확히 그 형태이고, 그것이 이 프로젝트의 **정본**이다 — 정본까지 빨개지는
// 핀은 아무도 쓰지 않으므로 그 형태는 반드시 통과해야 한다(아래 합성 대조군으로 증명한다).
//
// 검사 대상은 **셸 코드블록**뿐이다: 펜스 태그가 없거나 sh·bash·zsh·shell·console 인 블록.
// ```rust·```ts 처럼 구현을 인용한 블록과 산문·인라인 백틱은 '따라 치는 자리'가 아니다(거기까지
// 잡으면 오탐이 잦아지고, 오탐이 잦은 게이트는 결국 꺼진다).

type DocCmdKind = "ln" | "rm" | "mv" | "rmitem";
type DocCmdHit = { line: number; kind: DocCmdKind; code: string };

const SHELLISH_FENCE = new Set(["", "sh", "bash", "zsh", "shell", "console"]);
/// ★(MINOR-6 · 10R) PowerShell 블록. 태그 없는 블록은 셸일 수도 PowerShell 일 수도 있으므로 양쪽에 든다.
const PWSH_FENCE = new Set(["", "powershell", "pwsh", "ps", "ps1"]);

/// 줄 전체 주석(`#…`)과 줄 끝 주석(` #…`)을 걷는다. 과거 서술("예전에는 `sudo rm …` 였습니다")이
/// 코드블록 주석으로 남는 것은 정상이고, 그것까지 잡으면 이력을 지우게 된다.
function stripShellComment(line: string): string {
  if (line.trimStart().startsWith("#")) return "";
  const i = line.indexOf(" #");
  return i >= 0 ? line.slice(0, i) : line;
}

/// 명령 한 조각(파이프·`&&`·`;` 로 끊은 단위)을 판정한다. 명령 **위치**에 있을 때만 본다 —
/// 백틱·따옴표 안의 같은 글자는 명령이 아니다(`sudo ln -sf …` 를 치지 말라는 경고문이 그 형태다).
function classifyDocCommand(seg: string): DocCmdKind | null {
  const t = seg.trim().replace(/^sudo\s+(-\S+\s+)*/, "");
  const m = /^(ln|rm|mv)\s+(.+)$/.exec(t);
  if (!m) return null;
  const argv = (m[2] as string).split(/\s+/).filter(Boolean);
  const flags = argv.filter((a) => a.startsWith("-")).join("");
  const absolute = argv.some((a) => a.startsWith("/"));
  if (m[1] === "ln") return flags.includes("s") && flags.includes("f") ? "ln" : null;
  if (m[1] === "rm") return absolute ? "rm" : null;
  return absolute && !flags.includes("n") ? "mv" : null;
}

/// ★(MINOR-6 · 10R) PowerShell 의 '묻지 않고 트리를 지운다' 형태 — `Remove-Item … -Recurse -Force`.
/// 두 매개변수가 **함께** 있을 때만 본다(하나만으로는 정상 정리 명령이 흔하다: 리포 코드블록 실측
/// 4줄 중 3줄이 `-Force` 단독이고 전부 파일 하나를 지운다). PowerShell 은 매개변수 이름을 앞부분만 적어도
/// 받으므로(`-rec`·`-fo`·`-r`) **접두 일치**로 판정한다 — 축약형으로 빠져나가지 못하게.
///
/// 셸 `rm` 과 달리 경로 형태(절대·상대)를 보지 않는다. PowerShell 쪽에는 이 핀이 '가드'로 인정할
/// 수 있는 형태가 아직 정의돼 있지 않아서, 경로가 무엇이든 그 조합 자체를 신호로 쓴다.
function classifyPwshCommand(seg: string): DocCmdKind | null {
  const t = seg.trim();
  if (!/^Remove-Item\b/i.test(t)) return null;
  let recurse = false;
  let force = false;
  for (const a of t.split(/\s+/).slice(1)) {
    if (!a.startsWith("-")) continue;
    const tok = a.slice(1).replace(/:.*$/, "").toLowerCase();
    if (tok.length > 0 && "recurse".startsWith(tok)) recurse = true;
    if (tok.length > 0 && "force".startsWith(tok)) force = true;
  }
  return recurse && force ? "rmitem" : null;
}

/// 문서 한 편의 **셸 코드블록**만 훑어 위 세 갈래를 찾는다(순수 — 문자열만 먹는다).
function scanDocCommands(text: string): DocCmdHit[] {
  const hits: DocCmdHit[] = [];
  const lines = text.split("\n");
  let open = false;
  let tag = "";
  let block: { n: number; raw: string }[] = [];
  const flush = () => {
    const shellish = SHELLISH_FENCE.has(tag);
    const pwshish = PWSH_FENCE.has(tag);
    if (!shellish && !pwshish) return;
    const body = block.map((b) => b.raw).join("\n");
    // 버튼과 같은 두 검사가 **명령 안에** 있으면 그 블록은 가드된 것이다(§B 정본의 형태).
    // ★(MINOR-6) 이 면제는 **셸 갈래에만** 적용한다 — 그 두 검사는 PowerShell 명령에 대해
    //   아무것도 말해 주지 않는다(가드 인정 규칙은 언어마다 따로 있어야 한다).
    const guarded = body.includes("readlink") && body.includes("[ -L ");
    for (const { n, raw } of block)
      for (const seg of stripShellComment(raw).split(/&&|\|\||[;|]/)) {
        const kind =
          (shellish && !guarded ? classifyDocCommand(seg) : null) ??
          (pwshish ? classifyPwshCommand(seg) : null);
        if (kind) hits.push({ line: n, kind, code: raw.trim().slice(0, 120) });
      }
  };
  for (let i = 0; i < lines.length; i++) {
    const s = (lines[i] as string).trimStart();
    if (s.startsWith("```")) {
      if (open) {
        flush();
        open = false;
        tag = "";
        block = [];
      } else {
        open = true;
        tag = s.slice(3).trim().toLowerCase();
        block = [];
      }
      continue;
    }
    if (open) block.push({ n: i + 1, raw: lines[i] as string });
  }
  return hits;
}

/// 검사 대상 = `docs/` 아래 **모든** .md + 리포 루트의 .md. 목록을 손으로 적지 않는다 —
/// 8R 이 네 파일만 지정했다가 다섯 번째 파일(GUIDE-clean-reset-KR.md)이 사용자 파일을 파괴한
/// 것이 이 핀을 만든 이유다.
function listRepoMarkdown(): string[] {
  const out: string[] = [];
  const walk = (rel: string) => {
    for (const e of readdirSync(repoUrl(rel), { withFileTypes: true })) {
      if (e.isDirectory()) walk(`${rel}${e.name}/`);
      else if (e.name.endsWith(".md")) out.push(`${rel}${e.name}`);
    }
  };
  walk("docs/");
  for (const e of readdirSync(REPO_ROOT, { withFileTypes: true }))
    if (e.isFile() && e.name.endsWith(".md")) out.push(e.name);
  return out.sort();
}

/// 예외는 **그 예외가 필요한 문서 자신** 안에 적는다 — 머리에 한 줄:
///
///   <!-- doc-command-pin: allow(unguardedLn, unguardedRm) — reason: … -->
///
/// ★왜 테스트 파일의 목록이 아니라 문서 안인가: 이 라운드들이 반복해서 낸 결함이 **정본 이원화**다
/// (같은 절차가 두 곳에 있으면 한쪽만 고쳐진다). 예외를 테스트 파일에 모아 두면, 문서를 읽는
/// 사람은 자기가 보는 블록이 면제됐다는 사실을 **알 수 없고**, 문서가 지워지거나 이름이 바뀌어도
/// 목록은 남아 유령 항목이 된다. 예외는 면제받는 그 자리에 있어야 한다.
///
/// 규칙 셋(전부 아래 테스트가 집행한다):
///   ① `reason:` 이 없거나 30자 미만이면 **마커 자체가 실패**다(사유 없는 예외는 구멍이다).
///   ② 마커는 `docs/plans/` 아래 **이력·설계 문서**에만 놓을 수 있다. 사용자가 따라 치는 문서
///      (docs/INSTALL.md·GUIDE-*·README·USER-MANUAL 등)에 마커가 붙는 것은 그 자체가 결함이다 —
///      거기서 필요한 것은 면제가 아니라 **명령 수리**다.
///   ③ 마커를 단 파일 수에 상한을 둔다. 예외가 늘어나는 것을 조용히 넘기지 않는다.
///
/// 현재 마커를 가진 문서는 `docs/plans/2026-06-29-cli-path-install.md` 하나다(2026-06-29 에
/// 완료되고 2026-08-25 설계정본이 상위대체한 이력 문서 — 원문을 보존해야 "왜 그때 그렇게
/// 결정했는가"를 되물을 수 있다).
const DOC_PIN_MARKER = /doc-command-pin:\s*allow\(([^)]*)\)([\s\S]{0,400}?)(?:-->|$)/;
const MARKER_KIND: Record<string, DocCmdKind> = {
  unguardedln: "ln",
  unguardedrm: "rm",
  unguardedmv: "mv",
  unguardedremoveitem: "rmitem",
};

/// 문서 머리의 면제 마커를 읽는다(없으면 null). 사유는 `reason:` 뒤 전부.
function readDocPinMarker(text: string): { kinds: DocCmdKind[]; reason: string } | null {
  const m = DOC_PIN_MARKER.exec(text);
  if (!m) return null;
  const kinds = (m[1] as string)
    .split(",")
    .map((k) => MARKER_KIND[k.trim().toLowerCase()])
    .filter((k): k is DocCmdKind => !!k);
  const r = /reason:\s*([\s\S]*)$/.exec(m[2] as string);
  return { kinds, reason: (r?.[1] ?? "").replace(/\s+/g, " ").trim() };
}

// ★알려진 한계 — 9R 판은 여기에 **변수 경로 하나만** 적어 두고 "일부러 그렇게 뒀다"로 끝냈다.
// 그것이 사각의 전부인 것처럼 읽혔지만 아니었다. 10R 실측으로 확인한 사각은 셋이고, 아래가 그
// 전부다(하나는 이 라운드에서 닫았고, 둘은 못 닫았다 — 못 닫은 것은 이유까지 적는다).
//
//   ① **홈 상대경로 — 못 잡는다.** `rm -rf ~/.cys ~/.local/state/cys …` 는 통과한다.
//      `classifyDocCommand` 가 인자 중 하나라도 `/` 로 시작할 때만 `rm` 을 잡기 때문이다.
//      범위를 넓히지 **않았다**: 지금 docs/GUIDE-clean-reset-KR.md 의 완전 초기화 절차가 정확히
//      그 형태로 세 줄(launchd plist · 데이터 폴더 · 웹뷰 저장값) 있고, 그 셋은 문서가 사용자에게
//      **시키려는 절차 자체**다. 이들을 잡으려면 홈 경로용 '가드 인정' 규칙이 먼저 있어야 하는데,
//      그런 규칙은 아직 없다 — §B 정본의 `[ -L ]`+`readlink` 는 **링크 자리**에 대한 검사이지
//      데이터 폴더 삭제에 대한 검사가 아니다. 규칙 없이 범위만 넓히면 정본이 빨개지고, 정본이
//      빨개지는 핀은 결국 꺼진다. 이 사각은 다른 통제가 덮는다: 그 절차가 앱의 보존 계약
//      (`src/factory_reset.rs` — 미등록 파일·`local` 오버레이 보존)보다 넓게 지운다는 경고를
//      문서 자신이 지고 있어야 한다(10R MINOR-5).
//   ② **PowerShell — 이 라운드에서 닫았다.** `Remove-Item … -Recurse -Force`(매개변수 접두
//      축약 포함)를 ```powershell·```pwsh·태그 없는 블록에서 잡는다(`classifyPwshCommand`).
//      넓힐 때 오탐이 없는지 리포 전체로 확인했다 — 현재 .md 전수 결과 이 갈래 0건이고, 코드블록
//      안의 기존 `Remove-Item` 4줄(전부 docs/WINDOWS-UPGRADE-ATOMICITY-CHECKLIST.md · 단일 파일
//      정리)은 `-Recurse` 가 없어 걸리지 않는다.
//   ③ **변수 경로 — 못 잡는다.** `D=/usr/local/bin/cys` 로 받아 `rm -f "$D"` 로 지우면 보지
//      못한다. 그대로 둔다: 세 번의 실제 사고가 전부 **붙여넣는 한 줄**(`sudo ln -sfn <절대>
//      <절대>` · `sudo rm -f <절대> <절대>` · `sudo mv <절대> <절대>`)의 형태였고, 변수까지
//      좇으려면 이 리포의 가드 블록들(백업 이름 규칙 검사 · `CFBundleIdentifier` 검사 ·
//      `[ -e ]`+`[ -L ]` 검사)이 전부 걸려 '가드 인정' 규칙을 한없이 느슨하게 만들어야 한다.
//      예외가 많은 게이트는 결국 꺼진다. 닫아야 할 사정이 생기면 **가드 인정 규칙을 먼저 정하고**
//      그다음에 범위를 넓혀라.
//
// ★9R 전수 스윕에서 예외 후보로 올라온 둘은 **면제가 아니라 수리로** 닫혔다(기록):
//   · docs/INSTALL.md "필요 없으면 버리기" — 맨 `sudo rm <절대경로>` → 백업 이름 규칙 검사
//     (`case "$B" in /usr/local/bin/cys.cys-backup-[0-9]*`) + 존재 검사 + 변수화.
//   · docs/GUIDE-clean-reset-KR.md ④ 앱 삭제 — 맨 `sudo rm -rf /Applications/cys.app` →
//     `CFBundleIdentifier` 가 `com.cysjavis.terminal` 일 때만 지우는 블록으로 교체.

describe("★MINOR-5(9R) — 배포 문서 코드블록에 파괴적 명령이 다시 들어오지 못한다", () => {
  const FILES = listRepoMarkdown();

  it("대상 파일을 실제로 열거했다 — 0건이면 핀이 아니라 장식이다", () => {
    expect({
      문서_20편_이상: FILES.length >= 20,
      정본_포함: FILES.includes("docs/INSTALL.md") && FILES.includes("USER-MANUAL.md"),
      하위폴더_포함: FILES.some((f) => f.startsWith("docs/plans/")),
    }).toEqual({ 문서_20편_이상: true, 정본_포함: true, 하위폴더_포함: true });
  });

  it("★전수 — 맨 `ln -sf`/`ln -sfn`, 가드 없는 절대경로 `rm`, `-n` 없는 절대경로 `mv`, PowerShell `Remove-Item -Recurse -Force` 가 없다", () => {
    const offenders: string[] = [];
    for (const rel of FILES) {
      const text = readRepoDoc(rel);
      const allow = readDocPinMarker(text)?.kinds ?? [];
      for (const h of scanDocCommands(text))
        if (!allow.includes(h.kind)) offenders.push(`${rel}:${h.line} [${h.kind}] ${h.code}`);
    }
    expect({ 위반: offenders }).toEqual({ 위반: [] });
  });

  // ★면제 마커가 **살아 있는 약속**인지 여기서 집행한다. 문서에 적힌 마커를 이 핀이 읽지 않으면
  // 그 주석은 '있지도 않은 테스트를 보증으로 내세우는' 바로 그 형태가 된다(7R MINOR-5 와 같은 결함).
  it("★면제 마커 — 사유가 붙어 있고, 이력·설계 문서(docs/plans/)에만 있고, 수가 늘지 않았다", () => {
    const marked = FILES.map((rel) => ({ rel, m: readDocPinMarker(readRepoDoc(rel)) })).filter(
      (x): x is { rel: string; m: NonNullable<ReturnType<typeof readDocPinMarker>> } => x.m !== null,
    );
    expect({
      // ① 사유 없는 마커는 마커가 아니다.
      사유가_부실한_문서: marked.filter((x) => x.m.reason.length < 30).map((x) => x.rel),
      // ② 사용자가 따라 치는 문서에는 면제가 없다 — 거기서 필요한 것은 수리다.
      사용자_문서에_붙은_마커: marked.filter((x) => !x.rel.startsWith("docs/plans/")).map((x) => x.rel),
      // ③ 아무 갈래도 열지 않는 마커(오타)는 조용히 통과하지 않는다.
      갈래를_못_읽은_마커: marked.filter((x) => x.m.kinds.length === 0).map((x) => x.rel),
      // ④ 예외가 조용히 늘어나지 않게.
      마커_셋_이내: marked.length <= 3,
    }).toEqual({
      사유가_부실한_문서: [],
      사용자_문서에_붙은_마커: [],
      갈래를_못_읽은_마커: [],
      마커_셋_이내: true,
    });
  });

  it("★마커 파서 자체의 변이 — 사유 없는 마커·오타 갈래를 실제로 걸러낸다", () => {
    const 정상 = readDocPinMarker(
      "<!-- doc-command-pin: allow(unguardedLn, unguardedRm) — reason: 상위대체된 이력 문서라 원문을 보존해야 이력으로서 값이 있다. 실행 정본은 docs/INSTALL.md §B 다. -->",
    );
    expect({ 갈래: 정상?.kinds, 사유가_30자_이상: (정상?.reason.length ?? 0) >= 30 }).toEqual({
      갈래: ["ln", "rm"],
      사유가_30자_이상: true,
    });
    expect(readDocPinMarker("<!-- doc-command-pin: allow(unguardedLn) -->")?.reason).toBe("");
    expect(readDocPinMarker("<!-- doc-command-pin: allow(전부) — reason: 아무거나 -->")?.kinds).toEqual([]);
    expect(readDocPinMarker("면제 마커가 없는 평범한 문서")).toBeNull();
  });

  // ── 변이시험 — 이 핀이 실제로 빨개지는가 ────────────────────────────────────
  // 핀은 "지금 초록"이 아니라 "잘못이 들어오면 빨개짐"으로 증명된다. 이 라운드들에서 초록인 채로
  // 결함을 봉인한 테스트가 여럿 있었으므로(계약 드리프트 픽스처·주석 우회), 합성 변이를 상주시킨다.
  it("★변이 — 실제로 사고를 낸 세 형태를 그대로 넣으면 셋 다 잡힌다", () => {
    const 변이 = [
      "```sh",
      "sudo ln -sfn /Applications/cys.app/Contents/MacOS/cys /usr/local/bin/cys",
      "sudo rm -f /usr/local/bin/cys /usr/local/bin/cysd",
      "sudo mv /usr/local/bin/cys.cys-backup-1756089600 /usr/local/bin/cys",
      "```",
    ].join("\n");
    expect(scanDocCommands(변이).map((h) => h.kind)).toEqual(["ln", "rm", "mv"]);
  });

  it("★변이 — `&&` 로 이어 붙여도, 태그 없는 펜스여도 잡힌다(우회 형태)", () => {
    const 변이 = ["```", "mkdir -p /usr/local/bin && ln -sf $SRC/cys /usr/local/bin/cys", "```"].join("\n");
    expect(scanDocCommands(변이).map((h) => h.kind)).toEqual(["ln"]);
  });

  it("★대조군 — §B 정본(가드 블록)은 통과한다", () => {
    const 정본 = [
      "```sh",
      "sudo sh -c '",
      "for d in /usr/local/bin/cys /usr/local/bin/cysd; do",
      '  if [ -L "$d" ]; then',
      '    case "$(readlink "$d")" in',
      '      */cys.app/Contents/MacOS/cys|*/cys.app/Contents/MacOS/cysd) rm -f "$d" ;;',
      "    esac",
      "  fi",
      '  ln -sfn "$SRC/$f" "$d" || exit 1',
      "done",
      "'",
      "```",
    ].join("\n");
    expect(scanDocCommands(정본)).toEqual([]);
  });

  it("★대조군 — 산문·주석·비셸 블록은 잡지 않는다(오탐이 잦은 게이트는 결국 꺼진다)", () => {
    // ① 코드블록 밖의 경고 산문(문서가 "치지 마세요" 라고 말하는 바로 그 형태)
    expect(scanDocCommands("⚠ 맨 `sudo ln -sfn …` 를 단독으로 치지 마세요.")).toEqual([]);
    // ② 코드블록 안의 과거 서술 주석
    expect(scanDocCommands("```sh\n# 예전에는 sudo rm /usr/local/bin/cys 한 줄이었습니다\n```")).toEqual([]);
    // ③ 구현을 인용한 비셸 블록
    expect(scanDocCommands('```rust\nformat!("ln -sf {c} {tc}")\n```')).toEqual([]);
    // ④ 변수 인자 — 절대경로를 그대로 적은 것만 본다(위 알려진 한계 ③ · **사각이다**)
    expect(scanDocCommands('```sh\nrm -f "$d"\nmv "$d" "$b"\n```')).toEqual([]);
    // ⑤ 가드가 붙은 `mv -n`
    expect(scanDocCommands("```sh\nsudo mv -n /usr/local/bin/cys.cys-backup-1 /usr/local/bin/cys\n```")).toEqual([]);
  });

  // ── ★MINOR-6(10R) 알려진 한계를 **시험으로** 고정한다 ─────────────────────────
  // 주석에 "못 잡는다" 라고 적어 두기만 하면, 나중에 누군가 범위를 넓혔을 때 그 주석이 조용히
  // 거짓이 된다(이 라운드가 반복해 닫는 병의 문서판이다). 사각도 대조군으로 못박아, 사각이
  // 사각이 아니게 되는 순간 이 시험이 빨개져 주석을 함께 고치게 만든다.
  it("★사각 ① 홈 상대경로는 아직 못 잡는다 — 주석과 실물이 같은 말을 한다", () => {
    expect(scanDocCommands("```bash\nrm -rf ~/.cys ~/.local/state/cys\n```")).toEqual([]);
    expect(scanDocCommands("```bash\nrm -f ~/Library/LaunchAgents/com.cysjavis.cysd.plist\n```")).toEqual([]);
  });

  it("★사각 ② PowerShell 은 이 라운드에서 닫았다 — `-Recurse -Force` 조합을 잡는다", () => {
    const 변이 = ["```powershell", 'Remove-Item "$env:LOCALAPPDATA\\cys" -Recurse -Force', "```"].join("\n");
    expect(scanDocCommands(변이).map((h) => h.kind)).toEqual(["rmitem"]);
    // 축약형으로 빠져나가지 못한다(PowerShell 은 매개변수 앞부분만 적어도 받는다).
    expect(scanDocCommands("```pwsh\nRemove-Item C:\\cys -rec -fo\n```").map((h) => h.kind)).toEqual(["rmitem"]);
    // 태그 없는 블록에 적어도 잡는다(펜스 태그로 빠져나가지 못한다).
    expect(scanDocCommands("```\nRemove-Item $HOME\\.cys -Recurse -Force\n```").map((h) => h.kind)).toEqual(["rmitem"]);
  });

  it("★그 확장이 오탐을 내지 않는다 — 리포에 실재하는 `Remove-Item` 형태는 통과한다", () => {
    // 실물 인용(docs/WINDOWS-UPGRADE-ATOMICITY-CHECKLIST.md · docs/RELEASE_NOTES_0.14.19.md).
    const 실물 = [
      "```powershell",
      "Remove-Item $log -ErrorAction SilentlyContinue",
      'Remove-Item "$INST\\cys.exe" -Force',
      'Remove-Item "$INST\\cys.exe","$INST\\cys-install-failure.txt" -Force -ErrorAction SilentlyContinue',
      "Get-Process cys,cysd,cys-app -ErrorAction SilentlyContinue | Stop-Process -Force",
      "```",
    ].join("\n");
    expect(scanDocCommands(실물)).toEqual([]);
    // 산문 속 되돌리기 안내(코드블록 밖)도 잡지 않는다.
    expect(scanDocCommands("되돌리기: `Remove-Item $HOME\\.cys\\win-no-alt-screen`.")).toEqual([]);
  });

  it("★사각 ③ 변수 경로는 아직 못 잡는다 — 주석과 실물이 같은 말을 한다", () => {
    expect(scanDocCommands('```sh\nD=/usr/local/bin/cys\nrm -f "$D"\n```')).toEqual([]);
  });
});

// ══════════════════════════════════════════════════════════════════════════════
// ★I7 / N4 — main.ts 배선 핀: 등급색을 **낼 때마다** 다시 못박는가
// ══════════════════════════════════════════════════════════════════════════════
// N4 의 수리 본체는 순수 함수가 아니라 main.ts 의 한 줄이다(재사용 엘리먼트에 className 재적용).
// 예전에는 그 한 줄을 아무도 지키지 않았고, 대신 소비되지 않는 ToastEmit.className 이 "계획이
// 등급색을 정한다"는 거짓 계약만 세워 두었다. 필드를 지운 자리에 **진짜 핀**을 박는다.
const MAIN_SRC = readFileSync(new URL("./main.ts", import.meta.url), "utf8");
/// ★(MINOR · 2026-08-25 8R) 핀은 **주석을 걷어낸 코드**만 본다.
///
/// 반증자 실측: `refreshCliInstallState({ notice: false })` 와 `showCliToast(withCliNotice(…))`
/// 배선을 지우면서 그 줄을 `// was: …` 주석으로 남기자, G2 계약(한 액션에 한 알림)이 실제로
/// 깨졌는데도 201/201 초록이었다. 'was:' 주석은 실제 리팩터에서 흔한 형태이므로 이 우회는
/// 가정이 아니다. 같은 파일이 clipath.ts 쪽에서는 이미 CLIPATH_CODE 로 주석을 걷고 검사하면서
/// main.ts 계열 핀 7개에만 그 처리를 하지 않은 **비대칭**이었다 — 자리를 하나로 맞춘다.
///
/// (한 줄짜리 블록 주석만 걷는다: main.ts 의 `/* … */` 는 실측상 전부 한 줄이고, 여러 줄을 걷는
///  정규식은 문자열·정규식 리터럴을 삼키는 사고를 만들 수 있다.)
///
/// ★그리고 **줄 끝 주석까지** 걷는다. 전줄 주석만 걷던 첫 판은 변이시험에서 살아남았다:
///   `refreshCliInstallState(), // was: refreshCliInstallState({ notice: false, force: true }),`
/// 처럼 지운 배선을 **같은 줄 뒤에** 남기면 바늘이 주석 안에서 발견돼 초록이었다. 그것이 실제
/// 리팩터에서 가장 흔한 형태이므로, 그 자리를 열어 두면 가드가 아니라 장식이다.
///
/// `//` 를 주석으로 볼지는 **따옴표 밖인지**로 판정한다(문자열 속 `https://` 오탐 방지):
/// 앞부분의 홑·겹따옴표·백틱 개수가 각각 짝수여야 코드 문맥으로 본다. 그리고 `//` 바로 앞이
/// 공백이어야 한다 — `://`·정규식의 `\/\/` 를 주석으로 오인하지 않기 위해서다.
function stripLineComment(line: string): string {
  for (let i = 0; i < line.length - 1; i++) {
    if (line[i] !== "/" || line[i + 1] !== "/") continue;
    const head = line.slice(0, i);
    const even = (c: string) => (head.split(c).length - 1) % 2 === 0;
    if (!even('"') || !even("'") || !even("`")) continue; // 문자열 안 — 코드가 아니다
    if (i > 0 && !/\s/.test(line[i - 1] as string)) continue; // `://`·`\/\/` 오탐 방지
    return head;
  }
  return line;
}
const MAIN_CODE = MAIN_SRC.replace(/\/\*[^\n]*?\*\//g, "")
  .split("\n")
  .map(stripLineComment)
  .join("\n");
function mainFnBody(name: string): string {
  const i = MAIN_CODE.indexOf(`function ${name}(`);
  expect({ 함수: name, 존재: i >= 0 }).toEqual({ 함수: name, 존재: true });
  const end = MAIN_CODE.indexOf("\n}\n", i);
  return MAIN_CODE.slice(i, end > i ? end + 3 : undefined);
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

/// CLI 설치·해제 버튼의 **클릭 핸들러 한 덩어리**(주석 걷은 코드). 순서를 보는 핀은 반드시 이
/// 범위 안에서만 봐야 한다 — 파일 전체(383KB)에서 두 바늘의 위치를 비교하면, 다른 곳의 같은
/// 문자열이 순서를 대신 통과시켜 준다(그것이 MINOR-7 의 검사 쪽 결함이었다).
function cliClickHandler(): string {
  const head = 'document.getElementById("btn-install-cli")?.addEventListener("click"';
  const i = MAIN_CODE.indexOf(head);
  expect({ 핸들러: "btn-install-cli click", 존재: i >= 0 }).toEqual({
    핸들러: "btn-install-cli click",
    존재: true,
  });
  const end = MAIN_CODE.indexOf("\n});\n", i);
  expect({ 핸들러: "btn-install-cli click", 끝을_찾음: end > i }).toEqual({
    핸들러: "btn-install-cli click",
    끝을_찾음: true,
  });
  const body = MAIN_CODE.slice(i, end + 4);
  // 빈 조각·파일 끝까지 통째로 삼킨 조각을 검사하는 사고 방지(둘 다 핀을 장식으로 만든다).
  expect({ 핸들러_길이_적정: body.length > 800 && body.length < 12000 }).toEqual({
    핸들러_길이_적정: true,
  });
  return body;
}

// ══════════════════════════════════════════════════════════════════════════════
// ★MAJOR-3(10R) — 등급색 핀이 **부분 문자열 검사**라 우회됐다(변이 M8)
// ══════════════════════════════════════════════════════════════════════════════
// 8R 은 이 핀의 바늘을 `"toastClassName(category)"` 에서 `"el.className = toastClassName(category)"`
// 로 좁히고 "이 계열을 닫았다"고 적었다. **닫히지 않았다.** 반증자 재현(M8):
//
//   main.ts 6098 의 `el.className = toastClassName(category);` 를 **지우지 않고**
//   바로 다음 줄에 `el.className = "toast";` 를 추가한다.
//
// 앱 전역 volatile 토스트의 등급색(테두리색 = 등급을 표시하는 유일한 장치)이 전부 죽는데, 찾던
// 부분 문자열은 여전히 그 자리에 **있으므로** 핀은 초록이었다. 부분 문자열 검사는 "그 줄이 있는가"
// 만 묻고 "그 줄이 **끝까지 유효한가**" 는 묻지 못한다 — 마지막 대입이 이긴다는 것이 JS 의 규칙이다.
//
// 그래서 검사를 **마지막 대입이 무엇인가** 로 바꾼다. 그리고 대입이 하나뿐임까지 못박는다:
// 같은 값을 두 번 대입하는 코드는 정상 코드에 없고, 둘 이상이면 그 자체가 이 결함의 형태다.
//
// ★알려진 한계(정직): 이 핀은 main.ts 를 **읽는다**(실행하지 않는다). 앱 엔트리는 최상위에서
// DOM 을 만지므로 테스트에서 import 할 수 없다. 그래서 "실제 DOM 요소의 최종 className" 이 아니라
// "본문 안의 최종 className **대입식**" 을 본다. 이 함수 밖에서(예: 부모가 나중에) 클래스를 바꾸는
// 형태는 여전히 보지 못한다 — 그 경우는 el 이 붙는 컨테이너 쪽 핀이 따로 필요하다.
/// 함수 본문에서 `<식별자>.className = <식>` 대입을 **나온 순서대로** 뽑는다(주석은 이미 걷혔다).
function classNameAssignments(body: string): { target: string; value: string }[] {
  const out: { target: string; value: string }[] = [];
  const re = /(\w+)\.className\s*=\s*([^;\n]+)/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(body)) !== null)
    out.push({ target: m[1] as string, value: (m[2] as string).trim() });
  return out;
}

/// 토스트 엘리먼트의 등급색 대입이 **하나뿐이고, 그 하나가 등급 함수** 인가.
/// 덧붙여 className 을 우회해 지우는 형태(classList·setAttribute·removeAttribute)도 막는다.
function pinFinalClassName(fn: string) {
  const body = mainFnBody(fn);
  const asgn = classNameAssignments(body);
  expect({
    함수: fn,
    className_대입: asgn.map((a) => `${a.target}.className = ${a.value}`),
    클래스_우회: [".classList", 'setAttribute("class"', 'removeAttribute("class"'].filter((n) =>
      body.includes(n),
    ),
  }).toEqual({
    함수: fn,
    className_대입: ["el.className = toastClassName(category)"],
    클래스_우회: [],
  });
}

describe("main.ts 배선 핀 — 토스트 등급색·CLI 클릭 분기", () => {
  it("(N4) stickyToast 의 **마지막** className 대입이 현재 등급이다(재사용 엘리먼트)", () => {
    pinFinalClassName("stickyToast");
  });
  it("★(M8) volatile toast() 도 마찬가지 — 뒤에 한 줄 덧대는 우회가 통하지 않는다", () => {
    pinFinalClassName("toast");
  });
  it("★검사기 자체의 변이 — M8 형태(뒤에 덧댄 대입)를 실제로 읽어낸다", () => {
    // 핀이 장식이 아님을 핀 자신이 증명한다. 아래 두 본문은 **부분 문자열 검사로는 구별되지
    // 않는다**(둘 다 `el.className = toastClassName(category)` 를 포함한다).
    const 정상 = 'el.className = toastClassName(category);\nbox.appendChild(el);';
    const 변이 = 'el.className = toastClassName(category);\nel.className = "toast";\nbox.appendChild(el);';
    expect(정상.includes("el.className = toastClassName(category)")).toBe(true);
    expect(변이.includes("el.className = toastClassName(category)")).toBe(true); // ← 옛 핀이 초록이던 이유
    expect(classNameAssignments(정상).map((a) => a.value)).toEqual(["toastClassName(category)"]);
    expect(classNameAssignments(변이).map((a) => a.value)).toEqual([
      "toastClassName(category)",
      '"toast"',
    ]);
  });
  it("(I7) main.ts 가 사라진 emit.className 을 참조하지 않는다", () => {
    pinLacks("main.ts", MAIN_CODE, "emit.className");
  });
  it("(I2) 클릭 분기가 cliStatus.button 이 아니라 cliButtonIntent 를 쓴다(라벨과 행동 일치)", () => {
    pinHas("main.ts", MAIN_CODE, 'cliButtonIntent(cliStatus.button, cliLastInstall) === "uninstall"');
    pinLacks("main.ts", MAIN_CODE, 'const wantUninstall = cliStatus.button === "installed"');
  });
  it("(I2) 라벨 산출도 같은 래치를 먹는다(툴팁·라벨과 클릭 분기가 한 판정에서 나온다)", () => {
    pinHas("applyCliButtonView", mainFnBody("applyCliButtonView"), "cliButtonView(cliStatus.button, cliNoticeLines(cliStatus), cliLastInstall)");
  });
  it("(I2) 래치는 Control Center 를 열 때 풀린다(해제 경로가 영구히 막히지 않는다)", () => {
    pinHas("setCcOpen", mainFnBody("setCcOpen"), "cliLastInstall = null");
  });
  it("(I3②) 해제 확인 문구에 현재 상태(notes)와 잔존 백업(backups)을 함께 넘긴다", () => {
    pinHas(
      "main.ts",
      MAIN_CODE,
      "uninstallConfirmText(cliStatus.notes, cliStatus.backups, cliStatus.linkState)",
    );
  });

  // ★G2 — 한 액션에 한 알림. 배선이 본체라 여기에 핀을 박는다(순수 함수만으로는 못 지킨다).
  it("(G2) 액션 직후 재조회는 상시 고지 토스트를 내지 않는다", () => {
    pinHas("main.ts", MAIN_CODE, "refreshCliInstallState({ notice: false, force: true })");
  });
  it("(G2) 결과 토스트 하나에 고지 줄을 접어 넣는다(cli-install 과 cli-status-notes 가 겹치지 않는다)", () => {
    pinHas("main.ts", MAIN_CODE, "showCliToast(withCliNotice(plan, cliNoticeLines(cliStatus)))");
  });
  it("(G2) 결과 토스트를 재조회 **전에** 따로 내던 옛 배선이 남아 있지 않다", () => {
    pinLacks("main.ts", MAIN_CODE, "showCliToast(installResultToast(rep))");
    pinLacks("main.ts", MAIN_CODE, "showCliToast(uninstallResultToast(rep))");
  });
  it("(G2) 상시 경로(CC 열기)는 그대로 고지를 낸다 — 억제한 것은 중복뿐이다", () => {
    pinHas("refreshCliInstallState", mainFnBody("refreshCliInstallState"), "statusNoticePlan(cliStatus)");
  });

  // ★R1 — 복구 명령의 조립 후보(링크 경로)를 넘기는 것은 배선이라 순수 함수만으로는 못 지킨다.
  it("(R1) 해제 결과 토스트에 링크 경로를 넘긴다(그래야 'sudo rm <경로>' 가 조립된다)", () => {
    pinHas("main.ts", MAIN_CODE, "uninstallResultToast(rep, [cliStatus.cysLink, cliStatus.cysdLink])");
  });
});

// ══════════════════════════════════════════════════════════════════════════════
// ★8R — 핀 자체가 장식이 아님을 먼저 증명한다(주석 걷어내기가 코드를 먹지 않았는가)
// ══════════════════════════════════════════════════════════════════════════════
describe("main.ts 핀의 검사 대상 — 주석만 걷고 코드는 남았다", () => {
  it("MAIN_CODE 가 여전히 우리가 못박는 배선을 담고 있다(빈 문자열을 검사하는 사고 방지)", () => {
    for (const anchor of [
      "function refreshCliInstallState(",
      "el.className = toastClassName(category)",
      'document.getElementById("btn-install-cli")?.addEventListener("click"',
    ])
      expect({ 앵커: anchor, 남음: MAIN_CODE.includes(anchor) }).toEqual({ 앵커: anchor, 남음: true });
    // 통째로 지워지는 사고(정규식이 코드를 삼킴)를 길이로도 막는다 — 주석 비율은 실측 40% 안팎이다.
    expect({ 남은_비율_50퍼_이상: MAIN_CODE.length > MAIN_SRC.length * 0.5 }).toEqual({
      남은_비율_50퍼_이상: true,
    });
  });
  it("★그리고 주석은 실제로 걷혔다 — 'was:' 주석 우회가 더는 통하지 않는다", () => {
    // 이 파일 자신이 그 우회를 재현해 본다: 주석으로 남긴 배선은 MAIN_CODE 에 없어야 한다.
    const 위장 = "  // was: refreshCliInstallState({ notice: false, force: true }),\n";
    const 걷힌 = 위장.replace(/^[ \t]*\/\/.*$/gm, "");
    expect({ 주석_잔존: 걷힌.includes("refreshCliInstallState") }).toEqual({ 주석_잔존: false });
  });
});

// ══════════════════════════════════════════════════════════════════════════════
// ★MAJOR-2(8R) — 판독기는 **IPC 경계 위에** 있다(관통 시험이 진짜 경로를 묘사하는 근거)
// ══════════════════════════════════════════════════════════════════════════════
// 위 관통 시험은 "raw → 판독기 → 문구" 를 검사한다. 그 사슬이 실제 앱의 사슬과 같으려면,
// main.ts 가 invoke 응답을 **판독기에 먼저** 통과시켜야 한다. 그 한 줄을 여기서 못박는다.
// (예전 결함의 본체가 정확히 `as T ?? {}` 캐스트였다 — 캐스트는 모양을 검사하지 않는다.)
describe("★MAJOR-2 — 세 IPC 응답이 모두 판독기를 거친다(캐스트 우회 금지)", () => {
  it("설치 응답", () => {
    pinHas("main.ts", MAIN_CODE, 'readInstallReport(await invoke("install_cli_to_path"))');
  });
  it("해제 응답", () => {
    pinHas("main.ts", MAIN_CODE, 'readUninstallReport(await invoke("uninstall_cli_from_path"))');
  });
  it("상태 조회 응답", () => {
    pinHas("refreshCliInstallState", mainFnBody("refreshCliInstallState"), 'readCliStatus(await invoke("cli_install_status"))');
  });
});

// ══════════════════════════════════════════════════════════════════════════════
// ★MAJOR-3(8R) — async 전환이 연 동시성을 배선으로 닫는다
// ══════════════════════════════════════════════════════════════════════════════
// 7R 이 Rust `cli_install_status` 를 async 로 내리면서 IPC 핸들러의 직렬화가 사라졌다.
// 수리의 본체가 main.ts 의 배선이라 순수 함수로는 못 지킨다 — 핀으로 못박는다.
describe("★MAJOR-3 — 상태 조회 동시성 가드(중복 억제 + 세대 카운터)", () => {
  const BODY = () => mainFnBody("refreshCliInstallState");

  it("진행 중이면 새 프로브(로그인 셸)를 띄우지 않는다 — 관측 재진입은 버린다", () => {
    pinHas("refreshCliInstallState", BODY(), "if (cliStatusBusy && !opts.force) return;");
  });

  it("★세대 카운터의 다섯 조각이 전부 제자리에 있다(last-writer-wins 차단의 재료)", () => {
    pinHas("refreshCliInstallState", BODY(), "const gen = ++cliStatusGen;");
    pinHas("refreshCliInstallState", BODY(), "if (gen !== cliStatusGen) return;");
    // ★(MAJOR-2 · 10R) 8R 주석은 이 자리를 "가드가 장식이 되는 **유일한** 형태를 막는다" 고
    // 단언했다. **거짓이었다.** `cliStatus = readCliStatus(await invoke` 는 그 병의 한 가지
    // 표현일 뿐이고, 병 자체는 '검사보다 먼저 쓰기가 일어난다' 는 **순서** 문제다. 지역 변수로
    // 받아도 대입을 검사 앞으로 한 줄 올리면 그대로 되살아난다(아래 M9 절이 그 자리를 닫는다).
    // 이 다섯 바늘이 보는 것은 재료의 존재일 뿐, 조립 순서가 아니다 — 그 사실을 여기 적어 둔다.
    pinLacks("refreshCliInstallState", BODY(), "cliStatus = readCliStatus(await invoke");
    pinHas("refreshCliInstallState", BODY(), "view = readCliStatus(await invoke");
    pinHas("refreshCliInstallState", BODY(), "cliStatus = view;");
  });

  // ══════════════════════════════════════════════════════════════════════════
  // ★MAJOR-2(10R) — 순서를 **소스에서 뽑아 실제 비동기로 흘려 본다**(변이 M9)
  // ══════════════════════════════════════════════════════════════════════════
  // 반증자 재현(M9): `cliStatus = view;` 를 `if (gen !== cliStatusGen) return;` **앞으로** 한 줄
  // 옮기면 last-writer-wins 가 되살아나는데, 위 다섯 바늘은 전부 그 자리에 남아 있어 초록이었다.
  //
  // main.ts 는 앱 엔트리라 테스트에서 import 할 수 없다(최상위에서 DOM 을 만지고 invoke 를 부른다).
  // 그래서 부분 문자열을 하나 더 늘리는 대신, 본문에서 **여섯 단계의 순서를 뽑아** 그 순서대로 두
  // 요청을 진짜 비동기로 흘려 본다. 판정은 지표의 대소 비교가 아니라 결과다 —
  // **먼저 시작하고 늦게 끝난 옛 요청이 최신 상태를 덮었는가.**
  //
  // 모델의 범위(정직): `if (cliStatusBusy && !opts.force) return;` 은 넣지 않는다. 여기서 재현하는
  // 것은 **force 로 억제를 건너뛰는 두 요청**(설치·해제 직후의 재조회)이 겹치는 실제 경로이고,
  // 세대 카운터는 바로 그 경로를 위해 존재하기 때문이다. 억제 갈래는 위 별도 핀이 지킨다.
  type RefreshStep = "gen" | "busyOn" | "await" | "guard" | "busyOff" | "commit";
  const REFRESH_NEEDLES: [RefreshStep, string][] = [
    ["gen", "const gen = ++cliStatusGen;"],
    ["busyOn", "cliStatusBusy = true;"],
    ["await", "view = readCliStatus(await invoke"],
    ["guard", "if (gen !== cliStatusGen) return;"],
    ["busyOff", "cliStatusBusy = false;"],
    ["commit", "cliStatus = view;"],
  ];

  /// 본문에서 여섯 단계를 찾아 **나온 순서대로** 돌려준다. 하나라도 없거나 두 번 나오면 실패다
  /// (없으면 모델이 실물과 다르고, 두 번이면 어느 자리를 흉내 내는지 말할 수 없다).
  function refreshSteps(): RefreshStep[] {
    const body = BODY();
    const found = REFRESH_NEEDLES.map(([step, needle]) => {
      const i = body.indexOf(needle);
      expect({ 단계: step, 있음: i >= 0, 중복: i >= 0 && body.indexOf(needle, i + 1) >= 0 }).toEqual({
        단계: step,
        있음: true,
        중복: false,
      });
      return { step, i };
    });
    return found.sort((a, b) => a.i - b.i).map((f) => f.step);
  }

  type RefreshWorld = { gen: number; busy: boolean; status: string };
  /// 뽑은 순서를 그대로 실행한다. `await` 단계에서 실제로 지연이 들어가므로, 두 요청을 겹쳐
  /// 돌리면 완료 순서가 시작 순서와 **뒤바뀐다**(그것이 이 가드가 존재하는 이유다).
  async function runRefresh(
    steps: readonly RefreshStep[],
    world: RefreshWorld,
    view: string,
    delayMs: number,
  ): Promise<void> {
    let mine = 0;
    for (const step of steps) {
      if (step === "gen") mine = ++world.gen;
      else if (step === "busyOn") world.busy = true;
      else if (step === "await") await new Promise((r) => setTimeout(r, delayMs));
      else if (step === "guard") {
        if (mine !== world.gen) return; // 내 세대가 더는 최신이 아니다 — 아무것도 쓰지 않고 물러난다
      } else if (step === "busyOff") world.busy = false;
      else world.status = view;
    }
  }

  it("★(M9) 늦게 끝난 옛 요청이 최신 상태를 덮지 않는다 — 소스의 순서를 그대로 흘려 본 결과", async () => {
    const steps = refreshSteps();
    const world: RefreshWorld = { gen: 0, busy: false, status: "초기" };
    const 옛요청 = runRefresh(steps, world, "옛 응답", 40); // 먼저 시작 · 늦게 끝난다
    const 새요청 = runRefresh(steps, world, "새 응답", 5); // 나중에 시작 · 먼저 끝난다
    await Promise.all([옛요청, 새요청]);

    // ★결과와 순서를 **한 단언**에 담는다. 따로 두면 순서 단언이 먼저 터져 정작 중요한 것
    // (낡은 응답이 최신을 덮었다는 사실)이 출력에서 사라진다 — 실패 메시지도 설계 대상이다.
    expect({ 최종_상태: world.status, 프로브_표시: world.busy, 본문에서_읽은_순서: steps }).toEqual({
      최종_상태: "새 응답",
      프로브_표시: false,
      본문에서_읽은_순서: ["gen", "busyOn", "await", "guard", "busyOff", "commit"],
    });
  });

  it("★모델 자체의 변이 — 대입을 검사 앞으로 옮기면 이 시험이 실제로 빨개진다", () => {
    // 위 시험이 '순서에 반응하는가' 를 시험 자신이 증명한다. M9 을 손으로 재현한 순서를 넣고
    // 같은 모델을 돌려, **덮어쓰기가 일어나는지** 확인한다(여기서는 그것이 기대값이다).
    const 변이순서: RefreshStep[] = ["gen", "busyOn", "await", "commit", "guard", "busyOff"];
    const world: RefreshWorld = { gen: 0, busy: false, status: "초기" };
    const p = Promise.all([
      runRefresh(변이순서, world, "옛 응답", 40),
      runRefresh(변이순서, world, "새 응답", 5),
    ]);
    return p.then(() => {
      expect({ 변이를_넣었을_때_최종_상태: world.status }).toEqual({
        변이를_넣었을_때_최종_상태: "옛 응답", // ← 낡은 응답이 최신을 덮는다 = 위 시험이 빨개진다
      });
    });
  });

  it("액션 직후 재조회만 억제를 건너뛴다(force) — 라벨·고지 줄이 낡지 않게", () => {
    pinHas("main.ts", MAIN_CODE, "refreshCliInstallState({ notice: false, force: true })");
    // 상시 경로(CC 열기)는 force 가 없다 — 여기에 force 를 붙이면 억제가 통째로 무력해진다.
    pinHas("setCcOpen", mainFnBody("setCcOpen"), "void refreshCliInstallState();");
  });

  it("★타이머·폴링을 새로 만들지 않았다(이 코드베이스의 상시 원칙)", () => {
    pinLacks("refreshCliInstallState", BODY(), "setInterval");
    pinLacks("refreshCliInstallState", BODY(), "setTimeout");
  });

  // ★MINOR-7(9R) — 이 순서 핀이 **전역 indexOf** 였다.
  //
  // 오버레이(.modal-overlay)는 마우스만 가린다 — 포커스는 방금 누른 버튼에 남고, 전역 키
  // 핸들러는 수식키 없는 입력을 흘려보내므로(`if (!mod) return`) Enter/Space 로 재진입한다.
  // 그래서 `b.disabled = true` 는 **첫 await 앞**에 있어야 한다. 8R 이 그렇게 고쳤는데, 검사는
  // 파일 **전체**에서 두 바늘의 위치를 비교하고 있었다: 파일 어딘가 다른 곳에 같은 문자열이
  // 생기면 그쪽이 순서를 대신 통과시켜 주고, 정작 이 핸들러 안에서 가드가 다시 모달 뒤로
  // 내려가도 초록일 수 있다. 검사 범위를 **그 핸들러 한 덩어리**로 좁힌다.
  //
  // 그리고 바늘을 `await confirmModal(...)` 에서 **첫 await 무엇이든**으로 바꾼다 — 해제 경로만
  // 지키면 설치 경로는 다시 열린다(같은 가드를 경로마다 다른 자리에 두는 것이 8R 이 지적한
  // 어긋남의 본체였다). 계열로 닫는다.
  it("★재진입 차단이 **첫 await 앞**에 있다 — 해제·설치 두 경로 모두(계열)", () => {
    const H = cliClickHandler();
    const disabled = H.indexOf("b.disabled = true;");
    const firstAwait = H.indexOf("await ");
    expect({
      가드_존재: disabled >= 0,
      첫_await_존재: firstAwait >= 0,
      가드가_먼저: disabled >= 0 && firstAwait >= 0 && disabled < firstAwait,
    }).toEqual({ 가드_존재: true, 첫_await_존재: true, 가드가_먼저: true });
    // 그 첫 await 가 실제로 확인 모달이다(우리가 재진입을 막으려는 바로 그 창).
    expect(H.slice(firstAwait, firstAwait + 40)).toContain("confirmModal");
  });

  it("★핸들러 진입 자체가 disabled 를 먼저 본다(모달이 떠 있는 동안 다시 들어와도 무효)", () => {
    pinHas("btn-install-cli click", cliClickHandler(), "if (!b || b.disabled) return;");
  });

  it("확인 창을 취소하면 버튼을 되살린다(영구 비활성 금지)", () => {
    pinHas("btn-install-cli click", cliClickHandler(), "      b.disabled = false;\n      return;");
  });

  // ★MINOR-7(9R) 이중 방어 — 호출부 가드는 버튼 하나를 지키고, 이 줄은 confirmModal 을 쓰는
  // **모든 자리**를 지킨다. 포커스가 확인 창 밖(방금 누른 버튼)에 남아 있는 한, 같은 재진입은
  // 다른 버튼에서 언제든 다시 열린다 — 지점이 아니라 계열로 닫는다.
  it("★확인 창이 포커스를 모달 안(취소 쪽)으로 가져간다 — 키보드 재진입의 뿌리", () => {
    pinHas("confirmModal", mainFnBody("confirmModal"), 'ov.querySelector(".modal-no") as HTMLElement).focus()');
  });
});
