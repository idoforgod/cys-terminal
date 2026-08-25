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
//        unverified_reason: Option<String>   ★2026-08-25 계약 확장(N3 / UNRESOLVED-1)
//          status=="unverified" 일 때만 Some 이고 값은 정확히 "not_on_path" | "probe_failed".
//          그 외 상태에서는 None(=null).
//            · "not_on_path"  = 검증 명령이 **정상 종료**했고 로그인 셸 PATH 에서 cys 를 못 찾았다
//            · "probe_failed" = 검증 명령을 실행하지 못했거나 비정상 종료·타임아웃이다
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

// ── (MINOR-N4 · N6) ToastPlan 하나를 실제로 화면에 낼 때의 순수 계획 ─────────────────
/// main.ts 의 토스트 배선에는 등급이 거짓말을 하는 구멍이 둘 있었다:
///
///   (N4) stickyToast 는 같은 id 로 다시 부르면 **엘리먼트를 재사용**하는데 본문(textContent)만
///        갈아끼우고 className(= 등급 테두리색)은 최초 생성값을 그대로 뒀다. 그래서
///        "⚠ 설치 실패"(watchdog) 뒤에 "✅ 셸 설치 완료"(system)가 같은 id 로 오면 **문구는 성공,
///        테두리는 경고색**인 토스트가 남는다. 색이 등급을 표시하는 유일한 장치인데 그 색이 틀린다.
///
///   (N6) volatile 토스트는 id 가 없어 같은 id 의 **살아 있는 sticky 를 내리지 못한다**. 재현:
///        설치 → 관리자 창 Cancel → 실패 sticky(60초) → 곧바로 다시 눌러 성공 → 성공 토스트가
///        실패 토스트 **옆에** 뜬다. 서로 모순되는 두 알림이 최대 60초 공존한다.
///
/// 둘 다 "무엇을 낼지"가 아니라 "어떻게 내는지"의 결정이라 순수 함수로 뽑아 테스트한다.
/// (이 모듈에 두는 이유: 토스트 수명 모듈 toastttl.ts 가 이 라운드의 수정 허용 범위 밖이다.
///  stickyToast 는 CLI 전용이 아니므로 className 규칙은 toastClassName 으로 따로 노출한다.)
export type ToastEmit = {
  /// sticky(60초·id 갱신) 인가 volatile(8초) 인가 — ToastPlan.sticky 그대로.
  sticky: boolean;
  /// 낼 때마다 이 값으로 className 을 **덮어쓴다**(재사용 엘리먼트의 낡은 등급색을 지운다).
  className: string;
  /// volatile 을 내기 **전에** 내려야 할 sticky 의 id. sticky 로 낼 때는 null
  /// (stickyToast 자신이 같은 id 를 갱신하므로 내렸다 다시 만들 필요가 없다).
  dismissStickyId: string | null;
};

/// 토스트 엘리먼트의 className — 등급(category)이 곧 테두리색이다. 생성·재사용 양쪽에서 쓴다.
export function toastClassName(category: string): string {
  return `toast ${category}`;
}

export function toastEmitPlan(plan: ToastPlan): ToastEmit {
  return {
    sticky: plan.sticky,
    className: toastClassName(plan.category),
    dismissStickyId: plan.sticky ? null : plan.id,
  };
}

/// sticky 토스트 id — 같은 id 재호출은 갱신(중복 스택 없음). main.ts 배선의 단일 진실.
export const INSTALL_TOAST_ID = "cli-install";
export const UNINSTALL_TOAST_ID = "cli-uninstall";
export const CLI_NOTES_TOAST_ID = "cli-status-notes";

// ── 설치 결과 등급 ─────────────────
/// Rust InstallCliReport.status 계약 — 정확히 세 값.
///   installed          = 심링크 생성 + 로그인 셸 기준 `which -a cys` 1순위가 /usr/local/bin/cys
///   installed_shadowed = 심링크는 생겼으나 PATH 앞을 가리는 다른 cys가 있다
///   unverified         = 확인을 못 했다(명령 실패·비정상 종료·타임아웃) **또는** 로그인 셸 PATH에서
///                        cys를 못 찾았다 — 두 갈래다(MINOR-9). 어느 쪽인지는 **기계 필드**
///                        `unverified_reason` 하나로 가른다(N3 — 산문 파싱 폐기).
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
  /// (N3) unverified 두 갈래의 **기계 판별자**. status 와 같은 규약으로 `string`(느슨) 으로 받고
  /// 값 검증은 unverifiedCause 가 한다 — 판독기는 응답을 옮기고, 판정은 판정 함수가 한다.
  unverified_reason: string | null;
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
    // 필드가 없는 구 백엔드는 null 이 되고, null 은 unverifiedCause 에서 "unknown"(원인 불명)으로
    // 접힌다 — 없는 값을 있는 값으로 추측하지 않는다.
    unverified_reason: strOrNull(r.unverified_reason),
    warnings: strList(r.warnings),
  };
}

/// 알 수 없는 값·필드 누락은 전부 "unverified"로 접는다 — **측정 불능은 통과가 아니다**(헌장).
/// 구버전 백엔드(status 필드 없음)와 붙어도 성공으로 둔갑하지 않는 것이 이 함수의 존재 이유다.
export function normalizeInstallStatus(raw: unknown): CliInstallStatus {
  if (raw === "installed" || raw === "installed_shadowed" || raw === "unverified") return raw;
  return "unverified";
}

// ── (N3 · MINOR-9) unverified 의 두 갈래 — **기계 필드로만** 가른다 ─────────────────
/// Rust classify_install_status 는 성질이 다른 두 종단을 같은 status("unverified")로 접는다:
///   ① probe_failed — 확인 명령 자체를 못 돌렸다(실행 실패·비정상 종료·타임아웃).
///                    무엇이 잡히는지 **모른다**.
///   ② not_on_path  — 확인은 정상 종료했는데 로그인 셸 PATH에서 cys 를 못 찾았다. 원인이 특정된다
///                    (PATH에 /usr/local/bin 이 없다). 예전 UI 문구는 이 경우까지 '검증 명령 실패
///                    또는 응답 없음'으로 단정해 **사실과 달랐다**(docs/INSTALL.md 도 같은 오단정).
///
/// ★2026-08-25 계약 확장(N3 / UNRESOLVED-1): 분기의 유일한 근거는 `unverified_reason` 필드다.
/// 그 전에는 warnings **산문을 정규식으로 파싱**했는데, 그것이 틀린 이유는 세 겹이었다 —
///   (1) **계약 불일치**: Rust 는 "경고문 첫 구절(`PATH 확인 결과:` / `PATH 확인 실패:`)을 안정
///       판별자로 고정한다"고 선언했는데, TS 정규식은 그 접두가 아니라 문장 **속** 어절
///       ('찾지 못했'·'타임아웃')을 봤다. 양쪽이 서로 다른 것을 계약이라 부르고 있었다.
///   (2) **판정 대상 오염**: 같은 warnings 배열에 백업 통보문("…로 백업한 뒤 링크를 만들었습니다")
///       이 합류한다. 남의 문장에 들어 있는 어절이 등급 판정을 흔든다.
///   (3) **산문은 계약이 될 수 없다**: 문구는 다듬기·번역으로 언제든 바뀌고, 바뀌는 순간 조용히
///       오분기한다(테스트가 같은 문자열을 픽스처로 쓰고 있으면 초록인 채로 봉인된다).
/// 값이 없거나(구 백엔드) 계약 밖 값이면 "unknown" 으로 접는다 — 모르면 모른다고 말한다.
export type UnverifiedReason = "not_on_path" | "probe_failed";
/// 계약값 둘 + 값이 없을 때의 "unknown"(구 백엔드·미상 — 원인을 단정하지 않는다).
export type UnverifiedCause = UnverifiedReason | "unknown";

export function unverifiedCause(reason: string | null | undefined): UnverifiedCause {
  return reason === "not_on_path" || reason === "probe_failed" ? reason : "unknown";
}

// ── (MINOR-N7) 이 확인이 **무엇을 잰 것인지** 밝히는 단서 ─────────────────
/// Rust 는 `$SHELL -lc 'which -a cys'` 로 잰다. 그것은 **비대화형 로그인 셸**이라 사용자의 대화형
/// rc 를 읽지 않는다 — 실측(2026-08-25): `zsh -lc` 는 `.zshenv`·`.zprofile`·`.zlogin` 만 읽고
/// `.zshrc` 는 건너뛴다(`bash -lc` 도 `.bash_profile` 만 읽고 `.bashrc` 는 건너뛴다).
/// 그래서 PATH 를 `~/.zshrc` 에 넣어 둔 사용자는 **터미널에서 cys 가 멀쩡히 동작하는데도** 이
/// 확인만 실패한다. 그 거짓 경고를 그대로 두면 사용자는 고칠 것이 없는 것을 고치러 간다.
///
/// ★대화형(-lic)으로 바꾸는 안은 채택하지 않는다: 버튼 클릭의 부작용으로 사용자의 대화형 rc 를
/// 실행하면 nvm·conda·oh-my-zsh 같은 것이 백그라운드 프로세스를 띄울 수 있다. 정직한 문구가 답이다.
export const NONINTERACTIVE_PROBE_NOTE =
  "이 확인은 비대화형 로그인 셸 기준입니다 — 터미널에서 cys 가 이미 동작한다면 무시해도 됩니다.";

/// (MINOR-N7) PATH 를 정말 고쳐야 할 때 **실제로 읽히는 파일**만 지목한다. `~/.zshrc` 를 고치라던
/// 예전 안내는 그대로 따라도 이 경고가 사라지지 않는 실행 불가능한 지시였다.
export const LOGIN_SHELL_PATH_FILES =
  "비대화형 로그인 셸이 실제로 읽는 파일(zsh: ~/.zshenv · ~/.zprofile · ~/.zlogin / " +
  "bash: ~/.bash_profile)";

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

  // unverified — 원인 두 갈래를 **기계 필드로** 구분해 말한다(N3 · MINOR-9).
  // warnings 는 여기서 읽지 않는다: tail 로 사용자에게 그대로 보여줄 뿐, 판정 근거로는 쓰지 않는다.
  const cause = unverifiedCause(rep.unverified_reason);
  if (cause === "not_on_path") {
    return {
      category: "watchdog",
      title: "⚠ 셸 설치 미완료 — PATH에서 cys를 찾지 못했습니다",
      body:
        `심링크(${links})는 만들었지만, 로그인 셸의 PATH에서 cys 를 찾지 못했습니다 — ` +
        `PATH에 /usr/local/bin 이 들어 있지 않을 수 있습니다(설치 자체가 실패한 것은 아닙니다). ` +
        `${NONINTERACTIVE_PROBE_NOTE} 그래도 안 잡히면 ${LOGIN_SHELL_PATH_FILES}에 ` +
        `/usr/local/bin 을 PATH로 추가하세요 — ~/.zshrc 는 이 확인에서 읽히지 않습니다.${tail}`,
      sticky: true,
      id: INSTALL_TOAST_ID,
    };
  }
  if (cause === "probe_failed") {
    return {
      category: "watchdog",
      title: "⚠ 셸 설치 확인 불가",
      body:
        `심링크(${links})는 만들었지만, 확인 명령(which -a cys)이 실패했거나 비정상 종료·무응답이라 ` +
        `실제로 어떤 cys가 잡히는지 확인하지 못했습니다. 새 터미널에서 'which -a cys' 를 직접 ` +
        `실행해 1순위가 /usr/local/bin/cys 인지 확인하세요. ${NONINTERACTIVE_PROBE_NOTE}${tail}`,
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
      `단정하지 않습니다). 새 터미널에서 'which -a cys' 를 직접 실행해 확인하세요. ` +
      `${NONINTERACTIVE_PROBE_NOTE}${tail}`,
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
