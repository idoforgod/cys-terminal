// macOS 폴더 접근(TCC) 안내 문구의 단일 진실(SOT) — 순수 모듈(최상위 부수효과 0 · DOM/Tauri 무관).
//
// ★(0.14.41 · U14/U18) 사실 근거(phase1 조사·반박 보고서):
//   · 시스템 설정 목록에 실제로 뜨는 이름은 앱 번들 표시 이름 「cys」 하나다. 좌석의 claude 가
//     폴더를 읽을 때 책임 프로세스가 launchd 의 cysd 든 앱이 띄운 부서 데몬이든 macOS 는 전부
//     번들(com.cysjavis.terminal)로 묶는다(tccd 로그 164/164 · 반박 재확인 177건). 그 이름의 원천은
//     src-tauri/tauri.conf.json 의 productName 이고, 파리티 핀(folderaccess.test.ts)이 둘을 묶는다.
//   · 「파일 및 폴더」 첫 화면은 **앱 목록**이다(반박 R2 — 시스템 문자열 근거). 그래서 동선은 한 줄로
//     쓰고("「파일 및 폴더」에서 「cys」의 데스크탑 접근") 버전별 동선은 매뉴얼로 보낸다.
//   · 토스트는 **두 문장**이다: 무엇이 막혔나 + 누르면 설정 화면이 열린다. 전체 디스크 접근 ＋추가는
//     자율 에이전트가 도는 앱에 과한 권한이고 거의 닿지 않는 경우라(반박 D2) 매뉴얼의 최후 수단으로만 둔다.
//   · 로그인 항목의 데몬 줄은 앱이 아니라 **개발자 이름** 줄(서명자)에 붙어 보일 수 있다(BTM 기록
//     `Parent Identifier: yoonsik choi` — 반박 M5).
//
// ★구형 WKWebView 호환: lookbehind·.at()·findLast·structuredClone·Object.hasOwn 사용 0.

/** 시스템 설정 목록에 보이는 앱 이름 — tauri.conf.json productName 과 같아야 한다(파리티 핀). */
export const PRIVACY_APP_NAME = "cys";

/** 로그인 항목 「백그라운드에서 허용」에서 데몬 스위치가 붙는 개발자 이름 줄(Developer ID 서명자). */
export const SIGNER_DISPLAY_NAME = "yoonsik choi";

/** 폴더 접근 안내 sticky 토스트 id 접두 — toastttl.ts 가 이 접두에 연장 수명을 준다. */
export const PERM_TOAST_PREFIX = "perm-";

/** `open_privacy_settings` 가 받는 고정 target(Rust 쪽 고정 URL 표와 짝). */
export type PrivacyTarget = "files" | "login";

export interface GuideToast {
  id: string;
  title: string;
  detail: string;
  target: PrivacyTarget;
}

const FOLDER_LABELS: Record<string, string> = {
  Desktop: "데스크탑",
  Documents: "문서",
  Downloads: "다운로드",
};

/** 홈 아래 보호 폴더 이름 → 한국어 표시 이름. 모르는 값은 null(거짓 '데스크탑' 금지 — 조사 R4). */
export function folderLabel(folder: unknown): string | null {
  if (typeof folder !== "string") return null;
  return Object.prototype.hasOwnProperty.call(FOLDER_LABELS, folder) ? FOLDER_LABELS[folder] : null;
}

const OPEN_HINT = "이 알림을 누르면 설정 화면이 열립니다.";

/**
 * GUI 자체 점검(앱 기동 시 데스크탑·문서 읽기 — src-tauri nudge_folder_permissions)이 거부를 본 경우.
 * 폴더 값이 없으면 null(렌더하지 않는다).
 */
export function permWarningToast(folder: unknown, appName: string = PRIVACY_APP_NAME): GuideToast | null {
  if (typeof folder !== "string" || !folder) return null;
  const label = folderLabel(folder);
  if (label) {
    return {
      id: `${PERM_TOAST_PREFIX}${folder}`,
      title: `⚠ macOS ${label} 폴더 접근이 꺼져 있습니다`,
      detail: `「파일 및 폴더」에서 「${appName}」의 ${label} 접근이 꺼져 있어 팀 자리가 이 폴더를 읽지 못합니다. ${OPEN_HINT}`,
      target: "files",
    };
  }
  return {
    id: `${PERM_TOAST_PREFIX}${folder}`,
    title: `⚠ macOS 폴더 접근이 꺼져 있습니다: ${folder}`,
    detail: `「파일 및 폴더」에서 「${appName}」의 접근 허용을 확인해야 팀 자리가 ${folder} 를 읽을 수 있습니다. ${OPEN_HINT}`,
    target: "files",
  };
}

/** 3초 목록 루프가 넘기는 좌석 1건(scope = 데몬 소켓 식별 — 부서마다 같은 역할명이 있다). */
export interface CwdBlockedEntry {
  scope: string;
  role: string | null | undefined;
  cwd_blocked: unknown;
}

interface BlockedFact {
  path: string;
  folder: string | null;
}

function readBlocked(v: unknown): BlockedFact | null {
  if (!v || typeof v !== "object") return null;
  const o = v as { path?: unknown; folder?: unknown };
  if (typeof o.path !== "string" || !o.path) return null;
  return { path: o.path, folder: typeof o.folder === "string" ? o.folder : null };
}

const MAX_ROLES_LISTED = 4;

/**
 * 좌석 작업 폴더 막힘(U18 — 데몬 surface.list 의 cwd_blocked) → 폴더별 고정 토스트.
 *
 * 규약:
 *   · 같은 폴더(보호 폴더면 그 이름 · 아니면 경로)의 좌석은 **한 장**으로 묶는다.
 *   · 이미 알린 좌석(scope|role|path)은 다시 알리지 않는다 — 3초마다 같은 id 를 다시 띄우면 TTL 이
 *     영원히 리셋되고(오너 요구 = 종류 불문 소멸) 알람 이력도 잠식한다.
 *   · 그 폴더에 **새 좌석**이 오면 그 폴더 한 장을 갱신한다(지금 막힌 역할 전부 나열).
 *   · 역할 없는 항목·형식이 깨진 값은 건너뛴다(throw 0).
 * 순수 함수 — 입력 집합을 변형하지 않고 새 집합을 돌려준다.
 */
export function cwdBlockedNotices(
  seen: ReadonlySet<string>,
  entries: readonly CwdBlockedEntry[],
  appName: string = PRIVACY_APP_NAME,
): { notices: GuideToast[]; seen: Set<string> } {
  const next = new Set(seen);
  const groups = new Map<string, { label: string | null; path: string; roles: string[]; fresh: boolean }>();
  for (const e of entries) {
    if (!e || typeof e.role !== "string" || !e.role) continue;
    const b = readBlocked(e.cwd_blocked);
    if (!b) continue;
    const label = folderLabel(b.folder);
    const gkey = label ? String(b.folder) : b.path;
    const key = `${e.scope}|${e.role}|${b.path}`;
    let g = groups.get(gkey);
    if (!g) {
      g = { label, path: b.path, roles: [], fresh: false };
      groups.set(gkey, g);
    }
    if (g.roles.indexOf(e.role) < 0) g.roles.push(e.role);
    if (!next.has(key)) {
      next.add(key);
      g.fresh = true;
    }
  }
  const notices: GuideToast[] = [];
  groups.forEach((g, gkey) => {
    if (!g.fresh) return;
    const shown = g.roles.slice(0, MAX_ROLES_LISTED).join(", ");
    const more = g.roles.length > MAX_ROLES_LISTED ? ` 외 ${g.roles.length - MAX_ROLES_LISTED}` : "";
    const who = `${shown}${more}`;
    if (g.label) {
      notices.push({
        id: `${PERM_TOAST_PREFIX}seat-${gkey}`,
        title: `⚠ 팀 자리가 ${g.label} 폴더를 읽지 못합니다`,
        detail: `「파일 및 폴더」에서 「${appName}」의 ${g.label} 접근이 꺼져 있어 ${who} 자리가 작업 폴더를 읽지 못합니다. ${OPEN_HINT}`,
        target: "files",
      });
    } else {
      notices.push({
        id: `${PERM_TOAST_PREFIX}seat-${gkey}`,
        title: `⚠ 팀 자리가 작업 폴더를 읽지 못합니다: ${g.path}`,
        detail: `macOS 폴더 접근 권한 때문에 ${who} 자리가 ${g.path} 를 읽지 못합니다 — 「파일 및 폴더」에서 「${appName}」의 허용을 확인하세요. ${OPEN_HINT}`,
        target: "files",
      });
    }
  });
  return { notices, seen: next };
}

/**
 * 데몬 대기 안내(로그인 항목) — 목록에 실제로 보이는 두 줄을 말한다.
 * `clickable` = 이 알림에 '설정 열기' 클릭이 붙었는가(macOS 만). 붙지 않았으면 그 문장을 말하지 않는다
 * (이 안내는 모든 OS 에서 뜨는 경로라, 클릭이 없는 곳에서 "누르면 열린다"고 말하면 거짓이 된다).
 */
export function loginItemsGuide(
  clickable: boolean = true,
  appName: string = PRIVACY_APP_NAME,
  signer: string = SIGNER_DISPLAY_NAME,
): string {
  return (
    `백그라운드 서비스(cysd) 시작을 기다리고 있습니다. 계속 이 상태면 시스템 설정 → 일반 → 로그인 항목의 ` +
    `「백그라운드에서 허용」에서 「${appName}」와 개발자 이름 줄(「${signer}」)을 모두 켜 주세요 — ` +
    (clickable ? `이 알림을 누르면 그 화면이 열리고, ` : ``) +
    `허용 즉시 자동으로 연결됩니다.`
  );
}

/** macOS 폴더 접근 거부(EPERM · os error 1)인가 — EACCES(13)·Windows 5 는 아니다. */
export function isMacFolderPermissionError(err: unknown): boolean {
  const s = err instanceof Error ? err.message : typeof err === "string" ? err : "";
  return /\(os error 1\)/.test(s) || /^Operation not permitted/.test(s);
}

/** 파일 트리에서 읽기가 막힌 폴더 자리에 보이는 한 줄(빈 폴더처럼 보이지 않게). */
export const FT_BLOCKED_TEXT = `⚠ macOS 폴더 접근이 막혀 읽을 수 없습니다 — 눌러서 「파일 및 폴더」 열기`;
