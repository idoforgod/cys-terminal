// '셸에 cys 설치 / 셸 cys 해제' 버튼의 순수 판정 — 플랫폼 노출·버튼 라벨·결과 토스트 등급을 여기서만 정한다.
//
// main.ts의 #btn-install-cli 핸들러는 이 함수들에 배선만 하고(invoke 호출·DOM 갱신), 플랫폼
// 판별에 쓸 userAgent 문자열도 호출측이 넘긴다(테스트 격리 — shellquote.ts 규약 계승).
// 순수 함수라 실제 관리자 승격(osascript) 없이 결정론 회귀 테스트가 가능하다(clipath.test.ts).
//
// 이력: 2026-06-29 신설 → 2026-08-20(3685af9) 버튼 제거 → 결함 4종(플랫폼 게이팅 없음·그림자화를
// 성공으로 보고·해제 경로 없음·검증 실패를 성공으로 접음) 수리와 함께 복원
// → 2026-08-25 계약 드리프트 수리(MAJOR-5·BLOCK-1(d)·MINOR-9·MINOR-10).
//
// ══════════════════════════════════════════════════════════════════════════════
// ★★ Rust 확정 계약 (src-tauri/src/main.rs — serde rename 없음 = snake_case 그대로 JS 노출)
// ══════════════════════════════════════════════════════════════════════════════
// 이 파일의 타입은 **Rust 실물의 사본**이다. 추측·편의로 모양을 바꾸지 않는다.
//
//   cli_install_status()      -> CliInstallStatusReport
//        platform_supported: bool / installed: bool / state: String
//        ("absent"|"ours"|"partial"|"foreign"|"unsupported")
//        cys_link: String / cysd_link: String / notes: Vec<String>
//
//   uninstall_cli_from_path() -> UninstallCliReport
//        ok: bool / removed: Vec<String> / skipped: Vec<String>("경로 — 사유") / warnings: Vec<String>
//        ※ skipped 는 **문자열 배열**이다. 예전 TS 는 {path,reason}[] 로 읽고 `.filter(s => s.path)`
//          로 걸러 **모든 skip 을 소멸**시켰고(부분 실패가 성공 토스트로 둔갑), warnings·ok 는 타입에
//          아예 없어 유일한 복구 명령('sudo rm <경로>')이 사용자에게 도달하지 않았다.
//          그때 단위테스트가 초록이었던 이유는 픽스처가 Rust 실물이 아니라 **잘못된 TS 모양**을
//          먹였기 때문이다 — clipath.test.ts 의 계약-드리프트 가드가 그 재발을 막는다.
//
//   install_cli_to_path()     -> InstallCliReport
//        ok: bool(= status=="installed") / status: String / target_dir·cys_link·cysd_link·source_cys: String
//        effective_cys: Option<String> / shadowed_by: Option<String> / warnings: Vec<String>
// ══════════════════════════════════════════════════════════════════════════════

// ── 플랫폼 게이팅 ─────────────────
// 원 결함: HTML에 플랫폼 분기가 없어 Windows/Linux에서도 버튼이 렌더됐고, Rust는 macOS 외에서
// 즉시 Err를 돌려줬다 — 사용자는 **보이는 버튼**을 누르고 실패 토스트만 받았다.
// 그래서 `!IS_WINDOWS`(부정 판정)로는 부족하다. Linux가 그대로 통과해 같은 결함이 재현된다.
// macOS **양성 판정**만 노출 조건이다. 모바일 토큰은 데스크톱 앱에 나올 일이 없지만, iPadOS의
// 데스크톱 모드 UA가 "Macintosh"를 그대로 쓰므로 방어적으로 배제한다.
export function isMacUserAgent(ua: string): boolean {
  if (/iPhone|iPad|iPod|Android|Windows/i.test(ua)) return false;
  return /Macintosh|Mac OS X/i.test(ua);
}

// ── 토스트 계획(등급 + 수명) ─────────────────
/// `sticky` = main.ts의 stickyToast(60초 · id로 갱신·중복 흡수) / false = 기존 volatile 토스트(8초).
///
/// (MINOR-10) 등급을 낮춘 경고 본문은 200자 안팎이라 8초에 사라지면 **읽히지 않는다**.
/// 규칙: 사용자가 아직 할 일이 남은 결과(그림자화·확인 불가·해제 부분 완료·경고 동반)는 sticky,
/// 아무 할 일이 없는 성공만 volatile.
export type ToastPlan = { category: string; title: string; body: string; sticky: boolean; id: string };

/// sticky 토스트 id — 같은 id 재호출은 갱신(중복 스택 없음). main.ts 배선의 단일 진실.
export const INSTALL_TOAST_ID = "cli-install";
export const UNINSTALL_TOAST_ID = "cli-uninstall";
export const CLI_NOTES_TOAST_ID = "cli-status-notes";

// ── 설치 결과 등급 ─────────────────
/// Rust InstallCliReport.status 계약 — 정확히 세 값.
///   installed          = 심링크 생성 + 로그인 셸 기준 `which -a cys` 1순위가 /usr/local/bin/cys
///   installed_shadowed = 심링크는 생겼으나 PATH 앞을 가리는 다른 cys가 있다
///   unverified         = 확인을 못 했다(명령 실패·타임아웃) **또는** 로그인 셸 PATH에서 cys를 못 찾았다
///                        — 두 갈래다(MINOR-9). 어느 쪽인지는 unverifiedCause(warnings)로 가른다.
export type CliInstallStatus = "installed" | "installed_shadowed" | "unverified";

/// Rust `InstallCliReport` 의 TS 사본. **선택 필드가 없다** — Rust 가 항상 전부 보낸다.
/// 응답을 못 읽었을 때의 안전한 기본값은 readInstallReport 가 만든다(타입을 느슨하게 만들지 않는다).
export type InstallCliReport = {
  ok: boolean;
  status: string;
  target_dir: string;
  cys_link: string;
  cysd_link: string;
  source_cys: string;
  effective_cys: string | null;
  shadowed_by: string | null;
  warnings: string[];
};

function str(v: unknown): string {
  return typeof v === "string" ? v : "";
}
function strOrNull(v: unknown): string | null {
  return typeof v === "string" ? v : null;
}
function strList(v: unknown): string[] {
  return Array.isArray(v) ? v.filter((x): x is string => typeof x === "string" && x.length > 0) : [];
}

/// invoke 응답(unknown) → 계약 모양. **모르는 값은 안전한 쪽으로** 접는다:
/// status 미상은 unverified(측정 불능은 통과가 아니다 — 헌장), ok 미상은 false.
export function readInstallReport(raw: unknown): InstallCliReport {
  const r = (raw && typeof raw === "object" ? raw : {}) as Record<string, unknown>;
  const status = normalizeInstallStatus(r.status);
  return {
    // 판독기는 **응답을 있는 그대로 옮긴다**(ok 를 status 로 다시 계산해 덮지 않는다 — 두 값이
    // 어긋나면 그 사실이 보여야 한다). 등급 판정은 어차피 status 하나만 본다.
    ok: r.ok === true,
    status,
    target_dir: str(r.target_dir),
    cys_link: str(r.cys_link),
    cysd_link: str(r.cysd_link),
    source_cys: str(r.source_cys),
    effective_cys: strOrNull(r.effective_cys),
    shadowed_by: strOrNull(r.shadowed_by),
    warnings: strList(r.warnings),
  };
}

/// 알 수 없는 값·필드 누락은 전부 "unverified"로 접는다 — **측정 불능은 통과가 아니다**(헌장).
/// 구버전 백엔드(status 필드 없음)와 붙어도 성공으로 둔갑하지 않는 것이 이 함수의 존재 이유다.
export function normalizeInstallStatus(raw: unknown): CliInstallStatus {
  if (raw === "installed" || raw === "installed_shadowed" || raw === "unverified") return raw;
  return "unverified";
}

// ── (MINOR-9) unverified 의 두 갈래 ─────────────────
/// Rust classify_install_status 는 성질이 다른 두 종단을 같은 status("unverified")로 접는다:
///   ① probe-failed  — 확인 명령 자체를 못 돌렸다(실행 실패·타임아웃). 무엇이 잡히는지 **모른다**.
///   ② not-on-path   — 확인은 정상이었는데 로그인 셸 PATH에서 cys 를 못 찾았다. 원인이 특정된다
///                     (PATH에 /usr/local/bin 이 없다). 예전 UI 문구는 이 경우까지 '검증 명령 실패
///                     또는 응답 없음'으로 단정해 **사실과 달랐다**(docs/INSTALL.md 도 같은 오단정).
///
/// 계약(InstallCliReport)에는 분기 플래그가 없으므로 신호는 warnings 문장이다 — Rust 가 두 갈래에
/// 서로 다른 문장을 담는다. 문구가 바뀌어 어느 쪽도(또는 양쪽 다) 걸리면 "unknown" 으로 떨어지고,
/// UI 는 원인을 단정하지 않는 문구를 쓴다(모르면 모른다고 말한다 — 오단정이 이 결함의 본체였다).
export type UnverifiedCause = "probe-failed" | "not-on-path" | "unknown";

const NOT_ON_PATH_MARK = /찾지 못했|not found/i;
const PROBE_FAILED_MARK = /확인\(which|타임아웃|timed? ?out|실행 실패|상태 확인 실패|응답 없음/i;

export function unverifiedCause(warnings: readonly string[]): UnverifiedCause {
  const joined = warnings.join("\n");
  const notOnPath = NOT_ON_PATH_MARK.test(joined);
  const probeFailed = PROBE_FAILED_MARK.test(joined);
  if (notOnPath === probeFailed) return "unknown"; // 둘 다거나 아무것도 아니면 단정하지 않는다
  return notOnPath ? "not-on-path" : "probe-failed";
}

/// 설치 결과 → 토스트 등급·문구. installed 만 성공("system"), 나머지 둘은 경고("watchdog")로
/// 등급을 낮춘다. 이 코드베이스의 토스트 등급은 테두리색 하나뿐이고 watchdog은 완료 알림에도
/// 쓰이므로(main.ts "✅ 데몬 재시작 완료"), 제목 접두 ✅/⚠ 로 등급을 눈에 보이게 못박는다.
///
/// warnings 는 등급과 무관하게 본문 말미에 붙는다 — BLOCK-1(c)의 **백업 경로 통보**가 이 줄로
/// 사용자에게 도달한다. 그래서 성공(installed)이라도 warnings 가 있으면 sticky 로 올린다:
/// "당신의 파일을 <경로>.cys-backup-… 으로 옮겼습니다" 가 8초 만에 사라지면 안 된다.
export function installResultToast(rep: InstallCliReport): ToastPlan {
  const status = normalizeInstallStatus(rep.status);
  const links = [rep.cys_link, rep.cysd_link].filter(Boolean).join(" · ");
  const warn = rep.warnings.filter(Boolean);
  const tail = warn.length > 0 ? `\n⚠ ${warn.join("\n⚠ ")}` : "";

  if (status === "installed") {
    return {
      category: "system",
      title: "✅ 셸 설치 완료",
      body: `${links} — 새 터미널에서 'cys' 를 바로 쓸 수 있습니다.${tail}`,
      sticky: warn.length > 0,
      id: INSTALL_TOAST_ID,
    };
  }
  if (status === "installed_shadowed") {
    const by = rep.shadowed_by || "(경로 미상)";
    return {
      category: "watchdog",
      title: "⚠ 셸 설치 미완료 — 다른 cys가 앞을 가립니다",
      body:
        `심링크(${links})는 만들었지만, 로그인 셸 기준으로는 PATH 앞쪽의 ${by} 가 먼저 잡힙니다 — ` +
        `터미널에서 'cys' 를 치면 아직 그쪽이 실행됩니다. 그 파일을 지우거나 PATH에서 ` +
        `/usr/local/bin 을 앞으로 옮긴 뒤, 새 터미널에서 'which -a cys' 로 1순위를 확인하세요.${tail}`,
      sticky: true,
      id: INSTALL_TOAST_ID,
    };
  }

  // unverified — 원인 두 갈래를 구분해 말한다(MINOR-9).
  const cause = unverifiedCause(warn);
  if (cause === "not-on-path") {
    return {
      category: "watchdog",
      title: "⚠ 셸 설치 미완료 — PATH에서 cys를 찾지 못했습니다",
      body:
        `심링크(${links})는 만들었지만, 로그인 셸의 PATH에서 cys 를 찾지 못했습니다 — ` +
        `PATH에 /usr/local/bin 이 들어 있지 않을 수 있습니다(설치 자체가 실패한 것은 아닙니다). ` +
        `새 터미널에서 'which -a cys' 를 실행해 보고, 아무것도 나오지 않으면 셸 설정 파일에 ` +
        `/usr/local/bin 을 PATH로 추가하세요.${tail}`,
      sticky: true,
      id: INSTALL_TOAST_ID,
    };
  }
  if (cause === "probe-failed") {
    return {
      category: "watchdog",
      title: "⚠ 셸 설치 확인 불가",
      body:
        `심링크(${links})는 만들었지만, 확인 명령(which -a cys)이 실패했거나 응답하지 않아 ` +
        `실제로 어떤 cys가 잡히는지 확인하지 못했습니다. 새 터미널에서 'which -a cys' 를 직접 ` +
        `실행해 1순위가 /usr/local/bin/cys 인지 확인하세요.${tail}`,
      sticky: true,
      id: INSTALL_TOAST_ID,
    };
  }
  return {
    category: "watchdog",
    title: "⚠ 셸 설치 확인 불가",
    body:
      `심링크(${links})는 만들었지만, 실제로 어떤 cys가 잡히는지 확인하지 못했습니다 — ` +
      `확인 명령이 실패했거나, 로그인 셸의 PATH에 /usr/local/bin 이 없는 경우입니다(어느 쪽인지는 ` +
      `단정하지 않습니다). 새 터미널에서 'which -a cys' 를 직접 실행해 확인하세요.${tail}`,
    sticky: true,
    id: INSTALL_TOAST_ID,
  };
}

// ── 버튼 상태(설치 ↔ 해제) ─────────────────
/// 버튼 하나를 상태 2종으로 쓴다. "unknown" 은 상태 조회 실패·미응답 —
/// **모르면 설치 쪽**으로 둔다(설치는 멱등하고, 해제는 비가역에 가깝다).
export type CliButtonState = "installed" | "absent" | "unknown";

/// Rust CliInstallStatusReport.state 계약 — 다섯 값 + 판독 실패용 "unknown".
export type CliLinkState = "absent" | "ours" | "partial" | "foreign" | "unsupported" | "unknown";

/// Rust `CliInstallStatusReport` 의 TS 사본(선택 필드 없음 — Rust 가 항상 전부 보낸다).
export type CliInstallStatusReport = {
  platform_supported: boolean;
  installed: boolean;
  state: string;
  cys_link: string;
  cysd_link: string;
  notes: string[];
};

/// 상태 조회 응답을 UI가 실제로 쓰는 모양으로 판독한 결과.
export type CliStatusView = {
  /// platform_supported=false 일 때만 false — 판독 실패로 버튼을 숨기지 않는다(기능 소실 방지).
  supported: boolean;
  button: CliButtonState;
  linkState: CliLinkState;
  /// (BLOCK-1(d)) 실체 파일·타 대상 링크 등 **사용자 고지용 문장**. Rust 는 이미 만들어 보내는데
  /// 예전 TS 타입에는 필드 자체가 없어 한 글자도 노출되지 않았다.
  notes: string[];
  cysLink: string;
  cysdLink: string;
};

const LINK_STATES: readonly string[] = ["absent", "ours", "partial", "foreign", "unsupported"];

/// cli_install_status(읽기 전용·승격 없음) 응답 판독. 계약대로 `installed: bool` 하나가 라벨을
/// 가르고, state·notes 는 고지에 쓴다. 계약에 없는 모양(구 TS 가 상상했던 cys_installed·status
/// enum 등)은 **받지 않는다** — 그런 관용이 곧 드리프트를 덮는 뚜껑이었다(MAJOR-5).
export function readCliStatus(raw: unknown): CliStatusView {
  const r = (raw && typeof raw === "object" ? raw : {}) as Record<string, unknown>;
  const state = str(r.state);
  return {
    supported: r.platform_supported !== false,
    button: typeof r.installed === "boolean" ? (r.installed ? "installed" : "absent") : "unknown",
    linkState: (LINK_STATES.includes(state) ? state : "unknown") as CliLinkState,
    notes: strList(r.notes),
    cysLink: str(r.cys_link),
    cysdLink: str(r.cysd_link),
  };
}

/// 라벨 판정만 필요한 호출부용 축약(= readCliStatus(raw).button).
export function readInstallState(raw: unknown): CliButtonState {
  return readCliStatus(raw).button;
}

/// (BLOCK-1(d)) 남의 파일이 자리에 있을 때 **버튼을 누르면 무슨 일이 일어나는지** 미리 말한다.
/// 설치 스크립트는 심볼릭이 아닌 실체 파일을 지우지 않고 `<경로>.cys-backup-<시각>` 으로 옮긴 뒤
/// 링크를 만든다(파괴 아님). 이 문장이 없으면 사용자는 자기 파일이 어디로 갔는지 알 수 없다.
export const FOREIGN_BACKUP_NOTICE =
  "설치를 누르면 이 자리에는 cys 링크가 놓입니다 — 심볼릭 링크가 아닌 것(실제 파일·폴더)은 " +
  "지우지 않고 같은 폴더에 '<경로>.cys-backup-<시각>' 으로 옮겨 보관하며, 다른 곳을 가리키던 " +
  "링크는 새 링크로 바뀝니다.";

/// 상태 → 버튼 라벨·툴팁. 라벨만 바꾸고 title을 두면 '해제' 버튼이 '설치' 안내를 달고 있게 되므로
/// 둘을 **같은 함수에서 함께** 산출한다(cc-header 라벨 동적 변경 선례: applyCcDensity·applyGlanceFace).
/// notes 가 있으면 툴팁 말미에 그대로 덧붙인다 — 토스트는 사라지지만 툴팁은 버튼에 상주한다.
export function cliButtonView(
  state: CliButtonState,
  notes: readonly string[] = [],
): { label: string; title: string } {
  const extra = notes.filter(Boolean);
  const suffix =
    extra.length > 0
      ? `\n\n${extra.join("\n")}${state === "installed" ? "" : `\n${FOREIGN_BACKUP_NOTICE}`}`
      : "";
  if (state === "installed") {
    return {
      label: "셸 cys 해제",
      title:
        "/usr/local/bin 의 cys·cysd 심링크 제거(1회 관리자 승인) — 확인 창이 먼저 뜹니다" + suffix,
    };
  }
  if (state === "absent") {
    return {
      label: "셸에 cys 설치",
      title: "외부 터미널에서 cys 명령 쓰기(1회 관리자 승인)" + suffix,
    };
  }
  return {
    label: "셸에 cys 설치",
    title:
      "외부 터미널에서 cys 명령 쓰기(1회 관리자 승인) — 현재 설치 상태는 확인하지 못했습니다" + suffix,
  };
}

/// (BLOCK-1(d)) 상태 조회의 notes 를 **토스트로도** 낸다. Control Center를 열 때 1회 호출되며,
/// 알릴 것이 없으면 null 이다(정상은 말이 없어야 한다 — 무음 규약).
/// sticky 인 이유: 이 안내를 읽고 나서 버튼을 누를지 결정해야 하는데 8초로는 못 읽는다.
export function statusNoticePlan(view: CliStatusView): ToastPlan | null {
  const notes = view.notes.filter(Boolean);
  if (notes.length === 0) return null;
  const backup = view.button === "installed" ? "" : `\n${FOREIGN_BACKUP_NOTICE}`;
  return {
    category: "watchdog",
    title: "⚠ /usr/local/bin 에 이 앱의 것이 아닌 cys 파일이 있습니다",
    body: `${notes.join("\n")}${backup}`,
    sticky: true,
    id: CLI_NOTES_TOAST_ID,
  };
}

// ── 해제 확인 ─────────────────
/// 해제는 root 소유 심링크를 지우는 비가역에 가까운 행위다 — 클릭 즉시 집행하지 않고 확인을 받는다.
/// alert()/confirm() 은 이 WKWebView에서 억제된 실측(B-11)이 있어 쓰지 않는다: main.ts의 순수 DOM
/// confirmModal(title, body, yes, no)에 이 문구를 배선한다.
export function uninstallConfirmText(): {
  title: string;
  body: string;
  yes: string;
  no: string;
} {
  return {
    title: "셸 cys 해제",
    body: [
      "/usr/local/bin/cys · /usr/local/bin/cysd 심링크를 제거합니다 (관리자 승인 1회).",
      "",
      "· 제거 대상은 이 앱이 만든 심볼릭 링크뿐입니다. 같은 이름의 일반 파일(다른 도구가 설치한 실체 바이너리)이나 다른 앱을 가리키는 링크는 건드리지 않고 건너뜁니다.",
      "· 해제 후에는 외부 터미널에서 'cys' 명령을 쓸 수 없습니다. 앱 pane 안에서는 PATH가 자동 주입되므로 그대로 동작합니다.",
      "· 다시 필요하면 같은 버튼으로 언제든 설치할 수 있습니다.",
    ].join("\n"),
    yes: "해제",
    no: "취소",
  };
}

/// Rust `UninstallCliReport` 의 TS 사본. **skipped 는 문자열 배열**("경로 — 사유")이다.
export type UninstallCliReport = {
  ok: boolean;
  removed: string[];
  skipped: string[];
  warnings: string[];
};

/// invoke 응답(unknown) → 계약 모양. ok 미상은 false(측정 불능은 통과가 아니다).
export function readUninstallReport(raw: unknown): UninstallCliReport {
  const r = (raw && typeof raw === "object" ? raw : {}) as Record<string, unknown>;
  return {
    ok: r.ok === true,
    removed: strList(r.removed),
    skipped: strList(r.skipped),
    warnings: strList(r.warnings),
  };
}

/// skip 중 **아무 조치도 필요 없는 것**(애초에 그 경로가 없었다 = 이미 해제된 상태)만 골라낸다.
/// Rust plan_cli_uninstall 은 부재 경로도 skipped 에 넣는다("<경로> — 없음(이미 해제된 상태)") —
/// 이것까지 '부분 완료 ⚠'로 올리면, 한쪽 링크만 있던 정상 해제가 실패처럼 보고된다(오보고).
/// 판정이 안 서는 문장은 **조치 필요 쪽**으로 남긴다(안전한 방향은 덜 알리는 쪽이 아니다).
export function isBenignSkip(line: string): boolean {
  return /이미 해제/.test(line);
}

/// 해제 결과 → 토스트. 등급 규약은 설치와 같다: 사용자가 더 할 일이 없을 때만 성공("system"),
/// 하나라도 남았으면 경고("watchdog") + sticky 로 올리고 **사유와 복구 명령을 그대로 보여준다**.
/// warnings 에는 "'<경로>' 가 아직 남아 있습니다 — sudo rm <경로>" 같은 유일한 복구 경로가 들어 있다.
export function uninstallResultToast(rep: UninstallCliReport): ToastPlan {
  const removed = rep.removed.filter(Boolean);
  const skipped = rep.skipped.filter(Boolean);
  const warnings = rep.warnings.filter(Boolean);
  const blocking = skipped.filter((s) => !isBenignSkip(s));
  const benign = skipped.filter((s) => isBenignSkip(s));
  const failed = rep.ok !== true || warnings.length > 0;

  if (failed || blocking.length > 0) {
    const lines: string[] = [
      removed.length > 0 ? `제거: ${removed.join(" · ")}` : "제거한 항목 없음",
    ];
    if (skipped.length > 0) {
      lines.push(`건너뜀 ${skipped.length}건 — 직접 확인하세요:`);
      for (const s of skipped) lines.push(`   • ${s}`);
    }
    if (warnings.length > 0) {
      lines.push("남은 조치:");
      for (const w of warnings) lines.push(`   ⚠ ${w}`);
    }
    return {
      category: "watchdog",
      title: removed.length > 0 ? "⚠ 셸 cys 해제 부분 완료" : "⚠ 셸 cys 해제 — 지운 것이 없습니다",
      body: lines.join("\n"),
      sticky: true,
      id: UNINSTALL_TOAST_ID,
    };
  }

  if (removed.length > 0) {
    const tail = benign.length > 0 ? `\n(이미 없던 항목: ${benign.join(" · ")})` : "";
    return {
      category: "system",
      title: "✅ 셸 cys 해제 완료",
      body: `${removed.join(" · ")} 를 제거했습니다 — 새 터미널에서는 'cys' 명령이 더 이상 잡히지 않습니다.${tail}`,
      sticky: false,
      id: UNINSTALL_TOAST_ID,
    };
  }

  return {
    category: "watchdog",
    title: "⚠ 해제할 심링크 없음",
    body:
      "/usr/local/bin 에 이 앱이 만든 cys·cysd 심링크가 없습니다 — 지운 것이 없습니다." +
      (benign.length > 0 ? `\n${benign.join("\n")}` : ""),
    sticky: false,
    id: UNINSTALL_TOAST_ID,
  };
}
