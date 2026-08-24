// '셸에 cys 설치 / 셸 cys 해제' 버튼의 순수 판정 — 플랫폼 노출·버튼 라벨·결과 토스트 등급을 여기서만 정한다.
//
// main.ts의 #btn-install-cli 핸들러는 이 함수들에 배선만 하고(invoke 호출·DOM 갱신), 플랫폼
// 판별에 쓸 userAgent 문자열도 호출측이 넘긴다(테스트 격리 — shellquote.ts 규약 계승).
// 순수 함수라 실제 관리자 승격(osascript) 없이 결정론 회귀 테스트가 가능하다(clipath.test.ts).
//
// 이력: 2026-06-29 신설 → 2026-08-20(3685af9) 버튼 제거 → 결함 4종(플랫폼 게이팅 없음·그림자화를
// 성공으로 보고·해제 경로 없음·검증 실패를 성공으로 접음) 수리와 함께 복원.

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

// ── 설치 결과 등급 ─────────────────
/// Rust InstallCliReport.status 계약 — 정확히 세 값.
///   installed          = 심링크 생성 + `which -a cys` 1순위가 /usr/local/bin/cys
///   installed_shadowed = 심링크는 생겼으나 PATH 앞을 가리는 다른 cys가 있다
///   unverified         = which -a 검증을 수행하지 못했다(실행 실패·타임아웃)
export type CliInstallStatus = "installed" | "installed_shadowed" | "unverified";

export type InstallCliReport = {
  ok?: boolean;
  status?: unknown;
  target_dir?: string;
  cys_link?: string;
  cysd_link?: string;
  source_cys?: string;
  effective_cys?: string | null;
  shadowed_by?: string | null;
  warnings?: string[];
};

/// 알 수 없는 값·필드 누락은 전부 "unverified"로 접는다 — **측정 불능은 통과가 아니다**(헌장).
/// 구버전 백엔드(status 필드 없음)와 붙어도 성공으로 둔갑하지 않는 것이 이 함수의 존재 이유다.
export function normalizeInstallStatus(raw: unknown): CliInstallStatus {
  if (raw === "installed" || raw === "installed_shadowed" || raw === "unverified") return raw;
  return "unverified";
}

export type ToastPlan = { category: string; title: string; body: string };

/// 설치 결과 → 토스트 등급·문구. installed 만 성공("system"), 나머지 둘은 경고("watchdog")로
/// 등급을 낮춘다. 이 코드베이스의 토스트 등급은 테두리색 하나뿐이고 watchdog은 완료 알림에도
/// 쓰이므로(main.ts "✅ 데몬 재시작 완료"), 제목 접두 ✅/⚠ 로 등급을 눈에 보이게 못박는다.
export function installResultToast(rep: InstallCliReport): ToastPlan {
  const status = normalizeInstallStatus(rep.status);
  const links = [rep.cys_link, rep.cysd_link].filter(Boolean).join(" · ");
  const warn = (rep.warnings ?? []).filter(Boolean);
  const tail = warn.length > 0 ? `\n⚠ ${warn.join("\n⚠ ")}` : "";

  if (status === "installed") {
    return {
      category: "system",
      title: "✅ 셸 설치 완료",
      body: `${links} — 새 터미널에서 'cys' 를 바로 쓸 수 있습니다.${tail}`,
    };
  }
  if (status === "installed_shadowed") {
    const by = rep.shadowed_by || "(경로 미상)";
    return {
      category: "watchdog",
      title: "⚠ 셸 설치 미완료 — 다른 cys가 앞을 가립니다",
      body:
        `심링크(${links})는 만들었지만, PATH 앞쪽의 ${by} 가 먼저 잡힙니다 — 터미널에서 'cys' 를 치면 ` +
        `아직 그쪽이 실행됩니다. 그 파일을 지우거나 PATH에서 /usr/local/bin 을 앞으로 옮긴 뒤, ` +
        `새 터미널에서 'which -a cys' 로 1순위를 확인하세요.${tail}`,
    };
  }
  return {
    category: "watchdog",
    title: "⚠ 셸 설치 확인 불가",
    body:
      `심링크(${links})는 만들었지만, 실제로 어떤 cys가 잡히는지 확인하지 못했습니다` +
      `(검증 명령 실패 또는 응답 없음). 새 터미널에서 'which -a cys' 를 직접 실행해 ` +
      `1순위가 /usr/local/bin/cys 인지 확인하세요.${tail}`,
  };
}

// ── 버튼 상태(설치 ↔ 해제) ─────────────────
/// 버튼 하나를 상태 2종으로 쓴다. "unknown" 은 상태 조회 실패·미응답 —
/// **모르면 설치 쪽**으로 둔다(설치는 멱등한 `ln -sf`, 해제는 비가역에 가깝다).
export type CliButtonState = "installed" | "absent" | "unknown";

export type CliInstallStatusReport = {
  installed?: unknown;
  cys_installed?: unknown;
  status?: unknown;
  cys_link?: string;
  cysd_link?: string;
};

/// cli_install_status(읽기 전용·승격 없음) 응답의 관용 판독기. `installed: bool` 이 1순위 계약이고,
/// 경로별 플래그(`cys_installed`)·설치 결과와 같은 status enum도 받아 준다 — 계약이 조금 어긋났을 때
/// 해제 버튼이 **조용히 도달 불가**가 되는 것을 막는 보험이다. 그 밖엔 전부 "unknown"(=설치 라벨).
export function readInstallState(raw: unknown): CliButtonState {
  if (!raw || typeof raw !== "object") return "unknown";
  const r = raw as CliInstallStatusReport;
  if (typeof r.installed === "boolean") return r.installed ? "installed" : "absent";
  if (typeof r.cys_installed === "boolean") return r.cys_installed ? "installed" : "absent";
  if (r.status === "installed" || r.status === "installed_shadowed") return "installed";
  if (r.status === "absent" || r.status === "not_installed") return "absent";
  return "unknown";
}

/// 상태 → 버튼 라벨·툴팁. 라벨만 바꾸고 title을 두면 '해제' 버튼이 '설치' 안내를 달고 있게 되므로
/// 둘을 **같은 함수에서 함께** 산출한다(cc-header 라벨 동적 변경 선례: applyCcDensity·applyGlanceFace).
export function cliButtonView(state: CliButtonState): { label: string; title: string } {
  if (state === "installed") {
    return {
      label: "셸 cys 해제",
      title: "/usr/local/bin 의 cys·cysd 심링크 제거(1회 관리자 승인) — 확인 창이 먼저 뜹니다",
    };
  }
  if (state === "absent") {
    return {
      label: "셸에 cys 설치",
      title: "외부 터미널에서 cys 명령 쓰기(1회 관리자 승인)",
    };
  }
  return {
    label: "셸에 cys 설치",
    title: "외부 터미널에서 cys 명령 쓰기(1회 관리자 승인) — 현재 설치 상태는 확인하지 못했습니다",
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

export type UninstallSkip = { path?: string; reason?: string };
export type UninstallCliReport = {
  ok?: boolean;
  removed?: string[];
  skipped?: UninstallSkip[];
};

/// 해제 결과 → 토스트. 설치와 같은 등급 규약: 전부 정리됐을 때만 성공("system"),
/// 하나라도 건너뛰었거나 지운 게 없으면 경고("watchdog")로 낮추고 사유를 문장으로 준다.
export function uninstallResultToast(rep: UninstallCliReport): ToastPlan {
  const removed = (rep.removed ?? []).filter(Boolean);
  const skipped = (rep.skipped ?? []).filter((s) => s && s.path);
  const skipText = skipped.map((s) => `   • ${s.path}: ${s.reason || "사유 미상"}`).join("\n");

  if (skipped.length === 0 && removed.length > 0) {
    return {
      category: "system",
      title: "✅ 셸 cys 해제 완료",
      body: `${removed.join(" · ")} 를 제거했습니다 — 새 터미널에서는 'cys' 명령이 더 이상 잡히지 않습니다.`,
    };
  }
  if (removed.length === 0 && skipped.length === 0) {
    return {
      category: "watchdog",
      title: "⚠ 해제할 심링크 없음",
      body: "/usr/local/bin 에 이 앱이 만든 cys·cysd 심링크가 없습니다 — 지운 것이 없습니다.",
    };
  }
  return {
    category: "watchdog",
    title: "⚠ 셸 cys 해제 부분 완료",
    body:
      (removed.length > 0 ? `제거: ${removed.join(" · ")}\n` : "제거한 항목 없음\n") +
      `건너뜀 ${skipped.length}건 — 직접 확인하세요:\n${skipText}`,
  };
}
