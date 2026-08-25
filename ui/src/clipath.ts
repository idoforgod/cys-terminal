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
// ★그리고 2026-08-25(I6)부터 그 "사본"은 **손으로 지키지 않는다**: Rust 의 mod tests 가 세 리포트의
//   키 집합·타입 태그를 `ui/src/__contract__.json` 으로 덤프하고, clipath.test.ts 가 그 파일을 읽어
//   모양을 검사한다. 아래 주석은 사람이 읽는 요약일 뿐이고, **판정의 근거는 그 생성 파일**이다.
//   (예전에는 테스트 안의 손으로 쓴 표가 기준이라, Rust 가 바뀌어도 게이트가 빨개지지 않았다.)
//
//   cli_install_status()      -> CliInstallStatusReport
//        platform_supported: bool / installed: bool / state: String
//        ("absent"|"ours"|"partial"|"foreign"|"unsupported")
//        cys_link: String / cysd_link: String / notes: Vec<String>
//        backups: Vec<String>              ★2026-08-25 계약 확장(I3①)
//          `/usr/local/bin` 에 남아 있는 **우리 백업본 전체 경로**. 기계 필드다 —
//          되돌리기 명령 **문구는 UI 소유**이므로 여기서 만든다(백엔드는 사실만).
//
//   uninstall_cli_from_path() -> UninstallCliReport
//        ok: bool / removed: Vec<String> / skipped: Vec<String>("경로 — 사유") / warnings: Vec<String>
//        skipped_reasons: Vec<String>      ★2026-08-25 계약 확장(C3) skipped 와 **인덱스 1:1**.
//          값은 정확히 "absent" | "not_symlink" | "foreign_target".
//        skipped_benign: bool              ★2026-08-25 계약 확장(C3 — 해제 등급의 기계 판별자)
//          건너뛴 항목이 **전부** "애초에 지울 게 없었다"(absent)인가. skip 이 없으면 true.
//        restored: Vec<String>             ★2026-08-25 계약 확장(I3③) 해제하며 **되돌린 원본** 경로.
//          해제는 우리 링크를 지운 자리에 설치 때의 백업본을 다시 놓는다(파괴가 아니라 복원).
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
  /// volatile 을 내기 **전에** 내려야 할 sticky 의 id. sticky 로 낼 때는 null
  /// (stickyToast 자신이 같은 id 를 갱신하므로 내렸다 다시 만들 필요가 없다).
  dismissStickyId: string | null;
};

/// 토스트 엘리먼트의 className — 등급(category)이 곧 테두리색이다. 생성·재사용 양쪽에서 쓴다.
///
/// ★(N4) 이 함수가 **등급색의 단일 진실**이고, main.ts 의 toast()·stickyToast() 는 낼 때마다
/// 이것으로 className 을 덮어쓴다(재사용 엘리먼트의 낡은 등급색을 지운다). 그 재적용이 곧 N4 의
/// 수리 본체이며, clipath.test.ts 의 "main.ts 배선 핀"이 그 한 줄을 못박는다.
export function toastClassName(category: string): string {
  return `toast ${category}`;
}

/// ★(I7 · 2026-08-25) 예전 ToastEmit 에는 `className` 필드가 있었지만 **아무도 읽지 않았다** —
/// main.ts 의 toast()·stickyToast() 는 각자 toastClassName(category) 를 직접 부르고, showCliToast
/// 는 emit.className 에 손대지 않았다. 값이 늘 같으니 버그로 드러나지도 않았고, 그래서 "계획이
/// 등급색을 정한다"는 **거짓 계약**만 남았다(두 번째 진실원 = 이 라운드가 닫는 계열 결함).
/// 필드를 지우고, 등급색의 진실은 toastClassName 하나로 되돌린다. 소비되지 않는 필드는 계약이
/// 아니라 장식이다.
export function toastEmitPlan(plan: ToastPlan): ToastEmit {
  return {
    sticky: plan.sticky,
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

// ── (C3 대칭 점검) shadowed_by 는 **셸이 뱉은 한 줄**이다 ─────────────────
/// Rust 는 `which -a cys` 의 출력 한 줄을 그대로 `shadowed_by` 로 실어 보낸다. 그 줄은 사용자의
/// 로그인 프로필이 stdout 에 찍은 배너일 수도 있고, zsh 함수 래퍼 본문의 한 줄일 수도 있다
/// (adv1·adv7 계열). 그런데 예전 토스트는 그 문자열을 그대로 받아 **"그 파일을 지우세요"** 라는
/// 파괴적 지시에 끼워 넣었다 — 산문 한 줄을 검증 없이 믿고 사용자에게 rm 을 시킨 셈이다.
///
/// Rust 쪽 경화(표식 격리·공백 줄 배제·Path::is_file 재관측)와 **같은 방향으로** UI 도 한 겹
/// 더 잠근다: 절대경로 한 개로 읽히지 않는 값이면 경로를 지목하지 않고, 파괴적 지시도 빼고,
/// 사용자에게 직접 확인하라고만 말한다. **모르면 모른다고 말한다** — 이것은 등급 판정을 산문에서
/// 읽는 것(C3 가 없애는 것)과 다르다. 등급은 status 가 정하고, 여기서는 이미 정해진 등급의
/// 문구에 **경로를 넣어도 되는지**만 본다.
export type ShadowTarget = { path: string | null; label: string };

export function shadowTarget(raw: string | null | undefined): ShadowTarget {
  const v = typeof raw === "string" ? raw.trim() : "";
  // 절대경로 한 개 = `/` 로 시작 + 공백·개행·따옴표 없음. zsh 함수 래퍼 본문("  /opt/foo/cys --wrap")
  // 이나 배너 문장("/opt/corp/toolchain/env loaded")은 공백 때문에 여기서 걸린다.
  const looksLikeOnePath = v.length > 1 && v.startsWith("/") && !/[\s"'`]/.test(v);
  return looksLikeOnePath ? { path: v, label: v } : { path: null, label: v || "(경로 미상)" };
}

/// 설치 결과 → 토스트 등급·문구. installed 만 성공("system"), 나머지 둘은 경고("watchdog")로
/// 등급을 낮춘다. 이 코드베이스의 토스트 등급은 테두리색 하나뿐이고 watchdog은 완료 알림에도
/// 쓰이므로(main.ts "✅ 데몬 재시작 완료"), 제목 접두 ✅/⚠ 로 등급을 눈에 보이게 못박는다.
///
/// warnings 는 등급과 무관하게 본문 말미에 붙는다 — BLOCK-1(c)의 **백업 경로 통보**가 이 줄로
/// 사용자에게 도달한다. 그래서 성공(installed)이라도 warnings 가 있으면 sticky 로 올린다:
/// "당신의 파일을 <경로>.cys-backup-… 으로 옮겼습니다" 가 8초 만에 사라지면 안 된다.
///
/// ★G14(2026-08-25 5R) **⚠ 는 ✅ 안에 숨을 수 없다.** 예전에는 warnings 가 있어도 제목이
/// "✅ 셸 설치 완료" 였고, 본문 말미에만 `⚠ …` 줄이 붙었다. 그래서 cysd 그림자 경고(C5)처럼
/// "설치는 됐지만 데몬 버전이 어긋날 수 있다" 는 사실이 **성공 알림 안의 한 줄**로 나갔다 —
/// 제목이 ✅ 면 사람은 본문을 읽지 않는다(등급 표시가 곧 읽기 여부를 정한다).
///
/// 그래서 `status=="installed"` 라도 warnings 가 하나라도 있으면 등급을 낮춘다. 근거는
/// **기계 신호 하나**(`warnings.length`)이고 문구는 읽지 않는다(N3 규약 유지).
///
/// ★이 규칙이 정당한 이유(= 설치 경로에서만 쓰는 이유): Rust 의 설치 warnings 는 실측상 전부
/// '사용자가 확인할 것'뿐이다(main.rs `install_cli_to_path`) — ①백업 통보(당신의 파일이 옮겨졌다)
/// ②백업 미확인(옮겼는지 모른다) ③PATH 확인 결과(verdict) ④cysd 그림자. 하나도 '좋은 소식'이 아니다.
/// **해제 경로는 다르다**: 거기 warnings 에는 "원본을 되돌렸습니다"(성공의 일부)가 합류하므로
/// 같은 규칙을 쓰면 정상 해제가 ⚠ 로 오보고된다 — C3 가 없앤 바로 그 결함이다. 그래서 해제 쪽은
/// 등급 규칙(ok · skipped_benign)을 그대로 두고, ✅ 본문의 ⚠ 글리프만 걷어 낸다(대칭이 아니라
/// **의도된 비대칭**이며, 그 근거가 이 문단이다).
export function installResultToast(rep: InstallCliReport): ToastPlan {
  const status = normalizeInstallStatus(rep.status);
  const links = [rep.cys_link, rep.cysd_link].filter(Boolean).join(" · ");
  const warn = rep.warnings.filter(Boolean);
  const tail = warn.length > 0 ? `\n⚠ ${warn.join("\n⚠ ")}` : "";

  if (status === "installed" && warn.length === 0) {
    return {
      category: "system",
      title: "✅ 셸 설치 완료",
      body: `${links} — 새 터미널에서 'cys' 를 바로 쓸 수 있습니다.`,
      sticky: false,
      id: INSTALL_TOAST_ID,
    };
  }
  if (status === "installed") {
    // (G14) 링크는 만들어졌다는 사실은 그대로 말하되, 확인할 것이 남았으므로 등급은 ⚠ 다.
    //
    // ★R3(2026-08-25 6R) **확인 항목을 본문 맨 앞에 둔다.** 예전에는 성공 서술을 한 문단 읽은 뒤
    // 맨 끝에 `⚠ …` 줄이 붙었고, 그 자리에 오는 대표 사례가 cysd 그림자 경고였다 — 사람은 앞줄에서
    // "쓸 수 있습니다"를 읽는 순간 뒤를 읽지 않으므로, 정작 확인해야 할 문장이 본문에 묻혔다.
    // (등급은 건드리지 않는다 — cysd 경고가 성공 등급을 낮추는지는 이번 라운드의 결정 사항이
    //  아니며 master 가 '하지 않는다'로 확정했다. 여기서 바꾸는 것은 **배치**뿐이다.)
    return {
      category: "watchdog",
      title: "⚠ 셸 설치 완료 — 확인할 항목이 있습니다",
      body:
        `확인이 필요한 항목:${tail}\n\n` +
        `${links} — 링크는 만들어졌고 새 터미널에서 'cys' 를 바로 쓸 수 있습니다` +
        `(설치 자체가 실패한 것은 아닙니다).`,
      sticky: true,
      id: INSTALL_TOAST_ID,
    };
  }
  if (status === "installed_shadowed") {
    // (C3 대칭) 경로로 읽히는 값일 때만 그 경로를 지목하고 'rm' 쪽 안내를 붙인다.
    const by = shadowTarget(rep.shadowed_by);
    const advice = by.path
      ? `그 파일(${by.path})을 지우거나 PATH에서 /usr/local/bin 을 앞으로 옮긴 뒤, `
      : `가리는 경로를 하나로 특정하지 못했으므로(측정 출력이 경로 한 줄이 아닙니다) 지울 대상을 ` +
        `단정하지 않습니다 — PATH에서 /usr/local/bin 을 앞으로 옮긴 뒤, `;
    return {
      category: "watchdog",
      title: "⚠ 셸 설치 미완료 — 다른 cys가 앞을 가립니다",
      body:
        `심링크(${links})는 만들었지만, 로그인 셸 기준으로는 PATH 앞쪽의 ${by.label} 가 먼저 잡힙니다 — ` +
        `터미널에서 'cys' 를 치면 아직 그쪽이 실행됩니다. ${advice}` +
        `새 터미널에서 'which -a cys' 로 1순위를 확인하세요.${tail}`,
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
  /// (I3①) `/usr/local/bin` 에 남아 있는 우리 백업본 경로 — 기계 필드. 문구는 UI 가 만든다.
  backups: string[];
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
  /// (I3①) 잔존 백업본 경로. 해제 확인 창·상태 고지의 분기 근거이며, 없으면 빈 배열이다.
  backups: string[];
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
    backups: strList(r.backups),
    cysLink: str(r.cys_link),
    cysdLink: str(r.cysd_link),
  };
}

/// 라벨 판정만 필요한 호출부용 축약(= readCliStatus(raw).button).
export function readInstallState(raw: unknown): CliButtonState {
  return readCliStatus(raw).button;
}

/// (BLOCK-1(d)) 남의 파일이 자리에 있을 때 **버튼을 누르면 무슨 일이 일어나는지** 미리 말한다.
/// 이 문장이 없으면 사용자는 자기 파일이 어디로 갔는지 알 수 없다.
///
/// ★MAJOR-7(2026-08-25 5R) **동의 화면이 거짓말을 하고 있었다.** 이 문구는 "다른 곳을 가리키던
/// 링크는 (백업 없이) 새 링크로 바뀝니다" 라고 말했는데, C1 수리 이후 설치 스크립트의 실제 조건은
/// 그것이 아니다(main.rs `build_install_script` 실측):
///
///   자리에 무엇이든 있으면(`-e` 또는 `-L`) 일단 백업 대상으로 잡고,
///   **그것이 우리 번들(`*/cys.app/Contents/MacOS/{cys,cysd}`)을 가리키는 심볼릭일 때만** 백업을 뺀다.
///
/// 즉 **남의 심볼릭 링크도 실체 파일과 똑같이 백업된다.** 누르기 전에 보여 주는 동의 문구가 실제
/// 집행과 다르면, 사용자가 승인한 것은 실제로 일어나는 일이 아니다 — 그것이 이 상수의 결함이었다.
/// (같은 오고지가 docs/INSTALL.md·USER-MANUAL.md 에도 있었고 같은 라운드에서 함께 고쳤다.)
///
/// 백업 이름 규칙은 `<원래 경로>.cys-backup-<epoch초>` 다(main.rs `backup_path_for`·`backup_stamp`).
/// 사람이 읽는 날짜가 아니라 **숫자(초)** 라는 사실을 여기서 말해 둔다 — 문서 예시가 날짜 형식이라
/// 사용자가 없는 파일을 찾게 만든 전례가 있다(MINOR-N13).
export const FOREIGN_BACKUP_NOTICE =
  "설치를 누르면 이 자리에는 cys 링크가 놓입니다 — 지금 있는 것이 실제 파일·폴더든 다른 곳을 " +
  "가리키던 심볼릭 링크든, 이 앱이 만든 링크가 아니면 지우지 않고 같은 폴더에 " +
  "'<원래 경로>.cys-backup-<숫자>' 로 옮겨 보관합니다(<숫자>는 백업한 시각의 epoch 초). " +
  "옮긴 경로는 결과 알림과 이 버튼 툴팁에 그대로 나오며, 되돌리는 명령은 " +
  "'sudo mv <백업본> <원래 경로>' 입니다. 해제할 때 그 자리가 우리 링크면 앱이 백업본을 " +
  "자동으로 제자리에 되돌립니다.";

// ── (I2 · adv8) 버튼이 **직전에 한 일**과 어긋나지 않게 한다 ─────────────────
/// 결함: 설치를 눌러 `installed_shadowed`·`unverified`(= "설치 미완료" 경고)를 받은 직후,
/// 상태 재조회는 **링크의 존재**만 보므로 installed=true 를 돌려준다 → 버튼 라벨이 '해제'로
/// 뒤집힌다. 사용자는 방금 "미완료니 다시 해 보라"는 안내를 읽었는데 재시도 경로가 사라지고,
/// 같은 자리를 누르면 **정반대 행동(비가역 해제)** 이 나간다(adv8 실측 성립).
///
/// 두 진실원이 서로 다른 것을 재기 때문이다:
///   · 설치 판정(install_cli_to_path.status) = **PATH 에서 실제로 잡히는가**
///   · 상태 조회(cli_install_status.installed) = **링크가 파일시스템에 있는가**
/// 둘 다 참일 수 있다(링크는 있는데 앞이 가려짐). 그래서 라벨은 둘의 **합**으로 정한다.
///
/// 래치 해제 시점: Control Center 를 다시 열 때(main.ts setCcOpen) 와 해제 성공 직후. 즉 이
/// '다시 설치' 표시는 **그 세션의 잔상**이고, 패널을 닫았다 열면 현재 상태 그대로 돌아온다 —
/// 해제 경로가 영영 막히지 않는다(그 사실을 툴팁에 적는다).
export type CliButtonIntent = "install" | "reinstall" | "uninstall";

/// 직전 설치 시도의 결과(없으면 null). CliInstallStatus 를 그대로 쓴다 — 새 어휘를 만들지 않는다.
export type LastInstallOutcome = CliInstallStatus | null;

export function cliButtonIntent(state: CliButtonState, last: LastInstallOutcome): CliButtonIntent {
  // 직전 설치가 '완료'로 확인되지 않았다면, 링크가 생겼더라도 다음 동작은 해제가 아니라 재시도다.
  if (last === "installed_shadowed" || last === "unverified") return "reinstall";
  return state === "installed" ? "uninstall" : "install";
}

/// 상태 → 버튼 라벨·툴팁. 라벨만 바꾸고 title을 두면 '해제' 버튼이 '설치' 안내를 달고 있게 되므로
/// 둘을 **같은 함수에서 함께** 산출한다(cc-header 라벨 동적 변경 선례: applyCcDensity·applyGlanceFace).
/// notes 가 있으면 툴팁 말미에 그대로 덧붙인다 — 토스트는 사라지지만 툴팁은 버튼에 상주한다.
///
/// ★반환값에 `intent` 를 함께 낸다: main.ts 의 클릭 분기는 **라벨을 만든 그 판정**을 그대로 써야
/// 한다(라벨과 행동이 다른 창이 생기지 않게 — 결함의 본체가 그 어긋남이었다).
export function cliButtonView(
  state: CliButtonState,
  notes: readonly string[] = [],
  last: LastInstallOutcome = null,
): { label: string; title: string; intent: CliButtonIntent } {
  const intent = cliButtonIntent(state, last);
  const extra = notes.filter(Boolean);
  const suffix =
    extra.length > 0
      ? `\n\n${extra.join("\n")}${intent === "uninstall" ? "" : `\n${FOREIGN_BACKUP_NOTICE}`}`
      : "";
  if (intent === "reinstall") {
    return {
      label: "셸에 cys 다시 설치",
      intent,
      title:
        "직전 설치가 '완료'로 확인되지 않았습니다(PATH 앞을 다른 cys가 가리거나 확인에 실패) — " +
        "이 버튼은 해제가 아니라 설치를 다시 시도합니다. 해제하려면 Control Center 를 닫았다 " +
        "다시 열어 현재 상태로 라벨을 되돌리세요." + suffix,
    };
  }
  if (intent === "uninstall") {
    return {
      label: "셸 cys 해제",
      intent,
      title:
        "/usr/local/bin 의 cys·cysd 심링크 제거(1회 관리자 승인) — 확인 창이 먼저 뜹니다" + suffix,
    };
  }
  if (state === "absent") {
    return {
      label: "셸에 cys 설치",
      intent,
      title: "외부 터미널에서 cys 명령 쓰기(1회 관리자 승인)" + suffix,
    };
  }
  return {
    label: "셸에 cys 설치",
    intent,
    title:
      "외부 터미널에서 cys 명령 쓰기(1회 관리자 승인) — 현재 설치 상태는 확인하지 못했습니다" + suffix,
  };
}

// ── (I3①) 백업본 문구는 **UI 소유** ─────────────────
/// Rust 는 백업본 경로만 사실로 보내고(`backups`), "되돌리려면 …" 같은 표현은 만들지 않는다
/// (I7 의 '백엔드는 사실만, 표현은 UI 소유' 를 이 계열에도 적용한 결과다 — 같은 문장이 Rust 와
/// TS 양쪽에 존재해 토스트에 두 번 나오던 결함의 재발 방지).
///
/// 백업 이름 규칙은 **우리가 만든 것**이다: `<원래 경로>.cys-backup-<epoch초>`. 그래서 원래 경로를
/// 되찾는 것은 남의 산문을 파싱하는 일이 아니라 **우리 규약을 되읽는 일**이다. 규칙에 맞지 않으면
/// null 을 돌려주고, 그때는 되돌리기 명령을 만들어 주지 않는다(추측한 경로로 mv 를 시키지 않는다).
export const CYS_BACKUP_MARK = ".cys-backup-";

/// ★(MINOR-N13 · 2026-08-25 5R) 스탬프는 **epoch 초(숫자)** 다. Rust `is_our_backup_name`(main.rs)
/// 이 복원 후보를 고를 때 `stamp.chars().all(is_ascii_digit)` 로 거르므로, 숫자가 아닌 꼬리를
/// 가진 이름은 **앱이 절대 되돌리지 않는다**. UI 가 그런 이름에 'sudo mv' 를 제시하면 판정(Rust)과
/// 안내(UI)가 갈린다 — MAJOR-6 이 셸/Rust 사이에서 닫은 것과 같은 형태의 격차다. 그래서 여기서도
/// 같은 규칙을 쓴다: 숫자 스탬프가 아니면 원래 경로를 **되찾았다고 말하지 않는다**.
/// (정규식을 쓰지 않는 것은 이 파일의 '산문 파싱 금지' 가드와 눈으로도 충돌하지 않게 하려는 것이다 —
///  여기서 보는 것은 남의 산문이 아니라 **우리가 만든 이름 규약**이다.)
function isEpochStamp(s: string): boolean {
  return s.length > 0 && [...s].every((c) => c >= "0" && c <= "9");
}

export function backupOrigin(backupPath: string): string | null {
  const i = backupPath.lastIndexOf(CYS_BACKUP_MARK);
  if (i <= 0) return null;
  const origin = backupPath.slice(0, i);
  const stamp = backupPath.slice(i + CYS_BACKUP_MARK.length);
  // 스탬프가 비었거나 숫자가 아니면 우리 규칙이 아니다(`/x/cys.cys-backup-` · `…-20260825-101112`).
  return isEpochStamp(stamp) ? origin : null;
}

/// 백업본 한 줄의 사용자 문구. 원래 경로를 알 때만 복원 명령을 제시한다.
export function backupNoticeLine(backupPath: string): string {
  const origin = backupOrigin(backupPath);
  return origin
    ? `${backupPath} — 설치 때 여기로 옮겨 둔 원본입니다. 되돌리려면 'sudo mv ${backupPath} ${origin}', 필요 없으면 'sudo rm ${backupPath}'.`
    : `${backupPath} — 설치 때 옮겨 둔 원본으로 보입니다(원래 경로를 이름에서 확정하지 못했습니다). 필요 없으면 'sudo rm ${backupPath}'.`;
}

/// (I3①) 사용자에게 **상시** 보여줄 고지 줄 — 남의 파일 사유(notes 그대로) + 잔존 백업본(경로에서
/// 문구 생성). 토스트와 버튼 툴팁이 **같은 함수**를 보게 해서 두 표면이 다른 말을 하지 않게 한다
/// (토스트는 60초 뒤 사라지고 툴팁은 남는다 — 남는 쪽이 덜 말하면 정보가 소실된다).
export function cliNoticeLines(view: {
  notes: readonly string[];
  backups: readonly string[];
}): string[] {
  const lines: string[] = view.notes.filter(Boolean).slice();
  for (const b of view.backups.filter(Boolean)) lines.push(backupNoticeLine(b));
  return lines;
}

// ── (G2 · 2026-08-25 5R) 같은 사실을 **두 토스트로 내지 않는다** ─────────────────
/// 실측 재현: 남의 파일이 있는 자리에 설치를 1클릭 하면 sticky 토스트가 **둘** 떴다.
///   ① `cli-install`      = 결과 토스트. Rust warnings 에 실려 온 백업 문장.
///   ② `cli-status-notes` = 직후의 상태 재조회가 낸 상시 고지. 같은 백업 경로를 UI 문장으로 다시.
/// 같은 사건을 **서로 다른 문장**으로 두 번 말하면 사용자는 두 개의 다른 일이 일어났다고 읽는다
/// (그리고 어느 쪽 복구 명령을 따라야 하는지 알 수 없다). 그래서 액션 직후 경로에서는 ②를 억제하고,
/// 그 줄들을 ①의 본문 아래에 **접어 넣는다** — 한 사건, 한 알림.
///
/// ★분업은 그대로다: 백엔드는 사실(기계 필드 `backups`·`notes`·`restored`)만 보내고, 'sudo mv …'
/// 같은 **복구 명령 문장은 UI 가 조립한다**(backupNoticeLine). 이 함수는 그 조립물을 결과 토스트에
/// 합류시키는 지점이며, Rust 가 산문을 더 내든 덜 내든 UI 쪽 안내는 이 경로로 항상 도달한다.
///
/// 상시 경로(Control Center 를 열 때)는 그대로 `statusNoticePlan` 이 담당한다 — 억제하는 것은
/// **액션 직후의 중복**뿐이고, 정보를 없애는 것이 아니다(툴팁에도 같은 줄이 상주한다).
export const CLI_NOTICE_HEADING = "지금 /usr/local/bin 에 남아 있는 것:";

export function withCliNotice(plan: ToastPlan, lines: readonly string[]): ToastPlan {
  const keep = lines.filter(Boolean);
  if (keep.length === 0) return plan;
  return {
    ...plan,
    body: `${plan.body}\n\n${CLI_NOTICE_HEADING}\n${keep.map((l) => `   · ${l}`).join("\n")}`,
    // 잔존물 안내가 8초에 사라지면 상시 경로(다음 CC 열기)까지 아무도 말해 주지 않는다.
    sticky: true,
  };
}

/// (BLOCK-1(d)) 상태 조회의 notes 를 **토스트로도** 낸다. Control Center를 열 때 1회 호출되며,
/// 알릴 것이 없으면 null 이다(정상은 말이 없어야 한다 — 무음 규약).
/// sticky 인 이유: 이 안내를 읽고 나서 버튼을 누를지 결정해야 하는데 8초로는 못 읽는다.
///
/// ★(G2) 이 계획은 **상시 경로 전용**이다. 설치·해제 직후의 재조회에서는 내지 않는다 —
/// 같은 사실이 결과 토스트에 이미 실려 있어 sticky 가 둘이 되기 때문이다(main.ts 는 그 경로에서
/// `withCliNotice(결과, cliNoticeLines(status))` 로 하나만 낸다).
/// (MAJOR-D) 상시 고지 토스트의 제목 셋. **제목이 곧 등급 표시**이므로 세 종류를 구분한다.
export const NOTICE_TITLE_FOREIGN = "⚠ /usr/local/bin 에 이 앱의 것이 아닌 cys 파일이 있습니다";
export const NOTICE_TITLE_BACKUP = "설치 때 백업해 둔 원본이 남아 있습니다";
export const NOTICE_TITLE_INFO = "셸 cys 설치 상태 안내";

export function statusNoticePlan(view: CliStatusView): ToastPlan | null {
  const notes = view.notes.filter(Boolean);
  const backups = view.backups.filter(Boolean);
  if (notes.length === 0 && backups.length === 0) return null;
  // (I3①) 잔존 백업본은 **기계 필드**로 오고 문구는 UI 가 만든다. 설치 직후의 sticky 는 60초 뒤
  // 사라지고 그 수용처(알람 이력)는 메모리 전용이라, 이 상시 경로가 없으면 사용자는 자기 원본이
  // 어디로 갔는지 다시는 알 수 없다 — BLOCK-1 이 1클릭을 정당화한 근거가 그 지점에서 무너진다.
  const lines: string[] = cliNoticeLines(view);
  if (notes.length > 0 && view.button !== "installed") lines.push(FOREIGN_BACKUP_NOTICE);
  // ★MAJOR-D(2026-08-25 6R) **제목이 notes 의 유무만 보던 것이 결함이었다.**
  // G4 가 `notes` 에 PATH-그림자·프로브실패 문장을 새로 실으면서, 남의 파일이 하나도 없는 정상
  // 설치 사용자도 Control Center 를 열 때마다 "이 앱의 것이 아닌 cys 파일이 있습니다" 라는
  // **거짓 경고**를 보게 됐다(반증자 실측 재현). notes 는 성격이 다른 문장들이 합류하는 채널이라
  // '들어 있음' 자체가 종류를 말해 주지 않는다 — 채널의 내용물 유무로 판정을 추정하는,
  // 이 계열이 반복해 온 바로 그 형태다.
  //
  // 그래서 제목은 **Rust 가 이미 내는 기계 필드**로만 정한다(산문은 본문에만 싣는다 · 새 필드 금지):
  //   · linkState=="foreign"  → 남의 파일 경고(그때만 ⚠ 다)
  //   · backups 가 비어 있지 않음 → 백업 고지
  //   · 그 외(notes 만 있음)   → 중립 정보(⚠ 아님 · 테두리색도 중립인 "system")
  //   · 둘 다 없음            → 애초에 null(무음 규약 — 위에서 이미 돌아갔다)
  const body = lines.join("\n");
  if (view.linkState === "foreign")
    return { category: "watchdog", title: NOTICE_TITLE_FOREIGN, body, sticky: true, id: CLI_NOTES_TOAST_ID };
  if (backups.length > 0)
    return { category: "watchdog", title: NOTICE_TITLE_BACKUP, body, sticky: true, id: CLI_NOTES_TOAST_ID };
  // 중립이라도 sticky 는 유지한다 — 그림자 안내는 200자 안팎이라 8초로는 읽히지 않는다(MINOR-10).
  return { category: "system", title: NOTICE_TITLE_INFO, body, sticky: true, id: CLI_NOTES_TOAST_ID };
}

// ── 해제 확인 ─────────────────
/// 해제는 root 소유 심링크를 지우는 비가역에 가까운 행위다 — 클릭 즉시 집행하지 않고 확인을 받는다.
/// alert()/confirm() 은 이 WKWebView에서 억제된 실측(B-11)이 있어 쓰지 않는다: main.ts의 순수 DOM
/// confirmModal(title, body, yes, no)에 이 문구를 배선한다.
/// ★(I3②) 확인 창은 **결정의 순간에** 현재 상태를 함께 보여준다.
///
/// 예전에는 "설치 때 당신의 원본을 어디로 옮겼는지"를 알리는 통로가 설치 직후의 60초 sticky 하나뿐
/// 이었다(수용처 alarmHistory 는 메모리 전용). 그 토스트를 놓친 사용자는 해제 버튼을 누르는 순간까지
/// 자기 파일의 행방을 모른 채였고, 그러면 BLOCK-1 이 확인 모달 없는 1클릭 설치를 정당화한 근거
/// ("잃는 것이 없다")가 무너진다. 그래서 **해제를 승인하는 화면**에 그 사실을 싣는다.
///
/// 분기의 근거는 전부 **기계 값**이다: `notes.length`(남의 파일 고지)와 `backups.length`(잔존
/// 백업본). 문구를 읽어 분류하지 않는다(C3 와 같은 원리) — notes 는 그대로 옮기고, 백업본 문구는
/// UI 가 경로에서 만든다(백엔드는 사실만).
///
/// ★그리고 해제는 **되돌린다**: 우리 링크를 지운 자리에 설치 때의 백업본이 있으면 그것을 다시
/// 놓는다(Rust I3③ `restored`). 비가역 파괴가 아니라 복구이므로 확인 창이 미리 말해야 한다 —
/// 사용자가 승인하는 것이 '삭제'만이 아니기 때문이다.
export function uninstallConfirmText(
  notes: readonly string[] = [],
  backups: readonly string[] = [],
): {
  title: string;
  body: string;
  yes: string;
  no: string;
} {
  const state = notes.filter(Boolean);
  const kept = backups.filter(Boolean);
  const stateLines =
    state.length > 0 ? ["", "현재 /usr/local/bin 상태:", ...state.map((n) => `   • ${n}`)] : [];
  const backupLines =
    kept.length > 0
      ? [
          "",
          "설치 때 백업해 둔 원본이 남아 있습니다 — 해제하면서 제자리에 되돌립니다(되돌리지 못한 것은 결과 알림에 그대로 남습니다):",
          ...kept.map((b) => `   • ${backupNoticeLine(b)}`),
        ]
      : [];
  return {
    title: "셸 cys 해제",
    body: [
      "/usr/local/bin/cys · /usr/local/bin/cysd 심링크를 제거합니다 (관리자 승인 1회).",
      "",
      "· 제거 대상은 이 앱이 만든 심볼릭 링크뿐입니다. 같은 이름의 일반 파일(다른 도구가 설치한 실체 바이너리)이나 다른 앱을 가리키는 링크는 건드리지 않고 건너뜁니다.",
      "· 설치할 때 그 자리에 있던 것(실제 파일이든 다른 곳을 가리키던 심볼릭 링크든)을 '<원래 경로>.cys-backup-<숫자>' 로 옮겨 두었다면, 해제하면서 그 원본을 제자리에 되돌립니다(<숫자>는 백업한 시각의 epoch 초 · 되돌린 경로는 결과 알림에 나옵니다).",
      "· 해제 후에는 외부 터미널에서 'cys' 명령을 쓸 수 없습니다. 앱 pane 안에서는 PATH가 자동 주입되므로 그대로 동작합니다.",
      "· 다시 필요하면 같은 버튼으로 언제든 설치할 수 있습니다.",
      ...backupLines,
      ...stateLines,
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
  /// ★(C3) `skipped` 와 **인덱스가 1:1 대응**하는 기계 태그. 값은 정확히 `"absent"` |
  /// `"not_symlink"` | `"foreign_target"`. 줄별 분류가 필요할 때 문구 대신 이것을 본다.
  skipped_reasons: string[];
  /// ★(C3 · 2026-08-25) 해제 등급의 **기계 판별자**. 건너뛴 항목이 전부 '애초에 지울 게 없었다'
  /// (=이미 해제된 상태)인가. 판정은 Rust 의 UninstallAction 이 하고 TS 는 그 bool 만 읽는다.
  ///
  /// 왜 필드가 필요했나: 예전 TS 는 `/이미 해제/` 정규식으로 Rust **산문**을 파싱해 등급을 갈랐다
  /// (성공 volatile ↔ ⚠부분완료 sticky). 설치 경로는 같은 병을 N3 에서 이미 고쳤는데(산문 →
  /// unverified_reason) 해제 경로에는 그 수리가 적용되지 않았다 — 같은 결함의 거울쌍이 살아 있었다.
  /// Rust 가 "없음(이미 해제된 상태)" 를 한 단어만 다듬어도 정상 해제가 조용히 '부분 완료'로
  /// 오보고된다. 문구는 계약이 될 수 없다.
  skipped_benign: boolean;
  /// ★(I3③) 해제하며 **되돌린 원본** 경로. 해제는 우리 링크를 지운 자리에 설치 때의 백업본을 다시
  /// 놓는다 — 그러므로 그 자리에 파일이 '남아 있는' 것이 정상이고, 실패가 아니다.
  restored: string[];
  warnings: string[];
};

/// (C3) skip 사유 태그 중 **무해**한 것 — 애초에 지울 게 없었다. Rust `SKIP_REASON_ABSENT` 의 사본.
export const SKIP_REASON_ABSENT = "absent";

/// (C3) 건너뛴 줄을 **기계 태그로** 둘로 가른다 — 문구를 읽지 않는다.
/// 태그 배열의 길이가 어긋나면(계약 위반·구 백엔드) 줄별 분류를 포기하고 `skipped_benign` 하나에
/// 맡긴다. 그때도 **모르면 조치 필요 쪽**이다(판정 불가가 성공으로 둔갑하지 않는다).
export function partitionSkips(
  skipped: readonly string[],
  reasons: readonly string[],
  benign: boolean,
): { benign: string[]; blocking: string[] } {
  if (reasons.length === skipped.length) {
    return {
      benign: skipped.filter((_, i) => reasons[i] === SKIP_REASON_ABSENT),
      blocking: skipped.filter((_, i) => reasons[i] !== SKIP_REASON_ABSENT),
    };
  }
  return benign ? { benign: [...skipped], blocking: [] } : { benign: [], blocking: [...skipped] };
}

/// invoke 응답(unknown) → 계약 모양. ok 미상은 false(측정 불능은 통과가 아니다).
///
/// ★`skipped_benign` 의 미상(구 백엔드·타입 불일치)은 **false** 로 접는다. false 는 "무해하다고
/// 말할 수 없다" 이고, 그러면 건너뛴 항목이 조치 필요로 남아 ⚠ 등급이 된다 — 판정 불가가 성공으로
/// 둔갑하지 않는 방향이다(헌장: 측정 불능은 통과가 아니다). 반대로 접으면 실패가 조용히 성공이 된다.
export function readUninstallReport(raw: unknown): UninstallCliReport {
  const r = (raw && typeof raw === "object" ? raw : {}) as Record<string, unknown>;
  return {
    ok: r.ok === true,
    removed: strList(r.removed),
    skipped: strList(r.skipped),
    skipped_reasons: strList(r.skipped_reasons),
    skipped_benign: r.skipped_benign === true,
    restored: strList(r.restored),
    warnings: strList(r.warnings),
  };
}

/// 해제 결과 → 토스트. 등급 규약은 설치와 같다: 사용자가 더 할 일이 없을 때만 성공("system"),
/// 하나라도 남았으면 경고("watchdog") + sticky 로 올리고 **사유와 복구 명령을 그대로 보여준다**.
///
/// ★(C3 · 2026-08-25 4R) 등급의 근거는 **기계 필드 둘뿐**이다 — `ok` 와 `skipped_benign`
/// (+줄별 분류는 `skipped_reasons`). 건너뛴 **문장은 읽지 않는다**.
///
/// ★그리고 `warnings.length > 0` 을 더는 실패로 읽지 않는다. 예전에는 그것도 실패 신호였는데,
/// 백엔드가 같은 배열에 **성격이 다른 세 가지**를 싣게 되었기 때문이다:
///   ① "…가 아직 남아 있습니다 — sudo rm …"  = 진짜 실패(그러나 이때는 `ok=false` 다)
///   ② "…원본을 그 자리에 되돌렸습니다"       = 복원 통보(성공의 일부)
///   ③ "…백업본이 아직 남아 있습니다"          = 고지
/// ②③까지 실패로 읽으면 **정상 해제가 ⚠부분 완료로 오보고**된다 — 이 라운드가 닫는 계열 결함
/// (판정을 채널의 '내용물 유무'로 추정하는 것)과 정확히 같은 형태다. 등급은 `ok` 가 말한다.
/// 다만 warnings 는 여전히 **읽혀야** 하므로, 성공이라도 warnings 가 있으면 sticky 로 올린다
/// (installResultToast 의 규약과 같은 모양 — 계열 대칭).
// ── (R1 · G2 UI 후속) 해제 뒤 **아직 남아 있는 우리 링크** — 복구 명령은 UI 가 조립한다 ──────
/// 5R 에서 백엔드가 `sudo rm …` 산문을 뺐다(사실만 보낸다: "…가 아직 남아 있습니다"). 그런데 UI 가
/// 그 명령을 조립하지 않아서, 해제가 부분 실패했을 때 **사용자가 복구 수단을 잃었다** — 문서
/// (docs/INSTALL.md)는 여전히 "복구 명령(`sudo rm <경로>`)을 그대로 보여줍니다" 라고 약속하고 있다.
/// 그 약속을 코드로 되돌린다. 새 필드는 만들지 않는다 — 근거는 전부 기존 기계 값이다:
///   후보 = `cli_install_status` 가 준 두 링크 경로(cys_link · cysd_link)
///   빼기  = `removed`(사후 재관측으로 정말 사라진 것)
///
/// ★건너뛴 항목은 **절대 후보가 아니다.** 그것들은 우리 것이 아니어서 손대지 않은 남의 파일이고,
/// 거기에 'sudo rm' 을 붙이면 우리가 지키기로 한 파일을 사용자 손으로 지우게 만든다(해제 확인 창이
/// "건드리지 않습니다" 라고 약속한 것과 정반대). 그래서 `skipped` 줄이 가리키는 경로도 뺀다.
///
/// 이때 읽는 것은 줄 **머리에 있는 경로 하나**뿐이다 — 그 경로 문자열은 우리가 이미 들고 있는
/// 기계 값이고, 사유 문구는 한 글자도 읽지 않는다(등급은 여전히 `skipped_reasons`·`skipped_benign`
/// 이 정한다 — C3 규약 불변). 그리고 **줄과 후보가 전부 대응하지 않으면 뺄셈을 신뢰하지 않고 빈
/// 배열을 돌려준다**: 형식이 바뀌면 '아무 것도 지목하지 않는' 쪽으로 무너지고, 파괴적 지시가 새어
/// 나가는 방향으로는 절대 무너지지 않는다(fail-closed).
export function uninstallLeftovers(rep: UninstallCliReport, links: readonly string[]): string[] {
  const removed = rep.removed.filter(Boolean);
  const skipped = rep.skipped.filter(Boolean);
  const candidates = links.filter(Boolean).filter((p) => !removed.includes(p));
  // "<경로>" 또는 "<경로> — 사유" 인가. 경로 뒤 공백까지 요구하므로 `/…/cys` 가 `/…/cysd` 줄을
  // 삼키지 않는다. 정규식이 아니라 **우리가 들고 있는 경로와의 동일성 검사**다.
  const owns = (line: string, path: string) => line === path || line.startsWith(`${path} `);
  const matched = skipped.filter((line) => candidates.some((p) => owns(line, p)));
  if (matched.length !== skipped.length) return [];
  return candidates.filter((p) => !skipped.some((line) => owns(line, p)));
}

export function uninstallResultToast(rep: UninstallCliReport, links: readonly string[] = []): ToastPlan {
  const removed = rep.removed.filter(Boolean);
  const skipped = rep.skipped;
  const warnings = rep.warnings.filter(Boolean);
  const restored = rep.restored.filter(Boolean);
  const parts = partitionSkips(skipped, rep.skipped_reasons, rep.skipped_benign === true);
  // 두 기계 신호 중 하나라도 "무해가 아니다" 라고 하면 조치 필요 쪽이다(안전한 방향).
  const needsAttention = parts.blocking.length > 0 || (skipped.length > 0 && rep.skipped_benign !== true);
  const failed = rep.ok !== true;
  const restoredNote =
    restored.length > 0
      ? `\n설치 때 백업해 둔 원본 ${restored.length}건을 그 자리에 되돌렸습니다(아래 목록 참조).`
      : "";
  // ★(G14 대칭) 글리프는 **그 토스트의 등급**을 따른다. 예전에는 성공(✅) 본문 안에도 `⚠` 줄이
  // 들어갔는데, 그러면 한 알림이 두 등급을 동시에 주장한다(제목은 괜찮다, 본문은 경고다). 해제는
  // warnings 에 "원본을 되돌렸습니다"(=성공의 일부)가 섞이므로 **등급을 낮추면 안 되고**(C3),
  // 대신 ✅ 안에서는 글리프를 중립(·)으로 낸다. 문구는 한 글자도 줄이지 않는다.
  const warnTail = (glyph: string) =>
    warnings.length > 0 ? `\n남은 조치·안내:\n${warnings.map((w) => `   ${glyph} ${w}`).join("\n")}` : "";

  if (failed || needsAttention) {
    const lines: string[] = [
      removed.length > 0 ? `제거: ${removed.join(" · ")}` : "제거한 항목 없음",
    ];
    if (restored.length > 0) lines.push(`되돌린 원본: ${restored.join(" · ")}`);
    if (skipped.length > 0) {
      lines.push(`건너뜀 ${skipped.length}건 — 직접 확인하세요:`);
      for (const sk of skipped) lines.push(`   • ${sk}`);
    }
    // (R1) 지우려 했으나 남은 우리 링크에는 **복구 명령**을 붙인다 — 백엔드는 사실만 보내므로
    // 이 문장이 없으면 사용자는 손으로 정리할 방법을 어디에서도 듣지 못한다.
    const leftovers = uninstallLeftovers(rep, links);
    if (leftovers.length > 0) {
      lines.push(`아직 남아 있는 링크 ${leftovers.length}건 — 직접 지우려면:`);
      for (const p of leftovers) lines.push(`   • ${p} — 'sudo rm ${p}'`);
    }
    if (warnings.length > 0) {
      lines.push("남은 조치·안내:");
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
    const tail = parts.benign.length > 0 ? `\n(이미 없던 항목: ${parts.benign.join(" · ")})` : "";
    return {
      category: "system",
      title: "✅ 셸 cys 해제 완료",
      body:
        `${removed.join(" · ")} 를 제거했습니다 — 새 터미널에서는 'cys' 명령이 더 이상 잡히지 않습니다.` +
        `${restoredNote}${tail}${warnTail("·")}`,
      // 복원 통보·백업 고지는 8초에 사라지면 안 된다 — 설치 쪽 규약과 같은 모양.
      sticky: warnings.length > 0,
      id: UNINSTALL_TOAST_ID,
    };
  }

  return {
    category: "watchdog",
    title: "⚠ 해제할 심링크 없음",
    body:
      "/usr/local/bin 에 이 앱이 만든 cys·cysd 심링크가 없습니다 — 지운 것이 없습니다." +
      (parts.benign.length > 0 ? `\n${parts.benign.join("\n")}` : "") +
      warnTail("⚠"),
    sticky: warnings.length > 0,
    id: UNINSTALL_TOAST_ID,
  };
}
