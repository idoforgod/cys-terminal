// cys UI — xterm.js panes over the cysd socket (thin client).
// 세션 영속은 구조로 해결: 세션(PTY)은 데몬 소유, UI는 attach만 한다.

import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { isPermissionGranted, requestPermission, sendNotification } from "@tauri-apps/plugin-notification";
import { imeStep, initialImeState, isHangulText, type ImeEvent } from "./ime";
import { shellQuote } from "./shellquote";
import { baseName, insertionText, isStreaming, splitPath } from "./ftdrop";
import { transferTrees } from "./transfer";
import { updatePlan } from "./updateplan";
import { DEFAULT_BG, readableForeground } from "./theme";
import { reorderWorkspace, reorderGroup } from "./reorder";
import { classifyDrainVerifyFallback, drainVerifyFallbackToast } from "./drainverify";
import { classifyPendingFeed, CYCLE_VERIFY_NOTE, CYCLE_VERIFY_DISMISS_TITLE } from "./feedclass";
import {
  deptPlaceholderLabel,
  deptSlugOfSocket,
  pickDeptWorkspace,
  isActiveDeptSocket,
  DEFAULT_SOCKET_KEY,
  type DeptSwitchOutcome,
} from "./deptlabel";
import { purgeNameMatches, purgeMismatchHint, PURGE_INPUT_GUARDS } from "./purgeconfirm";
import {
  RESET_PHRASE,
  resetPhraseMatches,
  resetMismatchHint,
  resetNoticeLines,
  resetResultTitle,
  resetResultBody,
  type ResetPreview,
  type ResetResult,
} from "./resetconfirm";
import { ccEffectiveZoom } from "./ccscale";
import { clampWsbarWidth, clampWsbarFont, WSBAR_W_DEFAULT, WSBAR_FONT_STEP } from "./wsbar";
import { composeFontFamily, FONT_CHOICES, ROLE_COLOR, roleDotColor } from "./appearance";
import { routeOnData } from "./mousefilter";
import { MouseTrackingFilter, MOUSE_ALL_OFF } from "./trackfilter";
import {
  shouldSuppressWheel,
  shouldSuppressWheelWin,
  wheelHandlerKind,
  macGateInputs,
  winGateInputs,
} from "./wheelgate";
import { ceoPaletteEntries } from "./selfdiag";
import {
  isMacUserAgent,
  installResultToast,
  readCliStatus,
  readInstallReport,
  readUninstallReport,
  cliButtonView,
  cliButtonIntent,
  cliNoticeLines,
  withCliNotice,
  normalizeInstallStatus,
  statusNoticePlan,
  uninstallConfirmText,
  uninstallResultToast,
  toastClassName,
  toastEmitPlan,
  INSTALL_TOAST_ID,
  UNINSTALL_TOAST_ID,
  type CliStatusView,
  type LastInstallOutcome,
  type ToastPlan,
} from "./clipath";
import {
  toastTtl,
  toastTimerPlan,
  needsExpiryBanner,
  expiryBannerText,
  pushAlarm,
  formatAlarmTime,
  type AlarmRecord,
} from "./toastttl";

declare global {
  interface Window {
    __TAURI__: {
      core: { invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown> };
      event: {
        listen: (
          name: string,
          handler: (e: { payload: unknown }) => void,
        ) => Promise<() => void>;
      };
    };
  }
}
// 플랫폼 판별은 세션·페인마다 다시 할 이유가 없다 — 모듈 로드 시 1회만 읽는다(마우스 보고 필터가 소비).
const IS_WINDOWS = /Windows/i.test(navigator.userAgent);
// 셸 설치/해제 버튼은 macOS 전용(Rust install_cli_to_path 가 그 밖에서는 즉시 Err). 부정 판정
// (!IS_WINDOWS)이면 Linux가 통과해 "보이는데 안 되는 버튼" 결함이 그대로 재현되므로 양성 판정을 쓴다.
const IS_MACOS = isMacUserAgent(navigator.userAgent);

const invoke = (cmd: string, args?: Record<string, unknown>) => window.__TAURI__.core.invoke(cmd, args);
const listen = (name: string, handler: (e: { payload: unknown }) => void) =>
  window.__TAURI__.event.listen(name, handler);

// ---------- layout model (v2: multiple workspaces, splits with ratio) ----------

type Node =
  | { type: "split"; dir: "row" | "col"; ratio?: number; a: Node; b: Node }
  | { type: "pane"; sid: number };

interface Workspace {
  id: number;
  name: string;
  tree: Node | null;
  // 멀티마스터 F4: 이 workspace가 붙은 부서 데몬 소켓(undefined=기본 데몬). 한 ws의 모든 pane은 같은 socket.
  socket?: string;
  // 부서 런칭 중 임시 placeholder 표식 — 무거운 launch await 동안 탭을 즉시 표시(체감 지연 0)하기 위함.
  // launch 완료 시 false로 내리고, 실패 시 ws 자체를 제거한다. 직렬화 제외(normalizeWorkspaces)로 디스크/복원 누수 차단.
  pending?: boolean;
  // 06: 소속 그룹 id(undefined=ungrouped). 부서 ws도 그룹에 들어가면 set. 진실원=localStorage(cys-layout-v2).
  groupId?: number;
}

// 06: 워크스페이스 그룹 메타데이터. 진실원=localStorage(cys-layout-v2). 데몬은 모름(그룹=UI/solution 층).
// 부서(데몬)도 일반 그룹과 동일 구조로 표현 — anchorSocket이 있으면 부서 그룹(읽기전용 표식·teardown은 ws close가 담당).
interface GroupMeta {
  id: number;
  name: string;
  collapsed: boolean;
  pinned: boolean;
  color?: string; // hex(미지정 시 id 기반 WS_COLORS 폴백)
  anchorSocket?: string; // 부서 그룹이면 부서 데몬 socket
}

const LAYOUT_KEY = "cys-layout-v2";

// pane 식별 복합키 — 서로 다른 데몬이 같은 surface_id를 독립 발급하므로 (socket, sid)로 구분한다.
const paneKey = (sid: number, socket?: string): string => `${socket ?? ""}#${sid}`;

interface PaneRuntime {
  sid: number;
  socket?: string;
  el: HTMLElement;
  termHost: HTMLElement;
  roleEl: HTMLElement; // 제목 앞 역할 신호 점(깜박임) — refreshPaneTitles가 role로 채색
  titleEl: HTMLElement;
  usageEl: HTMLElement;
  term: Terminal;
  fit: FitAddon;
  unlisten: (() => void)[];
  observer: ResizeObserver;
  // 바닥 고정 재적용 — 리사이즈(fitPane) 뒤 뷰포트가 바닥에서 밀려나면 복귀시킨다.
  snapToBottom: () => void;
  // 마지막 PTY 출력 시각(ms) — 경로 주입의 스트리밍 가드(ftdrop.isStreaming)가 읽는다.
  lastOutputAt: () => number;
  // IME 조합 pending 여부 — 조합 중 주입의 순서 역전(자모 뒤섞임) 차단.
  imeBusy: () => boolean;
  // 마우스 트래킹 정합기(스펙 D4) — agent.exited 리셋 훅이 세대 캡처·장부 소거에 쓴다.
  trackFilter: MouseTrackingFilter;
}

// ---------- T5 사용량 관측 배지 (pane 헤더) ----------

interface RateWindow {
  label: string;
  used_pct: number;
  resets_at: number | null;
}
interface ObservedUsage {
  agent: string;
  ctx_tokens: number | null;
  ctx_window: number | null;
  ctx_pct: number | null;
  rate: RateWindow[];
  source: string;
  session_file: string;
  updated_at: number;
}

const sevClass = (pct: number, warn: number, crit: number): string =>
  pct >= crit ? "crit" : pct >= warn ? "warn" : "";

// 컨텍스트는 60%(/clear 사이클 임계)·80%, rate limit은 70%·90%에서 단계 상승
function renderUsage(el: HTMLElement, u: ObservedUsage | null | undefined) {
  el.replaceChildren();
  if (!u) {
    el.title = "";
    return;
  }
  const parts: { text: string; cls: string }[] = [];
  if (u.ctx_pct !== null && u.ctx_pct !== undefined)
    parts.push({ text: `CTX ${u.ctx_pct}%`, cls: sevClass(u.ctx_pct, 60, 80) });
  for (const w of u.rate ?? [])
    parts.push({ text: `${w.label} ${Math.round(w.used_pct)}%`, cls: sevClass(w.used_pct, 70, 90) });
  if (!parts.length) {
    el.title = "";
    return;
  }
  parts.forEach((p, i) => {
    const s = document.createElement("span");
    s.textContent = (i ? "·" : "") + p.text;
    if (p.cls) s.className = p.cls;
    el.appendChild(s);
  });
  const tip: string[] = [`${u.agent} 사용량 (관측: ${u.source})`];
  if (u.ctx_tokens != null && u.ctx_window != null)
    tip.push(`context ${u.ctx_tokens.toLocaleString()} / ${u.ctx_window.toLocaleString()} tokens`);
  for (const w of u.rate ?? []) {
    const reset = w.resets_at ? ` — reset ${new Date(w.resets_at * 1000).toLocaleString()}` : "";
    tip.push(`rate ${w.label}: ${w.used_pct}%${reset}`);
  }
  const age = Math.max(0, Math.round(Date.now() / 1000 - u.updated_at));
  if (age > 120) tip.push(`⚠ ${Math.round(age / 60)}분 전 관측 (stale)`);
  el.title = tip.join("\n");
  el.classList.toggle("stale", age > 120);
}

// ---------- T6 Control Center (전용 풀 패널 — 네이티브 실시간 모니터링) ----------
let ccOpen = false;
let ccTimer: number | null = null;
let ccHwTimer: number | null = null;
let ccClockTimer: number | null = null;
let ccUptimeBase = 0;
let ccUptimeFetchedAt = 0;
type CcTab =
  | "live" | "eff" | "skills" | "sessions" | "weekly" | "learn"
  | "board" | "tasks" | "feed" | "alarms" | "office";
let ccTab: CcTab = "office";
let ccEffWindow = "today";
let ccSkillsWindow = "today";
let ccSessionsWindow = "7d";
let ccSessionsStarOnly = false;
let ccSessionsRedact = false;
let ccSessionSelected: string | null = null;
// 계정 Rate Limit(전 조직 병합 뷰) — usage_accounts_all 최신 스냅샷. Live KPI/게이지가 이 데이터를 폴백보다 우선.
let ccAccounts: any[] = [];
// 계정 라벨(이메일) 가림 토글 — 스크린샷 공유용. 결정론 해시 6자로 치환.
let ccAcctRedact = localStorage.getItem("cys-cc-acct-redact") === "1";
// 스킬 보드 카탈로그 캐시 + 검색어 — 검색은 재fetch 없이 renderBoardDomains 재렌더(깜빡임 방지).
let ccBoardCatalog: any = { domains: [], actions: [] };
let ccBoardSearch = "";
// 보드 버튼 호출수 뱃지(B2) — control_skills 7d by_skill 캐시(스킬명→호출수).
let ccBoardCalls: Record<string, number> = {};

// HUD-5: 밀도 모드 — 비기술자 Glance(오늘 큰 글씨) ↔ 엔지니어 Ops(6탭). body class 1개가 진실원.
type CcDensity = "ops" | "glance";
let ccDensity: CcDensity =
  (localStorage.getItem("cys-cc-density") as CcDensity) === "glance" ? "glance" : "ops";
// Tasks Control Center: Glance 모드 안에서 보여줄 면(Live=시스템부하 ↔ tasks=부서 업무) — 오너 선택.
let ccGlanceFace: "live" | "tasks" =
  localStorage.getItem("cys-cc-glance-face") === "tasks" ? "tasks" : "live";
// 마지막 org_fleet 스냅샷 — 실시간 이벤트(task.changed/status.changed)가 셀 단위로 패치한다.
let lastFleet: any = null;

const CC_ROLE_COLOR = ROLE_COLOR; // 역할색 단일 출처 = appearance.ts (pane 역할 점과 공유)
const CC_STATE: Record<string, { cls: string; label: string }> = {
  working: { cls: "working", label: "작업중" }, idle: { cls: "idle", label: "대기" },
  error: { cls: "error", label: "오류" }, offline: { cls: "offline", label: "오프라인" },
};
const ccEsc = (s: string) =>
  s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]!);
const ccFmtTokens = (n: number) => (n >= 10000 ? `${(n / 10000).toFixed(1)}만` : n.toLocaleString());
// 비용: $1 미만은 4자리(소액 가시), 이상은 2자리.
const ccMoney = (v: number) => `$${v > 0 && v < 1 ? v.toFixed(4) : v.toFixed(2)}`;
const CC_TOK_SEG: [string, string, string][] = [
  ["input", "입력", "#3b82f6"], ["output", "출력", "#00e676"],
  ["cache_creation", "캐시생성", "#ffa726"], ["cache_read", "캐시읽기", "#8b5cf6"],
];

function ccUptimeStr(s: number): string {
  const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), sec = s % 60;
  return [h, m, sec].map((x) => String(x).padStart(2, "0")).join(":");
}
function ccReset(label: string, epoch: number | null): string {
  if (!epoch) return "";
  const d = new Date(epoch * 1000);
  const p = (x: number) => String(x).padStart(2, "0");
  return label === "7d"
    ? `리셋 ${p(d.getMonth() + 1)}/${p(d.getDate())}`
    : `리셋 ${p(d.getHours())}:${p(d.getMinutes())}`;
}
function ccAggRate(fleet: any[]): Record<string, { used: number; reset: number | null }> {
  const agg: Record<string, { used: number; reset: number | null }> = {};
  for (const f of fleet) {
    for (const w of f.usage?.rate ?? []) {
      const cur = agg[w.label] ?? { used: 0, reset: null };
      if (w.used_pct > cur.used) cur.used = w.used_pct;
      if (w.resets_at != null && (cur.reset == null || w.resets_at < cur.reset)) cur.reset = w.resets_at;
      agg[w.label] = cur;
    }
  }
  return agg;
}

// 시:분(epoch 초) — 계정 소진 예상·최근 실행 시작시각의 간단 표기.
function ccHHMM(epoch: number): string {
  if (!Number.isFinite(epoch) || epoch <= 0) return "";
  const d = new Date(epoch * 1000);
  const p = (x: number) => String(x).padStart(2, "0");
  return `${p(d.getHours())}:${p(d.getMinutes())}`;
}
// 자산 mtime(epoch 초) → YYYY.MM.DD — 자산 칩 툴팁.
function ccAssetDate(mtime: any): string {
  const n = Number(mtime);
  if (!Number.isFinite(n) || n <= 0) return "";
  const d = new Date(n * 1000);
  const p = (x: number) => String(x).padStart(2, "0");
  return `${d.getFullYear()}.${p(d.getMonth() + 1)}.${p(d.getDate())}`;
}
// 결정론 해시 6자(djb2) — 계정 라벨 가림(스크린샷용). 같은 라벨은 항상 같은 해시.
function ccHash6(s: string): string {
  let h = 5381;
  for (let i = 0; i < s.length; i++) h = ((h << 5) + h + s.charCodeAt(i)) >>> 0;
  return h.toString(16).padStart(8, "0").slice(0, 6);
}
const ccAcctLabel = (label: string): string => (ccAcctRedact ? `#${ccHash6(label)}` : label);
// 지정 rate 라벨(5h/7d)에서 사용률 최고 계정 — Live KPI/게이지의 "최고 사용 계정 기준" 값.
function ccAcctMax(label: string): { used: number; reset: number | null; acct: string } | null {
  let best: { used: number; reset: number | null; acct: string } | null = null;
  for (const a of ccAccounts) {
    for (const r of a.rate ?? []) {
      if (r.label !== label) continue;
      const used = Number(r.used_pct);
      if (!Number.isFinite(used)) continue;
      if (!best || used > best.used)
        best = { used, reset: r.resets_at ?? null, acct: String(a.label ?? a.account_id ?? "?") };
    }
  }
  return best;
}

// 계정 Rate Limit 섹션 — 전 조직 병합 계정을 provider·라벨·plan·5h/7d 게이지·리셋·관측 뱃지로 렌더.
function renderAccounts() {
  const host = document.getElementById("cc-accounts");
  if (!host) return;
  if (!ccAccounts.length) {
    host.innerHTML = `<div class="cc-empty">관측된 계정 없음</div>`;
    return;
  }
  host.innerHTML = ccAccounts
    .map((a) => {
      const prov = ccEsc(String(a.provider ?? "?"));
      const label = ccEsc(ccAcctLabel(String(a.label ?? a.account_id ?? "?")));
      const plan = a.plan ? `<span class="cc-acct-plan">${ccEsc(String(a.plan))}</span>` : "";
      // 게이지 — rate limit 임계(70/90)로 sevClass. cc-tbar 재사용.
      const gauges = ["5h", "7d"]
        .map((lab) => {
          const r = (a.rate ?? []).find((x: any) => x.label === lab);
          const used = r ? Math.round(Number(r.used_pct)) : 0;
          const reset = r && r.resets_at != null ? ccReset(lab, r.resets_at) : "";
          const fill = r ? `<span class="cc-tbar-fill ${sevClass(used, 70, 90)}" style="width:${Math.min(100, used)}%"></span>` : "";
          return `<div class="cc-tbar"><span class="cc-tbar-lab">${lab}</span><span class="cc-tbar-track">${fill}</span><span class="cc-tbar-pct">${r ? used + "%" : "—"}</span><span class="cc-tbar-reset">${reset}</span></div>`;
        })
        .join("");
      const badges: string[] = [];
      if (a.updated_at == null) badges.push(`<span class="cc-acct-badge">관측 없음</span>`);
      if (a.adapter === false) badges.push(`<span class="cc-acct-badge">관측 어댑터 없음</span>`);
      const stale = Number(a.stale_secs);
      if (Number.isFinite(stale) && stale > 120) badges.push(`<span class="cc-acct-badge">${Math.round(stale / 60)}분 전 관측</span>`);
      if (a.exhaust_at != null) badges.push(`<span class="cc-acct-badge warn">이 속도면 ${ccHHMM(Number(a.exhaust_at))} 소진</span>`);
      return `<div class="cc-acct-row"><span class="cc-acct-prov">[${prov}]</span><span class="cc-acct-label">${label}</span>${plan}<div class="cc-acct-gauges">${gauges}</div><span class="cc-acct-badges">${badges.join("")}</span></div>`;
    })
    .join("");
}

// HUD-5: 밀도 전환 — 순수 CSS(body class)가 진실원. JS는 class 토글 + 영속 + 버튼 라벨만.
function applyCcDensity(mode: CcDensity) {
  ccDensity = mode;
  document.body.classList.toggle("cc-glance", mode === "glance");
  localStorage.setItem("cys-cc-density", mode);
  const b = document.getElementById("btn-cc-density");
  if (b) b.textContent = mode === "glance" ? "🔍 상세보기" : "👁 한눈에";
  // Glance는 단일 면 — 오너 선택(Live=시스템부하 ↔ tasks=부서 업무)으로 전환. 분석 전용 탭이면 그 면으로.
  if (mode === "glance") applyGlanceFace(ccGlanceFace);
}

// Glance 면 토글(오너: Live↔작업, 선택된 면을 크게). 토글 버튼은 Glance에서만 보인다(CSS).
function applyGlanceFace(face: "live" | "tasks") {
  ccGlanceFace = face;
  localStorage.setItem("cys-cc-glance-face", face);
  const fb = document.getElementById("btn-cc-glance-face");
  if (fb) fb.textContent = face === "tasks" ? "📊 Live" : "📋 작업";
  if (ccDensity === "glance") setCcTab(face);
}

// ── 셸 cys 설치/해제 버튼(macOS 전용) ─────────────────
// 버튼 하나가 상태 2종을 겸한다(미설치=설치 / 설치됨=해제). 라벨·툴팁·클릭 분기의 진실은 이 모듈
// 변수 하나이고, 판정은 clipath.ts 순수 함수가 한다(main.ts는 배선만 — clipath.test.ts가 잠근다).
//
// ★(BLOCK-1(d)) cli_install_status 의 notes 를 **반드시 사용자에게 보여준다**. Rust 는 "이 자리에
// 남의 실체 파일이 있다"를 이미 탐지해 문장으로 보내는데, 예전 TS 타입에는 notes 필드 자체가 없어
// 한 글자도 노출되지 않았다 — 사용자는 버튼을 누르는 순간 자기 파일이 어떻게 되는지 모른 채 눌렀다.
// 노출 경로는 둘이다: ①버튼 툴팁(상주) ②CC 열 때 sticky 토스트 1회(같은 id 재호출은 갱신).
const CLI_STATUS_UNKNOWN: CliStatusView = {
  supported: true,
  button: "unknown",
  linkState: "unknown",
  notes: [],
  backups: [],
  cysLink: "",
  cysdLink: "",
};
let cliStatus: CliStatusView = { ...CLI_STATUS_UNKNOWN };

// ★(I2 · adv8) 직전 설치 시도의 결과 래치. 상태 조회는 **링크의 존재**만 보므로, 그림자화·확인
// 불가로 끝난 설치 직후에도 installed=true 를 돌려주고 버튼이 '해제'로 뒤집힌다 — 방금 "미완료"
// 라고 읽은 사용자가 같은 자리를 누르면 정반대(비가역 해제)가 나간다. 라벨은 이 둘의 합으로 정한다.
// 래치는 Control Center 를 다시 열 때와 해제 성공 직후에 풀린다(해제 경로가 영영 막히지 않게).
let cliLastInstall: LastInstallOutcome = null;

function applyCliButtonView() {
  const b = document.getElementById("btn-install-cli") as HTMLButtonElement | null;
  if (!b) return;
  // (I3①) 툴팁은 토스트와 **같은 고지 줄**을 단다 — 토스트는 60초 뒤 사라지고 툴팁은 상주하므로,
  // 남는 쪽이 덜 말하면 잔존 백업본 정보가 그대로 소실된다.
  const v = cliButtonView(cliStatus.button, cliNoticeLines(cliStatus), cliLastInstall);
  b.textContent = v.label;
  b.title = v.title;
  // platform_supported=false 는 Rust 의 명시 부정이다(macOS 아님) — 그때만 되숨긴다.
  // 판독 실패(응답 없음)로는 숨기지 않는다: 기능이 조용히 사라지는 쪽이 더 나쁘다.
  if (!cliStatus.supported) b.hidden = true;
}

// 결과 토스트 배선 — clipath.ts 가 등급(category)과 **수명(sticky)** 까지 정한다(MINOR-10).
// 경고 본문은 200자 안팎이라 volatile 8초로는 읽히지 않는다: 아직 할 일이 남은 결과만 60초 sticky.
//
// ★(MINOR-N6) volatile 로 낼 때는 같은 id 의 **살아 있는 sticky 를 먼저 내린다**. 예전엔 그러지
// 않아, 설치 실패 sticky(60초)가 떠 있는 상태에서 다시 눌러 성공하면 '⚠ 실패'와 '✅ 완료'가
// 최대 60초 나란히 공존했다 — 사용자는 둘 중 무엇이 현재 상태인지 알 수 없다.
function showCliToast(plan: ToastPlan) {
  const emit = toastEmitPlan(plan);
  if (emit.dismissStickyId) dismissToast(emit.dismissStickyId);
  if (emit.sticky) stickyToast(plan.id, plan.category, plan.title, plan.body);
  else toast(plan.category, plan.title, plan.body);
}

// ══════════════════════════════════════════════════════════════════════════════
// ★MAJOR-3(2026-08-25 8R) 상태 조회의 **동시성 가드** — 진행 중 억제 + 세대 카운터
// ══════════════════════════════════════════════════════════════════════════════
// 7R 이 Rust `cli_install_status` 를 async 로 내리면서(#[tauri::command] 는 함수의 asyncness 로
// ExecutionContext 를 정한다) IPC 핸들러의 **직렬화가 사라졌다**. 그 전에는 Blocking 컨텍스트라
// 조회가 도는 동안 다음 조회가 시작조차 못 했는데, 지금은 Control Center 를 빠르게 여닫는 만큼
// `$SHELL -lc 'which -a …'` **로그인 셸이 동시에 뜬다**(Rust probe_path_shadows: 기한 5초 +
// -lc 미지원 셸 폴백 재시도 1회 = 최대 10초/회). 7R 주석은 "새 동시성도 열리지 않는다"고 단정하며
// 근거로 '버튼 disabled' 를 들었지만, 상태 조회의 실제 호출부는 버튼이 아니라 setCcOpen 안의
// `void refreshCliInstallState()` 다 — 그 경로에는 가드가 없었다.
//
// 둘째 결함은 **last-writer-wins** 였다. `cliStatus = readCliStatus(await invoke(...))` 에 요청
// 구분이 없어, 늦게 끝난 옛 프로브가 액션 직후의 재조회 결과를 덮을 수 있다. 그러면 결과 토스트에
// 접어 넣는 고지 줄(withCliNotice)과 버튼 라벨이 **낡은 사실**로 되돌아간다 — I2 가 닫은 '라벨과
// 실제가 어긋나는 창'과 같은 계열이다.
//
// 관례는 이 파일에 이미 있다. 재진입 억제는 `boardBusy`(runSkillButton), 늦은 응답의 덮어쓰기
// 차단은 **세대 카운터**(trackFilter.generation / clearLedgerIfGeneration) 다. 그 둘을 그대로
// 쓴다 — 새 타이머·폴링은 만들지 않는다(이 코드베이스의 상시 원칙).
let cliStatusGen = 0; // 최신 요청 세대. 응답은 자기 세대가 아직 최신일 때만 상태를 쓴다.
let cliStatusBusy = false; // 프로브가 떠 있는가(= 로그인 셸이 아직 살아 있는가).

// 읽기 전용 상태 조회(관리자 승격 없음). ★폴링 금지(WINAUDIT 타이머 증식) — CC 열 때 1회 +
// 설치/해제 직후 1회뿐이다. 실패해도 기능이 죽지 않는 부수 조회라 조용히 unknown으로 두고,
// unknown의 라벨은 '설치'다(멱등한 설치 쪽 — 비가역 해제로 기울지 않는다).
//
// ★(G2 · 2026-08-25 5R) `notice` 인자: 상시 고지 토스트(cli-status-notes)를 낼 것인가.
// 설치·해제 **직후**의 재조회에서는 false 다 — 같은 사실(옮겨 둔 원본·남의 파일)이 결과 토스트에
// 이미 실려 있어서, 그대로 두면 서로 다른 문장의 sticky 가 둘 뜬다(한 사건, 두 알림). 그 경로에서는
// 호출측이 withCliNotice 로 결과 토스트 **하나**에 접어 넣는다. 정보는 줄지 않는다(툴팁에도 상주).
//
// ★(MAJOR-3) `force`: 액션(설치·해제) **직후**의 재조회인가. 이 한 갈래만 중복 억제를 건너뛴다.
async function refreshCliInstallState(opts: { notice?: boolean; force?: boolean } = {}) {
  if (!IS_MACOS) return;
  // (MAJOR-3) 진행 중이면 **새 로그인 셸을 띄우지 않는다.** 관측만 하는 재진입(CC 토글·팔레트
  // act:cc·부팅 경로)은 버려도 잃는 것이 없다 — 이미 떠 있는 프로브가 곧 같은 답을 낸다.
  // 예외는 액션 직후 재조회(force)뿐이다: 그것은 **설치·해제가 일어난 뒤의 사실**을 읽어야 하므로
  // 액션 이전에 시작된 프로브의 답으로 대신할 수 없다(버리면 라벨과 고지 줄이 낡은 채 남는다 —
  // MINOR-11 이 닫은 창의 재개방). 그래서 동시 프로브 수의 상한은 '사용자가 누른 액션 수'로 묶인다.
  if (cliStatusBusy && !opts.force) return;
  const gen = ++cliStatusGen;
  cliStatusBusy = true;
  // 응답은 **지역 변수로 받는다.** await 뒤에 곧바로 cliStatus 에 대입하면 세대 검사를 할 자리가
  // 없어져(대입이 이미 끝난 뒤다) 가드가 장식이 된다.
  let view: CliStatusView;
  try {
    view = readCliStatus(await invoke("cli_install_status"));
  } catch {
    view = { ...CLI_STATUS_UNKNOWN };
  }
  // 늦게 도착한 낡은 응답은 최신 상태를 **덮지 않는다**(last-writer-wins 차단). busy 도 최신
  // 세대만 내린다 — 옛 응답이 내리면 아직 살아 있는 프로브를 '없다'고 말하게 되어 억제가 뚫린다.
  // (프로브가 영영 돌아오지 않는 환경에서는 busy 가 남아 관측 재조회가 멎지만, 그때는 애초에 읽을
  //  수 있는 상태가 없다 — 라벨은 안전한 쪽('설치')에 머물고, 다음 액션의 force 가 그 자물쇠를 푼다.)
  if (gen !== cliStatusGen) return;
  cliStatusBusy = false;
  cliStatus = view;
  applyCliButtonView();
  if (opts.notice === false) return;
  const notice = statusNoticePlan(cliStatus); // 고지할 것(notes·잔존 백업)이 없으면 null — 정상은 무음
  if (notice) showCliToast(notice);
}

function setCcOpen(open: boolean) {
  ccOpen = open;
  document.getElementById("cc-panel")!.hidden = !open;
  if (open) {
    applyCcDensity(ccDensity); // 저장된 밀도 모드 복원(class·버튼 라벨)
    // 기본 탭(오피스) 정합: index.html 초기 hidden/active와 ccTab 상태를 열 때마다 동기화.
    // glance 밀도는 applyCcDensity→applyGlanceFace가 이미 면(live/tasks)을 강제했으므로 건드리지 않는다.
    if (ccDensity !== "glance") setCcTab(ccTab);
    refreshControlCenter();
    refreshHw();
    tickCc();
    // 셸 설치 상태는 CC를 열 때 1회만 확인한다(주기 조회 없음). ★반드시 fire-and-forget —
    // 여기서 await 하면 아래 타이머 생성이 응답을 기다리며 막힌다(e2e shim의 invoke는 영구 pending).
    // (I2) 패널을 다시 여는 것은 '처음부터'의 자연스러운 경계다 — 직전 설치 래치를 여기서 푼다.
    // 그래야 '다시 설치' 표시가 그 세션의 잔상으로 끝나고, 해제 경로가 영구히 막히지 않는다.
    cliLastInstall = null;
    void refreshCliInstallState();
    if (ccTimer == null) ccTimer = setInterval(refreshControlCenter, 5000) as unknown as number;
    if (ccHwTimer == null) ccHwTimer = setInterval(refreshHw, 2000) as unknown as number;
    if (ccClockTimer == null) ccClockTimer = setInterval(tickCc, 1000) as unknown as number;
  } else {
    if (ccTimer != null) { clearInterval(ccTimer); ccTimer = null; }
    if (ccHwTimer != null) { clearInterval(ccHwTimer); ccHwTimer = null; }
    if (ccClockTimer != null) { clearInterval(ccClockTimer); ccClockTimer = null; }
    // 대기 렌더 상한 복원 — feedPendingExpanded 선언 주석이 '패널을 닫으면 되돌린다'고 적고
    // 있었는데 이 경로에 그 코드가 없어 **거짓 계약**이었다(성찰3 설계렌즈 minor). 재열기가
    // 리셋을 보장하지도 않는다: 재열기의 setCcTab(ccTab) 은 `ccDensity !== "glance"` 조건부라
    // glance 밀도에서는 상한이 풀린 채 돌아온다. 주석을 좁히는 대신 코드를 참으로 만든다 —
    // 패널을 닫는 것은 자연스러운 '처음부터' 경계이고, 되살리는 비용은 '더 보기' 클릭 1회다.
    feedPendingExpanded = false;
  }
}

function tickCc() {
  const p = (x: number) => String(x).padStart(2, "0");
  const clk = document.getElementById("cc-clock");
  if (clk) {
    const n = new Date();
    clk.textContent = `${n.getFullYear()}.${p(n.getMonth() + 1)}.${p(n.getDate())} ${p(n.getHours())}:${p(n.getMinutes())}:${p(n.getSeconds())}`;
  }
  const up = document.getElementById("cc-uptime-val");
  if (up && ccUptimeFetchedAt) {
    up.textContent = ccUptimeStr(ccUptimeBase + Math.floor(Date.now() / 1000 - ccUptimeFetchedAt));
  }
}

async function refreshControlCenter() {
  if (!ccOpen) return;
  // 계정 Rate Limit(전 조직 병합) — Live KPI/게이지가 이 데이터를 쓰므로 대시보드 렌더 전에 최신화.
  if (ccTab === "live") {
    try {
      ccAccounts = ((await invoke("usage_accounts_all")) as any)?.accounts ?? [];
    } catch {
      /* 데몬 일시 부재 — 직전 스냅샷 유지, 다음 틱 재시도 */
    }
  }
  try {
    renderControlCenter(await invoke("control_dashboard"));
    ccFailStreak = 0;
  } catch {
    // 데몬 일시 부재 — 다음 틱 재시도. 연속 실패는 stale 배너로 표면화(B-11).
    ccFailStreak++;
  }
  updateCcStale();
  try {
    renderAlerts((await invoke("control_alerts")) as any);
  } catch {
    /* graceful */
  }
  if (ccTab === "eff") refreshEfficiency();
  if (ccTab === "skills") refreshSkills();
  // B-7: sessions·weekly도 동일 5초 주기 — 구 구현은 탭 진입 1회 로드 후 정지였다.
  if (ccTab === "sessions") refreshSessions();
  if (ccTab === "weekly") refreshWeekly();
  if (ccTab === "learn") refreshLearn();
  // Tasks 안전망 reconcile: 이벤트 누락·부서 신규 기동을 5초 폴링으로 보정(평시는 이벤트 드리븐).
  if (ccTab === "tasks") refreshTasks();
  if (ccTab === "feed") refreshFeed();
  // 보드는 카탈로그 전체 재렌더 없이 최근 실행 카드만 5초 갱신(깜빡임 방지).
  if (ccTab === "board") refreshBoardRuns();
}

// B-11: 연속 실패 표면화 — 3틱(15초) 연속 실패면 footer를 경고로 전환(조용한 stale 오인 차단)
let ccFailStreak = 0;
function updateCcStale() {
  const f = document.getElementById("cc-footer");
  if (!f) return;
  if (ccFailStreak >= 3) {
    f.textContent = "⚠ 데몬 응답 없음 — 표시 중인 값은 마지막 성공 시점 기준(자동 재시도 중)";
    f.classList.add("stale");
  } else {
    f.classList.remove("stale");
  }
}

// E6 경보 — 헤더 배지(개수) + Live 뷰 상단 스트립. severity: warn(주황)/crit(빨강).
function renderAlerts(a: any) {
  const list: any[] = a?.alerts ?? [];
  const crit = list.filter((x) => x.severity === "crit").length;
  const badge = document.getElementById("cc-alertbadge")!;
  badge.hidden = list.length === 0;
  badge.textContent = list.length ? `⚠ ${list.length}` : "";
  badge.className = "cc-alert-badge " + (crit > 0 ? "crit" : "warn");
  document.getElementById("cc-alerts")!.innerHTML = list
    .map(
      (x) =>
        `<div class="cc-alert-row ${x.severity === "crit" ? "crit" : "warn"}"><span class="cc-alert-icon">${x.severity === "crit" ? "🔴" : "🟠"}</span><span class="cc-alert-msg">${ccEsc(x.message ?? x.kind ?? "")}</span></div>`,
    )
    .join("");
}

async function refreshEfficiency() {
  try {
    renderEfficiency(await invoke("control_analytics", { window: ccEffWindow }));
  } catch {
    /* graceful */
  }
}

async function refreshSkills() {
  try {
    renderSkills(await invoke("control_skills", { window: ccSkillsWindow }));
  } catch {
    /* graceful */
  }
}

async function refreshSessions() {
  try {
    renderSessions((await invoke("control_sessions", { window: ccSessionsWindow, redact: ccSessionsRedact })) as any);
  } catch {
    /* graceful */
  }
}

async function refreshWeekly() {
  try {
    renderWeekly((await invoke("control_weekly")) as any);
  } catch {
    /* graceful */
  }
}

function setCcTab(view: CcTab) {
  ccTab = view;
  document.getElementById("cc-view-live")!.hidden = view !== "live";
  document.getElementById("cc-view-eff")!.hidden = view !== "eff";
  document.getElementById("cc-view-skills")!.hidden = view !== "skills";
  document.getElementById("cc-view-sessions")!.hidden = view !== "sessions";
  document.getElementById("cc-view-weekly")!.hidden = view !== "weekly";
  document.getElementById("cc-view-learn")!.hidden = view !== "learn";
  document.getElementById("cc-view-board")!.hidden = view !== "board";
  document.getElementById("cc-view-tasks")!.hidden = view !== "tasks";
  document.getElementById("cc-view-feed")!.hidden = view !== "feed";
  document.getElementById("cc-view-alarms")!.hidden = view !== "alarms";
  document.getElementById("cc-view-office")!.hidden = view !== "office";
  // 오피스 탭 전면 모드 — cc-body의 대시보드 폭 상한(780px)을 해제해 3D를 창 크기에 연동(cc-glance 패턴).
  document.body.classList.toggle("cc-office", view === "office");
  document.querySelectorAll("#cc-tabs .cc-tab").forEach((b) =>
    b.classList.toggle("active", (b as HTMLElement).dataset.view === view),
  );
  if (view === "live") {
    refreshHw();
    refreshControlCenter(); // 탭 복귀 즉시 본문 갱신(B-6 가드로 이탈 중엔 재생성 안 했으므로)
  }
  if (view === "eff") refreshEfficiency();
  if (view === "skills") refreshSkills();
  if (view === "sessions") refreshSessions();
  if (view === "weekly") refreshWeekly();
  if (view === "learn") refreshLearn();
  if (view === "board") refreshBoard();
  if (view === "tasks") refreshTasks();
  if (view === "feed") {
    // 탭 진입 시 대기 렌더 상한을 되살린다 — '더 보기'로 푼 확장이 세션 내내 상주하면
    // 5초 주기 refreshFeed 가 매번 전건 DOM 을 재구성한다(상한을 둔 취지가 사라진다).
    feedPendingExpanded = false;
    refreshFeed();
  }
  if (view === "alarms") renderAlarmHistory();
  if (view === "office") openOfficeView();
}

// 메타버스 오피스 탭 — 로컬 브리지(127.0.0.1:8642, 3D 실시간 오피스)를 iframe으로 내장.
// 탭 진입 시에만 로드(상시 연결 방지)·브리지 부재 시 기동 안내만 표시.
const OFFICE_URL = "http://127.0.0.1:8642/";
async function openOfficeView() {
  const frame = document.getElementById("cc-office-frame") as HTMLIFrameElement | null;
  const hint = document.getElementById("cc-office-hint");
  if (!frame || !hint) return;
  try {
    // no-cors: 도달성 프로브만(응답은 opaque). tauri://localhost → http://127.0.0.1 은
    // 교차출처라 CORS-fetch는 ACAO 없이 reject되어 브리지가 살아있어도 hint에 갇혔다(근본 수리).
    await fetch(OFFICE_URL + "world", { mode: "no-cors", signal: AbortSignal.timeout(1500) });
    hint.hidden = true;
    if (!frame.src) frame.src = OFFICE_URL;
  } catch {
    hint.hidden = false;
    frame.removeAttribute("src");
  }
}

// D5: 스킬 버튼 보드 — 카탈로그 큐레이션 렌더 + 일회용 워커 실행 + 산출물 회수(터미널 입력 0회).
async function refreshBoard() {
  ccBoardCatalog = (await invoke("read_board_catalog").catch(() => ({ domains: [], actions: [] }))) as any;
  // 호출수 뱃지 데이터(7d) — 실패는 뱃지 생략(보드 본기능 무영향).
  try {
    const sk = (await invoke("control_skills", { window: "7d" })) as any;
    ccBoardCalls = {};
    for (const x of sk?.summary?.by_skill ?? []) if (x?.name) ccBoardCalls[String(x.name)] = Number(x.calls) || 0;
  } catch {
    /* graceful */
  }
  renderBoardDomains();
  refreshBoardRuns();
  // 회수 패널 — list_dir 재사용(결정론 위치 skill_out_dir)
  const outHost = document.getElementById("cc-board-out")!;
  let dirs: any[] = [];
  try {
    const dir = (await invoke("skill_out_dir")) as string;
    dirs = (await invoke("list_dir", { path: dir })) as any[];
  } catch {
    /* 아직 산출물 없음 */
  }
  outHost.innerHTML =
    !dirs || dirs.length === 0
      ? `<div class="cc-empty">산출물 없음 (~/.cys/_round/skill-out)</div>`
      : dirs
          .map((x: any) => {
            const p = x.path ?? "";
            const nm = x.name ?? p;
            return `<button class="cc-board-out-item" data-path="${ccEsc(p)}">📄 ${ccEsc(nm)}</button>`;
          })
          .join("");
  outHost.querySelectorAll<HTMLElement>(".cc-board-out-item").forEach((b) =>
    b.addEventListener("click", () => { if (b.dataset.path) void openPathChecked(b.dataset.path); }),
  );
}

// 오너 즐겨찾기 pin — localStorage "cys-board-pins"(name 배열). 설계(board-pins.json) 대비 의도적 로컬 축소.
function boardPins(): string[] {
  try {
    const v = JSON.parse(localStorage.getItem("cys-board-pins") || "[]");
    return Array.isArray(v) ? v : [];
  } catch {
    return [];
  }
}
function toggleBoardPin(name: string) {
  const pins = boardPins();
  const i = pins.indexOf(name);
  if (i >= 0) pins.splice(i, 1);
  else pins.push(name);
  localStorage.setItem("cys-board-pins", JSON.stringify(pins));
  renderBoardDomains();
}
// actions(write-a-skill 등)를 스킬 객체로 정규화 — 도메인 스킬과 동일 실행 경로.
function boardActionSkill(a: any): any {
  return {
    name: a.name,
    label: a.label ?? a.name,
    scope: "새 스킬 만들기 (write-a-skill — 일상 워크플로우를 스킬로 codify)",
    success: "SKILL.md 4칸 본문 생성·트리거 명확",
    gate: "hitl",
  };
}
// 보드 버튼 1개 — 클릭=실행, 우클릭=즐겨찾기 토글, pin이면 ★ 표시.
function makeBoardBtn(s: any): HTMLButtonElement {
  const b = document.createElement("button");
  b.className = "cc-board-btn";
  const pinned = boardPins().includes(s.name);
  if (pinned) b.classList.add("pinned");
  const calls = ccBoardCalls[String(s.name)] ?? 0;
  const callChip = calls > 0 ? `<span class="cc-board-calls" title="최근 7일 호출수">${Math.floor(calls)}</span>` : "";
  b.innerHTML = `${pinned ? `<span class="cc-board-pin">★</span>` : ""}${ccEsc(String(s.label ?? s.name ?? ""))}${callChip}`;
  b.title = `${s.scope ?? ""}${s.gate === "hitl" ? " · 미리보기 확인 필요" : ""} · 우클릭=즐겨찾기`;
  b.onclick = () => runSkillButton(s);
  b.oncontextmenu = (e) => {
    e.preventDefault();
    toggleBoardPin(String(s.name));
  };
  return b;
}
// 카탈로그 도메인·actions 렌더 — 검색어(name/label/scope 부분일치) 필터 + 즐겨찾기 상단 그룹.
// 검색은 재fetch 없이 이 함수만 재호출(깜빡임 방지).
function renderBoardDomains() {
  const cat = ccBoardCatalog;
  const q = ccBoardSearch.trim().toLowerCase();
  const match = (s: any) => !q || [s.name, s.label, s.scope].some((x) => String(x ?? "").toLowerCase().includes(q));
  const host = document.getElementById("cc-board-domains");
  if (!host) return;
  host.innerHTML = "";
  // 카탈로그 전체 스킬(acl≤1) 색인 — 즐겨찾기 복제용. actions는 정규화.
  const all = new Map<string, any>();
  for (const d of cat.domains ?? []) for (const s of d.skills ?? []) if ((s.acl ?? 1) <= 1) all.set(s.name, s);
  for (const a of cat.actions ?? []) if ((a.acl ?? 1) <= 1) all.set(a.name, boardActionSkill(a));

  const appendGroup = (title: string, skills: any[]) => {
    if (!skills.length) return;
    const sec = document.createElement("div");
    sec.className = "cc-board-domain";
    sec.innerHTML = `<div class="cc-board-domain-h">${ccEsc(title)}</div>`;
    const wrap = document.createElement("div");
    wrap.className = "cc-board-btns";
    for (const s of skills) wrap.appendChild(makeBoardBtn(s));
    sec.appendChild(wrap);
    host.appendChild(sec);
  };

  // ★ 즐겨찾기(오너 pin) — 카탈로그 스킬 복제, 상단 그룹.
  const pinned = boardPins().map((n) => all.get(n)).filter((s) => s && match(s));
  appendGroup("★ 즐겨찾기", pinned);
  // 비기술자: acl≤1만 (민감/위험 스킬은 카탈로그 미포함=암묵 차단)
  for (const d of cat.domains ?? [])
    appendGroup(d.label ?? d.id ?? "", (d.skills ?? []).filter((s: any) => (s.acl ?? 1) <= 1 && match(s)));
  // SB-4: actions(write-a-skill 등) 1급 노출 — 도메인과 동일 실행 경로(신규 인프라 0)
  appendGroup("도구", (cat.actions ?? []).filter((a: any) => (a.acl ?? 1) <= 1 && match(a)).map(boardActionSkill));
}

// 최근 실행 카드 — skill_runs 폴링 렌더. 상태칩·시작시각·산출물 열기(open_path).
async function refreshBoardRuns() {
  const host = document.getElementById("cc-board-runs");
  if (!host) return;
  let runs: any[] = [];
  try {
    runs = ((await invoke("skill_runs", { limit: 20 })) as any)?.runs ?? [];
  } catch {
    /* 데몬 일시 부재 — 다음 틱 재시도 */
  }
  host.innerHTML = runs.length
    ? runs
        .map((r) => {
          const status = String(r.status ?? "");
          const chip =
            status === "done"
              ? `<span class="cc-run-status done">✅ 완료</span>`
              : status === "failed"
                ? `<span class="cc-run-status failed">❌ 실패${r.exit_note ? ` (${ccEsc(String(r.exit_note))})` : ""}</span>`
                : `<span class="cc-run-status launched">⏳ 진행중</span>`;
          const when = r.started_at ? ccHHMM(Number(r.started_at)) : "";
          const art = r.artifact_dir
            ? `<button class="cc-run-art" data-path="${ccEsc(String(r.artifact_dir))}">📂 산출물</button>`
            : "";
          return `<div class="cc-run-card"><span class="cc-run-label">${ccEsc(String(r.label ?? r.name ?? "?"))}</span><span class="cc-run-when">${ccEsc(when)}</span>${chip}${art}</div>`;
        })
        .join("")
    : `<div class="cc-empty">최근 실행 없음</div>`;
  host.querySelectorAll<HTMLElement>(".cc-run-art").forEach((b) =>
    b.addEventListener("click", () => { if (b.dataset.path) void openPathChecked(b.dataset.path); }),
  );
}

// ───────── Tasks Control Center — 모든 부서의 모든 노드가 지금 하는 업무(관측 전용) ─────────
// 데이터원: org_fleet(본부+각 부서 소켓 org.status fan-out 집계). 신규 DB 없이 기존 set-status
// 자기보고(task/state/context)를 부서 라벨과 함께 그린다. 평시 이벤트 드리븐, 5초 reconcile 폴링은 안전망.
let tasksForwardersEnsured = false;
const CC_TASK_STATE: Record<string, { cls: string; label: string }> = {
  working: { cls: "working", label: "작업중" }, waiting: { cls: "idle", label: "대기" },
  blocked: { cls: "error", label: "막힘" }, done: { cls: "offline", label: "완료" },
};
function ccAge(secs: number): string {
  const s = Math.max(0, Math.round(secs));
  if (s < 60) return `${s}초 전`;
  if (s < 3600) return `${Math.floor(s / 60)}분 전`;
  return `${Math.floor(s / 3600)}시간 전`;
}

async function refreshTasks() {
  if (!tasksForwardersEnsured) {
    tasksForwardersEnsured = true;
    invoke("ensure_dept_forwarders").catch(() => {}); // 전 부서 실시간 push 보장(멱등)
  }
  try {
    lastFleet = await invoke("org_fleet");
  } catch {
    /* 데몬 일시 부재 — 직전 스냅샷 유지, 다음 틱 재시도 */
  }
  renderTasks(lastFleet);
}

function renderTasks(fleet: any) {
  const host = document.getElementById("cc-tasks-depts");
  if (!host) return;
  const depts: any[] = fleet?.departments ?? [];
  if (!depts.length) {
    host.innerHTML = `<div class="cc-empty">부서 정보 없음 — 데몬 응답 대기</div>`;
    return;
  }
  // B-6: 재생성 전 펼침 상태 보존 — 구 구현은 이벤트 도착마다 전체 innerHTML 재생성으로
  // 펼쳐둔 task 전문이 즉시 접혔다(긴 업무 읽기 방해).
  const expanded = new Set(
    Array.from(host.querySelectorAll<HTMLElement>(".cc-task-row.expanded")).map((r) => r.dataset.key ?? ""),
  );
  host.innerHTML = depts
    .map((d) => {
      const deptKey = String(d.socket_slug ?? d.name ?? "");
      const surfaces: any[] = (d.surfaces ?? []).slice();
      surfaces.sort((a, b) => (a.surface_id ?? 0) - (b.surface_id ?? 0));
      const working = surfaces.filter(
        (s) =>
          s.status?.state === "working" ||
          (!s.status && !s.exited && (s.idle_secs ?? 999) <= 60),
      ).length;
      const deadBadge = d.error
        ? `<span class="cc-fail-badge crit">⚠ ${d.error === "timeout" ? "응답없음" : "도달불가"}</span>`
        : "";
      const head =
        `<div class="cc-tasks-dept-h"><span class="cc-tasks-dept-name">${ccEsc(d.display_name ?? d.name ?? "")}</span>` +
        `<span class="cc-tasks-dept-meta">노드 ${surfaces.length} · 작업중 ${working}</span>${deadBadge}</div>`;
      const rows = d.error
        ? `<div class="cc-empty">${d.error === "timeout" ? "부서 데몬 응답 없음(2초 초과)" : "부서 데몬 연결 실패 — 다운/기동 중"}</div>`
        : surfaces.length === 0
          ? `<div class="cc-empty">노드 없음</div>`
          : surfaces.map((s) => taskRow(s, deptKey)).join("");
      return `<div class="cc-section cc-tasks-dept">${head}${rows}</div>`;
    })
    .join("");
  // 행 클릭 → task 전문 펼치기(요약금지·읽기전용·PTY주입 0) + 보존된 펼침 복원
  host.querySelectorAll<HTMLElement>(".cc-task-row").forEach((row) => {
    if (expanded.has(row.dataset.key ?? "")) row.classList.add("expanded");
    row.addEventListener("click", () => row.classList.toggle("expanded"));
  });
}

function taskRow(s: any, deptKey: string): string {
  const role = String(s.role ?? "?");
  const color = CC_ROLE_COLOR[role] ?? "#64748b";
  const st = s.status; // 자기보고 {state, context_pct, task, age_secs} | null
  const selfReport = st != null;
  let cls: string, label: string;
  if (s.exited) {
    cls = "offline";
    label = "오프라인";
  } else if (selfReport) {
    const m = CC_TASK_STATE[st.state] ?? { cls: "idle", label: String(st.state) };
    cls = m.cls;
    label = m.label;
  } else {
    const idle = s.idle_secs ?? 999;
    cls = idle > 60 ? "idle" : "working";
    label = idle > 60 ? "대기" : "활동";
  }
  const trust = selfReport
    ? `<span class="cc-trust-badge self" title="노드가 cys set-status로 직접 보고한 상태">📍자기보고</span>`
    : `<span class="cc-trust-badge derived" title="출력 활동에서 데몬이 추정한 상태(자기보고 없음)">⚙파생</span>`;
  const task = selfReport && st.task ? String(st.task) : "(업무 미보고)";
  const ctx =
    selfReport && st.context_pct != null
      ? `<span class="cc-tbar" style="max-width:130px"><span class="cc-tbar-track"><span class="cc-tbar-fill ${st.context_pct >= 80 ? "crit" : st.context_pct >= 60 ? "warn" : ""}" style="width:${Math.min(100, st.context_pct)}%"></span></span><span class="cc-tbar-pct">${st.context_pct}%</span></span>`
      : "";
  const age = selfReport ? ccAge(st.age_secs ?? 0) : `idle ${s.idle_secs ?? 0}s`;
  const stale = selfReport && (st.age_secs ?? 0) > 120 ? " stale" : "";
  return (
    `<div class="cc-task-row${stale}" data-key="${ccEsc(deptKey)}:${s.surface_id ?? "?"}" title="${ccEsc(task)}">` +
    `<span class="cc-dot ${cls}"></span>` +
    `<span class="cc-task-role" style="color:${color}">${ccEsc(role)}</span>` +
    `<span class="cc-task-text">${ccEsc(task)}</span>` +
    ctx +
    `<span class="cc-task-meta">${trust} · ${ccEsc(age)} · ${ccEsc(label)}</span>` +
    `</div>`
  );
}

// 실시간 이벤트(task.changed/status.changed)로 부서×노드 셀 패치 — socket_slug로 부서, surface_id로 노드 식별.
function upsertTaskCell(slug: string, sid: number, payload: Record<string, unknown>) {
  if (!lastFleet?.departments) return;
  const dept = lastFleet.departments.find((d: any) => d.socket_slug === slug);
  if (!dept) return; // 아직 스냅샷에 없는 부서 — 다음 reconcile 폴링이 채운다
  dept.surfaces = dept.surfaces ?? [];
  const status = {
    state: String(payload.state ?? "working"),
    context_pct: payload.context_pct ?? null,
    task: payload.task ?? null,
    age_secs: 0,
  };
  const node = dept.surfaces.find((s: any) => s.surface_id === sid);
  if (node) {
    node.status = status;
    if (payload.role) node.role = payload.role;
  } else {
    dept.surfaces.push({
      surface_id: sid,
      surface_ref: `surface:${sid}`,
      role: payload.role ?? "?",
      status,
      idle_secs: 0,
    });
  }
  if (ccTab === "tasks") renderTasks(lastFleet);
}

let boardBusy = false;
// D5: 버튼 클릭 → 무계약 차단(make_ticket 경유) → 보이는 일회용 워커 실행. gate:hitl은 미리보기 확인 강제.
async function runSkillButton(s: any) {
  if (boardBusy) return;
  boardBusy = true;
  setTimeout(() => (boardBusy = false), 2000); // 연타 디바운스(surface 누적 방지)
  try {
    // 착수 전 자원 게이트 — hard(2)=차단, soft(1)=비차단 경고 toast 후 진행(이 조직 머신은
    // nodes 축이 상시 soft라 모달이면 매 실행이 막힌다 — 실측 2026-07-16). 조회 실패=통과(관측 부재).
    let gate: any = { exit_code: 0 };
    try {
      gate = await invoke("resource_gate_check");
    } catch {
      /* 게이트 조회 실패 — 통과 취급 */
    }
    if (Number(gate?.exit_code) === 2) {
      toast("watchdog", "자원 한계", "watchdog 차단 — 자원(서버·부하)을 정리한 뒤 다시 실행하세요");
      return;
    }
    if (Number(gate?.exit_code) === 1) {
      toast("watchdog", "자원 경고", "시스템 부하가 높은 상태에서 실행합니다(soft)");
    }
    let userInput = "";
    if (s.gate === "hitl") {
      // D6 제품 모드: HITL 입력(신뢰선 라벨·게이트 건너뛰기 금지). fields 있으면 다중 필드, 없으면 단일 원고.
      if (Array.isArray(s.fields) && s.fields.length) {
        const got = await fieldsModal(s.label ?? s.name, s.scope ?? "내용을 입력하세요", s.fields);
        if (got === null) return; // 취소
        userInput = got;
      } else {
        const got = await inputModal(
          s.label ?? s.name,
          s.scope ?? "내용을 입력하세요",
          "여기에 본문 원고나 주제를 붙여넣으세요…",
        );
        if (got === null) return; // 취소
        userInput = got;
      }
    }
    // ★무계약 차단: task-prompt 티켓을 먼저 생성(javis_orchestra 경유). 반환은 {ticket, run_id} 객체.
    const scope = userInput ? `${s.scope ?? ""} · 입력 원고: ${userInput}` : s.scope ?? "";
    const t = (await invoke("make_ticket", {
      task: s.label ?? s.name,
      scope,
      success: s.success ?? "",
      to: "worker",
      slug: s.name,
    })) as any;
    await invoke("run_skill", { name: s.name, ticket: t.ticket, agent: "claude", closeAfter: null, runId: t.run_id });
    // 일회용 워커 pane은 CC 오버레이(z-index 1500) **아래** 작업공간에 뜬다 — CC를 닫아야
    // 보인다(오너 실증 2026-07-03: "CC를 종료해야 나타난다"). 실행 성공 시 자동으로 닫는다.
    setCcOpen(false);
    toast("system", "skill.launched", `${s.label ?? s.name} — 일회용 워커 pane이 열렸습니다`);
  } catch (e) {
    toast("watchdog", "skill.failed", `${s.label ?? s.name} 실행 실패: ${e}`);
  }
}

// RSI 학습 탭 — learn.status(canonical state) 폴링 렌더 + 대기추천은 승인 Feed 탭(cc-view-feed) 재사용.
async function refreshLearn() {
  let state: any = {};
  try {
    state = (await invoke("learn_status")) as any;
  } catch {
    /* 데몬 일시 부재 — 다음 틱 재시도 */
  }
  const rounds = state?.rounds ?? {};
  const keys = Object.keys(rounds);
  const disc = state?.discovery ?? {};
  // gemini REVISE: discovery 값을 ccEsc/Number 없이 innerHTML 보간하면 XSS(오염 state.json) — 안전한
  // 0 이상 정수로 강제(KPI 합산·discovery 행 동일 helper). key/verdict/title은 이미 ccEsc.
  const discNum = (x: any): number => {
    const n = Number(x);
    return Number.isFinite(n) && n >= 0 ? Math.floor(n) : 0;
  };
  const dCap = discNum(disc.capability), dPer = discNum(disc.perspective), dKno = discNum(disc.knowledge);
  const totalStored = keys.reduce((n, k) => n + (rounds[k]?.stored?.length ?? 0), 0);
  const discTotal = dCap + dPer + dKno;

  document.getElementById("cc-learn-kpi")!.innerHTML = (
    [
      ["라운드", String(keys.length), "학습 사이클"],
      ["저장(memory)", String(totalStored), "confirmed/provisional"],
      ["발견", String(discTotal), "기능·관점·지식"],
    ] as [string, string, string][]
  )
    .map(([n, v, sub]) => `<div class="cc-card"><div class="cc-card-val">${v}</div><div class="cc-card-reset">${ccEsc(sub)}</div><div class="cc-card-name">${ccEsc(n)}</div></div>`)
    .join("");

  const vColor: Record<string, string> = { improved: "#3ad07a", regressed: "#e0606a", flat: "#9a9a9a" };
  document.getElementById("cc-learn-timeline")!.innerHTML = keys.length
    ? keys
        .map((k) => {
          const r = rounds[k];
          const v = String(r?.verdict ?? "-");
          // C2 v2: items(state/expires) 칩 — 구 라운드(items 부재)는 칩 0개로 관용(기존 렌더 불변).
          // 값 전부 ccEsc 경유(오염 state.json XSS 방어 — 상단 gemini REVISE와 동일 원칙).
          const its: any[] = Array.isArray(r?.items) ? r.items : [];
          const itemChips = its
            .map((it: any) => {
              const st = String(it?.state ?? "-");
              const ex = it?.expires ? ` ~${String(it.expires)}` : "";
              return `<span class="cc-chip cc-learn-item" title="type: ${ccEsc(String(it?.type ?? "?"))} · expires: ${ccEsc(String(it?.expires ?? "미기록"))}">${ccEsc(String(it?.name ?? "?"))} · ${ccEsc(st + ex)}</span>`;
            })
            .join("");
          return `<div class="cc-learn-row"><span class="cc-learn-round">${ccEsc(k)}</span><span class="cc-learn-verdict" style="color:${vColor[v] ?? "inherit"}">${ccEsc(v)}</span><span class="cc-learn-meta">저장 ${r?.stored?.length ?? 0} · harness ${r?.harness?.length ?? 0}</span>${itemChips}</div>`;
        })
        .join("")
    : `<div class="cc-empty">학습 라운드 기록 없음 — RSI 라운드(javis_rsi.py checkpoint)가 기록을 남기면 여기 표시됩니다</div>`;

  const ribbons: string[] = [];
  for (const k of keys)
    for (const h of rounds[k]?.harness ?? []) {
      // eval:{before,after} 있으면 개선 델타(%p)를 리본에 부기 — 없으면 기존 그대로.
      let delta = "";
      if (h.eval) {
        const bef = Number(h.eval.before), aft = Number(h.eval.after);
        if (Number.isFinite(bef) && Number.isFinite(aft)) {
          const dv = Math.round(aft - bef);
          delta = ` (${dv >= 0 ? "+" : ""}${dv}%p)`;
        }
      }
      ribbons.push(`${k}: ${h.retention ?? "?"}${delta}`);
    }
  document.getElementById("cc-learn-retention")!.innerHTML = ribbons.length
    ? ribbons.map((t) => `<span class="cc-learn-ribbon ${t.includes("keep") ? "keep" : "rollback"}" title="retention: keep=개선 채택 유지 / rollback=회귀로 되돌림">${ccEsc(t)}</span>`).join("")
    : `<div class="cc-empty">채택/롤백 기록 없음</div>`;

  // 📚 자산 성장 — 기억·스킬 개수(+7d 증가)·directives 개정, 각 행에 recent 이름 칩(클릭=open_path).
  const assets = state?.assets ?? {};
  const mem = assets.memory ?? {}, sk = assets.skills ?? {}, dir = assets.directives ?? {};
  const assetChip = (r: any) =>
    `<span class="cc-chip cc-asset-chip" data-path="${ccEsc(String(r.path ?? ""))}" title="${ccEsc(ccAssetDate(r.mtime))}">${ccEsc(String(r.name ?? "?"))}</span>`;
  const assetRows: string[] = [
    `<div class="cc-asset-row"><span class="cc-asset-lab">기억</span><span class="cc-asset-v"><span class="cc-asset-n">${discNum(mem.total)}개</span><span class="cc-dim">+${discNum(mem.added_7d)}/7d</span>${(mem.recent ?? []).map(assetChip).join("")}</span></div>`,
    `<div class="cc-asset-row"><span class="cc-asset-lab">스킬</span><span class="cc-asset-v"><span class="cc-asset-n">${discNum(sk.total)}개</span><span class="cc-dim">+${discNum(sk.added_7d)}/7d</span>${(sk.recent ?? []).map(assetChip).join("")}</span></div>`,
    `<div class="cc-asset-row"><span class="cc-asset-lab">directives</span><span class="cc-asset-v"><span class="cc-dim">최근 7d 개정 ${discNum(dir.changed_7d)}건</span>${(dir.recent ?? []).map(assetChip).join("")}</span></div>`,
  ];
  const assetsHost = document.getElementById("cc-learn-assets");
  if (assetsHost) {
    assetsHost.innerHTML = assetRows.join("");
    assetsHost.querySelectorAll<HTMLElement>(".cc-asset-chip").forEach((c) =>
      c.addEventListener("click", () => {
        if (c.dataset.path) void openPathChecked(c.dataset.path);
      }),
    );
  }

  document.getElementById("cc-learn-discovery")!.innerHTML = (
    [
      ["기능 (도구·스킬·기법)", dCap],
      ["관점 (다각·교차도메인)", dPer],
      ["지식 (새 출처·경로)", dKno],
    ] as [string, number][]
  )
    .map(([l, v]) => `<div class="cc-mix-row"><span class="cc-mix-name">${ccEsc(l)}</span><span class="cc-call-n">${v}</span></div>`)
    .join("");

  // 대기 배지 — 기존 feed에서 learn_proposal pending 필터(승인/거부는 승인 Feed 탭 재사용·중복 UI 0).
  try {
    const f = (await invoke("feed_list", { status: null })) as any;
    const items: any[] = f?.items ?? [];
    const lp = items.filter((i) => i?.status === "pending" && i?.kind === "learn_proposal");
    document.getElementById("cc-learn-pending")!.innerHTML = lp.length
      ? lp.map((i) => `<div class="cc-learn-pending-item">⏳ ${ccEsc(String(i.title ?? "학습 추천"))} <span class="cc-dim">— 승인 Feed 탭에서 승인/거부</span></div>`).join("")
      : `<div class="cc-empty">대기 중 자율추천 없음</div>`;
  } catch {
    document.getElementById("cc-learn-pending")!.innerHTML = `<div class="cc-empty">—</div>`;
  }
}

function renderEfficiency(a: any) {
  const s = a?.summary ?? {};
  const t = s.totals ?? {};
  const prod = s.productivity ?? {};
  const winLab = a?.window === "7d" ? "최근 7일" : a?.window === "all" ? "전체" : "오늘";

  // A-3: "캐시 ROI"(cache_roi_x) 폐기 — 클로드 전 모델 캐시단가=입력의 10%라 항상 0.9인
  // 무정보 상수였다. 재사용율(cache_efficiency)로 대체. B-12: 절감액도 "추정" 명시.
  document.getElementById("cc-eff-kpi")!.innerHTML = (
    [
      ["총 비용", ccMoney(t.cost_usd ?? 0), `${winLab} · 추정`],
      ["🔥캐시 절감", ccMoney(s.cache_savings_usd ?? 0), "재사용 할인 · 추정"],
      ["캐시 재사용율", `${Math.round((s.cache_efficiency ?? 0) * 100)}%`, "입력 중 캐시 히트"],
      ["메시지", String(t.msgs ?? 0), `세션 ${t.sessions ?? 0}`],
      ["토큰", ccFmtTokens(t.tokens ?? 0), "4분해 합"],
    ] as [string, string, string][]
  )
    .map(([n, v, sub]) => `<div class="cc-card"><div class="cc-card-val">${v}</div><div class="cc-card-reset">${ccEsc(sub)}</div><div class="cc-card-name">${ccEsc(n)}</div></div>`)
    .join("");

  // 토큰 4분해 — 가로 스택 바 + 범례
  const tokTotal = CC_TOK_SEG.reduce((acc, [k]) => acc + (t[k] ?? 0), 0) || 1;
  const stack = CC_TOK_SEG.map(([k, , color]) => {
    const v = t[k] ?? 0;
    const pct = (v / tokTotal) * 100;
    return pct > 0 ? `<span class="cc-stack-seg" style="width:${pct}%;background:${color}" title="${ccEsc(k)} ${ccFmtTokens(v)}"></span>` : "";
  }).join("");
  const legend = CC_TOK_SEG.map(([k, lab, color]) => {
    const v = t[k] ?? 0;
    const pct = Math.round((v / tokTotal) * 100);
    return `<span class="cc-leg"><span class="cc-leg-dot" style="background:${color}"></span>${lab} ${ccFmtTokens(v)} <span class="cc-leg-pct">${pct}%</span></span>`;
  }).join("");
  document.getElementById("cc-eff-tokens")!.innerHTML =
    `<div class="cc-stack">${stack}</div><div class="cc-legend">${legend}</div>`;

  // 모델별 비용 — 비용 점유율 바
  const models: any[] = s.by_model ?? [];
  const costMax = Math.max(1e-9, ...models.map((m) => m.cost_usd ?? 0));
  document.getElementById("cc-eff-models")!.innerHTML =
    models.length === 0
      ? `<div class="cc-empty">데이터 없음</div>`
      : models
          .map((m) => {
            const short = (m.model || "?").replace(/^claude-/, "").replace(/\[1m\]$/, "");
            const pct = ((m.cost_usd ?? 0) / costMax) * 100;
            // B-4: 단가표 미적중 모델은 Sonnet 폴백 추정 — 조용히 숨기지 않고 표시
            const unk = m.pricing_known === false ? `<span class="cc-price-unk" title="단가표 미등재 모델 — Sonnet 단가로 추정된 비용">단가미상</span>` : "";
            return `<div class="cc-mix-row"><span class="cc-mix-name" title="${ccEsc(m.model ?? "")}">${ccEsc(short || "?")}${unk}</span><span class="cc-tbar-track"><span class="cc-tbar-fill cc-mix-fill" style="width:${pct}%"></span></span><span class="cc-mix-pct">${ccMoney(m.cost_usd ?? 0)}</span></div>`;
          })
          .join("");

  // 에이전트 믹스 — 토큰 점유율 바
  const agents: any[] = s.by_agent ?? [];
  const agTotal = agents.reduce((acc, x) => acc + (x.tokens ?? 0), 0) || 1;
  document.getElementById("cc-eff-agents")!.innerHTML =
    agents.length === 0
      ? `<div class="cc-empty">데이터 없음</div>`
      : agents
          .map((x) => {
            const pct = Math.round(((x.tokens ?? 0) / agTotal) * 100);
            return `<div class="cc-mix-row"><span class="cc-mix-name">${ccEsc(x.agent ?? "?")}</span><span class="cc-tbar-track"><span class="cc-tbar-fill cc-mix-fill" style="width:${pct}%"></span></span><span class="cc-mix-pct">${pct}%</span></div>`;
          })
          .join("");

  // D3 조직 단위(tier·역할) 비용 — 비용 점유율 바 (by_model 패턴 복제·producer≠evaluator baseline 가시화)
  const tiers: any[] = s.by_tier ?? [];
  const tierMax = Math.max(1e-9, ...tiers.map((x) => x.cost_usd ?? 0));
  document.getElementById("cc-eff-tiers")!.innerHTML =
    tiers.length === 0
      ? `<div class="cc-empty">데이터 없음</div>`
      : tiers
          .map((x) => {
            const pct = ((x.cost_usd ?? 0) / tierMax) * 100;
            return `<div class="cc-mix-row"><span class="cc-mix-name" title="역할 ${ccEsc(x.tier ?? "")}">${ccEsc(x.tier ?? "?")}</span><span class="cc-tbar-track"><span class="cc-tbar-fill cc-mix-fill" style="width:${pct}%"></span></span><span class="cc-mix-pct">${ccMoney(x.cost_usd ?? 0)}</span></div>`;
          })
          .join("");

  // 생산성
  document.getElementById("cc-eff-prod")!.innerHTML = (
    [
      ["턴/세션", (prod.turns_per_session ?? 0).toFixed(1), "메시지/세션"],
      ["토큰/턴", ccFmtTokens(Math.round(prod.tokens_per_turn ?? 0)), "메시지당"],
      ["비용/세션", ccMoney(prod.cost_per_session ?? 0), "세션당"],
      ["세션 길이", ccUptimeStr(Math.round(prod.avg_session_duration_secs ?? 0)), "평균"],
    ] as [string, string, string][]
  )
    .map(([n, v, sub]) => `<div class="cc-stat"><div class="cc-stat-t">${ccEsc(n)}</div><div class="cc-stat-v">${v}</div><div class="cc-stat-sub">${ccEsc(sub)}</div></div>`)
    .join("");
}

// E3 스킬·에이전트 — 실패율 색상(0=초록, ≥10%=경고, ≥30%=위험)
const ccFailSev = (rate: number) => (rate >= 0.3 ? "crit" : rate >= 0.1 ? "warn" : "");
// 호출 TOP 바 1줄 — 라벨·바(점유율)·calls·실패배지
function ccCallRow(name: string, calls: number, max: number, fail: number, rate: number | null): string {
  const pct = max > 0 ? (calls / max) * 100 : 0;
  const badge = fail > 0 && rate != null
    ? `<span class="cc-fail-badge ${ccFailSev(rate)}">✗${fail} ${Math.round(rate * 100)}%</span>`
    : "";
  return `<div class="cc-mix-row"><span class="cc-mix-name" title="${ccEsc(name)}">${ccEsc(name)}</span><span class="cc-tbar-track"><span class="cc-tbar-fill cc-mix-fill" style="width:${pct}%"></span></span><span class="cc-call-n">${calls}</span>${badge}</div>`;
}

function renderSkills(a: any) {
  const s = a?.summary ?? {};
  const t = s.totals ?? {};

  document.getElementById("cc-skills-kpi")!.innerHTML = (
    [
      ["툴 호출", String(t.tool_calls ?? 0), "실행 시도 기준"],
      ["스킬 호출", String(t.skill_calls ?? 0), "Skill 툴"],
      ["위임", String(t.agent_calls ?? 0), "서브에이전트"],
      ["🔥실패율", `${Math.round((t.fail_rate ?? 0) * 100)}%`, `✗ ${t.fail_calls ?? 0}건`],
    ] as [string, string, string][]
  )
    .map(([n, v, sub], i) => {
      const sev = i === 3 ? ccFailSev(t.fail_rate ?? 0) : "";
      return `<div class="cc-card ${sev}"><div class="cc-card-val">${v}</div><div class="cc-card-reset">${ccEsc(sub)}</div><div class="cc-card-name">${ccEsc(n)}</div></div>`;
    })
    .join("");

  // 🔥 반복 실패 — fail desc
  const fails: any[] = s.failures ?? [];
  const failMax = Math.max(1, ...fails.map((x) => x.fail ?? 0));
  document.getElementById("cc-skills-fail")!.innerHTML =
    fails.length === 0
      ? `<div class="cc-empty">실패 이벤트 없음 ✓</div>`
      : fails.map((x) => ccCallRow(x.name ?? "?", x.fail ?? 0, failMax, x.fail ?? 0, x.fail_rate ?? 0)).join("");

  // 스킬 호출 TOP
  const skills: any[] = s.by_skill ?? [];
  const skMax = Math.max(1, ...skills.map((x) => x.calls ?? 0));
  document.getElementById("cc-skills-skills")!.innerHTML =
    skills.length === 0
      ? `<div class="cc-empty">스킬 호출 없음</div>`
      : skills.map((x) => ccCallRow(x.name ?? "?", x.calls ?? 0, skMax, x.fail ?? 0, x.fail_rate ?? 0)).join("");

  // 툴 호출 TOP
  const tools: any[] = s.by_tool ?? [];
  const tlMax = Math.max(1, ...tools.map((x) => x.calls ?? 0));
  document.getElementById("cc-skills-tools")!.innerHTML =
    tools.length === 0
      ? `<div class="cc-empty">데이터 없음</div>`
      : tools.map((x) => ccCallRow(x.name ?? "?", x.calls ?? 0, tlMax, x.fail ?? 0, x.fail_rate ?? 0)).join("");

  // 서브에이전트 위임 — calls + 호출 역할
  const agents: any[] = s.by_agent ?? [];
  const agMax = Math.max(1, ...agents.map((x) => x.calls ?? 0));
  document.getElementById("cc-skills-agents")!.innerHTML =
    agents.length === 0
      ? `<div class="cc-empty">위임 없음</div>`
      : agents
          .map((x) => {
            const roles = (x.by_role ?? []).map((r: any) => `${ccEsc(r.role)}×${r.count}`).join(" · ");
            const pct = agMax > 0 ? ((x.calls ?? 0) / agMax) * 100 : 0;
            return `<div class="cc-mix-row"><span class="cc-mix-name" title="${ccEsc(x.name ?? "")}">${ccEsc(x.name ?? "?")}</span><span class="cc-tbar-track"><span class="cc-tbar-fill cc-mix-fill" style="width:${pct}%"></span></span><span class="cc-call-n">${x.calls ?? 0}</span><span class="cc-agent-roles">${roles}</span></div>`;
          })
          .join("");
}

// E4 세션 — 시각 helper (epoch초 → "MM/DD HH:MM") + 지속시간(초 → "Xm"/"Xh Ym")
function ccShortTime(epoch: number): string {
  const d = new Date(epoch * 1000);
  const p = (x: number) => String(x).padStart(2, "0");
  return `${p(d.getMonth() + 1)}/${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
}
function ccDur(secs: number): string {
  const s = Math.round(secs);
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  return `${Math.floor(s / 3600)}h ${Math.floor((s % 3600) / 60)}m`;
}
// 활동 리본 — 8px 색상 strip(강도별 불투명도). 빈 칸은 흐리게.
function ccRibbon(buckets: number[]): string {
  const max = Math.max(1, ...buckets);
  return (
    `<span class="cc-ribbon">` +
    buckets
      .map((v) => `<span class="cc-ribbon-cell" style="opacity:${v === 0 ? 0.12 : 0.35 + 0.65 * (v / max)}"></span>`)
      .join("") +
    `</span>`
  );
}

function renderSessions(a: any) {
  let list: any[] = a?.sessions ?? [];
  if (ccSessionsStarOnly) list = list.filter((s) => s.starred);
  const listEl = document.getElementById("cc-sessions-list")!;
  if (list.length === 0) {
    listEl.innerHTML = `<div class="cc-empty">${ccSessionsStarOnly ? "⭐ 세션 없음" : "세션 없음"}</div>`;
  } else {
    listEl.innerHTML = list
      .map((s) => {
        const role = s.role || "?";
        const color = CC_ROLE_COLOR[role] ?? "#64748b";
        const fail = (s.fail_calls ?? 0) > 0 ? `<span class="cc-fail-badge crit">✗${s.fail_calls}</span>` : "";
        const star = s.starred ? "★" : "☆";
        const skill = s.top_skill ? `· ${ccEsc(s.top_skill)}` : "";
        const sel = s.session_id === ccSessionSelected ? " sel" : "";
        // B-8: ⭐노트 표시 — note가 있으면 별 툴팁으로 노출(구 구현은 write-only 데드 컬럼)
        const starTip = s.star_note ? `즐겨찾기 노트: ${s.star_note}` : "즐겨찾기";
        return (
          `<div class="cc-sess-row${sel}" data-sid="${ccEsc(s.session_id)}" style="--rc:${color}">` +
          `<button class="cc-star" data-sid="${ccEsc(s.session_id)}" data-on="${s.starred ? 1 : 0}" title="${ccEsc(starTip)}">${star}</button>` +
          `<span class="cc-sess-when">${ccShortTime(s.ended_at ?? 0)}</span>` +
          `<span class="cc-sess-role">${ccEsc(role)}·${ccEsc(s.agent || "?")}</span>` +
          ccRibbon(s.ribbon ?? []) +
          `<span class="cc-sess-meta">${ccDur(s.duration_secs ?? 0)} · ${s.msgs ?? 0}턴 · ${ccFmtTokens(s.tokens ?? 0)} · ${ccMoney(s.cost_usd ?? 0)} ${skill}</span>` +
          fail +
          `</div>`
        );
      })
      .join("");
    // 행 클릭 → 상세(★PII 가림 모드=집계만이라 드릴다운 비활성), 별 클릭 → 토글
    if (!ccSessionsRedact) {
      listEl.querySelectorAll(".cc-sess-row").forEach((row) =>
        row.addEventListener("click", (e) => {
          if ((e.target as HTMLElement).classList.contains("cc-star")) return;
          openSessionDetail((row as HTMLElement).dataset.sid!);
        }),
      );
    } else {
      document.getElementById("cc-session-detail")!.hidden = true;
    }
    listEl.querySelectorAll(".cc-star").forEach((btn) =>
      btn.addEventListener("click", async (e) => {
        e.stopPropagation();
        const el = btn as HTMLElement;
        const on = el.dataset.on === "1";
        await invoke("control_session_star", { sessionId: el.dataset.sid, starred: !on }).catch(() => {});
        refreshSessions();
      }),
    );
  }
}

async function openSessionDetail(sid: string) {
  ccSessionSelected = sid;
  const el = document.getElementById("cc-session-detail")!;
  el.hidden = false;
  let d: any;
  try {
    d = await invoke("control_session_detail", { sessionId: sid });
  } catch {
    el.innerHTML = `<div class="cc-empty">상세 로드 실패</div>`;
    return;
  }
  const t = d?.summary?.totals ?? {};
  const tl: any[] = d?.timeline ?? [];
  const head =
    `<div class="cc-h">세션 상세 · ${ccEsc(sid.split("/").pop() || sid)} ${ccSourceBadge("control.session_detail")}</div>` +
    `<div class="cc-sess-detail-kpi">${ccFmtTokens(t.tokens ?? 0)} 토큰 · ${ccMoney(t.cost_usd ?? 0)} · ${t.msgs ?? 0}턴 · 이벤트 ${tl.length}</div>`;
  const rows =
    tl.length === 0
      ? `<div class="cc-empty">이벤트 없음</div>`
      : tl
          .map((e) => {
            const name = e.is_skill ? `Skill:${e.skill_name ?? "?"}` : e.is_agent ? `Task:${e.agent_type ?? "?"}` : e.tool_name ?? "?";
            const fail = e.exit_code != null && e.exit_code !== 0;
            const tag = e.event_type === "POST_TOOL" ? (fail ? "✗" : "✓") : "▸";
            // HUD-2 근거 추출(우선순위): result_path > evidence > sot_url > sha. 없으면 비점프(graceful·회귀0).
            const ev = String(e.result_path ?? e.evidence ?? e.sot_url ?? e.sha ?? "");
            const jump = ev ? ` cc-evidence" data-evidence="${ccEsc(ev)}` : "";
            return `<div class="cc-tl-row ${fail ? "crit" : ""}${jump}"><span class="cc-tl-tag">${tag}</span>` +
              `<span class="cc-tl-name">${ccEsc(name)}</span><span class="cc-tl-role">${ccEsc(e.role ?? "")}</span>` +
              (ev ? `<span class="cc-tl-jump" title="근거로 이동">↗</span>` : "") +
              `</div>`;
          })
          .join("");
  // B-9(E4 최소구현): 전사 발췌 — 데몬이 세션 파일 꼬리를 온디맨드로 읽어 제공(DB 적재 0)
  const tx: any[] = d?.transcript ?? [];
  const txHtml = tx.length
    ? `<div class="cc-h" style="margin-top:12px">전사 발췌 · 최근 ${tx.length}턴 (턴당 400자)</div>` +
      tx
        .map((m) => `<div class="cc-tx-row ${m.role === "user" ? "user" : "asst"}"><span class="cc-tx-role">${m.role === "user" ? "👤" : "🤖"}</span><span class="cc-tx-text">${ccEsc(String(m.text ?? ""))}</span></div>`)
        .join("")
    : `<div class="cc-sess-note">전사 발췌 없음(구 세션이거나 파일 접근 불가 — 이벤트 타임라인 참조)</div>`;
  el.innerHTML = head + `<div class="cc-timeline">${rows}</div>` + txHtml;
  // HUD-2: 근거 행 클릭 위임 — innerHTML 재생성마다 재바인딩(producer≠evaluator UI)
  el.querySelectorAll<HTMLElement>(".cc-tl-row.cc-evidence").forEach((row) =>
    row.addEventListener("click", () => jumpEvidence(row.dataset.evidence!)),
  );
}

// HUD-2: 근거 1개 문자열 → 종류 판별 후 점프(로컬경로/SHA/외부URL). open_url은 Rust측 HARD 화이트리스트 게이트.
function jumpEvidence(ev: string) {
  if (!ev) return;
  if (/^https?:\/\//.test(ev)) {
    invoke("open_url", { url: ev }).catch(() =>
      toast("watchdog", "🔒 근거 링크 차단", `허용 목록 외 도메인: ${ev}`),
    );
  } else if (/^[0-9a-f]{7,40}$/i.test(ev)) {
    toast("feed", "🔗 커밋 근거", ev); // SHA — 표시(점프 대상 없음)
  } else {
    void openPathChecked(ev); // 실패 사유(비존재·실행형)별 정확한 안내는 헬퍼가 담당
  }
}

// HUD-5: 출처+신선도 배지(화면 파싱 금지·환각0 UI). source=출처 라벨, ts=관측 epoch(없으면 신선도 생략).
function ccSourceBadge(source: string, ts?: number): string {
  let fresh = "";
  if (ts) {
    const age = Math.max(0, Math.round(Date.now() / 1000 - ts));
    fresh = age > 120 ? ` · <span class="stale">${Math.round(age / 60)}분 전</span>` : "";
  }
  return `<span class="cc-source-badge">📍 ${ccEsc(source)}${fresh}</span>`;
}

// E5 추세·주간 — WoW 델타 KPI·일별 오버레이·효율 리더·스킬 자산
function ccDelta(d: number | null): string {
  if (d == null) return `<span class="cc-delta">신규</span>`;
  const up = d >= 0;
  const cls = up ? "up" : "down";
  return `<span class="cc-delta ${cls}">${up ? "▲" : "▼"} ${Math.abs(d).toFixed(0)}%</span>`;
}
function renderWeekly(a: any) {
  const s = a?.summary ?? {};
  const wow = s.wow ?? {};
  const fmt: Record<string, (v: number) => string> = {
    tokens: (v) => ccFmtTokens(v),
    cost: (v) => ccMoney(v),
    sessions: (v) => String(v),
    msgs: (v) => String(v),
  };
  const label: Record<string, string> = { tokens: "토큰", cost: "비용", sessions: "세션", msgs: "메시지" };
  document.getElementById("cc-weekly-wow")!.innerHTML = ["tokens", "cost", "sessions", "msgs"]
    .map((k) => {
      const w = wow[k] ?? {};
      return `<div class="cc-card"><div class="cc-card-val">${fmt[k](w.this ?? 0)}</div><div class="cc-card-reset">${ccDelta(w.delta_pct ?? null)} vs 지난주</div><div class="cc-card-name">${label[k]}</div></div>`;
    })
    .join("");

  // 일별 오버레이 — this(채움)·last(테두리) 7일 막대
  const daily = s.daily ?? {};
  const tw: number[] = daily.this ?? [];
  const lw: number[] = daily.last ?? [];
  const dmax = Math.max(1, ...tw, ...lw);
  document.getElementById("cc-weekly-daily")!.innerHTML =
    `<div class="cc-wk-overlay">` +
    tw.map((v, i) => {
      const lh = Math.round(((lw[i] ?? 0) / dmax) * 100);
      const th = Math.round((v / dmax) * 100);
      return `<span class="cc-wk-day" title="D${i + 1} · 이번주 ${ccFmtTokens(v)} / 지난주 ${ccFmtTokens(lw[i] ?? 0)}"><span class="cc-wk-last" style="height:${lh}%"></span><span class="cc-wk-this" style="height:${th}%"></span></span>`;
    }).join("") +
    `</div><div class="cc-wk-legend"><span class="cc-leg"><span class="cc-leg-dot" style="background:#00d4ff"></span>이번주</span><span class="cc-leg"><span class="cc-leg-dot" style="background:#475569"></span>지난주</span></div>`;

  // 효율 리더 — 토큰 점유율 바 + 세션/스킬다양성
  const leaders: any[] = s.leaders ?? [];
  const lmax = Math.max(1, ...leaders.map((x) => x.tokens ?? 0));
  document.getElementById("cc-weekly-leaders")!.innerHTML =
    leaders.length === 0
      ? `<div class="cc-empty">데이터 없음</div>`
      : leaders
          .map((x) => {
            const role = x.role || "?";
            const color = CC_ROLE_COLOR[role] ?? "#64748b";
            const pct = ((x.tokens ?? 0) / lmax) * 100;
            return `<div class="cc-mix-row" style="--rc:${color}"><span class="cc-mix-name">${ccEsc(role)}</span><span class="cc-tbar-track"><span class="cc-tbar-fill cc-mix-fill" style="width:${pct}%"></span></span><span class="cc-call-n">${ccFmtTokens(x.tokens ?? 0)}</span><span class="cc-agent-roles">${x.sessions ?? 0}세션 · 스킬 ${x.skill_diversity ?? 0}종</span></div>`;
          })
          .join("");

  // 스킬 자산 — 신규/휴면/최다
  const asset = s.skill_asset ?? {};
  const chips = (arr: string[], cls: string) =>
    (arr ?? []).length === 0 ? `<span class="cc-empty-inline">없음</span>` : (arr ?? []).map((n: string) => `<span class="cc-chip ${cls}">${ccEsc(n)}</span>`).join("");
  const top: any[] = asset.top ?? [];
  document.getElementById("cc-weekly-skills")!.innerHTML =
    `<div class="cc-asset-row"><span class="cc-asset-lab">🆕 신규</span><span class="cc-asset-v">${chips(asset.new, "new")}</span></div>` +
    `<div class="cc-asset-row"><span class="cc-asset-lab">💤 휴면</span><span class="cc-asset-v">${chips(asset.dormant, "dormant")}</span></div>` +
    `<div class="cc-asset-row"><span class="cc-asset-lab">🔝 최다</span><span class="cc-asset-v">${top.length === 0 ? `<span class="cc-empty-inline">없음</span>` : top.slice(0, 8).map((t) => `<span class="cc-chip top">${ccEsc(t.name)} ${t.calls}</span>`).join("")}</span></div>`;
}

function renderControlCenter(d: any) {
  const fleet: any[] = d.fleet ?? [];
  const active = fleet.filter((f) => f.state === "working");
  const online = fleet.filter((f) => f.state !== "offline");
  const ratio = online.length ? Math.round((active.length / online.length) * 100) : 0;
  const live = active.length > 0;

  const badge = document.getElementById("cc-livebadge")!;
  badge.textContent = live ? "LIVE" : "IDLE";
  badge.className = "cc-badge " + (live ? "live" : "idle");

  const radar = document.getElementById("cc-radar")!;
  radar.classList.toggle("active", live);
  document.getElementById("cc-radar-val")!.textContent = `${ratio}%`;
  document.getElementById("cc-radar-sub")!.textContent = `${active.length}/${online.length} 활성`;

  ccUptimeBase = d.uptime_secs ?? 0;
  ccUptimeFetchedAt = Date.now() / 1000;

  // B-6: Live 뷰 본문은 live 탭이 보일 때만 재생성 — 구 구현은 어느 탭에서든 5초마다
  // 숨겨진 Live DOM 전체를 다시 그렸다(불필요 재생성). 헤더(배지·레이더·업타임)는 항상 갱신.
  if (ccTab === "live") {
    renderLiveBody(d, fleet);
  }

  document.getElementById("cc-footer")!.textContent =
    `cys Control Center · v${d.version ?? ""} · 대시보드 5초 · 하드웨어 2초 갱신`;
}

function renderLiveBody(d: any, fleet: any[]) {
  const agg = ccAggRate(fleet);
  renderAccounts();
  // KPI 5h/7d = 계정 병합 데이터의 "최고 사용 계정" max. 서브텍스트=그 계정 라벨. 계정 부재 시 ccAggRate 폴백.
  document.getElementById("cc-kpi")!.innerHTML = ["5h", "7d"]
    .map((lab) => {
      const m = ccAcctMax(lab);
      const w = agg[lab];
      const used = m ? Math.round(m.used) : w ? Math.round(w.used) : 0;
      const name = lab === "5h" ? "세션 (5h)" : "주간 (7d)";
      const sub = m ? ccAcctLabel(m.acct) : w ? ccReset(lab, w.reset) : "";
      const tip = m
        ? "최고 사용 계정 기준"
        : lab === "5h"
          ? "최근 5시간 rate limit 사용률 (전 노드 최대값)"
          : "최근 7일 rate limit 사용률 (전 노드 최대값)";
      return `<div class="cc-card ${sevClass(used, 60, 80)}" title="${ccEsc(tip)}"><div class="cc-card-val">${used}%</div><div class="cc-card-reset">${ccEsc(sub)}</div><div class="cc-card-name">${name}</div></div>`;
    })
    .join("");

  document.getElementById("cc-fleet")!.innerHTML = fleet
    .map((f) => {
      const role = f.role ?? "?";
      const color = CC_ROLE_COLOR[role] ?? "#64748b";
      const st = CC_STATE[f.state] ?? CC_STATE.idle;
      const ctx = f.usage?.ctx_pct != null ? `<span title="컨텍스트 사용률 — 모델 컨텍스트 창 대비">CTX ${f.usage.ctx_pct}%</span>` : "";
      return `<div class="cc-fleet-row" style="--rc:${color}"><span class="cc-fleet-name">${ccEsc(role)}</span><span class="cc-fleet-agent">${ccEsc(f.agent ?? "")}</span><span class="cc-fleet-ctx">${ctx}</span><span class="cc-dot ${st.cls}"></span><span class="cc-fleet-state">${st.label}</span></div>`;
    })
    .join("");

  document.getElementById("cc-token-bars")!.innerHTML = ["5h", "7d"]
    .map((lab) => {
      const m = ccAcctMax(lab);
      const w = agg[lab];
      const used = m ? Math.round(m.used) : w ? Math.round(w.used) : 0;
      const reset = m ? m.reset : w ? w.reset : null;
      const name = lab === "5h" ? "세션" : "주간";
      return `<div class="cc-tbar"><span class="cc-tbar-lab">${name}</span><span class="cc-tbar-track"><span class="cc-tbar-fill ${sevClass(used, 60, 80)}" style="width:${Math.min(100, used)}%"></span></span><span class="cc-tbar-pct">${used}%</span><span class="cc-tbar-reset">${reset != null ? ccReset(lab, reset) : ""}</span></div>`;
    })
    .join("");

  const c = d.consumption ?? {};
  document.getElementById("cc-token-stats")!.innerHTML = (
    [
      // B-12: ccMoney 통일 — toFixed(2)는 $1 미만 소액을 "$0.00"으로 소실시켰다
      ["오늘 비용", ccMoney(c.today_cost_usd ?? 0), "추정"],
      ["최근 1시간", ccFmtTokens(c.last_1h_tokens ?? 0), "토큰"],
      // C-5: today_input은 input+cache_creation 합 — "입력"으로만 쓰면 오독
      ["오늘 소비", ccFmtTokens(c.today_tokens ?? 0), `입력+캐시생성 ${ccFmtTokens(c.today_input ?? 0)}`],
      ["세션 수", String(c.session_count ?? 0), `메시지 ${c.today_msgs ?? 0}`],
    ] as [string, string, string][]
  )
    .map(([t, v, sub]) => `<div class="cc-stat"><div class="cc-stat-t">${t}</div><div class="cc-stat-v">${v}</div><div class="cc-stat-sub">${sub}</div></div>`)
    .join("");

  // 모델 믹스 — 모델별 토큰 점유율 (claude/codex/agy 어느 모델에 얼마나)
  const mix = (c.model_mix ?? {}) as Record<string, number>;
  const mixRows = Object.entries(mix).sort((a, b) => b[1] - a[1]);
  const mixTotal = mixRows.reduce((s, [, v]) => s + v, 0) || 1;
  document.getElementById("cc-model-mix")!.innerHTML =
    mixRows.length === 0
      ? ""
      : `<div class="cc-mix-h">모델 믹스</div>` +
        mixRows
          .map(([m, v]) => {
            const pct = Math.round((v / mixTotal) * 100);
            const short = (m || "?").replace(/^claude-/, "").replace(/\[1m\]$/, "");
            return `<div class="cc-mix-row"><span class="cc-mix-name">${ccEsc(short || "?")}</span><span class="cc-tbar-track"><span class="cc-tbar-fill cc-mix-fill" style="width:${pct}%"></span></span><span class="cc-mix-pct">${pct}%</span></div>`;
          })
          .join("");

  const spark: number[] = d.sparkline ?? [];
  const max = Math.max(1, ...spark);
  document.getElementById("cc-spark")!.innerHTML =
    `<div class="cc-spark-label" title="최근 12시간 토큰 소비 추이(30분 단위)">12h</div><div class="cc-spark-bars">` +
    spark.map((v) => `<span class="cc-spark-bar" style="height:${Math.max(2, Math.round((v / max) * 100))}%" title="${ccFmtTokens(v)}"></span>`).join("") +
    `</div>`;
}

// 하드웨어 모니터링 — control.hw 2초 폴링 (CPU 코어별·GPU·NPU·MEM 실시간)
async function refreshHw() {
  if (!ccOpen || ccTab !== "live") return;
  try {
    renderHw(await invoke("control_hw"));
  } catch {
    /* 데몬 일시 부재 — 다음 틱 재시도 */
  }
}

function renderHw(d: any) {
  const el = document.getElementById("cc-hw");
  if (!el) return;
  const cpu = d.cpu ?? {};
  const mem = d.mem ?? {};
  const gpu = d.gpu ?? {};
  const npu = d.npu ?? {};
  const gb = (b: number) => (b / 1024 / 1024 / 1024).toFixed(1);
  const cores: number[] = cpu.per_core_pct ?? [];
  const cpuPct = Math.round(cpu.total_pct ?? 0);
  const pe = cpu.perf_cores != null && cpu.eff_cores != null ? ` (${cpu.perf_cores}P+${cpu.eff_cores}E)` : "";
  const memU = mem.used ?? 0;
  const memT = mem.total ?? 1;
  const memPct = Math.round((memU / memT) * 100);
  // pct=null → 이 플랫폼에서 측정 경로 없음("—")
  const bar = (lab: string, pct: number | null, right: string, warn = 60, crit = 85) =>
    pct == null
      ? `<div class="cc-tbar"><span class="cc-tbar-lab">${lab}</span><span class="cc-tbar-track"></span><span class="cc-tbar-pct">—</span></div>`
      : `<div class="cc-tbar"><span class="cc-tbar-lab">${lab}</span><span class="cc-tbar-track"><span class="cc-tbar-fill ${sevClass(pct, warn, crit)}" style="width:${Math.min(100, pct)}%"></span></span><span class="cc-tbar-pct">${right}</span></div>`;
  el.innerHTML =
    `<div class="cc-hw-head"><span class="cc-hw-title">CPU ${cores.length}코어${pe}</span><span class="cc-hw-brand">${ccEsc(cpu.brand ?? "")}</span><span class="cc-hw-pct">${cpuPct}%</span></div>` +
    `<div class="cc-core-grid">` +
    cores
      .map((v, i) => {
        const p = Math.round(v);
        return `<span class="cc-core" title="코어 ${i + 1}: ${p}%"><span class="cc-core-fill ${sevClass(p, 60, 85)}" style="height:${Math.max(4, Math.min(100, p))}%"></span></span>`;
      })
      .join("") +
    `</div>` +
    bar(`GPU ${gpu.cores != null ? gpu.cores + "코어" : ""}`, gpu.pct != null ? Math.round(gpu.pct) : null, `${Math.round(gpu.pct ?? 0)}%`) +
    npuRow(npu) +
    bar("MEM", memPct, `${gb(memU)}/${gb(memT)}G`, 70, 90);
}

// NPU 줄 — macOS는 활용률(%) 공개 API가 없어 실측 전력(W)으로 표시(환각 지표 생성 금지).
function npuRow(npu: any): string {
  const lab = `NPU ${npu.cores != null ? npu.cores + "코어" : ""}`;
  const val = npu.watts != null ? `${Number(npu.watts).toFixed(1)}W` : "—";
  return `<div class="cc-tbar" title="macOS는 NPU 활용률을 공개 API로 노출하지 않아 실측 전력(W)으로 표시"><span class="cc-tbar-lab">${lab}</span><span class="cc-tbar-track"></span><span class="cc-tbar-pct">${val}</span></div>`;
}

let fontSize = Number(localStorage.getItem("cys-font-size") || 13);
function applyZoom(delta: number | null) {
  fontSize = delta === null ? 13 : Math.min(32, Math.max(8, fontSize + delta));
  localStorage.setItem("cys-font-size", String(fontSize));
  for (const rt of panes.values()) {
    rt.term.options.fontSize = fontSize;
    fitPane(rt);
  }
}

// 터미널 폰트 선택(cys-font-face · 오너 요청 2026-07-12) — 선택 폰트를 기본 스택 앞에 합성
// (composeFontFamily · CJK 폴백 보존), null=기본. 폰트 메트릭 변화 → 셀 재계산(applyZoom과 동일 패턴).
let fontFace: string | null = localStorage.getItem("cys-font-face");
function applyFontFace(face: string | null) {
  fontFace = face && face.trim() ? face : null;
  if (fontFace === null) localStorage.removeItem("cys-font-face");
  else localStorage.setItem("cys-font-face", fontFace);
  for (const rt of panes.values()) {
    rt.term.options.fontFamily = composeFontFamily(fontFace);
    fitPane(rt);
  }
}

// Control Center 본문 전용 zoom — 터미널 fontSize와 분리(배율 단위).
// WebKit `zoom`을 #cc-body에만 적용(host #cc-panel은 fixed라 zoom 시 위치/스크롤 회귀 → 본문만 확대,
// sticky 헤더·탭은 1.0x 유지). 사이드바(ft/feed)는 터미널 작업공간 폭이라 zoom 비대상(터미널 fit 회귀 방지).
let panelZoom = Math.min(2, Math.max(0.6, Number(localStorage.getItem("cys-panel-zoom")) || 1)); // NaN·범위밖 방어
// CC 자동 배율 — 창 크기에 CC 본문을 비례 연동(오너 요청 2026-07-12: 모든 버튼·섹션이 창과 함께 커지고 작아지게).
// 배율 산식·클램프·합성 상한은 ccscale.ts(순수 로직·단위테스트 대상). 수동 Cmd +/-는 곱으로 합성.
// 오피스 탭은 CSS에서 zoom:1 고정 — 3D는 fit 카메라가 이미 창에 연동되므로 이중 스케일 금지(수동 zoom도 무효, 정책 확정 2026-07-12).
function applyPanelZoomVar() {
  document.documentElement.style.setProperty(
    "--panel-zoom",
    ccEffectiveZoom(panelZoom, window.innerWidth, window.innerHeight).toFixed(3),
  );
}
applyPanelZoomVar(); // 마운트 시 저장된 배율 복원
let panelZoomResizeTimer: number | undefined;
window.addEventListener("resize", () => {
  clearTimeout(panelZoomResizeTimer);
  panelZoomResizeTimer = setTimeout(applyPanelZoomVar, 80) as unknown as number;
});
function applyPanelZoom(delta: number | null) {
  panelZoom = delta === null ? 1 : Math.min(2, Math.max(0.6, +(panelZoom + delta * 0.1).toFixed(2)));
  localStorage.setItem("cys-panel-zoom", String(panelZoom));
  applyPanelZoomVar();
}

let workspaces: Workspace[] = [];
let activeWs = 0;
let wsCounter = 1;
let groups: GroupMeta[] = []; // 06: 그룹 메타 배열(진실원=localStorage)
let groupCounter = 1; // 06: 그룹 id 발급(ws의 wsCounter와 분리)
let focusedSid: number | null = null;
const panes = new Map<string, PaneRuntime>(); // 키 = paneKey(sid, socket)
// 부서 데몬 socket_slug(F3 백엔드 단일진실) → socket 경로. launch_dept_daemon 반환·daemon-event로 채운다.
const socketForSlug = new Map<string, string>();
// 사이드바 노드 신호 캐시(B3) — org.status 응답을 워크스페이스 행 집계용으로 보관.
type NodeSig = { role: string | null; state: string; ctx_pct: number | null; idle_secs: number; agent_alive: boolean | null };
const nodeSig = new Map<string, NodeSig>(); // 키 = `${socket}#${surface_id}`
let pendingApprovals = 0; // org.status feed.pending 전 소켓 합산(배지 구동 — 이 값만 배지가 쓴다)
// 같은 순회의 **소켓별** 대기 수. 배너("다른 워크스페이스에 N건")가 이 맵을 직접 읽는다.
// ★왜 합계를 나누는가(성찰3 설계렌즈 minor): 종전 배너는 `pendingApprovals - pendingItems.length`
//   라는 **스코프가 다른 두 카운터의 뺄셈**이었다 — 피감수는 기본 데몬 feed_list 목록 길이,
//   감수는 전 소켓 합산이라 (ⓐ부서 데몬 1개가 일시 미응답이면 그 소켓이 0으로 접혀 결과가
//   과소·음수(배너 소실) (ⓑ두 조회 사이 스큐로 과대(→ '다른 워크스페이스에 N건' 오안내)가
//   났다. 소켓별로 갖고 있으면 뺄셈이 사라지고 '기본 소켓이 아닌 것들의 합'을 직접 읽는다.
// 키 = Workspace.socket ?? DEFAULT_SOCKET_KEY(=기본 데몬). 값 = 마지막으로 **성공 조회한** 대기 수.
const pendingBySocket = new Map<string, number>();
// DEFAULT_SOCKET_KEY 의 정의처는 `deptlabel.ts` 다(부서 소켓 비교의 단일 정규화 지점) —
// 이 맵의 키와 이동 버튼 판정이 같은 값을 써야 행이 서로 어긋나지 않으므로 한 곳에서만 선언한다.
// ★결함#4-b(2026-08-22 오너 실사고) — 부서 데몬 대기의 **가시성**.
// 종전 승인 Feed 에는 "다른 워크스페이스(부서 데몬)의 대기 N건은 이 목록에 나오지 않습니다"
// 라는 경고문 한 줄뿐이었다. **어느 부서인지·어디로 가야 하는지**가 없어 부서에서 벌어지는
// 상태(단독 각성·티켓 대기·승인 대기)가 오너 화면에서 통째로 사라졌고, 오너는 "그냥 멈췄다"고
// 체감했다. 수리는 부서별 건수 + 그 부서로 가는 클릭 동선이다.
// ★새 조회를 만들지 않는다: refreshSidebarStatus 가 **이미 같은 순회에서** 소켓별 대기 수를
//   pendingBySocket 에 채워 둔다(추가 RPC 0 · 스큐 0 — 배너 값과 같은 스냅샷).
// ★종전 otherSocketPending(기본 소켓을 뺀 **합계 하나**)을 이 함수가 흡수했다 — 합계는
//   `deptPendingRows().reduce(...)` 로 그대로 나오고, 이제 **어느 부서인지**까지 함께 나온다.
//   합계 성질(파생 아닌 직접 합 · 음수 불가 · 기본 데몬 목록 길이·갱신 시점과 무관)은 불변이다.
// 부서 슬러그 표기(★F5)는 `deptlabel.ts` 의 순수 함수 `deptSlugOfSocket` 에 있다 — 이 파일
// 관례대로 순수 계산은 사이드 모듈에 두고 여기서는 배선만 한다(deptlabel.test.ts 가 회귀 핀).
type DeptPending = { socket: string; label: string; count: number };
const deptPendingRows = (): DeptPending[] => {
  const rows: DeptPending[] = [];
  for (const [sock, cnt] of pendingBySocket) {
    if (sock === DEFAULT_SOCKET_KEY || cnt <= 0) continue;
    // 라벨은 탭 이름(오너가 화면에서 부르는 이름). 탭이 없으면(레지스트리 잔재) 부서 슬러그로
    // 폴백 — 이름을 못 찾았다고 건수를 숨기면 '보이지 않는 대기'가 다시 생긴다.
    // ★F6②: `pending`(부서 데몬 기동 중) 탭도 이름을 갖고 있다 — 제외하면 기동 중인 부서만
    //   부제·라벨이 경로로 떨어져 식별이 나빠진다. 정상 탭을 우선하고 없으면 pending 을 쓴다.
    const ws =
      workspaces.find((w) => !w.pending && (w.socket ?? DEFAULT_SOCKET_KEY) === sock) ??
      workspaces.find((w) => (w.socket ?? DEFAULT_SOCKET_KEY) === sock);
    rows.push({ socket: sock, label: ws?.name ?? deptSlugOfSocket(sock), count: cnt });
  }
  return rows.sort((a, b) => b.count - a.count || a.label.localeCompare(b.label));
};
const root = document.getElementById("root")!;

// ---------- 배경 테마 커스텀 (cys-bg-color) ----------
// 색 선택 시 앱 캔버스(--bg)·캔버스 글자(--canvas-text)·모든 pane xterm 테마를 동기 적용 → 화면 일치.
// null = 기본(다크) 복원. 밝은 배경(휘도>0.5)이면 글자를 어둡게 자동 보정(가독).
// ★크롬 글자 --text는 건드리지 않는다 — 상단바·모달 등 배경이 안 바뀌는 var(--bar) 표면 가독 유지.
let bgColor: string | null = localStorage.getItem("cys-bg-color");
const currentBg = (): string => bgColor ?? DEFAULT_BG;
function applyBgColor(color: string | null): void {
  bgColor = color;
  const bg = color ?? DEFAULT_BG;
  const fg = readableForeground(bg);
  document.documentElement.style.setProperty("--bg", bg);
  document.documentElement.style.setProperty("--canvas-text", fg);
  for (const rt of panes.values()) rt.term.options.theme = { background: bg, foreground: fg };
  if (color === null) localStorage.removeItem("cys-bg-color");
  else localStorage.setItem("cys-bg-color", color);
}
applyBgColor(bgColor); // 마운트 시 저장된 배경색 복원(없으면 기본 유지)

const current = (): Workspace => workspaces[activeWs];

// 그룹의 anchor(부서) ws — anchorSocket이 일치하는 ws. 부서 그룹만 존재.
const anchorWsOf = (g: GroupMeta): Workspace | undefined =>
  g.anchorSocket ? workspaces.find((w) => w.socket === g.anchorSocket) : undefined;

// 부서 workspace는 socket 단위로 유일해야 한다(한 부서 데몬 = 한 탭). 저장·복원 양쪽에서 이 게이트를
// 통과시켜 중복(같은 socket 2탭)·id 중복이 저장→복원→재저장으로 증식하는 것을 차단한다.
// socket=undefined(기본 데몬) ws는 여러 개가 정상이므로 수렴 대상에서 제외.
function normalizeWorkspaces(list: Workspace[]): Workspace[] {
  const seenId = new Set<number>();
  const seenSock = new Map<string, Workspace>();
  const out: Workspace[] = [];
  for (const w of list) {
    if (w.pending) continue; // 런칭 중 임시 placeholder는 저장·복원에서 배제 (미완료 유령 탭 누수 차단)
    if (seenId.has(w.id)) continue;
    if (w.socket) {
      const prev = seenSock.get(w.socket);
      if (prev) {
        // 같은 부서 socket 중복: 비어있지 않은 트리를 우선 보존(사용자 분할 레이아웃 유실 방지)
        if (collectSids(w.tree).length && !collectSids(prev.tree).length) prev.tree = w.tree;
        continue;
      }
      seenSock.set(w.socket, w);
    }
    seenId.add(w.id);
    out.push(w);
  }
  return out;
}

// 06: 그룹 무결성 게이트 — normalizeWorkspaces와 같은 불변식 철학(save·restore 양쪽 통과로 유령/중복 증식 차단).
// 죽은 그룹 참조 청소(ws.groupId가 존재하지 않는 그룹을 가리키면 undefined화) + id중복 제거 + 멤버0 그룹 자동 해체(cmux ungroup 의미).
function normalizeGroups(ws: Workspace[], gs: GroupMeta[]): GroupMeta[] {
  const liveGids = new Set<number>();
  for (const w of ws) {
    if (w.groupId != null && !gs.some((g) => g.id === w.groupId)) w.groupId = undefined; // 죽은 그룹 참조 청소
    else if (w.groupId != null) liveGids.add(w.groupId);
  }
  const seen = new Set<number>();
  return gs.filter((g) => {
    if (seen.has(g.id)) return false; // id 중복 제거
    seen.add(g.id);
    return liveGids.has(g.id); // 멤버 0인 그룹 = 자동 해체
  });
}

function saveLayout() {
  const norm = normalizeWorkspaces(workspaces);
  const normG = normalizeGroups(norm, groups); // 06: norm 기준으로 그룹 청소
  groups = normG; // 06: 멤버0 그룹을 모듈 상태에서도 즉시 해체(유령 누적 방지 · 적대검증 교정)
  const activeId = workspaces[activeWs]?.id;
  const a = Math.max(0, norm.findIndex((w) => w.id === activeId));
  localStorage.setItem(
    LAYOUT_KEY,
    JSON.stringify({ workspaces: norm, groups: normG, active: a, counter: wsCounter, groupCounter }),
  );
}

function collectSids(node: Node | null, out: number[] = []): number[] {
  if (!node) return out;
  if (node.type === "pane") out.push(node.sid);
  else {
    collectSids(node.a, out);
    collectSids(node.b, out);
  }
  return out;
}

function replaceNode(node: Node, target: number, make: (old: Node) => Node | null): Node | null {
  if (node.type === "pane") {
    return node.sid === target ? make(node) : node;
  }
  const a = replaceNode(node.a, target, make);
  const b = replaceNode(node.b, target, make);
  if (a && b) return { ...node, a, b };
  return a ?? b; // one side removed → collapse to sibling
}

// ---------- pane lifecycle ----------

const b64ToBytes = (b64: string): Uint8Array => {
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
};

// Uint8Array → base64. 이미지(수백 KB)에서 fromCharCode(...전체)는 스택오버플로라 32KB 청크로 인코딩.
const bytesToB64 = (bytes: Uint8Array): string => {
  let bin = "";
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    bin += String.fromCharCode(...bytes.subarray(i, i + chunk));
  }
  return btoa(bin);
};

// 클립보드 이미지 MIME → 저장 파일 확장자. 미지·비표준은 png로 폴백.
const imageExtFromMime = (mime: string): string => {
  const m = mime.toLowerCase();
  if (m === "image/jpeg" || m === "image/jpg") return "jpg";
  if (m === "image/gif") return "gif";
  if (m === "image/webp") return "webp";
  return "png"; // image/png 및 기타
};

// surface도 번호 대신 이름 — 기본 자동 제목("surface N"·빈 문자열)이면 현재 디렉토리 경로 표시.
const isAutoTitle = (t: string | null | undefined) => !t || /^surface \d+$/.test(t);
const paneTitle = (title: string | null | undefined, liveCwd?: string | null) =>
  isAutoTitle(title) ? liveCwd || "…" : (title as string);

// pane 헤더 역할 점 — CC 깜박이 점(cc-blink)을 역할색으로 제목 앞에 표시(무역할 셸·종료 pane은 숨김).
function setRoleDot(el: HTMLElement, role: string | null) {
  const color = roleDotColor(role);
  el.style.display = color ? "" : "none";
  if (color) {
    el.style.background = color;
    el.title = `역할: ${role}`;
  }
}

// 주기적으로 데몬에 물어 자동 제목 pane의 현재 디렉토리(cd 추적)를 갱신.
// + 외부(CLI launch-agent·cys boot)에서 생성된 역할 노드 surface를 pane으로 자동 입양 —
//   이게 없으면 노드가 데몬 안에서 헤드리스로만 돌고 화면에 보이지 않는다.
let refreshing = false;
let started = false; // start()의 세션 복원이 끝나기 전 인터벌 자동 입양 차단 (이중 생성 방지)
async function refreshPaneTitles() {
  if (!started || refreshing) return; // 겹친 호출의 이중 입양 방지
  refreshing = true;
  try {
    // 멀티마스터 F4: workspace별 소켓을 순회 — 각 데몬의 surface를 그 소켓 ws에만 귀속시킨다.
    const sockets = [...new Set(workspaces.map((w) => w.socket))];
    let adopted = false;
    for (const sk of sockets) {
      const r = (await invoke("list_surfaces", { socket: sk })) as {
        surfaces: {
          surface_id: number;
          title: string;
          role: string | null;
          live_cwd: string | null;
          exited: boolean;
          usage?: ObservedUsage | null;
        }[];
      };
      for (const s of r.surfaces) {
        const rt = panes.get(paneKey(s.surface_id, sk));
        if (!rt) continue;
        renderUsage(rt.usageEl, s.exited ? null : s.usage); // 종료 pane은 배지 제거 (혼동 방지)
        setRoleDot(rt.roleEl, s.exited ? null : s.role); // 역할 점도 동일 주기 갱신
        if (rt.titleEl.isContentEditable) continue; // 이름 편집 중에는 덮어쓰지 않음
        rt.titleEl.textContent = paneTitle(s.title, s.live_cwd) + (s.exited ? " [exited]" : "");
      }
      // 자동 입양: 그 소켓의 role surface 중 UI에 없는 것 → '같은 소켓을 가진 ws'에만 표출.
      // ★소켓 일치 가드 — 부서A 노드가 부서B 탭에 잘못 입양되는 격리 누수 차단(검증 mustFix).
      // role 우선순위(master>cso>worker>reviewer) 정렬 — 부서 첫 입양 시 master가 첫 pane(좌측·focus)이 되도록.
      const rolePri = (role: string | null): number =>
        role === "master" ? 0 : role === "cso" ? 1 : role?.startsWith("worker") ? 2 : role?.startsWith("reviewer") ? 3 : 4;
      for (const s of [...r.surfaces].sort((a, b) => rolePri(a.role) - rolePri(b.role))) {
        if (s.exited || !s.role || panes.has(paneKey(s.surface_id, sk))) continue;
        // !w.pending — 런칭 중 placeholder(socket 미정)에는 입양 금지(타 데몬 surface 오입양 차단).
        const ws = workspaces.find((w) => !w.pending && (w.socket ?? undefined) === (sk ?? undefined));
        if (!ws || collectSids(ws.tree).includes(s.surface_id)) continue;
        setRoleDot((await makePane(s.surface_id, s.title, sk)).roleEl, s.role); // 입양 즉시 역할 점 채색(다음 틱 대기 없이)
        ws.tree = ws.tree
          ? { type: "split", dir: "row", a: ws.tree, b: { type: "pane", sid: s.surface_id } }
          : { type: "pane", sid: s.surface_id };
        adopted = true;
      }
    }
    if (adopted) {
      render();
      // 자동입양으로 pane이 생긴 활성 ws에 유효 포커스가 없으면 그 첫 pane에 포커스(포커스 회수, 탈취 아님).
      // 안 A: 부서 master 첫 등장 시 — 빈 셸이 없으므로 master pane으로 직행한다.
      const aSids = collectSids(current()?.tree ?? null);
      if (aSids.length && (focusedSid == null || !aSids.includes(focusedSid))) setFocus(aSids[0]);
    }
  } catch {
    /* 데몬 일시 미응답은 다음 틱에 */
  } finally {
    refreshing = false;
  }
  updateFtRoot(); // cd 추적 — 파일 트리 루트도 따라간다
}
setInterval(refreshPaneTitles, 3000);

// 2-click 삭제 확인의 armed 상태 아이콘 — 이모지(🗑)는 컬러 글리프라 CSS 틴트 불가, 인라인 SVG 사용
const TRASH_SVG =
  '<svg viewBox="0 0 24 24"><path d="M9 3h6l1 1h4v2H4V4h4l1-1zM6 8h12l-1 13a2 2 0 0 1-2 2H9a2 2 0 0 1-2-2L6 8z"/></svg>';

async function makePane(sid: number, title: string, socket?: string): Promise<PaneRuntime> {
  // 멱등 보장 — 같은 (소켓,surface)에 pane 런타임·리스너가 이중 생성되지 않게
  const existing = panes.get(paneKey(sid, socket));
  if (existing) return existing;
  const el = document.createElement("div");
  el.className = "pane";
  el.dataset.sid = String(sid); // 드래그 드롭존 탐색용
  const header = document.createElement("div");
  header.className = "pane-title";
  header.addEventListener("mousedown", (e) => {
    if (e.button !== 0 || titleEl.isContentEditable) return;
    if ((e.target as HTMLElement).classList?.contains("pane-close")) return;
    startPaneDrag(e, sid);
  });
  // 역할 신호 점(오너 요청 2026-07-12): Control Center의 깜박이 점을 제목 앞에 — 역할색 구별.
  // 색·표시 여부는 refreshPaneTitles(3초 주기)의 setRoleDot이 채운다(생성 시점엔 role 미상 → 숨김).
  const roleEl = document.createElement("span");
  roleEl.className = "pane-role-dot";
  roleEl.style.display = "none";
  const titleEl = document.createElement("span");
  titleEl.className = "pane-title-text";
  titleEl.textContent = paneTitle(title);
  const usageEl = document.createElement("span");
  usageEl.className = "pane-usage";
  // 배지 위 mousedown이 pane 드래그로 번지지 않게 — tooltip(hover) 확인 중 오발 방지
  usageEl.addEventListener("mousedown", (e) => e.stopPropagation());
  const closeBtn = document.createElement("span");
  closeBtn.className = "pane-close";
  closeBtn.textContent = "×";
  closeBtn.title = "surface 닫기 (셸 종료)";
  closeBtn.addEventListener("click", async () => {
    // WKWebView에서 confirm()은 무동작 — ws 탭과 동일한 2-click 확인 패턴
    if (closeBtn.dataset.arm !== "1") {
      closeBtn.dataset.arm = "1";
      closeBtn.innerHTML = TRASH_SVG;
      closeBtn.classList.add("close-armed");
      closeBtn.title = "한 번 더 누르면 삭제";
      setTimeout(() => {
        closeBtn.dataset.arm = "";
        closeBtn.textContent = "×";
        closeBtn.classList.remove("close-armed");
        closeBtn.title = "surface 닫기 (셸 종료)";
      }, 2500);
      return;
    }
    await invoke("close_surface", { socket, surfaceId: sid }).catch(() => {});
    destroyPaneRuntime(sid, socket);
    const ws = current();
    if (ws.tree) ws.tree = replaceNode(ws.tree, sid, () => null);
    if (focusedSid === sid) focusedSid = collectSids(ws.tree)[0] ?? null;
    render();
  });
  header.append(roleEl, titleEl, usageEl, closeBtn);
  header.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    showCtxMenu(e.clientX, e.clientY, [
      {
        label: "이름 변경",
        action: () => {
          titleEl.contentEditable = "true";
          titleEl.focus();
          window.getSelection()?.selectAllChildren(titleEl);
          const onKey = (ke: KeyboardEvent) => {
            if (ke.key === "Enter") {
              ke.preventDefault();
              titleEl.blur();
            }
          };
          const commit = () => {
            titleEl.removeEventListener("keydown", onKey); // rename마다 리스너 누적 방지
            titleEl.contentEditable = "false";
            const name = (titleEl.textContent || "").trim();
            // 빈 이름 = 자동 제목(경로)으로 복귀 — 데몬에 ""를 저장하면 isAutoTitle이 잡는다
            invoke("rename_surface", { socket, surfaceId: sid, title: name })
              .catch(() => {})
              .then(() => refreshPaneTitles());
          };
          titleEl.addEventListener("blur", commit, { once: true });
          titleEl.addEventListener("keydown", onKey);
        },
      },
    ]);
  });
  const termHost = document.createElement("div");
  termHost.className = "term-host";
  el.append(header, termHost);

  const term = new Terminal({
    // create_surface(아래 newSurface, rows:35/cols:120)로 띄운 PTY와 초기 폭을 일치시킨다.
    // 불일치(xterm 기본 80 < PTY 120) 시 zsh promptsp의 EOL 마커(반전 %)+(cols-1)공백이
    // 80폭에서 wrap돼 첫 줄(0,0)에 고립 표시된다. fit.fit()은 첫 프롬프트 뒤라 소급 정정 안 됨.
    cols: 120,
    rows: 35,
    // 폰트: 기본 스택(Latin 등폭을 CJK보다 앞에 — 셀 폭 측정 왜곡 방지)·선택 폰트 합성 = appearance.ts.
    fontFamily: composeFontFamily(fontFace),
    fontSize,
    // 배경 테마: 하드코딩 리터럴 대신 현재 색 상태 참조 — 새 pane도 커스텀 색으로 생성된다.
    theme: { background: currentBg(), foreground: readableForeground(currentBg()) },
    scrollback: 5000,
    // ★복사 불가 수리(2026-08 현장 제보): Claude Code TUI가 마우스 트래킹을 켜면 xterm은
    // 선택(selection)을 끄고, mac에서 강제 선택의 유일한 통로가 Option+드래그 && 이 옵션이다
    // (xterm SelectionService.shouldForceSelection — mac은 shift가 아니라 alt 경로).
    // 미설정이면 마우스 트래킹 중 어떤 조합으로도 드래그 선택·복사가 불가능했다.
    // 트레이드오프(의도된 결정): 이 옵션은 Option+클릭 커서 이동(altClickMovesCursor)을 mac
    // 전역에서 비활성화한다 — 스크롤백 복사 복구가 커서 점프보다 우선한다(오너 버그 수정 지시).
    macOptionClickForcesSelection: true,
  });
  const fit = new FitAddon();
  term.loadAddon(fit);
  term.open(termHost);

  // ── 출력 따라가기(바닥 고정) ──
  // xterm은 뷰포트가 정확히 바닥일 때만 새 출력을 따라간다 — 초기 크기(120x35)→fit 리플로우,
  // attach 스냅샷 재생, pane 분할 리사이즈로 한 번 바닥에서 어긋나면 이후 출력이 스크롤백으로만
  // 쌓여 하단 프롬프트 입력줄이 가려진다(수동 스크롤 강요). 출력 write 완료·리사이즈 후 바닥으로
  // 스냅하고, 사용자가 휠로 위로 올리면 해제(히스토리 읽기 보호), 바닥 복귀·키 입력 시 재고정.
  let follow = true;
  const atBottom = () => {
    const b = term.buffer.active;
    return b.viewportY >= b.baseY;
  };
  const snapToBottom = () => {
    if (follow && !atBottom()) term.scrollToBottom();
  };
  termHost.addEventListener(
    "wheel",
    (e: WheelEvent) => {
      // 위로 스크롤 = 즉시 해제 — rAF 판정까지 기다리면 스트리밍 중 write 스냅이 먼저 끌어내려
      // 사용자가 위로 못 올라가는 경주가 생긴다. 실제 위치 판정은 xterm이 휠을 처리한 뒤(rAF).
      if (e.deltaY < 0) follow = false;
      requestAnimationFrame(() => {
        follow = atBottom();
      });
    },
    { passive: true },
  );

  // WKWebView IME(한글 등 CJK) 조합 가드: 조합 중 keydown(keyCode 229/isComposing)을
  // xterm이 일반 키로 처리하면 자모가 분리 입력된다 — 조합 완성분만 onData로 흐르게 차단.
  term.attachCustomKeyEventHandler((e) => {
    if (e.isComposing || e.keyCode === 229) return false;
    // ★붙여넣기(F2): Ctrl/Cmd+V·Ctrl+Shift+V 를 xterm이 \x16(literal)로 삼키지 않게 false 반환 →
    // 브라우저 네이티브 paste 이벤트가 발화되고 아래 paste 리스너가 클립보드를 PTY로 보낸다.
    // (WebView2에서 xterm 기본 붙여넣기가 안 먹던 문제 — permission 불요의 clipboardData 경로.)
    if ((e.ctrlKey || e.metaKey) && (e.key === "v" || e.key === "V")) return false;
    // ★Shift+Enter = 줄바꿈(오너 요청 2026-07-12): Option/Alt+Enter가 보내는 것과 동일한
    // 바이트(ESC+CR)를 PTY로 전송 — claude 등 CLI가 meta-Enter로 해석해 프롬프트에 개행 삽입.
    // mac·Windows 공통(플랫폼 분기 불요). keydown에서만 전송하고 keypress/keyup은 흡수해 이중 전송 방지.
    if (e.key === "Enter" && e.shiftKey && !e.altKey && !e.ctrlKey && !e.metaKey) {
      if (e.type === "keydown") {
        // IME 잔여 pending(WKWebView 자모 버퍼)을 개행보다 먼저 확정 — 리듀서 우회 직송이면
        // ta keydown flush 리스너가 이 핸들러 '뒤'에 돌아 자모가 개행 뒤로 밀린다(순서 역전).
        // onData 경로의 flush("onData") 선행과 동일한 순서 보장. 뒤따르는 ta keydown 리스너의
        // 같은 keydown 재디스패치는 pending이 비어 no-op(디버그 계측만 중복).
        applyIme({ kind: "keydown", keyCode: e.keyCode, key: e.key });
        sendRaw("\x1b\r");
      }
      return false;
    }
    return true;
  });

  // 전송 직렬화 체인: 빠른 타자에서 비동기 IPC 호출이 경주하면 도착 순서가 뒤집힌다 —
  // promise 체인으로 같은 pane의 모든 입력을 발사 순서대로 보장한다.
  //
  // ★★불변식(delivery.rs 불변식 ② · 절대 보존): 이 경로는 **사용자가 자판으로 친 실키**
  // (term.onData·붙여넣기·Shift+Enter)다 — 여기에 `machineOrigin` 을 붙이면 오너가 친 문장이
  // 배달 원장에 남아 자기 해시와 매치돼 **기계로 접히고, 임무를 영영 줄 수 없다**(온보딩 사망).
  // UI 가 문자열을 조립해 보내는 호출(전출 지시·launchCmd·restartNode·injectRawToPane)에만
  // 표식을 붙인다. 새 자동 주입을 추가할 때 이 구분을 지키는 것이 R5 봉합의 유일한 전제다.
  let sendChain: Promise<unknown> = Promise.resolve();

  // ── 전송 실패 표면화(무음 삼킴 금지 · W-3) ─────────────────────────────────────
  // ★고친 결함: 종전 `.catch(() => {})` 는 데몬의 acl_denied·process_exited·write_stalled·
  //   write_failed(소켓/writer 절단)를 **전부 무음 처리**했다. 그래서 cso·worker pane 이 입력을
  //   전혀 받지 못하는 실제 결함이 "키를 쳐도 아무 일이 없다"로만 나타나고 화면에도 알람 이력에도
  //   흔적이 0이라, 사용자는 물론 조사팀조차 원인을 볼 수 없었다. 대조군 injectRawToPane(아래
  //   injectRawToPane 정의)은 같은 invoke("send_input") 실패를 토스트한다 — 그 형태를 따른다.
  // ★catch 는 반드시 유지한다(제거 금지): sendChain 은 pane 수명 전체를 잇는 **단일 promise
  //   체인**이라 거부가 체인에 남으면 이후 모든 then 이 건너뛰어져 그 pane 의 입력이 영구히 죽는다.
  //   ∴ 이 수리는 '삼킴 제거'가 아니라 '삼키던 자리에서 표면화'다.
  // ★토스트 폭주 방지(빠른 타자에서 매 키가 실패하면 초당 수십 건): ①sticky 토스트 id 를 pane
  //   당 하나로 고정 — stickyToast 는 같은 id 재호출 시 엘리먼트를 재사용하고 TTL 만 리셋하므로
  //   화면에는 언제나 한 줄만 남는다 ②표시 간격을 쿨다운(3초)으로 **무조건** 제한하고, 그 창의
  //   실패는 건수로 합산해 1회만 낸다.
  //   ★쿨다운에 예외를 두지 않는 이유(적대검증 지적 반영 — 초판 결함): 초판은 '사유가 바뀌면
  //   즉시 표시'라는 예외를 뒀는데, 그러면 **두 사유가 번갈아 오는 순간 가드가 완전히 무력**해진다
  //   (매 실패가 changed=true → wait=0 → 키마다 토스트 = 막겠다던 초당 수십 건이 그대로 재현).
  //   실패 사유가 흔들리는 상황은 드물지 않다(예: 절단 직후 write_failed 와 process_exited 교대).
  //   ∴ 예외를 없애고 대신 **정보는 지연될 뿐 소실되지 않게** 한다: 창 안에 새 사유가 오면 표시할
  //   사유를 최신 것으로 갈아끼우고 예약된 flush 를 그대로 두므로, 최대 3초 뒤 최신 사유가 뜬다.
  //   (첫 실패는 sendFailShownAt=0 이라 언제나 wait=0 = 즉시 — 최초 통지 지연은 없다.)
  //   ★건수 의미: '마지막 표시 이후 이 창에서 누적된 전 실패 수'다. 사유가 바뀌어도 리셋하지 않고
  //   이어 세며(표시되는 사유만 최신으로 교체), flush 가 표시 직후 0 으로 되돌린다 — 초판은 flush
  //   에서 리셋하지 않아 (3건)→(60건)→(1200건) 처럼 **단조 증가**했다. 소실되는 정보는 '창 안의
  //   이전 사유 문자열' 하나뿐이고, 창이 최대 3초라 실용상 무해하다.
  //   합산 대기 중인 타이머는 pane 당 최대 1개·최대 3초짜리이고, pane 파기 시 unlisten 배열의
  //   cancelSendFail 이 정리한다(닫힌 pane 의 유령 토스트 차단).
  // ★flush 는 **예외를 밖으로 내보내지 않는다**(try/catch 필수 — 위 '체인 유지' 계약의 짝):
  //   noteSendFail 은 `.catch(...)` 안에서 **동기로** 실행될 수 있고(wait===0 경로), 거기서 예외가
  //   하나라도 새면 catch 의 반환 promise 가 rejected 로 굳어 sendChain 이 영구 거부 = 그 pane 의
  //   입력이 죽는다 — 무음 삼킴을 고치려던 코드가 정확히 그 결함보다 나쁜 결함을 만드는 셈이다.
  //   현재 stickyToast 경로가 실제로 throw 하지는 않지만(#toasts 는 index.html 에 정적으로 존재),
  //   불변식이 **외부 DOM 의 존재**에만 의존하게 두지 않는다.
  // ★사유는 원문 그대로 보인다: send_input 은 rpc_on 경로라 데몬의 error.code 가 UI 까지 오지
  //   않고 message 만 온다(src-tauri/src/main.rs 의 rpc_on — error.message 만 String 으로 승격).
  //   ∴ feedReplyErrorText 같은 코드 기반 분류를 쓸 수 없다. 대신 데몬 메시지가 이미 자기설명적이다
  //   ("surface process has exited" / "surface input channel full (pane not consuming input)" 등).
  //   ★상한값 3초의 근거(임의 수가 아니다): ①이 창이 곧 표시 상한을 정의한다 — 표시는 창당
  //   1회이므로 **3초에 1회(≈0.33건/초)·pane 당**이 실제 상한이고, 화면에 남는 줄은 아래
  //   sendFailToastId 고정 재사용으로 언제나 1줄이다. ②창은 '합산이 실제로 일어나는' 길이여야
  //   한다: 빠른 타자·키 자동반복은 초당 수십 건까지 가므로 3초면 그 수십~수백 건이 1줄로
  //   접힌다(창이 100~200ms 면 접히는 게 거의 없어 가드가 사실상 없는 것과 같다).
  //   ③창은 sticky 토스트 수명(STICKY_TTL_MS=60초,
  //   toastttl.ts)보다 훨씬 짧아야 한다 — 창이 TTL 에 가까우면 갱신 전에 토스트가 만료돼
  //   "실패가 이어지는데 화면은 비어 있는" 구간이 생긴다. 3초는 ②와 ③ 사이에서 넉넉히 안전하다.
  const SEND_FAIL_COOLDOWN_MS = 3000;
  const sendFailToastId = `send-fail-${socket ?? ""}-${sid}`;
  let sendFailReason = "";
  let sendFailCount = 0;
  let sendFailShownAt = 0;
  let sendFailTimer: number | undefined;
  const flushSendFail = () => {
    sendFailTimer = undefined;
    sendFailShownAt = Date.now();
    const n = sendFailCount > 1 ? ` (${sendFailCount}건)` : "";
    const reason = sendFailReason;
    sendFailCount = 0; // 표시했으므로 창을 닫는다 — 미리셋 시 라벨이 단조 증가한다.
    try {
      const label = titleEl.textContent || `surface ${sid}`;
      stickyToast(sendFailToastId, "health", `입력 전송 실패${n}`, `${label} — ${reason}`);
    } catch (err) {
      // 표시 실패는 여기서 끝낸다 — 밖으로 새면 sendChain 이 영구 거부로 굳는다(위 계약).
      // 무음 삼킴은 아니다: 최후 폴백으로 콘솔에 남긴다. 그 콘솔 호출조차 실패할 수 있으므로
      // (WebView 확장·후킹이 console 을 갈아끼운 경우) 한 겹 더 감싸 침묵으로 끝낸다.
      // ※정직 표기: 릴리스 빌드에는 devtools 가 없어 이 줄은 개발 빌드에서만 읽힌다. 그래도
      //   토스트 경로가 통째로 깨졌을 때 남는 유일한 흔적이라 둔다(사용자 통지 수단은 아니다).
      try {
        console.error("[send-fail] 토스트 표시 실패", err);
      } catch {
        /* 최후 폴백의 폴백 — 여기서는 아무것도 하지 않는다(계약: 절대 throw 금지). */
      }
    }
  };
  const noteSendFail = (e: unknown) => {
    try {
      sendFailReason = String(e); // 표시할 사유는 언제나 최신 것(창 안 교체 — 지연될 뿐 미소실)
      sendFailCount++;
      // 마지막 표시로부터 쿨다운이 찰 때까지 합산한다(예외 없음 — 위 폭주 가드 근거).
      // ★Math.min 상한: Date.now() 는 단조가 아니다(NTP 보정·수동 시계 변경으로 뒤로 뛴다).
      //   그러면 sendFailShownAt 이 미래가 되어 wait 이 쿨다운을 훨씬 넘고, 토스트가 그 차이만큼
      //   (최악 수 시간) 지연되는 기아가 생긴다. 대기는 어떤 경우에도 창 길이를 넘지 않는다.
      const wait = Math.min(
        SEND_FAIL_COOLDOWN_MS,
        Math.max(0, SEND_FAIL_COOLDOWN_MS - (Date.now() - sendFailShownAt)),
      );
      if (wait === 0) {
        if (sendFailTimer !== undefined) {
          clearTimeout(sendFailTimer);
          sendFailTimer = undefined;
        }
        flushSendFail();
      } else if (sendFailTimer === undefined) {
        // 이미 예약돼 있으면 재예약하지 않는다 — 재예약은 실패가 이어지는 동안 경계를 계속
        // 뒤로 밀어 토스트가 **영영 안 뜨는** 기아를 만든다(초판의 clearTimeout+재예약 결함).
        sendFailTimer = window.setTimeout(flushSendFail, wait);
      }
    } catch (err) {
      // 상동 — 표면화 실패가 입력 경로를 죽이는 일은 없어야 한다.
      // (여기까지 오는 경로 예: String(e) 가 toString 이 던지는 객체를 받는 경우.)
      try {
        console.error("[send-fail] 실패 기록 실패", err);
      } catch {
        /* 최후 폴백의 폴백 — 아무것도 하지 않는다(계약: 절대 throw 금지). */
      }
    }
  };
  // ★거부 0 증명 — sendChain 이 rejected 로 굳는 경로가 없음을 코드로 확인한 결과(실측):
  //  ⓐ sendChain 이 코드에 등장하는 곳은 셋뿐이다 — 선언(`= Promise.resolve()`), 아래 sendRaw 의
  //    대입(`.then(() => invoke(...)).catch(noteSendFail)`), 그리고 그 `return sendChain`.
  //    ∴ 체인에 붙는 핸들러는 그 then·catch 한 쌍이 전부다(pane 클로저 밖으로 새지 않는다).
  //  ⓑ then 콜백의 동기 예외와 invoke 의 거부는 **바로 뒤 catch** 가 전부 받는다.
  //  ⓒ 그 catch 핸들러(noteSendFail)는 본문 전체가 try/catch 안이고, 내부에서 부르는
  //    flushSendFail 도 마찬가지이며, 두 catch 의 폴백(console)도 다시 try/catch 다.
  //    ∴ noteSendFail 은 throw 하지 않는다. 또 아무것도 반환하지 않으므로(undefined)
  //    catch 가 만드는 promise 는 thenable 흡수 없이 **항상 이행**된다.
  //  ⓓ ∴ 매 sendRaw 가 새로 대입하는 sendChain 은 언제나 fulfilled 로 정착한다 —
  //    이후 then 이 건너뛰어져 pane 입력이 죽는 경로는 0이다.
  //  ⓔ 반환값도 안전하다: sendRaw 의 호출자(onData 경로 · 초기 "\x1b\r" 전송)는 반환 promise 에
  //    핸들러를 달지 않으며, 달더라도 ⓓ에 의해 거부가 오지 않는다(unhandledrejection 0).
  // pane 파기 훅(unlisten 배열에 실어 destroyPaneRuntime 이 호출) — 대기 중 토스트 취소.
  const cancelSendFail = () => {
    if (sendFailTimer !== undefined) clearTimeout(sendFailTimer);
    sendFailTimer = undefined;
  };

  const sendRaw = (data: string) => {
    follow = true; // 입력 = 프롬프트 사용 의사 — 바닥 고정 재개(xterm scrollOnUserInput과 정합)
    sendChain = sendChain
      .then(() => invoke("send_input", { socket, surfaceId: sid, data }))
      .catch(noteSendFail); // 체인 유지(위 계약) + 실패를 사용자에게 보이게 한다
    return sendChain;
  };

  // ── 붙여넣기(clipboard → PTY) — WebView2/모든 플랫폼 ──
  // permission 불요: paste 이벤트의 clipboardData를 동기로 읽는다(navigator.clipboard 권한·Tauri 플러그인 불요).
  // capture(true)+preventDefault+stopPropagation 로 xterm 기본 paste 핸들러의 이중 처리·textarea 삽입을 차단하고,
  // term.paste()로 넘겨 bracketed-paste(멀티라인 자동실행 방지)·줄바꿈 정규화를 보존한 뒤 onData→sendRaw로 흐르게 한다.
  term.textarea?.addEventListener(
    "paste",
    (e: ClipboardEvent) => {
      const text = e.clipboardData?.getData("text") ?? "";
      e.preventDefault();
      e.stopPropagation();
      if (text) {
        term.paste(text);
        return;
      }
      // ★이미지 붙여넣기(F): 텍스트가 없고 클립보드에 이미지 파일이 있으면 임시 파일로 저장한 뒤
      // 그 경로를 셸 인용해 PTY로 타이핑한다(iTerm2 동작 — claude CLI 등이 경로로 이미지를 받게).
      // items·getAsFile·type은 이벤트 동안만 유효하므로 동기로 읽고, 파일 바이트만 비동기로 처리한다.
      const item = Array.from(e.clipboardData?.items ?? []).find(
        (it) => it.kind === "file" && it.type.startsWith("image/"),
      );
      const file = item?.getAsFile();
      if (!item || !file) return;
      const mime = item.type;
      file
        .arrayBuffer()
        .then((buf) =>
          invoke("save_pasted_image", {
            dataB64: bytesToB64(new Uint8Array(buf)),
            ext: imageExtFromMime(mime),
          }),
        )
        .then((path) => {
          const isWin = /Windows/i.test(navigator.userAgent);
          term.paste(shellQuote(path as string, isWin) + " ");
        })
        .catch((err) => toast("health", "이미지 붙여넣기 실패", String(err)));
    },
    true,
  );

  // ── WKWebView 한글 IME 조합 상태 머신 (판단 로직 = ime.ts 순수 리듀서 imeStep) ──
  // WKWebView는 표준 composition 없이 음절 첫 자모를 insertText로 커밋하거나(자모 유출), 혼성 프로필에선
  // 첫 자모를 insertText로 커밋한 뒤 나머지 조합을 표준 composition 이벤트로 진행한다.
  // 자모 pending, 병합 커밋, 음절 확정 flush, 조합 흡수 자모 폐기(drop) 판단은 ime.ts 리듀서가 하고,
  // 여기서는 DOM 이벤트를 리듀서에 배선만 한다. 계측: localStorage.cysImeDebug="1" 또는 파일
  // 게이트(~/.cys/ime-debug)/CYS_IME_DEBUG=1 시 이벤트 시퀀스를 log_ime로 기록(유실 경로를
  // 결정론으로 확정하는 채널 — 릴리스 빌드엔 devtools가 없어 파일 게이트가 최종 사용자 진단 경로). 평시 비용 0.
  let imeDbg = localStorage.getItem("cysImeDebug") === "1";
  if (!imeDbg) invoke("ime_debug_enabled").then((v) => { imeDbg = v === true; }).catch(() => {});
  const dbg = (line: string) => {
    if (imeDbg) invoke("log_ime", { line: `[s${sid}] ${line}` }).catch(() => {});
  };
  let imeState = initialImeState();
  const applyIme = (event: ImeEvent) => {
    const { state, actions } = imeStep(imeState, event);
    imeState = state;
    for (const a of actions) {
      if ("send" in a) sendRaw(a.send);
      else dbg(a.debug);
    }
  };

  // ★프로필 D 유출 감지(cys-neo, macOS 26.5.1 WKWebView): xterm의 Terminal._inputEvent가
  // inputType==='insertText'인 조합 첫 자모를 triggerDataEvent로 onData에 그대로 흘려보낸다
  // (음절 첫 자모 유출 = 이중 전송). 아래 WKWebView input 경로가 그 자모를 pending에 버퍼·확정하므로
  // 이 onData는 중복이다. 'input'(한글 insertText) 디스패치 중에 동기로 발화한 onData만 중복으로
  // 표시하기 위해, 부모 노드의 캡처 리스너로 유출 대상 자모를 기록한다(캡처 단계는 textarea 자체
  // 리스너 — xterm _inputEvent 포함 — 보다 먼저 실행되므로 유출 시점을 정확히 포착).
  let insertLeak: string | null = null;

  // ★앱 마우스 킬스위치 판독(새 pane부터 · 앱이 마우스를 갖는다 — 입·출력 양측 우회):
  // localStorage.cysAllowAppMouse="1"(devtools 있는 빌드용) 또는 ~/.cys/allow-app-mouse 파일 /
  // CYS_ALLOW_APP_MOUSE=1(릴리스 빌드용 — devtools 부재로 localStorage 설정 수단이 없다.
  // ime_debug 게이트와 동형). ★아래 onData 클로저가 참조하므로 등록 **앞**에서 정의한다 —
  // 뒤에 두면 attach await 구간의 입력이 TDZ ReferenceError 로 죽는다.
  const lsAllowAppMouse = localStorage.getItem("cysAllowAppMouse") === "1";

  // ★Windows 휠 가드 롤백 게이트 판독(새 pane부터 · 라이브 재판독 금지 — allowAppMouse와 동형):
  // localStorage.cysWinWheelGuardOff="1"(devtools 있는 빌드용) 또는 ~/.cys/win-wheel-guard-off 파일 /
  // CYS_WIN_WHEEL_GUARD_OFF=1(릴리스 빌드용 — devtools 부재로 localStorage 설정 수단이 없다.
  // Tauri 커맨드 win_wheel_guard_disabled · ime_debug/app_mouse 게이트와 동형). true면 아래 Windows
  // 휠 억제 핸들러를 **등록하지 않는다** = 종전(방향키 합성) 동작으로 즉시 복귀.
  // ★기존 allow-app-mouse 킬스위치를 이 롤백 용도로 재사용하면 안 된다 — 그것은 입·출력 양측을
  //   열어 Windows ConPTY 결함 1호(마우스 보고가 리터럴로 무한 타이핑)를 되살린다. 그래서 '출력측
  //   휠 억제만' 끄는 전용 게이트를 따로 둔다.
  // ★위치: allowAppMouse와 같은 자리(term.onData 등록 **앞**)다 — 뒤에 두면 attach await 구간에
  //   도착한 입력이 TDZ ReferenceError로 죽는다(바로 위 킬스위치 주석의 함정과 같은 이유).
  // ★IS_WINDOWS 단락평가: 이 값은 아래 Windows 분기에서만 소비된다. mac에서 invoke 왕복을 한 번
  //   더 태우면 pane attach가 그만큼 늦어지므로 조회 자체를 건너뛴다(mac에서는 항상 false이고,
  //   mac 경로는 이 값을 읽지 않는다 — 읽는 코드를 새로 만들지 말 것).
  const lsWinWheelGuardOff = localStorage.getItem("cysWinWheelGuardOff") === "1";

  // ★두 게이트 조회는 **병렬**이다(적대검증 2R note — 앵커 ④ 인접 결함 봉인). 직렬 await 두
  //   번이면 term.onData 등록 전 공백이 왕복 2회분으로 넓어지고, 그 창에 도착한 키 입력은
  //   조용히 유실된다(Windows 에서만 늘어나던 증분). 서로 독립이라 순서 의존이 없다.
  // ★단락평가는 보존한다: localStorage 로 이미 켜진 게이트는 invoke 를 아예 발사하지 않고,
  //   win_wheel_guard_disabled 는 IS_WINDOWS 일 때만 발사한다(mac 왕복 0 — 종전과 동일).
  // ★거부 폴백 방향도 종전과 동일하다: `.catch(() => false)` = **가드를 켠 채 유지**
  //   (fail-closed). 커맨드가 미등록인 빌드에서는 invoke 가 reject 되는데, 그때 가드가
  //   꺼지면(=결함 복원) 안 되기 때문이다. ※ 이 커맨드는 UI 와 같은 바이너리에 묶여 나가므로
  //   (ui/dist 임베드) 실제로는 버전 스큐가 생기지 않는다 — 그래도 폴백 방향은 안전측으로 둔다.
  const [beAllowAppMouse, beWinWheelGuardOff] = await Promise.all([
    lsAllowAppMouse ? false : invoke("app_mouse_enabled").catch(() => false),
    IS_WINDOWS && !lsWinWheelGuardOff ? invoke("win_wheel_guard_disabled").catch(() => false) : false,
  ]);
  const allowAppMouse = lsAllowAppMouse || beAllowAppMouse === true;
  const winWheelGuardOff = IS_WINDOWS && (lsWinWheelGuardOff || beWinWheelGuardOff === true);

  term.onData((data) => {
    // ★마우스 보고 필터 (현장 결함 1호 Windows 유출 + 2026-08 macOS 스크롤백 접근 불가).
    // Claude Code TUI가 마우스 트래킹(1003h/1006h)을 켜면 xterm.js가 마우스 보고를 PTY로 보낸다.
    // 판단은 전부 mousefilter.routeOnData(순수 함수·테스트가 고정)에 있고 여기는 실행만 한다:
    //   scroll  → 휠 보고를 PTY 전송 대신 로컬 스크롤로 번역 — **모든 플랫폼**(2026-08 개정:
    //             macOS도 트래킹 중 스크롤백을 읽을 수 있어야 한다. 방향키 시퀀스 주입은 금지 —
    //             의도된 결정. Claude Code의 입력 히스토리를 오염시킨다).
    //   discard → 비-휠 마우스 보고 무음 폐기 — **Windows 한정**(ConPTY가 시퀀스를 깨뜨려 선두
    //             ESC 소실 `[555;98;34M...`가 리터럴로 무한 타이핑되는 결함 1호 차단).
    //             macOS는 비-휠 보고를 forward 유지(앱의 클릭 소비 보존·오폐기=입력 소실 회피).
    //   forward → 마우스 보고가 아니면 바이트 하나 건드리지 않고 아래 IME 경로로 그대로.
    // ★opts(2026-08-12): allowAppMouse=킬스위치(분류 없이 원문 forward — 앱이 마우스를 갖는다는
    //   의미를 입력측까지 관철) · altScreen=대체 화면(스크롤백 없음 — 유출 트래킹의 휠은 로컬
    //   스크롤 no-op 대신 앱 forward/윈도우 폐기). 판단은 전부 routeOnData(테스트 고정)에 있다.
    const route = routeOnData(data, IS_WINDOWS, {
      allowAppMouse,
      altScreen: term.buffer.active.type === "alternate",
    });
    if (route.action === "scroll") {
      term.scrollLines(route.lines);
      return;
    }
    if (route.action === "discard") return;
    // 완성 음절은 그대로 PTY로 — 잔여 pending이 있으면 리듀서가 순서 보존 후 함께 전송(안전장치).
    // Windows 등 비-WKWebView에선 input 핸들러·insertLeak 감지 미배선이라 insertLeak이 항상 null →
    // duplicate=false → 순수 send(data)와 동일(회귀 0). WKWebView에서 insertText 자모 유출만 폐기.
    // route.data는 forward일 때 입력 data와 동일 참조다(mousefilter 계약) — IME 상태머신 무영향.
    applyIme({ kind: "onData", data: route.data, duplicate: insertLeak !== null && route.data === insertLeak });
  });

  // ★F: 위 조합 상태 머신은 macOS WKWebView 전용 우회다. Windows WebView2 등 Chromium 계열은
  // xterm.js 네이티브 composition이 완성 음절을 onData로 정확히 1회 발화하므로, 이 우회를 함께 켜면
  // input 핸들러가 pending에 버퍼한 글자를 리듀서가 보내고 onData의 send(data)가 다시 보내
  // 이중 전송된다("너"->"너너" 전 글자 중복 — Windows 실측).
  // ∴ WKWebView(AppleWebKit, 비-Chromium)에서만 input/keydown/blur/composition 리스너를 붙인다(macOS 회귀 0).
  const _ua = navigator.userAgent;
  const isWKWebView = /AppleWebKit/.test(_ua) && !/Chrome|Chromium|Edg\//.test(_ua);
  if (isWKWebView) {
    const ta = term.textarea;
    if (ta) {
      // ★프로필 D 유출 표식: 'input'(inputType==='insertText' && 한글) 디스패치가 시작될 때
      // insertLeak에 그 자모를 기록하고, 디스패치가 끝나면 해제한다. 부모 노드의 캡처 리스너는
      // textarea 자체 리스너(xterm _inputEvent 캡처 + 아래 리듀서 input 버블)보다 먼저 실행되므로,
      // xterm이 유출 onData를 발화하는 순간 insertLeak이 이미 세팅돼 있어 term.onData가 중복으로
      // 판정할 수 있다. 버블 리스너는 target 리스너 이후(디스패치 종료 시) 실행돼 표식을 해제한다.
      // 자모 유출(insertText)에만 한정 — Space·제어·붙여넣기·비한글·표준 composition onData는 무영향.
      const imeHost = ta.parentElement ?? ta;
      imeHost.addEventListener(
        "input",
        (e) => {
          const ie = e as InputEvent;
          insertLeak = ie.inputType === "insertText" && ie.data && isHangulText(ie.data) ? ie.data : null;
        },
        true, // 캡처 — textarea 리스너보다 먼저
      );
      imeHost.addEventListener("input", () => { insertLeak = null; }); // 버블 — 디스패치 종료 후 해제
      ta.addEventListener("input", (e) => {
        const ie = e as InputEvent;
        applyIme({ kind: "input", inputType: ie.inputType, data: ie.data });
      });
      // 혼성 프로필(C) 방어: 자모 insertText 커밋 후 조합이 표준 composition으로 이어지면
      // 리듀서가 흡수된 자모를 폐기한다. composition 3종 모두 배선(제5 프로필 진단 계측 포함).
      ta.addEventListener("compositionstart", () => applyIme({ kind: "compositionstart" }));
      ta.addEventListener("compositionupdate", () => applyIme({ kind: "compositionupdate" }));
      ta.addEventListener("compositionend", () => applyIme({ kind: "compositionend" }));
      ta.addEventListener("keydown", (e) => {
        // 일반 키(Enter·Space·화살표 등, IME 처리중 229 제외) 직전에 조합 확정(리듀서 flush).
        applyIme({ kind: "keydown", keyCode: e.keyCode, key: e.key });
        // 조합 중이 아닐 때 textarea 잔여 value 정리 (IME value 누적 방지)
        if (e.keyCode !== 229 && !imeState.pending && ta.value.length > 64) {
          (ta as HTMLTextAreaElement).value = "";
        }
      });
      ta.addEventListener("blur", () => applyIme({ kind: "blur" }));
    }
  }
  el.addEventListener("mousedown", () => setFocus(sid));
  term.textarea?.addEventListener("focus", () => setFocus(sid));

  // attach 먼저 — 백엔드가 (소켓 slug, surface_id) 이벤트명을 만들어 반환한다(단일 진실, UI 재계산 금지).
  const ev = (await invoke("attach_surface", { socket, surfaceId: sid })) as {
    output_event: string;
    exited_event: string;
  };
  const outStamp = { t: 0 }; // 마지막 출력 시각 — rt.lastOutputAt(스트리밍 가드)의 원천
  // ★마우스 트래킹 스트리핑+정합기(스펙 D4 · trackfilter.ts 계약 주석 참조): 앱의 DECSET
  // 1003h/1006h 류가 xterm 에 닿지 않게 걷어내 휠 스크롤·일반 드래그 선택·복사를 기본 동작으로
  // 복원하고, mac 은 alt 화면 구간만 장부 재생 주입으로 앱에 마우스를 돌려준다(fullscreen 휠
  // →프롬프트 히스토리 오염 봉인). pane 수명 전체 단일 인스턴스 — 스냅샷 재생과 라이브
  // 스트림이 같은 필터를 지난다. os·롤백 스위치는 pane 생성 시 캡처(라이브 재판독 금지 —
  // 수명 중 토글 시 필터/휠 억제 상태 분열. allowAppMouse 킬스위치와 동형 계약).
  // 킬스위치 allowAppMouse 는 onData 등록 앞(insertLeak 아래)에서 판독했다 — 여기서는 소비만.
  // 롤백 스위치 cysMouseReconcilerOff="1" = 정합기 전체 비활성(win 비활성 코드 경로 재사용).
  const reconcile = localStorage.getItem("cysMouseReconcilerOff") !== "1";
  const trackFilter = new MouseTrackingFilter({ os: IS_WINDOWS ? "win" : "mac", reconcile });
  // ★휠 억제 — OS로 **배타 분기**한다(mac=shouldSuppressWheel · Windows=shouldSuppressWheelWin).
  // (mac·스펙 D4) [alt buffer ∧ 장부 트래킹 요청 ∧ xterm 트래킹 미진입]이면 휠을 소비한다
  // (return false = xterm 기본 차단) — 미진입 창의 휠이 방향키로 합성돼 Claude Code 프롬프트
  // 히스토리를 오염시키는 결함 봉인. 판단은 wheelgate 순수 함수(테스트 고정). less/man(트래킹
  // 무요청)은 술어 불충족 → 방향키 합성 보존. mac 롤백 스위치 off(reconcile=false)면 미등록.
  //
  // (a) ★Windows도 이제 등록한다(스펙 C-2 — 종전에는 미등록이었다). 다만 술어는 **다른 함수**이고
  //     판별자를 1003(any-motion)으로 좁혔다: claude fullscreen은 1000+1002+1003+1006("full")을
  //     켜므로 충족(=억제)이고, vim `mouse=a`는 통상 1003을 켜지 않아 불충족(=방향키 합성 보존
  //     → 현행 Windows vim 휠 UX 무회귀)이다. 근거와 최대 미확정은 wheelgate.ts의 (b)·(d) 주석.
  // (b) ★종전 미등록의 근거였던 "Windows claude는 기본 inline이라 문제2가 미발현"은 **반증됐다**:
  //     Claude Code 2.1.233의 fullscreen 판정 함수 ra()에 순수 Windows→inline 분기가 없고
  //     (Windows 관련 분기는 Windows∧SSH 하나뿐), settings의 tui 키가 없으면 최종 판정은 서버측
  //     기능 게이트가 한다 — 즉 화면 모드는 OS가 아니라 **계정·롤아웃**이 결정한다. 아무것도
  //     옵트인하지 않은 Windows 사용자에게도 fullscreen이 뜰 수 있으므로 방어가 필요하다.
  // (c) ★롤백: winWheelGuardOff(위 판독 — env/파일 게이트)면 등록하지 않는다 = 종전 동작 복귀.
  // (d) ★정직 고지 — 억제는 그 휠 노치를 **버린다**: 억제가 걸린 창에서 휠은 스크롤도 방향키도
  //     아닌 **무동작**이 된다(alt 화면엔 스크롤백이 없어 로컬 스크롤이라는 대안 자체가 없다).
  //     그 대가로 막는 것은 프롬프트 히스토리 오염(원 결함)이고, 억제 대상은 1003 을 켠 앱으로
  //     한정된다 — vim·less·man 은 술어 불충족이라 종전 동작 그대로다.
  //     ※ 종전 이 자리에 있던 deltaMode=DOM_DELTA_PAGE 절대 상한(pageMode 항)은 **제거했다**
  //       (2026-08-17). 근거 전문은 wheelgate.ts (c) — 요지는 그 항이 덮는 영역이 이중 가정의
  //       사각뿐인데 비용은 'PAGE 보고 환경에서 페이저 휠 전멸'이고 탈출구가 가드 전체 끄기
  //       (=원 결함 복원)뿐이었다는 것. 그래서 이 배선에는 WheelEvent 를 읽는 코드가 없다.
  // ★Windows 분기에 reconcile(정합기 롤백 스위치) 조건이 없는 이유: 장부 기록은 소비(consume)와
  //   무관하게 공통이다 — trackfilter의 ReconcilerState.consume 주석대로 win·reconcile=false
  //   인스턴스도 ledger는 채운다. ∴ cysMouseReconcilerOff는 '정합기 소비'의 스위치이고, 이 가드의
  //   스위치는 winWheelGuardOff다(둘을 섞으면 롤백 의미가 흐려진다). 그 규칙의 정본은 이제
  //   wheelgate.wheelHandlerKind 이고 8조합 진리표가 고정한다.
  // ★attachCustomWheelEventHandler 는 인스턴스당 단일 슬롯(덮어쓰기)이다 — 두 갈래가 다 등록되면
  //   뒤가 앞을 조용히 덮어 mac 억제가 사라진다 = 설계 위반. 종전에는 그것을 `else if` 한 단어가
  //   지켰고 그 계약을 지키는 테스트가 0건이었다(성찰3 테스트렌즈 major). 이제 판정을 순수 함수로
  //   내려 **반환값이 하나뿐인 타입**으로 강제한다 — 여기서 OS·게이트를 다시 읽지 마라.
  //   판정에 먹이는 입력 조립(장부 접근자 선택·xterm 리터럴)도 같은 이유로 순수 함수다:
  //   ledgerWantsAnyMotion 자리에 인접 접근자 ledgerWantsMouse 를 쓰면 Windows vim 휠이 죽는데,
  //   인라인이던 시절엔 그 오배선을 잡는 단언이 저장소에 0건이었다.
  const wheelKind = wheelHandlerKind({ isWindows: IS_WINDOWS, reconcile, winWheelGuardOff });
  if (wheelKind === "mac") {
    term.attachCustomWheelEventHandler(
      () => !shouldSuppressWheel(macGateInputs(term, trackFilter, allowAppMouse, IS_WINDOWS)),
    );
  } else if (wheelKind === "win") {
    term.attachCustomWheelEventHandler(
      () => !shouldSuppressWheelWin(winGateInputs(term, trackFilter, allowAppMouse)),
    );
  }
  const un1 = await listen(ev.output_event, (e) => {
    outStamp.t = Date.now();
    const raw = b64ToBytes(e.payload as string);
    term.write(allowAppMouse ? raw : trackFilter.feed(raw), snapToBottom);
  });
  const un2 = await listen(ev.exited_event, () => {
    // 순서 고정(스펙 D4 ③): ①필터 잔여 carry 방류(시퀀스 중간 사망 시에도 바이트 소실 0)
    // ②정합기 reset — 장부 소거 + 8종 상수 DECRST 를 **필터 우회 term.write 직접** 기록
    //   (feed 재진입 금지 — 자기 스트리핑. 트래킹 소등·선택 복원, 1049l 미포함) ③종료 배너.
    const rest = trackFilter.flush();
    if (rest.length > 0) term.write(rest);
    term.write(trackFilter.reset());
    term.write("\r\n\x1b[31m[surface exited]\x1b[0m\r\n", snapToBottom);
  });
  // listen 등록을 마친 뒤에 스트림을 시작해야 초기 화면 snapshot(프롬프트)이 유실되지 않는다
  // (런치 시 첫 pane 빈 화면 버그 — snapshot이 listen 전에 emit되던 race 차단).
  await invoke("start_surface_stream", { socket, surfaceId: sid });

  let resizeTimer: number | undefined;
  const observer = new ResizeObserver(() => {
    clearTimeout(resizeTimer);
    resizeTimer = window.setTimeout(() => fitPane(rt), 60);
  });
  observer.observe(termHost);

  // unlisten 은 destroyPaneRuntime 이 전건 호출한다 — 이벤트 해제 외에 pane 수명에 묶인 타이머
  // 정리(cancelSendFail)도 여기 실어 필드를 늘리지 않는다(닫힌 pane 의 지연 토스트 차단).
  // ★cancelSendFail 을 **맨 앞**에 둔다: destroyPaneRuntime 의 forEach 는 앞 항목이 던지면
  //  뒤를 실행하지 못하므로, 뒤에 두면 un1/un2 가 던졌을 때 예약된 토스트가 죽은 pane 이름으로
  //  최대 3초 뒤 떠버린다. 순서 외의 의미는 없다(세 훅은 서로 독립).
  const rt: PaneRuntime = { sid, socket, el, termHost, roleEl, titleEl, usageEl, term, fit, unlisten: [cancelSendFail, un1, un2], observer, snapToBottom, lastOutputAt: () => outStamp.t, imeBusy: () => imeState.pending !== "", trackFilter };
  panes.set(paneKey(sid, socket), rt);
  return rt;
}

/// Fit only when actually laid out — a detached/hidden pane must not shrink the PTY.
function fitPane(rt: PaneRuntime) {
  if (rt.termHost.offsetWidth < 60 || rt.termHost.offsetHeight < 40) return;
  rt.fit.fit();
  rt.snapToBottom(); // 리사이즈 리플로우가 뷰포트를 바닥에서 밀어내는 경우 복귀
  invoke("resize_surface", { socket: rt.socket, surfaceId: rt.sid, rows: rt.term.rows, cols: rt.term.cols }).catch(() => {});
}

function destroyPaneRuntime(sid: number, socket?: string) {
  const rt = panes.get(paneKey(sid, socket));
  if (!rt) return;
  rt.observer.disconnect();
  rt.unlisten.forEach((u) => u());
  rt.term.dispose();
  rt.el.remove();
  panes.delete(paneKey(sid, socket));
}

// ---------- pane drag 이동 (탭을 끌어 자유 배치) ----------

type DropSide = "left" | "right" | "top" | "bottom";

function startPaneDrag(e0: MouseEvent, sid: number) {
  const start = { x: e0.clientX, y: e0.clientY };
  let dragging = false;
  let ghost: HTMLElement | null = null;
  let hint: HTMLElement | null = null;
  let target: { sid: number; side: DropSide } | null = null;
  let tabTarget: HTMLElement | null = null; // F6: 사이드바 ws 탭 위 드롭 = 전출

  const clearTabTarget = () => {
    tabTarget?.classList.remove("transfer-target");
    tabTarget = null;
  };
  const move = (e: MouseEvent) => {
    if (!dragging) {
      // 클릭(포커스)과 구분 — 6px 이상 움직여야 드래그 시작
      if (Math.abs(e.clientX - start.x) + Math.abs(e.clientY - start.y) < 6) return;
      dragging = true;
      ghost = document.createElement("div");
      ghost.id = "drag-ghost";
      ghost.textContent = panes.get(paneKey(sid, current()?.socket))?.titleEl.textContent || `surface ${sid}`;
      hint = document.createElement("div");
      hint.id = "drop-hint";
      hint.hidden = true;
      document.body.append(ghost, hint);
      document.body.classList.add("pane-dragging");
    }
    ghost!.style.left = `${e.clientX + 10}px`;
    ghost!.style.top = `${e.clientY + 10}px`;
    // F6: 사이드바 탭 히트테스트 — 탭 위면 전출 대상 표시(pane 분할 힌트와 배타)
    const hitEl = document.elementFromPoint(e.clientX, e.clientY) as HTMLElement | null;
    const tab = hitEl?.closest(".ws-tab") as HTMLElement | null;
    if (tab !== tabTarget) {
      clearTabTarget();
      if (tab?.dataset.wsId) {
        tabTarget = tab;
        tab.classList.add("transfer-target");
      }
    }
    if (tabTarget) {
      target = null;
      if (hint) hint.hidden = true;
      return;
    }
    const over = hitEl?.closest(".pane") as HTMLElement | null;
    target = null;
    if (over?.dataset.sid && Number(over.dataset.sid) !== sid) {
      const r = over.getBoundingClientRect();
      // 커서가 치우친 변 = 드롭 방향 (사분면 판정)
      const rx = (e.clientX - r.left) / r.width - 0.5;
      const ry = (e.clientY - r.top) / r.height - 0.5;
      const side: DropSide =
        Math.abs(rx) > Math.abs(ry) ? (rx < 0 ? "left" : "right") : (ry < 0 ? "top" : "bottom");
      target = { sid: Number(over.dataset.sid), side };
      const h = hint!;
      h.hidden = false;
      h.style.left = `${side === "right" ? r.left + r.width / 2 : r.left}px`;
      h.style.top = `${side === "bottom" ? r.top + r.height / 2 : r.top}px`;
      h.style.width = `${side === "left" || side === "right" ? r.width / 2 : r.width}px`;
      h.style.height = `${side === "top" || side === "bottom" ? r.height / 2 : r.height}px`;
    } else if (hint) {
      hint.hidden = true;
    }
  };
  const up = () => {
    window.removeEventListener("mousemove", move, true);
    window.removeEventListener("mouseup", up, true);
    ghost?.remove();
    hint?.remove();
    document.body.classList.remove("pane-dragging");
    const destWsId = tabTarget?.dataset.wsId;
    clearTabTarget();
    if (dragging && destWsId != null) void transferPaneToWs(sid, Number(destWsId));
    else if (dragging && target) movePane(sid, target.sid, target.side);
  };
  window.addEventListener("mousemove", move, true);
  window.addEventListener("mouseup", up, true);
}

// F6: pane 전출 — 동일 socket ws 간은 원자 트리 이동(transfer.ts 순수 로직),
// 크로스 부서(다른 socket)는 맥락보존 재인스턴스화(transferCrossDept).
async function transferPaneToWs(sid: number, destWsId: number) {
  const srcWs = current();
  const destWs = workspaces.find((w) => w.id === destWsId);
  if (!destWs || destWs === srcWs) return;
  if (destWs.pending) {
    toast("watchdog", "전출 불가", "대상 부서 데몬이 아직 준비 중입니다");
    return;
  }
  if ((destWs.socket ?? undefined) !== (srcWs.socket ?? undefined)) {
    return transferCrossDept(sid, srcWs, destWs);
  }
  // 가드: 전출 시점 pane 생존(reap 경합) — 죽은 sid를 대상 트리에 넣지 않는다
  if (!panes.has(paneKey(sid, srcWs.socket))) {
    toast("watchdog", "전출 불가", "pane이 이미 종료되었습니다");
    return;
  }
  const moved = transferTrees(srcWs.tree, destWs.tree, sid);
  if (!moved) {
    toast("watchdog", "전출 불가", "레이아웃에서 pane을 찾지 못했습니다");
    return;
  }
  // 원자 반영: 두 트리 동시 교체 → 단일 render(saveLayout 포함) — JS 단일 스레드의 동기 블록이라
  // 같은 sid가 두 트리에 걸친 중간 상태가 렌더·영속에 노출되지 않는다.
  srcWs.tree = moved.src;
  destWs.tree = moved.dest;
  if (focusedSid === sid) focusedSid = collectSids(srcWs.tree)[0] ?? null;
  render();
  if (focusedSid != null) setFocus(focusedSid);
  toast("feed", "pane 전출 완료", `→ ${destWs.name || UNTITLED}`);
}

// F6-2: 크로스 부서 전출 — 라이브 프로세스는 데몬 간 이주가 물리적으로 불가하므로,
// 핸드오프 문서로 맥락을 승계(HANDOFF_CONTRACT 5필드)한 뒤 대상 부서에서 재기동한다.
// 어느 단계든 실패 시 원본 무접촉(전출 실패=현상 유지). 원본 정리는 재기동 성공 후에만.
async function transferCrossDept(sid: number, srcWs: Workspace, destWs: Workspace) {
  const srcSock = srcWs.socket;
  if (!panes.has(paneKey(sid, srcSock))) {
    toast("watchdog", "전출 불가", "pane이 이미 종료되었습니다");
    return;
  }
  const r = (await invoke("list_surfaces", { socket: srcSock }).catch(() => null)) as {
    surfaces: { surface_id: number; live_cwd: string | null; role?: string | null; agent?: string | null }[];
  } | null;
  const me = r?.surfaces.find((s) => s.surface_id === sid);
  // fail-closed: RPC 실패·미발견이면 분류 불가 — 살아있는 에이전트를 '셸'로 오분류해
  // 핸드오프 없이 닫는 파괴 분기로의 fail-open 금지(적대검증 최강 공격 봉인).
  if (!me) {
    toast("watchdog", "전출 불가", "pane 상태를 확인할 수 없습니다(RPC 실패) — 원본은 그대로입니다");
    return;
  }
  const isAgent = !!(me.role || me.agent);
  const cwd = me.live_cwd ?? null;
  // 부서 노드는 cwd가 "/"(루트)로 뜨는 경우가 실측됨 — 루트류(/·드라이브 루트)·미확보 cwd는
  // 프로젝트 상대 경로(_round/handoffs)가 성립하지 않으므로 ~/.cys/transfers 로 폴백한다.
  const rootish = !cwd || cwd === "/" || /^[A-Za-z]:[\\/]?$/.test(cwd);
  // 역할 승계: 원 역할 그대로 재기동(무음 worker 강등 금지). 데몬 유래 값이지만 명령 조합에
  // 들어가므로 형식 가드([a-z0-9-]) — 벗어나면 worker 폴백. 리뷰어는 전용 CLI 처방(RESTART와 동일).
  const srcRole = me.role && /^[a-z0-9-]{1,32}$/.test(me.role) ? me.role : "worker";
  const LAUNCH_BY_ROLE: Record<string, string> = {
    "reviewer-gemini": "agy --dangerously-skip-permissions",
    "reviewer-codex": "codex --dangerously-bypass-approvals-and-sandbox",
  };
  const launchCmd = LAUNCH_BY_ROLE[srcRole] ?? `cys launch-agent --role ${srcRole} --agent claude`;
  const ok = await confirmModal(
    "부서 간 전출",
    isAgent
      ? `부서 간 전출은 핸드오프 문서로 맥락을 승계한 재기동입니다(${srcRole} 역할로 재기동). ` +
          "진행 중이던 응답(라이브 추론 상태)은 이어지지 않습니다. 진행하시겠습니까?"
      : "이 pane은 에이전트 미등록(셸)입니다 — 같은 경로의 새 셸을 대상 부서에 만들고 이 pane을 닫습니다. 진행하시겠습니까?",
    "전출",
  );
  if (!ok) return;
  try {
    let handoffPath: string | null = null;
    if (isAgent) {
      // ① 핸드오프 지시 주입 — clear_first(데몬 T3-13 권위 전달: Ctrl-U 정리→paste→지연 CR
      //    원자 제출). raw "\r" 동봉은 Claude CLI가 붙여넣기로 삼켜 미제출(e2e 실측 결함).
      const base = rootish
        ? `${(await invoke("home_dir_path")) as string}/.cys/transfers`
        : `${cwd}/_round/handoffs`;
      handoffPath = `${base}/transfer-${sid}-${Date.now()}.md`;
      const inst =
        `지금까지의 작업 상태를 HANDOFF_CONTRACT 5필드로 ${handoffPath} 에 기록하라` +
        `(디렉토리가 없으면 mkdir -p로 생성). 각 필드는 정확히 "## Decided" "## Rejected" "## Risks" ` +
        `"## Files" "## Remaining" 마크다운 헤더로 쓰고, 해당 없음은 "없음"으로 명기하라. ` +
        `5필드가 모두 기록된 파일이 전출 준비 완료 신호다.`;
      // ★R5 machineOrigin: 이 문안은 **UI 코드가 조립한 것**이지 사용자가 자판으로 친 것이
      // 아니다. 표식이 없으면 데몬이 오퍼레이터 토큰만 보고 배달 원장 기록을 억제하고, 그러면
      // 대상 pane 의 훅이 이 지시를 **오너 임무**로 기록해 자율 착수 게이트가 열린다(실측 관통).
      // 사용자 실키(sendRaw)에는 절대 붙이지 않는다 — 붙이면 오너 문장이 기계로 접힌다.
      await invoke("send_input", {
        socket: srcSock,
        surfaceId: sid,
        data: inst,
        clearFirst: true,
        machineOrigin: true,
      });
      // ② 5필드 내용 검증 대기(3초 간격·최대 120초) — 화면 파싱이 아니라 파일 내용 확인(결정론).
      stickyToast("transfer", "feed", "전출 준비 중", "핸드오프 기록 대기(최대 120초)…");
      // 파일 실존≠내용 유효 — 5필드(HANDOFF_CONTRACT)가 전부 갖춰질 때까지 대기한다.
      // 부분 기록(아직 쓰는 중)도 다음 틱에 재확인되므로 조기 통과가 없다.
      const FIELDS = ["## Decided", "## Rejected", "## Risks", "## Files", "## Remaining"];
      const deadline = Date.now() + 120_000;
      let ready = false;
      while (Date.now() < deadline) {
        await new Promise((res) => setTimeout(res, 3000));
        const head = (await invoke("read_text_head", { path: handoffPath }).catch(() => null)) as
          | string
          | null;
        if (head && FIELDS.every((f) => head.includes(f))) {
          ready = true;
          break;
        }
      }
      if (!ready) {
        toast(
          "watchdog",
          "전출 중단",
          "핸드오프가 기록되지 않았거나 5필드가 미완성입니다(120초) — 원본은 그대로입니다",
        );
        return;
      }
    }
    // ③ 대상 부서에 새 surface 생성 + 트리 편입(같은 경로에서 시작 — 루트류 cwd는 승계하지
    //    않고 데몬 기본값(home)으로: 루트 cwd pane 재생산 차단)
    const newSid = await newSurface(rootish ? null : cwd, destWs.socket);
    destWs.tree = destWs.tree
      ? { type: "split", dir: "row", a: destWs.tree, b: { type: "pane", sid: newSid } }
      : { type: "pane", sid: newSid };
    // ③ 이후 실패는 보상 트랜잭션 — 새 pane 회수+트리 복원으로 "원본 보존"을 거짓말이 아니게 한다.
    try {
      if (isAgent) {
        // ④ 에이전트 재기동(노드 재기동 처방과 동일 명령) — UI 가 조립한 명령이므로 machineOrigin
        await invoke("send_input", {
          socket: destWs.socket,
          surfaceId: newSid,
          data: `${launchCmd}\r`,
          machineOrigin: true,
        });
        // agent-ready 폴링(최대 60초): agent_meta 등록을 확인한 뒤 복원 지시를 보낸다 —
        // queued(조용 시점 배달)만으로는 부팅 중 quiet 순간에 떨어져 유실될 수 있다(이중 안전).
        stickyToast("transfer", "feed", "전출 진행 중", "새 워커 기동 대기…");
        const readyBy = Date.now() + 60_000;
        while (Date.now() < readyBy) {
          await new Promise((res) => setTimeout(res, 3000));
          const rr = (await invoke("list_surfaces", { socket: destWs.socket }).catch(() => null)) as {
            surfaces: { surface_id: number; role?: string | null; agent?: string | null }[];
          } | null;
          const ns = rr?.surfaces.find((s) => s.surface_id === newSid);
          if (ns?.agent || ns?.role) break; // 등록 확인 — 미확인이어도 queued가 2차 안전망
        }
        await invoke("send_input", {
          socket: destWs.socket,
          surfaceId: newSid,
          data: `너는 전출된 워커다. ${handoffPath} 를 읽고 작업을 이어가라.`,
          queued: true,
          // queued 는 배달자(Origin::Queue)가 별도로 기록하지만, 표식을 붙여 두면 경로가 바뀌어도
          // "UI 가 만든 문안"이라는 사실이 유지된다(누락 재발 방지 규칙: UI 조립 = 표식).
          machineOrigin: true,
        });
      }
      // ⑤ 재기동 성공 후에만 원본 정리
      await invoke("close_surface", { socket: srcSock, surfaceId: sid });
    } catch (e) {
      await invoke("close_surface", { socket: destWs.socket, surfaceId: newSid }).catch(() => {});
      destroyPaneRuntime(newSid, destWs.socket);
      if (destWs.tree) destWs.tree = replaceNode(destWs.tree, newSid, () => null);
      render();
      toast("watchdog", "전출 실패", `${e} — 원본 pane은 보존되고 새 pane은 회수했습니다`);
      return;
    }
    destroyPaneRuntime(sid, srcSock);
    if (srcWs.tree) srcWs.tree = replaceNode(srcWs.tree, sid, () => null);
    if (focusedSid === sid) focusedSid = collectSids(current()?.tree ?? null)[0] ?? null;
    render();
    toast("feed", "부서 전출 완료", `→ ${destWs.name || UNTITLED} (surface:${newSid})`);
  } catch (e) {
    toast("watchdog", "전출 실패", `${e} — 원본 pane은 보존됩니다`);
  } finally {
    dismissToast("transfer");
  }
}

/// sid pane을 트리에서 떼어 target pane의 side 쪽에 분할 삽입한다.
function movePane(sid: number, targetSid: number, side: DropSide) {
  const ws = current();
  if (!ws.tree || sid === targetSid) return;
  const sids = collectSids(ws.tree);
  if (!sids.includes(sid) || !sids.includes(targetSid)) return;
  ws.tree = replaceNode(ws.tree, sid, () => null);
  const moved: Node = { type: "pane", sid };
  if (!ws.tree) {
    ws.tree = moved;
  } else {
    const dir = side === "left" || side === "right" ? "row" : "col";
    const before = side === "left" || side === "top";
    ws.tree = replaceNode(ws.tree, targetSid, (old) => ({
      type: "split",
      dir,
      a: before ? moved : old,
      b: before ? old : moved,
    }));
  }
  render();
  setFocus(sid);
}

function setFocus(sid: number) {
  focusedSid = sid;
  const key = paneKey(sid, current()?.socket);
  for (const [id, rt] of panes) rt.el.classList.toggle("focused", id === key);
  panes.get(key)?.term.focus();
  updateFtRoot(); // 파일 트리가 열려 있으면 선택한 surface의 폴더로 전환
}

// 드롭 물리좌표(디바이스 픽셀)를 CSS px로 환산해 그 지점을 '직격'하는 pane만 찾는다.
// 폴백 없음 — 빗나간 드롭이 포커스 pane에 조용히 주입되던 오배달 footgun 제거.
// 호출측이 undefined를 무동작+토스트로 처리한다(무음 실패 금지).
function paneAtPointStrict(pos?: { x: number; y: number }): PaneRuntime | undefined {
  if (!pos) return undefined;
  const dpr = window.devicePixelRatio || 1;
  const hit = document.elementFromPoint(pos.x / dpr, pos.y / dpr) as HTMLElement | null;
  const paneEl = hit?.closest(".pane") as HTMLElement | null;
  if (!paneEl) return undefined;
  for (const rt of panes.values()) if (rt.el === paneEl) return rt;
  return undefined;
}

// ---------- render ----------

function render() {
  for (const rt of panes.values()) rt.el.remove();
  root.innerHTML = "";
  const ws = current();
  const tree = ws?.tree;
  if (tree) root.appendChild(renderNode(tree));
  else if (ws?.pending) root.appendChild(renderDeptPending()); // WP-10: 부서 준비 중 빈 pane 스피너·안내
  renderWsTabs();
  requestAnimationFrame(() => {
    for (const sid of collectSids(current()?.tree ?? null)) {
      const rt = panes.get(paneKey(sid, current()?.socket));
      if (rt) fitPane(rt);
    }
  });
  saveLayout();
}

// WP-10: 부서 데몬 준비(~12초·tree:null) 동안 빈 pane 호스트에 중앙 스피너+안내 문구를 표시한다.
// 성공 시 tree가 채워져 자연 교체되고, 실패 시 placeholder 탭이 롤백된다(addDeptWorkspace 3분기 로직 불변).
// aria-busy/aria-live 로 스크린리더에 진행/해소를 통지. 스피너 회전·정지는 CSS(prefers-reduced-motion)가 담당.
function renderDeptPending(): HTMLElement {
  const host = document.createElement("div");
  host.className = "pane dept-pending";
  host.setAttribute("aria-busy", "true");
  host.setAttribute("aria-live", "polite");
  const box = document.createElement("div");
  box.className = "dept-pending-box";
  const spin = document.createElement("div");
  spin.className = "dept-spinner";
  spin.setAttribute("aria-hidden", "true");
  const msg = document.createElement("div");
  msg.className = "dept-pending-msg";
  msg.textContent = "부서를 준비하고 있습니다 — 최대 십여 초 걸릴 수 있어요";
  box.append(spin, msg);
  host.appendChild(box);
  return host;
}

function renderNode(node: Node): HTMLElement {
  if (node.type === "pane") {
    const rt = panes.get(paneKey(node.sid, current()?.socket));
    if (rt) return rt.el;
    const placeholder = document.createElement("div");
    placeholder.className = "pane";
    placeholder.textContent = `surface:${node.sid} (없음)`;
    return placeholder;
  }
  const div = document.createElement("div");
  div.className = `split ${node.dir}`;
  const aEl = renderNode(node.a);
  const bEl = renderNode(node.b);
  const divider = document.createElement("div");
  divider.className = "divider";
  const ratio = node.ratio ?? 0.5;
  aEl.style.flex = `${ratio} 1 0%`;
  bEl.style.flex = `${1 - ratio} 1 0%`;
  attachDividerDrag(divider, div, node, aEl, bEl);
  div.append(aEl, divider, bEl);
  return div;
}

function attachDividerDrag(
  divider: HTMLElement,
  container: HTMLElement,
  node: Node & { type: "split" },
  aEl: HTMLElement,
  bEl: HTMLElement,
) {
  divider.addEventListener("mousedown", (down) => {
    down.preventDefault();
    divider.classList.add("dragging");
    const horizontal = node.dir === "row";
    const move = (e: MouseEvent) => {
      const rect = container.getBoundingClientRect();
      const pos = horizontal ? e.clientX - rect.left : e.clientY - rect.top;
      const size = horizontal ? rect.width : rect.height;
      const ratio = Math.min(0.85, Math.max(0.15, pos / size));
      node.ratio = ratio;
      aEl.style.flex = `${ratio} 1 0%`;
      bEl.style.flex = `${1 - ratio} 1 0%`;
    };
    const up = () => {
      divider.classList.remove("dragging");
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
      saveLayout();
      for (const sid of collectSids(node)) {
        const rt = panes.get(paneKey(sid, current()?.socket));
        if (rt) fitPane(rt);
      }
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  });
}

// ---------- 사이드바 드래그 순서 변경 (ws 탭·그룹 섹션) ----------
// HTML5 draggable API는 Tauri wry/WKWebView가 가로채 신뢰 불가 → attachDividerDrag처럼
// mousedown + window mousemove/mouseup로 직접 구현. 배열 변형은 reorder.ts 순수 함수가 담당,
// 여기선 히트테스트·삽입 표시선·render()만.

// 삽입 위치 표시선(fixed) — 앵커 rect의 위/아래 모서리에 2px 라인. pointer-events:none로 히트테스트 방해 차단.
function makeDropLine(): HTMLElement {
  const el = document.createElement("div");
  el.className = "ws-drop-indicator";
  el.hidden = true;
  document.body.appendChild(el);
  return el;
}
function placeDropLine(el: HTMLElement, left: number, edgeY: number, width: number) {
  el.hidden = false;
  el.style.left = `${left}px`;
  el.style.top = `${edgeY - 1}px`;
  el.style.width = `${width}px`;
}
// 실제 드래그(임계 초과) 뒤에 뒤따르는 합성 click을 1회 삼킨다(그룹 name focus 등 오발 방지).
// click이 안 오면 setTimeout으로 자기청소 → 미래의 무관한 click을 먹지 않는다.
function suppressNextClick() {
  const h = (ev: Event) => {
    ev.stopPropagation();
    ev.preventDefault();
    cleanup();
  };
  const cleanup = () => window.removeEventListener("click", h, true);
  window.addEventListener("click", h, true);
  setTimeout(cleanup, 0);
}

// ws 탭 드래그: ungrouped·그룹 body 내 재정렬 + 그룹 간 이동. 4px 임계 후에만 드래그 시작.
function startWsDrag(e0: MouseEvent, srcId: number) {
  const start = { x: e0.clientX, y: e0.clientY };
  let dragging = false;
  let line: HTMLElement | null = null;
  let drop: { destGroupId: number | undefined; anchorId: number | null; before: boolean } | null = null;

  const move = (e: MouseEvent) => {
    if (!dragging) {
      if (Math.abs(e.clientX - start.x) + Math.abs(e.clientY - start.y) < 4) return; // 클릭과 구분
      dragging = true;
      // 소스 노드는 mousedown 시 ws 전환 render()로 교체됐을 수 있어 id로 재조회
      document.querySelector(`#ws-tabs .ws-tab[data-ws-id="${srcId}"]`)?.classList.add("ws-dragging");
      line = makeDropLine();
      document.body.classList.add("ws-reordering");
    }
    const el = document.elementFromPoint(e.clientX, e.clientY) as HTMLElement | null;
    drop = null;
    const overTab = el?.closest<HTMLElement>(".ws-tab[data-ws-id]");
    if (overTab && Number(overTab.dataset.wsId) !== srcId) {
      const r = overTab.getBoundingClientRect();
      const before = e.clientY < r.top + r.height / 2; // 커서가 상반부면 앞
      const anchor = workspaces.find((w) => w.id === Number(overTab.dataset.wsId));
      drop = { destGroupId: anchor?.groupId, anchorId: anchor!.id, before };
      placeDropLine(line!, r.left, before ? r.top : r.bottom, r.width);
    } else if (overTab) {
      line!.hidden = true; // 소스 자기 위 = no-op
    } else {
      const sec = el?.closest<HTMLElement>(".ws-group[data-group-id]");
      if (sec) {
        // 그룹 헤더·body 빈 영역 위 → 그 그룹 끝에 추가
        drop = { destGroupId: Number(sec.dataset.groupId), anchorId: null, before: false };
        const r = sec.getBoundingClientRect();
        placeDropLine(line!, r.left, r.bottom, r.width);
      } else if (el?.closest("#ws-tabs")) {
        // ungrouped 빈 영역 → ungrouped 끝에 추가
        drop = { destGroupId: undefined, anchorId: null, before: false };
        const bar = document.getElementById("ws-tabs")!;
        const tabs = bar.querySelectorAll<HTMLElement>(":scope > .ws-tab[data-ws-id]");
        const lastR = (tabs[tabs.length - 1] ?? bar).getBoundingClientRect();
        placeDropLine(line!, lastR.left, tabs.length ? lastR.bottom : lastR.top, lastR.width);
      } else {
        line!.hidden = true;
      }
    }
  };
  const up = () => {
    window.removeEventListener("mousemove", move, true);
    window.removeEventListener("mouseup", up, true);
    line?.remove();
    document.body.classList.remove("ws-reordering");
    document.querySelector(`#ws-tabs .ws-tab[data-ws-id="${srcId}"]`)?.classList.remove("ws-dragging");
    if (dragging) suppressNextClick();
    if (dragging && drop) {
      // activeWs는 인덱스 — 배열 변형 전 활성 ws의 id를 잡아 변형 후 재계산(엉뚱한 탭 활성화 방지).
      // reorderWorkspace는 새 배열(그룹 이동 시 src는 클론)을 돌려주므로 참조가 아닌 id로 찾는다.
      const actId = workspaces[activeWs]?.id;
      const next = reorderWorkspace(workspaces, srcId, drop.destGroupId, drop.anchorId, drop.before);
      workspaces.splice(0, workspaces.length, ...next); // 배열 identity 유지(코드베이스가 splice로 변형)
      activeWs = Math.max(0, workspaces.findIndex((w) => w.id === actId));
      render(); // saveLayout 직접 호출 금지 — render가 부른다(멤버0 그룹 해체도 normalizeGroups가)
    }
  };
  window.addEventListener("mousemove", move, true);
  window.addEventListener("mouseup", up, true);
}

// 그룹 섹션 드래그: groups 배열 순서 변경. pinned/unpinned tier 분리는 reorderGroup이 클램프.
function startGroupDrag(e0: MouseEvent, srcId: number) {
  const start = { x: e0.clientX, y: e0.clientY };
  let dragging = false;
  let line: HTMLElement | null = null;
  let drop: { anchorId: number; before: boolean } | null = null;

  const move = (e: MouseEvent) => {
    if (!dragging) {
      if (Math.abs(e.clientX - start.x) + Math.abs(e.clientY - start.y) < 4) return;
      dragging = true;
      document.querySelector(`#ws-tabs .ws-group[data-group-id="${srcId}"]`)?.classList.add("ws-dragging");
      line = makeDropLine();
      document.body.classList.add("ws-reordering");
    }
    const el = document.elementFromPoint(e.clientX, e.clientY) as HTMLElement | null;
    const head = el?.closest<HTMLElement>(".ws-group-head");
    drop = null;
    const sec = head?.closest<HTMLElement>(".ws-group[data-group-id]");
    if (head && sec && Number(sec.dataset.groupId) !== srcId) {
      const r = head.getBoundingClientRect();
      const before = e.clientY < r.top + r.height / 2;
      drop = { anchorId: Number(sec.dataset.groupId), before };
      placeDropLine(line!, r.left, before ? r.top : r.bottom, r.width);
    } else {
      line!.hidden = true;
    }
  };
  const up = () => {
    window.removeEventListener("mousemove", move, true);
    window.removeEventListener("mouseup", up, true);
    line?.remove();
    document.body.classList.remove("ws-reordering");
    document.querySelector(`#ws-tabs .ws-group[data-group-id="${srcId}"]`)?.classList.remove("ws-dragging");
    if (dragging) suppressNextClick();
    if (dragging && drop) {
      groups = reorderGroup(groups, srcId, drop.anchorId, drop.before);
      render(); // 그룹 순서만 바뀌므로 activeWs 재계산 불요
    }
  };
  window.addEventListener("mousemove", move, true);
  window.addEventListener("mouseup", up, true);
}

// ---------- 정렬: 역할(role) 기반 고정 배치 ----------
// 현재 워크스페이스의 살아있는 surface를 역할별 표준 자리로 재배치한다:
//   · 왼쪽 끝 컬럼  = master(위) / cso(아래), 세로 3:1
//   · 가운데        = worker·미분류 surface를 같은 폭 컬럼으로 균등 분배(좌→우 순서 보존)
//   · 오른쪽 끝 컬럼 = reviewer-gemini(agy, 위) / reviewer-codex(codex, 아래), 세로 1:1
// 트리 위상만 새로 짜고 attachDividerDrag는 건드리지 않으므로 수동 크기 조절은 그대로 보존된다
// (정렬 후에도 divider를 다시 끌 수 있다 — 현재 크기만 표준 배치로 리셋될 뿐이다).
// divider 1px·pane 헤더 등으로 컬럼 폭엔 셀 1칸 이내 잔차가 있을 수 있다.
function evenComb(nodes: Node[], dir: "row" | "col"): Node {
  let acc = nodes[nodes.length - 1];
  for (let i = nodes.length - 2; i >= 0; i--) {
    acc = { type: "split", dir, ratio: 1 / (nodes.length - i), a: nodes[i], b: acc };
  }
  return acc;
}

function firstWithRole(sids: number[], roleOf: Map<number, string | null>, role: string): number | null {
  for (const sid of sids) if (roleOf.get(sid) === role) return sid;
  return null;
}

function roleLayout(sids: number[], roleOf: Map<number, string | null>): Node {
  const master = firstWithRole(sids, roleOf, "master");
  const cso = firstWithRole(sids, roleOf, "cso");
  const agy = firstWithRole(sids, roleOf, "reviewer-gemini"); // 안티그래피티
  const codex = firstWithRole(sids, roleOf, "reviewer-codex");
  const corners = new Set([master, cso, agy, codex].filter((x): x is number => x != null));
  const middle = sids.filter((sid) => !corners.has(sid)); // worker·미분류 전부 가운데
  const pane = (sid: number): Node => ({ type: "pane", sid });

  const columns: Node[] = [];
  // 왼쪽 끝: master(위) / cso(아래) = 3:1 (누락 시 있는 쪽이 컬럼 전체)
  if (master != null && cso != null) columns.push({ type: "split", dir: "col", ratio: 3 / 4, a: pane(master), b: pane(cso) });
  else if (master != null) columns.push(pane(master));
  else if (cso != null) columns.push(pane(cso));
  // 가운데: worker·미분류 균등 컬럼
  for (const sid of middle) columns.push(pane(sid));
  // 오른쪽 끝: agy(위) / codex(아래) = 1:1 (누락 시 있는 쪽이 컬럼 전체)
  if (agy != null && codex != null) columns.push({ type: "split", dir: "col", ratio: 1 / 2, a: pane(agy), b: pane(codex) });
  else if (agy != null) columns.push(pane(agy));
  else if (codex != null) columns.push(pane(codex));

  return evenComb(columns, "row"); // 컬럼들을 같은 폭으로 가로 배치
}

async function actionEqualize() {
  const ws = current();
  if (!ws?.tree) return;
  const live = collectSids(ws.tree).filter((sid) => panes.has(paneKey(sid, ws.socket))); // 죽은/placeholder 노드 제외 (F4 복합키)
  if (live.length < 2) return; // 0~1개는 정렬할 대상이 없음
  // 역할은 데몬 surface.list에서 조회 (UI 생성 pane은 role=null → 가운데로)
  const roleOf = new Map<number, string | null>();
  try {
    const r = (await invoke("list_surfaces", { socket: ws.socket })) as { surfaces: { surface_id: number; role: string | null }[] };
    for (const s of r.surfaces) roleOf.set(s.surface_id, s.role);
  } catch {
    /* 데몬 일시 미응답: role 없이 진행 → 전부 가운데 균등 */
  }
  ws.tree = roleLayout(live, roleOf);
  render(); // 새 트리로 DOM 재구성 + fitPane→resize_surface + saveLayout
}

// ---------- workspace tabs ----------

// org.status를 워크스페이스별 socket마다 1콜 조회해 노드 신호 맵에 캐싱한다(B3).
// 응답 키: 노드배열=surfaces, 대기수=중첩 feed.pending (top-level pending 아님).
async function refreshSidebarStatus() {
  const sockets = new Set(workspaces.map((w) => w.socket));
  let pend = 0;
  // 사라진 소켓(워크스페이스 폐기·부서 삭제)의 잔재를 남기지 않는다 — 남기면 배너가 존재하지
  // 않는 부서의 대기를 영원히 보고한다.
  for (const key of [...pendingBySocket.keys()])
    if (![...sockets].some((s) => (s ?? DEFAULT_SOCKET_KEY) === key)) pendingBySocket.delete(key);
  for (const sock of sockets) {
    try {
      const r = (await invoke("org_status", { socket: sock })) as {
        surfaces?: any[];
        feed?: { pending?: number };
      };
      pend += r.feed?.pending ?? 0;
      // 성공 조회만 기록한다. 실패(catch)는 **덮어쓰지 않는다** — 직전 성공값을 유지하는 편이
      // 0으로 접는 것보다 낫다(일시 미응답으로 배너가 사라지지 않는다). 그 대신 배지 합계
      // pend 는 종전대로 그 소켓을 0으로 세므로, 두 값이 잠시 어긋날 수 있다(배지=보수적 하한).
      pendingBySocket.set(sock ?? DEFAULT_SOCKET_KEY, r.feed?.pending ?? 0);
      for (const n of r.surfaces ?? [])
        nodeSig.set(`${sock}#${n.surface_id}`, {
          role: n.role,
          state: n.status?.state ?? (n.idle_secs > 60 ? "idle" : "working"),
          ctx_pct: n.status?.context_pct ?? n.usage?.ctx_pct ?? null,
          idle_secs: n.idle_secs,
          agent_alive: n.agent_alive,
        });
    } catch {
      /* 부서 데몬 일시 부재 */
    }
  }
  pendingApprovals = pend;
  updatePendingBadges(pend); // CC 버튼·승인 Feed 탭 배지 동기
  renderWsTabs(); // 신호 반영 재렌더
}

// 승인 대기 건수 배지 — 상단 Control Center 버튼 + 편입된 '승인 Feed' 탭 둘 다 갱신.
function updatePendingBadges(n: number) {
  for (const id of ["cc-pending-badge", "cc-feed-tabbadge"]) {
    const b = document.getElementById(id);
    if (!b) continue;
    b.hidden = n === 0;
    b.textContent = String(n);
  }
}

// ws별 고유색 (id 기반 — 세션 복원에도 같은 ws는 같은 색)
const WS_COLORS = ["#2f81f7", "#3fb950", "#d29922", "#f85149", "#a371f7", "#db61a2", "#39c5cf", "#e3b341"];

function renderWsTabs() {
  const bar = document.getElementById("ws-tabs")!;
  bar.innerHTML = "";
  // 06: 2계층 tier 정렬 — pinned 그룹 → unpinned 그룹 → ungrouped ws(배열 순서). 시각 순서≠배열 순서이므로
  // 탭 핸들러는 캡처 idx 대신 workspaces.indexOf(ws)로 활성 비교/전환(stale idx 회피, close 핸들러 패턴 일치).
  // 06: 멤버0 그룹은 렌더에서 제외(유령 헤더 차단 · 적대검증 교정 — saveLayout이 모듈 상태도 청소).
  const hasMembers = (g: GroupMeta) => workspaces.some((w) => !w.pending && w.groupId === g.id);
  const pinnedG = groups.filter((g) => g.pinned && hasMembers(g));
  const unpinnedG = groups.filter((g) => !g.pinned && hasMembers(g));
  for (const g of [...pinnedG, ...unpinnedG]) bar.appendChild(buildGroupSection(g));
  for (const ws of workspaces.filter((w) => !w.pending && w.groupId == null)) bar.appendChild(buildTab(ws));
}

// 06: ws 1행 탭 DOM 생성(기존 renderWsTabs forEach 본문을 외과적으로 추출 — idx→workspaces.indexOf(ws)만 치환).
function buildTab(ws: Workspace): HTMLElement {
  const color = WS_COLORS[ws.id % WS_COLORS.length];
  const tab = document.createElement("div");
  tab.className = "ws-tab" + (workspaces.indexOf(ws) === activeWs ? " active" : "");
  tab.dataset.wsId = String(ws.id); // 드래그 히트테스트용(startWsDrag)
  tab.style.borderLeftColor = color; // ws 고유색은 좌측 바 (사이드바 항목 식별)
  const titleRow = document.createElement("div");
  titleRow.className = "ws-title-row";
  const label = document.createElement("span");
  label.className = "ws-name";
  label.textContent = deptPlaceholderLabel(ws); // WP-10: pending이면 "부서 제작 중…" (멈춘 줄 오해 방지)
  const close = document.createElement("span");
  close.className = "ws-close";
  close.textContent = "×";
  close.title = "완전 삭제 — 클릭하면 확인 창이 열립니다 (부서면 데몬 종료·부활 차단 포함)";
  titleRow.append(label, close);
  // WP-10: 부서 준비 중 탭엔 스피너 글리프를 라벨 앞에 붙이고 aria-busy 로 진행을 알린다(CSS가 회전·정지 담당).
  if (ws.pending) {
    const spin = document.createElement("span");
    spin.className = "ws-tab-spinner";
    spin.setAttribute("aria-hidden", "true");
    titleRow.prepend(spin);
    tab.setAttribute("aria-busy", "true");
  }
  // 승인 대기 배지(B3): 중복 표시 방지 위해 활성 ws 행에만 1개 노출.
  if (pendingApprovals > 0 && workspaces.indexOf(ws) === activeWs) {
    const badge = document.createElement("span");
    badge.className = "ws-approve-badge";
    badge.textContent = `⚠${pendingApprovals}`;
    titleRow.append(badge);
  }
  // 서브라인: pane 수 + 대표 pane 제목 (항목 가독성)
  const sids = collectSids(ws.tree);
  const firstTitle =
    panes.get(paneKey(sids[0] ?? -1, ws.socket))?.titleEl.textContent ?? "";
  const sub = document.createElement("span");
  sub.className = "ws-sub";
  if (ws.pending) {
    sub.textContent = "부서 데몬 시작 중…";
    sub.classList.add("ws-sub-pending");
  } else {
    // 노드 신호 집계(B3): 상태 dot + worst CTX% + idle + dead 카운트. pane 수·title 표시는 보존.
    const sigs = sids
      .map((id) => nodeSig.get(`${ws.socket}#${id}`))
      .filter(Boolean) as NodeSig[];
    const worst = sigs.reduce((acc, s) => Math.max(acc, s.ctx_pct ?? 0), 0);
    const idleN = sigs.filter((s) => s.state === "idle" || s.idle_secs > 60).length;
    const dead = sigs.filter((s) => s.agent_alive === false).length;
    const dot = document.createElement("span");
    dot.className = "ws-dot " + (dead ? "error" : idleN ? "idle" : "working");
    sub.appendChild(dot);
    const txt = document.createElement("span");
    const bits = [`${sids.length} pane`];
    if (firstTitle) bits.push(firstTitle);
    if (worst >= 60) bits.push(`CTX ${worst}%`);
    if (idleN) bits.push(`💤${idleN}`);
    if (dead) bits.push(`❌${dead}`);
    txt.textContent = bits.join(" · ");
    if (worst >= 80) txt.className = "sev-crit";
    else if (worst >= 60) txt.className = "sev-warn";
    sub.appendChild(txt);
  }
  tab.append(titleRow, sub);
  tab.addEventListener("mousedown", (e) => {
    // 우클릭은 전환하지 않음 — render()가 탭 DOM을 재생성하면 컨텍스트 메뉴가 죽은 엘리먼트를 잡는다
    if (e.button !== 0 || e.target === close) return;
    if ((e.target as HTMLElement)?.isContentEditable) return; // rename 편집 중엔 전환·드래그 금지
    const i = workspaces.indexOf(ws); // 그룹 재배열로 시각 순서≠배열 순서 — 실시간 위치로 전환
    if (i !== activeWs) {
      activeWs = i;
      render();
      const first = collectSids(current().tree)[0];
      if (first != null) setFocus(first);
    }
    startWsDrag(e, ws.id); // 4px 임계 초과 시에만 재정렬 드래그(단순 클릭은 위 전환만)
  });
  const startRename = () => {
    // WKWebView에서 prompt()는 무동작 — 인라인 편집
    label.contentEditable = "true";
    label.focus();
    const sel = window.getSelection();
    sel?.selectAllChildren(label);
    const onKey = (ke: KeyboardEvent) => {
      if (ke.key === "Enter") {
        ke.preventDefault();
        label.blur();
      }
    };
    const commit = () => {
      label.removeEventListener("keydown", onKey); // rename마다 리스너 누적 방지
      label.contentEditable = "false";
      const name = (label.textContent || "").trim();
      ws.name = name || UNTITLED; // 이름을 지우면 미정 표시로 복귀
      render();
    };
    label.addEventListener("blur", commit, { once: true });
    label.addEventListener("keydown", onKey);
  };
  label.addEventListener("dblclick", startRename);
  tab.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    showCtxMenu(e.clientX, e.clientY, [
      { label: "이름 변경", action: startRename },
      ...wsGroupCtxItems(ws), // 06: 그룹 만들기/넣기/빼기
      // ★기능2: 부서 탭에만 — 대화기억까지 격리하고 부활을 영구 차단하는 완전 삭제(기존 2-click close 무접촉).
      ...(ws.socket ? [{ label: "완전 삭제(부활 차단)", action: () => purgeDept(ws) }] : []),
    ]);
  });
  close.addEventListener("click", async () => {
    // ★완전 삭제 확인(오너 2026-07-15 — 발견 불가 UX 수리): 숨은 2-click 무장 패턴을 설명형
    // 확인 다이얼로그로 교체(WKWebView confirm() 무동작 → 기존 confirmModal 재사용). 초보자가
    // "무엇이 어떻게 삭제되는지" 읽고 결정한다. pane 개별 ×(저위험)는 종전 2-click 유지.
    const wsName = ws.name || UNTITLED;
    const ok = await confirmModal(
      ws.socket ? `부서 "${wsName}" 완전 삭제` : `워크스페이스 "${wsName}" 완전 삭제`,
      (ws.socket
        ? "이 부서의 pane(에이전트 세션)이 전부 종료되고 부서 데몬도 종료됩니다. 삭제 의도가 기록되어 " +
          "앱을 재시작해도 부활하지 않습니다."
        : "이 워크스페이스의 pane(에이전트 세션)이 전부 종료되고 탭이 제거됩니다.") +
        "\n\n완전히 삭제하시겠습니까?",
      "삭제",
    );
    if (!ok) return;
    // ★WP-3 의도 선기록(제1행위): teardown 이전에 base 데몬에 dept 묘비 기록 — 이후 체인이
    // 무음 실패해도 재시작 부활을 차단한다. 실패=가시화(같은 탭 재삭제가 재시도 — 무음 삼킴 금지).
    if (ws.socket) {
      try {
        await invoke("dept_tombstone_by_socket", { socket: ws.socket });
      } catch (e) {
        toast("watchdog", "부서 삭제 의도 기록 실패", `${e} — 삭제는 계속 진행되나 재시작 시 부활할 수 있습니다. 같은 탭을 다시 삭제하면 재시도됩니다.`);
      }
    }
    for (const sid of collectSids(ws.tree)) {
      // pane 개별 close 실패는 관용(묘비가 이미 부활 차단 — per-pane 토스트는 스팸).
      await invoke("close_surface", { socket: ws.socket, surfaceId: sid }).catch(() => {});
      destroyPaneRuntime(sid, ws.socket);
    }
    const i = workspaces.indexOf(ws); // 캡처된 idx는 stale일 수 있음 — 실시간 위치로 식별
    if (i < 0) { render(); return; } // 이미 제거된 ws 재클릭 — no-op
    workspaces.splice(i, 1);
    // 부서 데몬 teardown은 '그 socket을 쓰는 마지막 탭'일 때만(중복 탭 잔존 시 다른 탭 보호)
    const stillUsed = ws.socket && workspaces.some((w) => w.socket === ws.socket);
    // socket 기준 teardown(order 8) — ws rename으로 name↔socket이 끊겨도 정확히 종료.
    // ★WP-3: 실패 가시화(.catch 삼킴 제거) — 묘비가 부활을 차단하므로 잔존 데몬은 '정리 대기'일 뿐이며
    // 차회 부팅 reaper가 수렴하지만, 사용자에게는 알린다.
    if (ws.socket && !stillUsed)
      await invoke("stop_dept_daemon_by_socket", { socket: ws.socket }).catch((e) =>
        toast("watchdog", "부서 데몬 종료 실패", `${e} — 부활은 차단됨(삭제 의도 기록됨)·다음 앱 시작 시 자동 정리를 재시도합니다.`),
      );
    if (workspaces.length === 0) {
      await addWorkspace(); // addWorkspace가 activeWs를 설정
    } else {
      if (i < activeWs) activeWs -= 1; // 활성보다 앞 탭을 닫으면 인덱스가 한 칸 당겨진다
      activeWs = Math.min(activeWs, workspaces.length - 1);
    }
    render();
  });
  return tab;
}

// 06: 그룹 섹션 = 헤더(chevron collapse·name·count·hover add) + body(collapsed면 멤버 DOM 미생성=성능 가드).
function buildGroupSection(g: GroupMeta): HTMLElement {
  const sec = document.createElement("div");
  sec.className = "ws-group" + (g.collapsed ? " collapsed" : "");
  sec.dataset.groupId = String(g.id); // 드래그 히트테스트용(startWsDrag·startGroupDrag)

  const head = document.createElement("div");
  head.className = "ws-group-head" + (g.pinned ? " pinned" : "");
  head.style.borderLeftColor = g.color || WS_COLORS[g.id % WS_COLORS.length];

  const chevron = document.createElement("span");
  chevron.className = "ws-group-chevron";
  chevron.textContent = g.collapsed ? "▸" : "▾";
  chevron.addEventListener("click", (e) => {
    e.stopPropagation();
    g.collapsed = !g.collapsed;
    render();
  });

  const name = document.createElement("span");
  name.className = "ws-group-name";
  name.textContent = g.name;
  // 헤더 이름 클릭 = anchor focus(부서 그룹) / 첫 멤버 focus(일반 그룹)
  name.addEventListener("click", () => {
    const anchor = anchorWsOf(g) ?? workspaces.find((w) => w.groupId === g.id);
    if (anchor) {
      activeWs = workspaces.indexOf(anchor);
      render();
      const first = collectSids(anchor.tree)[0];
      if (first != null) setFocus(first);
    }
  });

  const count = document.createElement("span");
  count.className = "ws-group-count";
  count.textContent = String(workspaces.filter((w) => !w.pending && w.groupId === g.id).length);

  const add = document.createElement("span"); // hover '+' = 이 그룹에 새 ws
  add.className = "ws-group-add";
  add.textContent = "+";
  add.title = "그룹에 워크스페이스 추가";
  add.addEventListener("click", async (e) => {
    e.stopPropagation();
    const ws = await addWorkspace();
    ws.groupId = g.id;
    render();
  });

  head.append(chevron, name, count, add);
  head.addEventListener("mousedown", (e) => {
    if (e.button !== 0) return;
    const t = e.target as HTMLElement;
    // 접기(chevron)·추가(+)·이름편집(rename 중)은 클릭 동작 보존 — 드래그 시작 금지
    if (t === chevron || t === add || t?.isContentEditable) return;
    startGroupDrag(e, g.id); // 4px 임계 초과 시에만 그룹 순서 드래그(단순 클릭은 name focus 보존)
  });
  head.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    showCtxMenu(e.clientX, e.clientY, groupCtxItems(g));
  });
  sec.appendChild(head);

  if (!g.collapsed) {
    const body = document.createElement("div");
    body.className = "ws-group-body";
    for (const ws of workspaces.filter((w) => !w.pending && w.groupId === g.id)) {
      const tab = buildTab(ws);
      tab.classList.add("in-group");
      body.appendChild(tab);
    }
    sec.appendChild(body);
  }
  return sec;
}

// 06: ws 우클릭 — 그룹 만들기/넣기/빼기. 모두 끝에 render() 1회(saveLayout 직접호출 금지 — render가 부른다).
function wsGroupCtxItems(ws: Workspace): { label: string; action: () => void }[] {
  const items: { label: string; action: () => void }[] = [];
  if (ws.groupId == null) {
    items.push({
      label: "새 그룹으로 묶기",
      action: () => {
        const g: GroupMeta = { id: groupCounter++, name: ws.name || "그룹", collapsed: false, pinned: false };
        groups.push(g);
        ws.groupId = g.id;
        render();
      },
    });
    for (const g of groups) {
      items.push({
        label: `“${g.name}” 그룹에 넣기`,
        action: () => {
          ws.groupId = g.id;
          render();
        },
      });
    }
  } else {
    items.push({
      label: "그룹에서 빼기",
      action: () => {
        ws.groupId = undefined;
        render(); // normalizeGroups가 멤버0 그룹 자동 제거
      },
    });
  }
  return items;
}

// 06: 그룹 헤더 우클릭 — 이름 변경/고정/해제(Ungroup)/삭제(Delete).
function groupCtxItems(g: GroupMeta): { label: string; action: () => void }[] {
  return [
    { label: "그룹 이름 변경", action: () => startGroupRename(g) },
    {
      label: g.pinned ? "고정 해제" : "맨 위 고정",
      action: () => {
        g.pinned = !g.pinned;
        render();
      },
    },
    {
      label: "그룹 해제(워크스페이스 보존)", // Ungroup — 멤버 ws는 ungrouped로 잔존
      action: () => {
        for (const w of workspaces) if (w.groupId === g.id) w.groupId = undefined;
        render(); // normalizeGroups가 멤버0 그룹 자동 제거
      },
    },
    { label: "그룹 삭제(워크스페이스 전부 닫기)", action: () => confirmDeleteGroup(g) }, // Delete(파괴적)
  ];
}

// 06: 그룹 이름 인라인 변경 — ws startRename의 contentEditable 패턴 차용(WKWebView prompt() 무동작 우회).
// 현재 렌더된 헤더의 .ws-group-name 엘리먼트를 그룹 색인으로 찾아 편집 진입.
function startGroupRename(g: GroupMeta) {
  const heads = Array.from(document.querySelectorAll<HTMLElement>("#ws-tabs .ws-group-head"));
  const renderedG = [...groups.filter((x) => x.pinned), ...groups.filter((x) => !x.pinned)];
  const idx = renderedG.indexOf(g);
  const label = idx >= 0 ? heads[idx]?.querySelector<HTMLElement>(".ws-group-name") : null;
  if (!label) return;
  label.contentEditable = "true";
  label.focus();
  const sel = window.getSelection();
  sel?.selectAllChildren(label);
  const onKey = (ke: KeyboardEvent) => {
    if (ke.key === "Enter") {
      ke.preventDefault();
      label.blur();
    }
  };
  const commit = () => {
    label.removeEventListener("keydown", onKey); // rename마다 리스너 누적 방지
    label.contentEditable = "false";
    const name = (label.textContent || "").trim();
    g.name = name || "그룹"; // 이름을 지우면 기본명으로 복귀
    render();
  };
  label.addEventListener("blur", commit, { once: true });
  label.addEventListener("keydown", onKey);
}

// 06: 그룹 삭제(파괴적) — WKWebView confirm() 무동작이라 2-click 확인 패턴(ws close 차용).
// 멤버 ws 각각에 기존 close 로직(close_surface + 부서면 stop_dept_daemon_by_socket) 재사용 → 부서 teardown 정합 유지.
let groupDeleteArm: number | null = null;
async function confirmDeleteGroup(g: GroupMeta) {
  if (groupDeleteArm !== g.id) {
    groupDeleteArm = g.id;
    setTimeout(() => {
      if (groupDeleteArm === g.id) groupDeleteArm = null;
    }, 2500);
    // 재실행 안내 — 그룹 메뉴를 다시 띄워 '정말 삭제' 항목을 노출.
    const m = document.getElementById("ctx-menu");
    const r = m?.getBoundingClientRect();
    showCtxMenu(r?.left ?? 0, r?.top ?? 0, [
      {
        label: "정말 삭제(워크스페이스 전부 닫기)",
        action: () => confirmDeleteGroup(g),
      },
    ]);
    return;
  }
  groupDeleteArm = null;
  const members = workspaces.filter((w) => w.groupId === g.id);
  for (const ws of members) {
    for (const sid of collectSids(ws.tree)) {
      await invoke("close_surface", { socket: ws.socket, surfaceId: sid }).catch(() => {});
      destroyPaneRuntime(sid, ws.socket);
    }
    const i = workspaces.indexOf(ws);
    if (i < 0) continue;
    workspaces.splice(i, 1);
    // 부서 데몬 teardown은 '그 socket을 쓰는 마지막 탭'일 때만(close 핸들러와 동일 정합).
    const stillUsed = ws.socket && workspaces.some((w) => w.socket === ws.socket);
    if (ws.socket && !stillUsed) await invoke("stop_dept_daemon_by_socket", { socket: ws.socket }).catch(() => {});
    if (i < activeWs) activeWs -= 1; // 활성보다 앞 탭을 닫으면 인덱스가 한 칸 당겨진다
  }
  if (workspaces.length === 0) {
    await addWorkspace(); // addWorkspace가 activeWs를 설정
  } else {
    activeWs = Math.min(activeWs, workspaces.length - 1);
  }
  render(); // normalizeGroups가 멤버0이 된 그룹 g를 자동 제거
}

// ws는 번호가 아니라 이름으로 구분 — 이름이 정해지지 않으면 "non title" 표시.
const UNTITLED = "non title";

// 커스텀 컨텍스트 메뉴 (WKWebView 기본 메뉴 대체) — 싱글톤, 바깥 클릭·Esc로 닫힘.
function showCtxMenu(
  x: number,
  y: number,
  items: { label: string; action: () => void; disabled?: boolean }[],
) {
  document.getElementById("ctx-menu")?.remove();
  const menu = document.createElement("div");
  menu.id = "ctx-menu";
  const closeMenu = () => {
    menu.remove();
    window.removeEventListener("mousedown", dismiss, true);
    window.removeEventListener("keydown", onKey, true);
  };
  const dismiss = (e?: Event) => {
    if (e instanceof MouseEvent && menu.contains(e.target as globalThis.Node)) return;
    closeMenu();
  };
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape") dismiss();
  };
  for (const it of items) {
    const row = document.createElement("div");
    // disabled = 표시 전용 행(경로 표시 등) — 클릭 무반응(CSS pointer-events 차단과 짝)
    row.className = "ctx-item" + (it.disabled ? " disabled" : "");
    row.textContent = it.label;
    if (!it.disabled)
      row.addEventListener("mousedown", (e) => {
        e.preventDefault();
        closeMenu();
        it.action();
      });
    menu.appendChild(row);
  }
  menu.style.left = `${x}px`;
  menu.style.top = `${y}px`;
  document.body.appendChild(menu);
  // 화면 밖으로 나가면 안쪽으로 보정
  const r = menu.getBoundingClientRect();
  if (r.right > window.innerWidth) menu.style.left = `${window.innerWidth - r.width - 4}px`;
  if (r.bottom > window.innerHeight) menu.style.top = `${window.innerHeight - r.height - 4}px`;
  window.addEventListener("mousedown", dismiss, true);
  window.addEventListener("keydown", onKey, true);
}

// 배경 테마 팝오버 — 컬러피커 + 기본값 복원. showCtxMenu의 바깥클릭·Esc 닫기 패턴 재사용.
// 컬러피커 input 이벤트마다 applyBgColor 라이브 적용(localStorage 영속은 applyBgColor 내부).
function openThemePopover(anchor: HTMLElement) {
  document.getElementById("theme-pop")?.remove();
  const pop = document.createElement("div");
  pop.id = "theme-pop";
  const close = () => {
    pop.remove();
    window.removeEventListener("mousedown", dismiss, true);
    window.removeEventListener("keydown", onKey, true);
  };
  const dismiss = (e?: Event) => {
    if (e instanceof MouseEvent && pop.contains(e.target as globalThis.Node)) return;
    close();
  };
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape") close();
  };

  const row = document.createElement("label");
  row.className = "theme-pop-row";
  row.textContent = "배경색";
  const picker = document.createElement("input");
  picker.type = "color";
  picker.value = currentBg();
  picker.addEventListener("input", () => applyBgColor(picker.value));
  row.appendChild(picker);

  // 폰트 선택(오너 요청 2026-07-12) — 선택지=appearance.ts FONT_CHOICES, 변경 즉시 전 pane 적용.
  const fontRow = document.createElement("label");
  fontRow.className = "theme-pop-row";
  fontRow.textContent = "폰트";
  const fontSel = document.createElement("select");
  for (const c of FONT_CHOICES) {
    const o = document.createElement("option");
    o.value = c.face ?? "";
    o.textContent = c.label;
    fontSel.appendChild(o);
  }
  fontSel.value = FONT_CHOICES.some((c) => c.face === fontFace) ? (fontFace ?? "") : "";
  fontSel.addEventListener("change", () => applyFontFace(fontSel.value || null));
  fontRow.appendChild(fontSel);

  const reset = document.createElement("button");
  reset.className = "theme-pop-reset";
  reset.textContent = "기본값 복원";
  reset.addEventListener("click", () => {
    applyBgColor(null);
    picker.value = DEFAULT_BG;
    applyFontFace(null);
    fontSel.value = "";
  });

  pop.append(row, fontRow, reset);

  // 앵커(테마 버튼) 하단에 배치 후 화면 밖으로 나가면 안쪽으로 보정.
  const r = anchor.getBoundingClientRect();
  pop.style.left = `${r.left}px`;
  pop.style.top = `${r.bottom + 4}px`;
  document.body.appendChild(pop);
  const pr = pop.getBoundingClientRect();
  if (pr.right > window.innerWidth) pop.style.left = `${window.innerWidth - pr.width - 4}px`;

  window.addEventListener("mousedown", dismiss, true);
  window.addEventListener("keydown", onKey, true);
}

/// ⚠ 내부 복구 경로(마지막 탭 닫힘·purge 후 빈 목록)도 이 함수를 부른다 — 여기서 던지거나
/// 막으면 그 복구가 깨진다. 리셋 가드는 **사용자 진입점**(btn-ws-new·팔레트)에만 배선한다(P1-3).
async function addWorkspace(): Promise<Workspace> {
  const sid = await newSurface();
  const ws: Workspace = { id: wsCounter++, name: UNTITLED, tree: { type: "pane", sid } };
  workspaces.push(ws);
  activeWs = workspaces.length - 1;
  render();
  setFocus(sid);
  return ws;
}

// 부서 socket 경로에서 원래 부서명 역산 — unix(~/.local/state/cys-dept-<name>/cys.sock)와
// ★Windows named pipe(\\.\pipe\cys-dept-<name> — RC-4 규약·dept_socket_path 정합) 양쪽 지원(2026-07-10).
// rename으로 ws.name이 바뀌어도 socket은 불변이므로, 재-launch가 '다른 소켓 새 데몬'을 만들어
// 원래 데몬을 고아화하는 것을 막는다(시나리오4). Windows 분기 이전엔 null→ws.name 폴백으로 이 가드가
// Windows에서 무동작(rename 후 재-launch가 고아 유발)이었다 — 분기 추가로 가드가 비로소 작동한다.
function deptNameFromSocket(sock: string | undefined): string | null {
  const m = /\/cys-dept-(.+?)\/cys\.sock$/.exec(sock ?? "");
  if (m) return m[1];
  const w = /^\\\\\.\\pipe\\cys-dept-(.+)$/.exec(sock ?? "");
  return w ? w[1] : null;
}

// 멀티마스터 F4: 새 '부서 workspace' 런칭 = 새 부서 데몬 spawn(cys-dept launch 단일 진입점).
// 첫 부서가 생기면 백엔드(cys-dept)가 기본 데몬을 CEO로 자동 승격한다.
// ① 표시 지연(안 C): 무거운 launch await(최대 ~12s) '전에' placeholder 탭을 즉시 render — 체감 지연 0.
// ② 고아 방지(안 A): 빈 newSurface를 만들지 않는다. cys-dept가 띄우는 role=master surface가
//    refreshPaneTitles 자동입양으로 '첫 pane'이 되게 한다(빈 셸 미생성 → 고아 0).
async function addDeptWorkspace(catalogKey?: string): Promise<Workspace> {
  // ★A4(성찰 확정): 이 경로는 **새 부서 cysd 를 spawn** 한다(allocate_dept_daemon→cys-dept launch) —
  // 리셋 진행/완료 중이면 격리 게이트를 Err 로 만들어 리셋을 반토막 내거나, 격리로 옮겨지는
  // ~/.cys·state 밑에 레지스트리를 재생성한다. 그래서 **모든 호출부가 daemonActionBlocked()로
  // 먼저 막는다**(여기서 throw 하지 않는 이유: 팔레트 경로가 미처리 rejection이 된다).
  // 신규 호출부를 추가하면 그 진입점에도 같은 가드를 반드시 배선하라.
  // 클릭 즉시 placeholder 탭(tree:null·socket 미정) push+render — launch await 동안 시각 피드백 제공.
  // 번호는 백엔드 allocate(레지스트리 flock RMW)가 확정하므로 placeholder name은 미정("…")으로 두고
  // 반환 info.name으로 확정한다(UI 번호 계산 폐기 → lowest-unused 재사용·멀티창 충돌0).
  const ws: Workspace = { id: wsCounter++, name: "…", tree: null, pending: true };
  workspaces.push(ws);
  activeWs = workspaces.length - 1;
  render();
  try {
    const info = (await invoke("allocate_dept_daemon", { catalogKey })) as {
      socket: string;
      socket_slug?: string;
      name: string;
      display_name?: string;
    };
    ws.name = info.display_name ?? info.name; // ★표시명(create 카탈로그) 또는 부서 번호(레거시)
    if (info.socket_slug && info.socket) socketForSlug.set(info.socket_slug, info.socket);
    // 멱등 합류 — 같은 부서 socket의 (이 placeholder가 아닌) 탭이 이미 있으면(연타·재호출이 같은 데몬을
    // 멱등 반환) placeholder를 폐기하고 기존 탭을 활성화한다. w !== ws 가드로 자기 자신과 오매칭 방지.
    const dup = workspaces.find((w) => w !== ws && w.socket && w.socket === info.socket);
    // placeholder가 launch await 중 탭 ×로 닫혔으면: 같은 소켓을 쓰는 다른 탭(dup)이 없을 때
    // 방금 spawn된 부서 데몬을 회수해 무탭 headless 누수를 막는다(close 핸들러는 socket 미정이라 미회수).
    if (workspaces.indexOf(ws) < 0) {
      if (!dup && info.socket) await invoke("stop_dept_daemon_by_socket", { socket: info.socket }).catch(() => {});
      return dup ?? ws;
    }
    if (dup) {
      const pi = workspaces.indexOf(ws);
      if (pi >= 0) workspaces.splice(pi, 1); // indexOf -1 시 splice(-1,1)이 엉뚱한 ws 제거하는 것 방지
      activeWs = Math.max(0, workspaces.indexOf(dup));
      render();
      const firstSid = collectSids(dup.tree)[0];
      if (firstSid != null) setFocus(firstSid);
      return dup;
    }
    // 안 A(C4 더블 surface 해소): cys-dept(create·allocate 모두 role=master '빈 셸' — WP-11 일원화)가 부서장
    // role=master surface를 띄우므로 UI는 plain 셸을 직접 만들지 않는다. socket 확정 + pending 해제 → refreshPaneTitles
    // 자동입양이 그 master(빈 셸)를 '첫 pane'으로 채운다(rolePri master=0 → 좌측·focus). 별도 UI 셸 0·더블 surface 0.
    // 탭이 await 중 닫혀도(close 핸들러가 socket 기준 데몬 teardown) 좀비 없음 — 별도 plain-셸 회수 불필요.
    ws.socket = info.socket;
    ws.pending = false;
    render();
    await refreshPaneTitles(); // 방금 띄운 master surface를 즉시 입양(3초 인터벌 대기 없이). 부팅 실패 시
    //                            tree:null로 남고 master 등장 시 인터벌이 재입양(start()의 비활성 부서 처리와 정합).
    return ws;
  } catch (e) {
    // 실패 시 placeholder 롤백 — 유령 탭이 남지 않게 제거.
    const i = workspaces.indexOf(ws);
    if (i >= 0) workspaces.splice(i, 1);
    if (activeWs >= workspaces.length) activeWs = Math.max(0, workspaces.length - 1);
    // newSurface가 데몬 spawn 후 실패하면 등록된 부서 데몬이 무탭 고아로 남는다 — socket 확정됐으면 회수.
    if (ws.socket) await invoke("stop_dept_daemon_by_socket", { socket: ws.socket }).catch(() => {});
    render();
    throw e;
  }
}

// ---------- actions ----------

async function newSurface(cwd: string | null = null, socket?: string): Promise<number> {
  const r = (await invoke("create_surface", { socket, cwd, title: null, rows: 35, cols: 120 })) as {
    surface_id: number;
  };
  await makePane(r.surface_id, "", socket); // 자동 제목 — 곧 refreshPaneTitles가 현재 경로로 채움
  refreshPaneTitles();
  return r.surface_id;
}

// 새 pane 시작 경로 = 홈 디렉터리 (cwd=null → 데몬 기본값 home_dir — 오너 결정 2026-07-06:
// 피닉스 복원 후에도 새 워크스페이스·pane은 항상 홈에서 시작. 첫 pane 경로 상속 폐기)

async function actionNew() {
  // ★P1-3: 리셋 진행/완료(래치) 중에는 새 pane 을 만들지 않는다. 종전엔 완료 후 '나중에'를
  // 고른 사용자가 + New·Split 를 눌러도 **토스트 하나 없이 무반응**이라 앱이 죽은 것처럼 보였다.
  if (daemonActionBlocked()) return;
  if (current()?.pending) return; // 부서 데몬 준비 중(빈 socket placeholder) — surface 생성 금지(기본 데몬 고아 차단)
  const sid = await newSurface(null, current().socket);
  const ws = current();
  ws.tree = ws.tree
    ? { type: "split", dir: "row", a: ws.tree, b: { type: "pane", sid } }
    : { type: "pane", sid };
  render();
  setFocus(sid);
}

async function actionSplit(dir: "row" | "col") {
  if (daemonActionBlocked()) return; // ★P1-3: 리셋 진행/완료 중 분할 차단(무반응 금지)
  const ws = current();
  // stale focusedSid 검증 — 트리에 없는 대상을 분할하면 replaceNode가 무음 no-op 되어
  // 보이지 않는 고아 surface(살아있는 PTY)가 생긴다
  if (focusedSid == null || !ws.tree || !collectSids(ws.tree).includes(focusedSid)) {
    return actionNew();
  }
  const target = focusedSid;
  const sid = await newSurface(null, ws.socket);
  if (!ws.tree || !collectSids(ws.tree).includes(target)) {
    // await 사이에 대상이 닫힌 경우 — 루트에 덧붙여 고아를 만들지 않는다
    ws.tree = ws.tree
      ? { type: "split", dir, a: ws.tree, b: { type: "pane", sid } }
      : { type: "pane", sid };
  } else {
    ws.tree = replaceNode(ws.tree, target, (old) => ({
      type: "split",
      dir,
      a: old,
      b: { type: "pane", sid },
    }));
  }
  render();
  setFocus(sid);
}

async function actionClose() {
  const ws = current();
  if (focusedSid == null || !ws.tree) return;
  const sid = focusedSid;
  await invoke("close_surface", { socket: ws.socket, surfaceId: sid }).catch(() => {});
  destroyPaneRuntime(sid, ws.socket);
  ws.tree = replaceNode(ws.tree, sid, () => null);
  focusedSid = collectSids(ws.tree)[0] ?? null;
  render();
  if (focusedSid != null) setFocus(focusedSid);
}

// 데몬에서 사라진(종료·닫힘·reap) surface의 UI pane을 자동 제거 — 멱등(이미 없으면 무동작).
// 데몬이 close_surface 하지 않은 자력종료라도 즉시 정리해 죽은 pane이 쌓이지 않게 한다.
// 복구는 보존: 60s grace 내 node-recover로 surface가 되살아나면 refreshPaneTitles 폴링이 재입양한다.
function removeDeadPane(sid: number, socket?: string) {
  const sameSock = (w: Workspace) => (w.socket ?? undefined) === (socket ?? undefined);
  const inLayout = workspaces.some((w) => sameSock(w) && w.tree != null && collectSids(w.tree).includes(sid));
  if (!panes.has(paneKey(sid, socket)) && !inLayout) return; // 이미 정리됨
  destroyPaneRuntime(sid, socket);
  for (const ws of workspaces) {
    if (sameSock(ws) && ws.tree != null && collectSids(ws.tree).includes(sid)) {
      ws.tree = replaceNode(ws.tree, sid, () => null);
    }
  }
  // 포커스 이동은 죽은 pane이 '활성 ws(동일 socket)' 소속일 때만 — 타부서 동일 sid 종료가 현 포커스를 오해제하지 않게.
  if (focusedSid === sid && (current()?.socket ?? undefined) === (socket ?? undefined))
    focusedSid = collectSids(current()?.tree ?? null)[0] ?? null;
  render();
  if (focusedSid != null) setFocus(focusedSid);
}

// ---------- 승인 Feed (Control Center 탭) ----------

interface FeedItem {
  request_id: string;
  kind: string;
  title: string;
  body: string;
  surface_id: number | null;
  status: string;
  decision: string | null;
  // W3: cysd가 title·body에서 파생한 위험 클래스("auto"|"high"|"human") + 자동결재 라우팅 여부.
  risk_class?: string | null;
  auto_route?: boolean;
  // 데몬이 스스로 발행한 항목인가 — cysd 의 feed.list 가 실어 주는 **파생 필드**(진리원 =
  // state::is_daemon_issued). 아래 isDaemonDetectedApproval 하나만 읽는다.
  // optional 인 이유는 구 데몬 프로세스가 살아 있는 스큐뿐이다(그 경우의 폴백도 그 함수에 있다).
  daemon_issued?: boolean;
}

// '데몬이 화면 패턴으로 감지해 올린 승인 항목'인가 — CC 패널(refreshFeed)과 커맨드 팔레트의
// 'feed 승인' 액션이 **같은 술어**를 써야 한다(팔레트가 패널에서 없앤 기만 버튼을 되살리던
// 결함의 수리 · 적대검증 2R major). 그래서 한 곳에 둔다.
//
// ★판별 기준 = **서버가 실어 준 사실**(2026-08-17 교체 — 성찰3 설계렌즈 major):
//   feed.list 응답의 파생 필드 `daemon_issued` 를 읽는다. 그 값의 정의처는 데몬의
//   `state::is_daemon_issued`(접두 상수 DAEMON_REQ_PREFIX) 하나이고, 데몬 자신의 세 소비자
//   (has_pending_daemon_approval · pending_daemon_approvals · governance 의 approval.stalled
//   스캔)도 같은 함수를 지난다. 종전에는 이 UI 가 `request_id.startsWith("daemon-")` 로 접두를
//   **재파싱**했다 — 교차 모듈 계약이 진리원 없는 매직 스트링 복제로 표현돼 있었다.
//   (그 자리의 종전 주석은 '서버 필드는 쓸 수 없다 — feed.list 가 직렬화하지 않는다'고 적었는데,
//    같은 라운드가 그 handlers.rs 를 편집하고 있었으므로 전제 자체를 없애는 편이 옳았다.)
// ★위조 불가: 데몬이 feed.push 경로에서 그 접두를 예약 네임스페이스로 거부한다(handlers.rs —
//   `request_id prefix 'daemon-' is reserved`). ∴ 이 분류는 클라이언트가 만들 수 없다.
// ★필드 부재 시(구 데몬이 살아 있는 스큐: cysd 는 GUI 와 수명이 달라 예전 프로세스가 계속 떠
//   있을 수 있다) — `daemon_issued === undefined` 면 **fail-closed** 로 접두를 본다.
//   오판 방향이 안전한 쪽이기 때문이다: true 로 잘못 보면 그 항목은 Allow 버튼 대신 점프·치우기
//   경로로 가고(승인 위조 0), false 로 잘못 보면 팔레트가 아무 효과 없는 승인을 시도한다(W-4 결함
//   재현). ∴ 모르면 데몬 항목으로 취급한다. 그 폴백 한 줄(feedclass.ts)이 저장소에 남은
//   마지막 접두 리터럴이다.
// ※ 특례 보존: ceo-promote-request 는 kind 가 달라 이 술어에 걸리지 않는다(Allow 경로 그대로).
// ★W4-B: 판정 본체는 feedclass.ts(classifyPendingFeed)로 이동했다 — cycle-verify 부류
//   신설과 함께 패널·팔레트 공용 술어를 **테스트 가능한 순수 모듈**로 격상(bun test 핀).
//   이 함수는 기존 호출처를 위한 얇은 위임이다(의미 무변경).
function isDaemonDetectedApproval(i: FeedItem): boolean {
  return classifyPendingFeed(i) === "daemon-detected";
}

// 승인 Feed는 Control Center의 '승인 Feed' 탭으로 편입됨(독립 패널 폐기).
// 여는 동작 = CC 패널 오픈 + 탭 활성(setCcTab이 refreshFeed 호출).
function openFeed() {
  if (!ccOpen) setCcOpen(true);
  setCcTab("feed");
}

// 승인 자동 화면전환 유예: master/CEO가 이 시간 안에 자동 승인(reply)하면 전환하지 않는다.
// 유예 후에도 pending인 항목 = 사람 수동 승인 필요 → 그때만 승인 Feed 탭으로 전환.
const FEED_SWITCH_GRACE_MS = 30_000; // 비대상(현행) 유예
// W3.4: auto_route 항목은 CEO 심의형 turn이 90초 초과가 흔하므로 기본 유예를 90초로 둔다.
const FEED_SWITCH_GRACE_AUTO_MS = 90_000;
// approval.stalled(5분) 경로가 사람 소환을 담당하므로 그 전까지만 동적 연장한다(그 뒤엔 escalation).
const FEED_SWITCH_MAX_MS = 300_000;

// W3.4: CEO 좌석이 살아서 생성 중(working·저idle)이면 결재 심의가 진행 중으로 본다 —
// nodeSig(refreshSidebarStatus가 org_status에서 채움)를 재갱신한 뒤 판정한다.
async function ceoIsActivelyGenerating(): Promise<boolean> {
  await refreshSidebarStatus().catch(() => {});
  for (const sig of nodeSig.values()) {
    if (sig.role === "ceo") {
      const alive = sig.agent_alive !== false; // null(미상)은 살아있다고 본다
      const working = sig.state === "working" || sig.idle_secs < 30;
      return alive && working;
    }
  }
  return false; // ceo 좌석 신호 부재 = 비활성 → 즉시 전환(사람 소환)
}

// autoRoute=true면 90초 기본 + CEO 활성 시 wait timeout 내 동적 연장, 아니면 30초 고정.
function scheduleFeedSwitchIfStillPending(requestId: string, autoRoute = false, elapsedMs = 0) {
  if (!requestId) return;
  const grace = autoRoute ? FEED_SWITCH_GRACE_AUTO_MS : FEED_SWITCH_GRACE_MS;
  setTimeout(async () => {
    const r = (await invoke("feed_list", { status: null }).catch(() => null)) as { items: FeedItem[] } | null;
    const item = r?.items.find((i) => i.request_id === requestId);
    if (item?.status !== "pending") return; // 이미 결재됨 — 전환 없음
    // auto_route: CEO가 살아서 생성 중이고 상한 이내면 유예 연장(전환 보류).
    if (autoRoute && elapsedMs + grace < FEED_SWITCH_MAX_MS && (await ceoIsActivelyGenerating())) {
      scheduleFeedSwitchIfStillPending(requestId, true, elapsedMs + grace);
      return;
    }
    openFeed(); // 유예 후에도 pending + CEO 비활성/상한 → 사람 개입
  }, grace);
}

// ---------- file tree (오른쪽 섹션 — 선택한 surface의 폴더 탐색) ----------

let ftOpen = false;
let ftRoot: string | null = null;
let ftPinned = false; // F5: 수동 재루팅(더블클릭/메뉴) 시 true — live_cwd 자동 추종 일시 해제
const ftExpanded = new Set<string>(); // 펼쳐진 하위 폴더 경로

function setFtOpen(open: boolean) {
  ftOpen = open;
  document.getElementById("ft-panel")!.hidden = !open;
  if (open) updateFtRoot(); // pane 폭 변화는 ResizeObserver가 자동 보정
}

// 포커스된 surface의 현재 경로를 트리 루트로 — 포커스 이동·cd 모두 추적.
// 수동 핀(ftPinned) 중엔 정지 — 다음 틱의 live_cwd 추종이 수동 이동을 되돌리는 경합 차단(F5).
async function updateFtRoot() {
  if (!ftOpen || focusedSid == null || ftPinned) return;
  try {
    const r = (await invoke("list_surfaces", { socket: current()?.socket })) as {
      surfaces: { surface_id: number; live_cwd: string | null }[];
    };
    const cwd = r.surfaces.find((s) => s.surface_id === focusedSid)?.live_cwd ?? null;
    if (cwd && cwd !== ftRoot) {
      ftRoot = cwd;
      ftExpanded.clear();
      renderFileTree();
    }
  } catch {
    /* 다음 틱에 */
  }
}

// F5: 수동 재루팅 — 핀을 세워 자동 추종을 멈춘다(헤더 📌 클릭 = 추종 복귀).
function setFtRoot(dir: string, pinned: boolean) {
  ftRoot = dir;
  ftPinned = pinned;
  ftExpanded.clear();
  renderFileTree();
}

async function renderFileTree() {
  const body = document.getElementById("ft-body")!;
  const label = document.getElementById("ft-root-label")!;
  if (!ftRoot) {
    body.innerHTML = "";
    label.textContent = "파일";
    return;
  }
  label.textContent = (ftPinned ? "📌 " : "") + (baseName(ftRoot) || ftRoot);
  label.title = ftPinned ? `${ftRoot}\n📌 고정됨 — 클릭하면 pane 경로 자동 추종으로 복귀` : ftRoot;
  label.onclick = ftPinned
    ? () => {
        ftPinned = false;
        void updateFtRoot();
        renderFileTree();
      }
    : null;
  // F4: 루트 전체 경로 상시 표시(헤더 서브라인) — "어디에 있는지"를 우클릭 없이도 보이게
  let pathLine = document.getElementById("ft-path");
  if (!pathLine) {
    pathLine = document.createElement("div");
    pathLine.id = "ft-path";
    document.getElementById("ft-head")!.after(pathLine);
  }
  pathLine.textContent = ftRoot;
  pathLine.title = ftRoot;
  const frag = await buildDirNodes(ftRoot, 0);
  body.innerHTML = "";
  // F5: 상위 폴더 행 — 더블클릭으로 한 단계 위로(단일 클릭 무동작 = 오클릭 방지).
  // parent===ftRoot(루트 "/"·"C:\")면 생략 — 자기 자신으로의 루프 방지.
  const parent = splitPath(ftRoot).parent;
  if (parent && parent !== ftRoot) {
    const up = document.createElement("div");
    up.className = "ft-row dir";
    up.style.paddingLeft = "8px";
    up.textContent = "▴ ..";
    up.title = `${parent} — 더블클릭으로 이동`;
    up.addEventListener("dblclick", () => setFtRoot(parent, true));
    body.appendChild(up);
  }
  body.appendChild(frag);
}

async function buildDirNodes(dir: string, depth: number): Promise<DocumentFragment> {
  const frag = document.createDocumentFragment();
  let entries: { name: string; is_dir: boolean }[] = [];
  try {
    entries = (await invoke("list_dir", { path: dir })) as { name: string; is_dir: boolean }[];
  } catch {
    return frag;
  }
  for (const ent of entries) {
    const full = dir === "/" ? `/${ent.name}` : `${dir}/${ent.name}`;
    const row = document.createElement("div");
    row.className = "ft-row" + (ent.is_dir ? " dir" : "");
    // 폴더 화살표만큼 파일을 더 들여 이름 시작선을 맞춘다
    row.style.paddingLeft = `${8 + depth * 14 + (ent.is_dir ? 0 : 14)}px`;
    row.textContent = ent.is_dir ? `${ftExpanded.has(full) ? "▾" : "▸"} ${ent.name}` : ent.name;
    row.title = full;
    row.addEventListener("click", () => {
      // 드래그로 소비된 mousedown의 후행 click 억제 — 펼침/열기 오발 방지(F2)
      if (ftDragConsumed) {
        ftDragConsumed = false;
        return;
      }
      if (ent.is_dir) {
        if (ftExpanded.has(full)) ftExpanded.delete(full);
        else ftExpanded.add(full);
        renderFileTree();
      } else {
        void openPathChecked(full); // F1: 실패 가시화 + 실행형 확인
      }
    });
    if (ent.is_dir) row.addEventListener("dblclick", () => setFtRoot(full, true)); // F5(D2b)
    row.addEventListener("contextmenu", (e) => {
      e.preventDefault();
      ftContextMenu(e, full, ent.is_dir); // F3
    });
    row.addEventListener("mousedown", (e) => {
      if (e.button === 0) startFtDrag(e, full); // F2: pane으로 드래그해 경로 주입
    });
    frag.appendChild(row);
    if (ent.is_dir && ftExpanded.has(full)) frag.appendChild(await buildDirNodes(full, depth + 1));
  }
  return frag;
}

// F1: 파일 열기 — 실패 무음 금지(feed_reply dead-button 수리와 동일 처방).
// 실행형 파일은 백엔드가 executable_confirm으로 거절 → 확인 후 force 재호출(fail-closed).
async function openPathChecked(full: string) {
  try {
    await invoke("open_path", { path: full });
  } catch (e) {
    if (String(e).includes("executable_confirm")) {
      const ok = await confirmModal(
        "실행 파일 열기",
        `${full}\n\n실행 권한이 있는 파일입니다 — 기본 앱으로 여는 대신 실행될 수 있습니다. 계속하시겠습니까?`,
        "열기",
      );
      if (!ok) return;
      await invoke("open_path", { path: full, force: true }).catch((e2) =>
        toast("watchdog", "파일 열기 실패", String(e2)),
      );
    } else {
      toast("watchdog", "파일 열기 실패", String(e));
    }
  }
}

// 클립보드 복사 — WKWebView에서 navigator.clipboard가 거부될 수 있어 execCommand 폴백.
function copyPath(s: string) {
  const fallback = () => {
    const ta = document.createElement("textarea");
    ta.value = s;
    document.body.appendChild(ta);
    ta.select();
    const ok = document.execCommand("copy");
    ta.remove();
    if (ok) toast("feed", "경로 복사됨", s);
    else toast("watchdog", "경로 복사 실패", "클립보드 접근이 거부되었습니다");
  };
  if (navigator.clipboard?.writeText) {
    navigator.clipboard.writeText(s).then(() => toast("feed", "경로 복사됨", s), fallback);
  } else fallback();
}

// F3: 파일 트리 컨텍스트 메뉴 — 확장 가능 항목 배열(도메인 액션 추가 = 원소 추가).
// 삽입 대상 pane을 라벨에 명시해 오배달 인지를 차단한다(현재 ws의 pane 나열).
function ftContextMenu(e: MouseEvent, full: string, isDir: boolean) {
  const sock = current()?.socket;
  const items: { label: string; action: () => void; disabled?: boolean }[] = [
    { label: full, action: () => {}, disabled: true }, // F4: 경로 표시(아래 복사와 짝)
    { label: "경로 복사", action: () => copyPath(full) },
  ];
  if (!isDir) items.push({ label: "열기", action: () => void openPathChecked(full) });
  items.push({
    label: "Finder에서 보기",
    action: () =>
      void invoke("reveal_path", { path: full }).catch((err) =>
        toast("watchdog", "Finder 표시 실패", String(err)),
      ),
  });
  for (const sid of collectSids(current()?.tree ?? null).slice(0, 6)) {
    const rt = panes.get(paneKey(sid, sock));
    if (!rt) continue;
    const name = rt.titleEl.textContent || `surface ${sid}`;
    items.push({ label: `➤ ${name} 에 경로 삽입`, action: () => void injectPathsToPane(rt, [full]) });
  }
  if (isDir) {
    items.push({ label: "패널 루트를 여기로", action: () => setFtRoot(full, true) });
    items.push({
      label: "cd 텍스트 삽입(전송 없음)",
      action: () => {
        const rt = focusedSid != null ? panes.get(paneKey(focusedSid, sock)) : undefined;
        if (!rt) return toast("watchdog", "삽입 대상 없음", "포커스된 pane이 없습니다");
        void injectRawToPane(rt, `cd ${shellQuote(full, /Windows/i.test(navigator.userAgent))}`);
      },
    });
  }
  showCtxMenu(e.clientX, e.clientY, items);
}

// F2: 경로 주입 파이프라인 — ①실존 재검증(스테일 트리 차단) ②스트리밍 가드 ③IME 가드
// ④형식 결정(에이전트=@멘션/미등록=셸 인용) ⑤주입+피드백. 자동 Return 없음 — 전송은 사람 몫.
async function injectPathsToPane(rt: PaneRuntime, paths: string[]) {
  for (const p of paths) {
    // Windows OS 드롭 경로는 "\" 구분 — splitPath가 양쪽 구분자를 인식(POSIX-only 파싱 회귀 방지)
    const { parent, name } = splitPath(p);
    const entries = (await invoke("list_dir", { path: parent }).catch(() => null)) as
      | { name: string }[]
      | null;
    if (!entries || !entries.some((en) => en.name === name)) {
      toast("watchdog", "경로가 더 이상 없음", `${p} — 트리를 새로고침합니다`);
      void renderFileTree();
      return;
    }
  }
  if (isStreaming(rt.lastOutputAt(), Date.now())) {
    const ok = await confirmModal(
      "에이전트 응답 중",
      "대상 pane이 출력 중입니다. 지금 삽입하면 프롬프트가 섞일 수 있습니다. 그래도 삽입하시겠습니까?",
      "삽입",
    );
    if (!ok) return;
  }
  if (rt.imeBusy()) {
    toast("watchdog", "한글 조합 중", "조합을 끝낸 뒤 다시 시도해 주세요");
    return;
  }
  const r = (await invoke("list_surfaces", { socket: rt.socket }).catch(() => null)) as {
    surfaces: { surface_id: number; live_cwd: string | null; role?: string | null; agent?: string | null }[];
  } | null;
  const me = r?.surfaces.find((s) => s.surface_id === rt.sid);
  const agent = !!(me?.role || me?.agent);
  const isWin = /Windows/i.test(navigator.userAgent);
  await injectRawToPane(rt, insertionText(paths, { agent, isWin, cwd: me?.live_cwd ?? null }));
}

// 주입 공통부 — 성공 피드백(헤더 플래시+토스트)·실패 토스트(무음 삼킴 금지). 자동 Return 없음.
// ★R5 machineOrigin: 경로 문자열도 **UI 가 조립한 문안**이다(사용자가 자판으로 친 것이 아니다).
// 자동 Return 이 없어 보통은 사용자가 뒤에 문장을 이어 붙여 제출하므로 프롬프트 전문과는 매치되지
// 않지만, 판정 불가는 기록하는 쪽이 fail-closed 다(delivery.rs 불변식 ③).
async function injectRawToPane(rt: PaneRuntime, data: string) {
  try {
    await invoke("send_input", { socket: rt.socket, surfaceId: rt.sid, data, machineOrigin: true });
    rt.el.classList.add("inject-flash");
    setTimeout(() => rt.el.classList.remove("inject-flash"), 700);
    toast("feed", "경로 삽입됨", `${rt.titleEl.textContent || rt.sid} — Enter를 눌러야 전송됩니다`);
  } catch (e) {
    toast("watchdog", "삽입 실패", String(e));
  }
}

let ftDragConsumed = false; // 드래그로 소비된 mousedown의 후행 click 억제 플래그

// F2: 트리 행 드래그 — pane 위 드롭 시 경로 주입. startPaneDrag와 동일한 mouse 기반(6px 임계).
// 드롭이 pane을 직격하지 못하면 무동작+토스트 — 포커스 pane 폴백 오배달(footgun) 금지.
function startFtDrag(e0: MouseEvent, full: string) {
  const start = { x: e0.clientX, y: e0.clientY };
  let dragging = false;
  let ghost: HTMLElement | null = null;
  let over: PaneRuntime | null = null;
  const clearOver = () => {
    over?.el.classList.remove("drop-target");
    over = null;
  };
  const move = (e: MouseEvent) => {
    if (!dragging) {
      if (Math.abs(e.clientX - start.x) + Math.abs(e.clientY - start.y) < 6) return;
      dragging = true;
      ftDragConsumed = true;
      ghost = document.createElement("div");
      ghost.id = "drag-ghost";
      ghost.textContent = baseName(full) || full;
      document.body.append(ghost);
    }
    ghost!.style.left = `${e.clientX + 10}px`;
    ghost!.style.top = `${e.clientY + 10}px`;
    const el = (document.elementFromPoint(e.clientX, e.clientY) as HTMLElement | null)?.closest(
      ".pane",
    ) as HTMLElement | null;
    let rt: PaneRuntime | null = null;
    if (el) for (const cand of panes.values()) if (cand.el === el) rt = cand;
    if (rt !== over) {
      clearOver();
      over = rt;
      over?.el.classList.add("drop-target");
    }
  };
  const up = () => {
    window.removeEventListener("mousemove", move, true);
    window.removeEventListener("mouseup", up, true);
    ghost?.remove();
    const target = over;
    clearOver();
    if (!dragging) return;
    // click은 mouseup과 같은 엘리먼트에서만 발화 — pane 위에서 끝난 드래그는 행 click이 없어
    // 플래그가 잔류하면 다음 정상 클릭 1회를 오발 억제한다 → 태스크 큐에서 무조건 청소.
    setTimeout(() => {
      ftDragConsumed = false;
    }, 0);
    if (!target) {
      toast("watchdog", "드롭 취소", "pane 위에 놓아야 삽입됩니다");
      return;
    }
    void injectPathsToPane(target, [full]);
  };
  window.addEventListener("mousemove", move, true);
  window.addEventListener("mouseup", up, true);
}

// ★GUI 오퍼레이터 승인(오너 2026-07-15): feed_reply 실패 사유를 사용자 문구로 분류 — 기존
// `.catch(() => {})` 은폐가 "버튼이 죽은 것처럼" 보이게 해 진단을 지연시킨 결함의 수리.
// 백엔드가 "코드: 메시지" 형식으로 코드를 보존해 주므로 코드 문자열로 분류한다.
function feedReplyErrorText(e: unknown): string {
  const s = String(e);
  if (s.includes("self_approval_denied"))
    return "자기승인 차단(§3.2) — 발행자와 같은 프로세스의 승인은 거부됩니다. 데몬이 구버전이면 업데이트 후 다시 시도하세요.";
  if (s.includes("not_found")) return "항목을 찾을 수 없습니다(만료·삭제되었을 수 있음).";
  if (s.includes("already resolved")) return "이미 처리된 항목입니다.";
  return `전송 오류: ${s}`;
}

// '알림 치우기' 공용 배선(W4-B에서 단일 헬퍼로 추출 — 소비자 둘: daemon-detected ·
// cycle-verify). decision="dismissed"(판정 어휘 아님 — deny 카운터 비오염 근거는
// refreshFeed 의 daemon-detected 분기 주석)로 항목만 resolved 로 바꾼다.
// in-flight 이중클릭 차단 → finally 에서 재활성. ★재렌더는 여기서 부르지 않는다 —
// feed.item.resolved 이벤트가 refreshFeed 를 이미 부르므로(이벤트 핸들러의 feed 분기)
// 여기서 또 부르면 클릭 1회당 전체 재렌더가 2회 난다(N 건 정리 = O(N²) DOM 재구성).
// 배지 갱신은 이벤트 경로에도 있으나(refreshSidebarStatus) 데몬 이벤트 유실 대비로 남긴다
// — 그것은 목록 DOM 을 만들지 않아 비용이 다르다.
function wireFeedDismiss(dismiss: HTMLButtonElement, requestId: string) {
  dismiss.addEventListener("click", async () => {
    dismiss.disabled = true;
    try {
      await invoke("feed_reply", { requestId, decision: "dismissed" });
    } catch (e) {
      toast("health", "알림 치우기 실패", feedReplyErrorText(e));
      refreshFeed(); // 실패 시엔 이벤트가 오지 않으므로 여기서 되돌린다(버튼 상태 복구)
    } finally {
      dismiss.disabled = false;
      refreshSidebarStatus(); // 해소 직후 집계 배지 즉시 갱신
    }
  });
}

// 대기 렌더 상한을 사용자가 명시적으로 푼 상태인가(아래 '더 보기' 버튼) — 되돌리는 경로는
// **정확히 둘**이다: ①feed 탭에 **진입**할 때(setCcTab 의 feed 분기) ②CC 패널을 **닫을 때**
// (setCcOpen 의 close 분기). 그 밖의 탭 이동은 되돌리지 않는다 — feed 탭이 아니면 refreshFeed
// 가 렌더 자체를 건너뛰므로(아래 `ccOpen && ccTab==="feed"` 가드) 비용이 발생하지 않는다.
// ※ ②는 2026-08-17에 **추가**했다. 종전 이 주석은 '패널을 닫으면 되돌린다'고 적었으나 그
//    코드가 없어 거짓 계약이었다(성찰3 설계렌즈 minor — 주석=계약 규약 위반).
let feedPendingExpanded = false;

// ★#4-b — '이 목록이 원리적으로 닿지 못하는 대기'를 **부서별 건수 + 이동 버튼**으로 그린다.
// 반환 = 그렇게 노출한 총 대기 수(0이면 아무것도 그리지 않는다).
//
// ★정직 유지(W-4 기만 버튼 금지와 같은 규율): 이 목록의 Allow/Deny 는 여전히 **기본 데몬 전용**
//   이다(feed_list·feed_reply 둘 다 default_socket 고정). 그래서 여기서 제공하는 것은 '처리'가
//   아니라 '이동'뿐이다 — 남의 데몬 항목을 처리하는 척하는 버튼을 만들면 W-4 가 없앤 기만 버튼을
//   되살리는 셈이다. 근본 수리는 feed_list/feed_reply 의 소켓 인지화이며 그 범위는 여전히 별건이다.
function renderOtherWorkspacePending(box: HTMLElement): number {
  const rows = deptPendingRows();
  const total = rows.reduce((n, r) => n + r.count, 0);
  if (total === 0) return 0;
  const warn = document.createElement("div");
  warn.className = "cc-empty";
  warn.textContent =
    `⚠ 다른 워크스페이스(부서 데몬)의 대기 ${total}건은 이 목록에서 처리할 수 없습니다 — ` +
    `아래에서 해당 부서로 이동해 그 pane 에서 처리하세요. (이 목록의 Allow/Deny 는 기본 데몬 전용입니다.)`;
  box.appendChild(warn);
  for (const r of rows) {
    const row = document.createElement("div");
    row.className = "feed-item pending";
    const title = document.createElement("div");
    title.className = "fi-title";
    title.textContent = `${r.label} — 대기 ${r.count}건`;
    const meta = document.createElement("div");
    meta.className = "fi-meta";
    // 소켓 **전체 경로**는 길어 줄을 넘긴다(.fi-meta 에 word-break 없음) — 부서 슬러그만
    // 보이고 전체 경로는 title 로 남긴다. 식별은 되게, 레이아웃은 안 깨지게.
    meta.textContent = deptSlugOfSocket(r.socket);
    meta.title = r.socket;
    // ★F6①: 이미 그 워크스페이스에 있으면 '이동'이 아니다 — 라벨이 실제 동작과 어긋나면
    //   사용자는 눌러 보고 나서야 안다. 상태에 맞춰 문구를 바꾼다(동작은 둘 다 '패널 닫기'로
    //   그 부서 pane 을 드러내는 것 — 이미 그 부서면 그게 유일하게 유용한 동작이다).
    const here = isActiveDeptSocket(workspaces, activeWs, r.socket);
    const go = document.createElement("button");
    go.textContent = here ? "지금 이 부서 — 패널 닫기" : "이 부서로 이동";
    go.title = here
      ? "이미 이 부서 워크스페이스입니다 — 패널을 닫아 pane 을 봅니다(승인은 그 pane 에서 직접)."
      : "해당 워크스페이스로 전환합니다 — 승인은 그 pane 에서 직접 처리하세요.";
    go.addEventListener("click", () => {
      // 전환에 성공하면 패널을 닫아 그 부서 pane 이 바로 보이게 한다(목록은 기본 데몬 것이라
      // 열어 둬도 내용이 바뀌지 않는다 — 열린 채면 '이동했는데 화면이 그대로'로 읽힌다).
      switch (switchToWorkspaceBySocket(r.socket)) {
        case "switched":
          setCcOpen(false);
          break;
        // ★F6②: **연결 중**(pending placeholder) 워크스페이스는 '닫힌 탭'이 아니다 —
        //   종전엔 `!w.pending` 조건 때문에 "탭이 이미 닫혔습니다"로 오안내했다. 전환은
        //   해 주되(그 탭이 준비 스피너를 보여준다) 상태를 사실대로 알린다.
        case "pending":
          setCcOpen(false);
          toast("feed", "부서 데몬 준비 중", `${r.label} 워크스페이스로 이동했습니다 — 기동이 끝나면 pane 이 나타납니다.`);
          break;
        case "missing":
          toast("feed", "이동 불가", `${r.label} 탭이 이미 닫혔습니다 — 부서를 다시 열어 주세요.`);
          break;
      }
    });
    row.append(title, meta, go);
    box.appendChild(row);
  }
  return total;
}

async function refreshFeed() {
  const r = (await invoke("feed_list", { status: null }).catch(() => null)) as
    | { items: FeedItem[] }
    | null;
  if (!r) return;
  const items = r.items.slice().reverse();

  // 대기 배지는 refreshSidebarStatus(전체 소켓 집계)가 단독 소유 — 여기선 목록만 렌더.
  // (feed_list는 기본 데몬 1개만 조회하므로 멀티부서 집계와 스코프가 달라 배지 구동에 부적합.)
  if (!(ccOpen && ccTab === "feed")) return;
  const box = document.getElementById("cc-feed-items")!;
  box.innerHTML = "";
  if (items.length === 0) {
    // ★'비어 있음'만 적으면 거짓말이 될 수 있다 — 이 목록은 기본 데몬 1개만 보므로 부서 데몬에
    //  대기가 남아 있어도 비기 때문이다. 그 경우 사유를 함께 적는다.
    //  ※ 수치는 **타 소켓 직접 합**(pendingBySocket 소켓별 값)이다 — 전 소켓 합산(pendingApprovals)을
    //    쓰면 기본 데몬 자신의 대기까지 '다른 워크스페이스'로 오안내한다(두 조회 사이 스큐).
    //  ★#4-b: 종전엔 여기서도 문장 한 줄로 끝나 '어느 부서'가 없었다 — 부서별 행+이동 버튼을
    //    그린다(빈 목록일 때야말로 오너가 '멈췄다'고 오해하는 화면이다).
    if (deptPendingRows().length === 0) {
      box.textContent = "(비어 있음)";
      return;
    }
    const empty = document.createElement("div");
    empty.className = "cc-empty";
    empty.textContent = "(기본 데몬에는 항목이 없습니다)";
    box.appendChild(empty);
    renderOtherWorkspacePending(box);
    return;
  }
  // ★대기 항목을 **먼저·많이** 렌더한다. 종전엔 최신순 50건만 잘랐는데, 데몬의 보존 한도가
  //  "pending 전부 + 종결 최근 1000건"(state.rs 의 FEED_RETAIN)이라 **오래된 pending 은 종결
  //  항목 더미에 밀려 화면에 안 나온다**. 그런데 대기 배지(refreshSidebarStatus)는 데몬 집계라
  //  그 항목까지 세므로 "배지에는 있는데 목록에는 없어 치울 수 없는" pending 이 생긴다 —
  //  아래 '알림 치우기'가 닿지 못하는 사각이다. ∴ pending 을 앞에 놓고 남는 자리만 종결로 채운다.
  //
  // ★상한을 둔다(PENDING_RENDER_CAP) — 종전의 '전건 렌더'는 무상한이었고 그것은 위험했다:
  //  pending 은 상한이 없는 집합이고(FEED_RETAIN 은 종결 항목에만 적용), 승인 프롬프트를 띄운
  //  채 pane 이 죽으면 stale-clear 가 종료 surface 를 건너뛰어(governance.rs) 그 항목은 영구
  //  잔존한다. refreshFeed 는 모든 feed.* 이벤트마다 돌므로 N 건을 하나씩 치우면 O(N²) DOM
  //  재구성이 된다 — 앵커 ④(전 pane 사망) 시나리오에서 UI 가 함께 굳는다.
  //  ※ 같은 라운드에 상수배도 줄였다: 치우기 핸들러의 finally 에 있던 refreshFeed 를 없애
  //    이벤트 구동 1회로 합쳤다(종전엔 클릭 1회당 2회 재렌더 — 아래 dismiss 주석 참조).
  //  상한을 두되 '지울 수 없는 pending 은 없어야 한다'는 목표는 아래 '나머지 보기'로 유지한다.
  //
  // ★스코프 정직 고지(종전 주석은 여기서 **거짓**이었다 — 적대검증 2R major):
  //  이 목록·치우기는 **기본 데몬 1개**만 본다(feed_list·feed_reply 둘 다 default_socket 고정).
  //  반면 대기 배지는 전 워크스페이스 소켓의 org_status.feed.pending 합산이다. ∴ 부서 데몬의
  //  pending 은 지금도 '배지에는 있으나 이 목록에 없는' 상태로 남는다 — 종전 주석의
  //  "절대 조건: GUI 에서 지울 수 없는 pending 은 없어야 한다(달성)" 은 기본 데몬 한정으로만
  //  참이었다. 그 차이를 숨기지 않고 아래 배너로 표시한다(사각을 아는 것이 사각을 없애기 전의
  //  최소 의무다). 근본 수리는 feed_list/feed_reply 의 소켓 인지화이며 이번 릴리스 범위 밖이다.
  const PENDING_RENDER_CAP = 200;
  const pendingItems = items.filter((i) => i.status === "pending");
  const settledItems = items.filter((i) => i.status !== "pending");
  const pendingShown = feedPendingExpanded ? pendingItems : pendingItems.slice(0, PENDING_RENDER_CAP);
  const pendingHidden = pendingItems.length - pendingShown.length;
  const shown = pendingShown.concat(settledItems.slice(0, Math.max(0, 50 - pendingShown.length)));

  // (a) 이 목록이 닿지 못하는 대기(=부서 데몬 등 타 소켓)를 명시한다.
  //     ★값의 출처(2026-08-17 교체 — 성찰3 설계렌즈 minor): 종전에는
  //     `pendingApprovals - pendingItems.length` 라는 **스코프가 다른 두 값의 뺄셈**이었다.
  //     피감수는 이 목록(기본 데몬 feed_list)의 pending 수, 감수는 전 소켓 합산이라
  //     ⓐ부서 데몬 하나가 일시 미응답이면(refreshSidebarStatus 의 catch 가 0으로 접는다)
  //       결과가 과소·음수가 되어 배너가 사라지고, ⓑ두 조회 사이 스큐로는 과대가 되어
  //       "다른 워크스페이스에 N건" 이 사실이 아닌 수를 단정했다.
  //     이제 refreshSidebarStatus 가 같은 순회에서 소켓별로 보관한 값을 직접 합산한다
  //     (deptPendingRows = 기본 소켓을 뺀 부서별 행, 그 합이 총계) — 뺄셈이 없으니 음수가
  //     불가능하고, 이 목록의 길이·갱신 시점과 무관하다.
  //     ★#4-b(2026-08-22): 그 값을 문장 한 줄로만 쓰던 것을 **부서별 행 + 이동 버튼**으로 바꾼다.
  //     "N건이 어딘가에 있다"는 고지는 사각을 아는 데까지만 데려다줄 뿐, 오너를 그 부서로
  //     데려가지 못했다(그래서 부서 상태가 '멈춤'으로 체감됐다).
  renderOtherWorkspacePending(box);
  // (b) 상한 초과분은 '나머지 보기'로 연다 — 지울 수 없는 pending 을 만들지 않기 위함이다.
  if (pendingHidden > 0) {
    const more = document.createElement("button");
    more.textContent = `대기 ${pendingHidden}건 더 보기 (총 ${pendingItems.length}건)`;
    more.title = "대량 렌더는 UI 를 느리게 만들 수 있습니다 — 필요할 때만 펼치세요.";
    more.addEventListener("click", () => {
      feedPendingExpanded = true;
      refreshFeed();
    });
    box.appendChild(more);
  }
  for (const item of shown) {
    const el = document.createElement("div");
    el.className = `feed-item ${item.status}`;
    const title = document.createElement("div");
    title.className = "fi-title";
    title.textContent = item.title;
    const meta = document.createElement("div");
    meta.className = "fi-meta";
    meta.textContent = `${item.kind} · ${item.request_id}` + (item.surface_id != null ? ` · surface:${item.surface_id}` : "");
    const body = document.createElement("div");
    body.className = "fi-body";
    body.textContent = item.body;
    el.append(title, meta, body);
    // ★데몬이 화면 패턴으로 감지해 올린 승인 항목은 Allow/Deny가 **아무 효과도 내지 못한다**(W-4).
    //  근거(코드 실측):
    //   ① 발행 경로는 governance.rs 의 check_approvals — 그 함수 doc이 "★자동 응답 절대 금지 —
    //      감지·격상만"이라고 못박고 있고, 발행에 쓰는 Daemon::push_feed_notification(state.rs)은
    //      **waiter를 등록하지 않는다**(feed.push의 wait 경로와 달리 oneshot 채널이 없다).
    //   ② feed.reply 핸들러(handlers.rs)는 항목을 resolved로 바꾸고 대기자를 깨우고 감사에 남길
    //      뿐, PTY에는 **한 바이트도 쓰지 않는다**(그 핸들러 전체에 WriteReq·try_write 사용 0건).
    //  ∴ 버튼을 눌러도 앱의 프롬프트는 그대로 남고 목록에서 항목만 사라진다 = 사용자를 속이는
    //  버튼이다. 실제 응답은 그 pane에서 사람이 직접 해야 하므로 '점프' 버튼으로 대체한다.
    //
    //  ★판별 기준은 isDaemonDetectedApproval(위 FeedItem 선언 옆)에 단일화돼 있다 — 커맨드
    //  팔레트의 'feed 승인' 액션이 **같은 술어**로 이 부류를 제외해야 하기 때문이다(팔레트가
    //  여기서 없앤 기만 버튼을 되살리던 결함의 수리 · 적대검증 2R). 근거·위조 불가 논증은
    //  그 함수 주석에 있다.
    //
    //  ★그러나 **수동 해소 경로는 반드시 남긴다**(적대검증 지적 반영 — 초판은 '점프'만 두어
    //  치우기를 없앴고 그것은 지울 수 없는 항목을 만들었다). 초판 주석은 "항목 정리는 데몬이
    //  한다(stale-clear)"고 적었는데 그 서술은 **불완전**하다:
    //   · stale-clear 를 도는 루프(governance.rs check_approvals)는 surface 목록을 돌면서
    //     `if s.exited { continue; }` 로 **종료된 surface 를 건너뛴다** → 그 surface 의 pending
    //     감지 항목은 stale-clear 를 영원히 못 받는다.
    //   · surface 종료·삭제 시 feed 항목을 해소하는 훅도 없다(resolve_feed_item 호출처는
    //     feed.reply 핸들러 · 채널 응답 · 위 stale-clear 셋뿐 — exit 훅 0건).
    //  ∴ 승인 프롬프트를 띄운 채 pane 이 죽으면 그 항목은 자동으로도 수동으로도 사라지지 않아
    //  대기 배지를 영구 오염시킨다. 그래서 '치우기'를 항상 제공한다.
    //
    //  ★두 버튼의 의미를 분리해 사용자를 속이지 않는다(이 수리의 원래 목적 보존):
    //   · '이 pane에서 직접 응답' = 점프만. 실제 응답은 사람이 그 pane 에서 한다.
    //   · '알림 치우기' = **목록 정리 전용**. feed_reply 로 항목만 resolved 로 바꾼다 — 앱의
    //     프롬프트에는 한 바이트도 가지 않는다(그 사실을 버튼 라벨·note·title 에 명시).
    //  ★decision="dismissed" 인 이유: "deny" 를 쓰면 데몬의 거부 카운터(record_approval_deny —
    //   approve_auto_route ON 일 때 back-pressure 집계)에 잡혀 '거부가 쌓였다'는 **거짓 신호**가
    //   된다. 그 카운터는 deny|no|reject 만 세므로 별도 어휘를 쓰면 집계에 오염이 없고, 해소된
    //   항목에는 `→ dismissed` 로 표시돼(아래 fi-decision 분기) 감사에서도 '앱 응답'과 구분된다.
    //   decision 문자열은 데몬이 검증하지 않는다 — feed.reply 핸들러는 param_str 로 받아
    //   resolve_feed_item_audited 에 그대로 넘길 뿐 어휘 화이트리스트가 없다(handlers.rs).
    //  ★자기승인 가드(§3.2)에 걸리지 않는 근거는 **두 겹**이다(둘 중 하나만으로도 충분):
    //   ① is_self_approval 은 `decision != "allow"` 면 즉시 false 다(state.rs 첫 줄 분기) —
    //     "dismissed" 는 어떤 발행자 조합에서도 가드에 닿지 않는다.
    //   ② 데몬 발행 항목은 publisher_pid·publisher_pgid·publisher_surface 가 **모두 None** 이라
    //     (state.rs push_feed_notification) pid/pgid 일치·surface 분기 세 갈래가 전부 불성립이다.
    //     ※ publisher_surface 까지 None 인 것이 중요하다 — Some 이면 surface 미귀속 호출자
    //       (GUI)는 fail-closed 로 차단되므로 ②만으로는 안전하지 않았을 것이다.
    const daemonDetected = isDaemonDetectedApproval(item);
    if (item.status === "pending" && daemonDetected) {
      const note = document.createElement("div");
      note.className = "fi-meta";
      // ★'치우기'의 데몬측 부작용을 숨기지 않는다(적대검증 2R minor). "앱에는 아무것도 전달되지
      //  않습니다"는 참이지만 '아무 일도 일어나지 않는다'로 읽힌다 — 실제로는 두 가지가 일어난다:
      //   ① 화면에 승인 프롬프트가 **아직 살아 있으면** 데몬의 재발행 억제(L3 코얼레싱 가드
      //      has_pending_daemon_approval)가 풀려, (surface,pattern) 60초 debounce 뒤 같은
      //      에피소드가 다시 올라온다(≤60초 내 재출현 + 토스트·OS 배너 재발화). 정상 동작이다.
      //   ② 새 항목은 created_at 이 갱신되므로 '사람 개입 필요' 격상(approval.stalled)의
      //      방치 시계가 그때마다 처음부터 다시 간다.
      //  ∴ 치우기는 '이미 사람이 응답했거나 pane 이 죽은' 항목에 쓰는 정리 버튼이다.
      note.textContent =
        "데몬이 화면에서 감지한 승인 대기입니다 — 여기서 승인/거부해도 앱에는 전달되지 않습니다. 해당 pane에서 직접 응답하세요. " +
        "('알림 치우기'는 이 목록에서만 지웁니다. 그 pane에 승인 프롬프트가 아직 떠 있으면 데몬이 잠시 뒤 다시 감지해 항목이 재등장할 수 있고, 치울 때마다 방치 경보 대기시간이 다시 시작됩니다.)";
      const actions = document.createElement("div");
      actions.className = "fi-actions";
      const jump = document.createElement("button");
      jump.textContent = "이 pane에서 직접 응답";
      const target = item.surface_id;
      if (target == null) {
        // surface 미상(구 영속 라인 등) — 점프 대상이 없으면 비활성해 헛클릭을 막는다.
        // ★이때도 아래 '치우기'는 살아 있다 — 그것이 이 항목에 남는 유일한 동작이다(무동작 금지).
        jump.disabled = true;
        jump.title = "대상 surface 미상 — 해당 pane을 직접 찾아 응답하세요";
      } else {
        jump.addEventListener("click", () => {
          // feed_list는 기본 데몬 1개만 조회한다(위 refreshFeed 주석) → socket 미지정 = 기본 데몬 ws.
          jumpToSurface(target);
          setCcOpen(false); // CC 패널이 pane을 가리므로 닫는다 — 프롬프트를 바로 보고 답하게.
        });
      }
      const dismiss = document.createElement("button");
      dismiss.textContent = "알림 치우기";
      dismiss.title =
        "이 알림 항목만 목록에서 지웁니다 — 앱에는 아무것도 전달되지 않습니다(pane 종료 등으로 데몬 자동 정리가 닿지 않는 항목의 수동 해소 경로).\n" +
        "⚠ 해당 pane에 승인 프롬프트가 아직 떠 있으면 데몬이 잠시 뒤(최대 1분) 다시 감지해 항목이 재등장합니다 — 정상 동작입니다.\n" +
        "⚠ 치울 때마다 '사람 개입 필요' 방치 경보(approval.stalled)의 대기시간이 처음부터 다시 시작됩니다.";
      wireFeedDismiss(dismiss, item.request_id); // 클릭 배선 = 공용 헬퍼(주석·근거는 그쪽)
      actions.append(jump, dismiss);
      el.append(note, actions);
    } else if (item.status === "pending" && classifyPendingFeed(item) === "cycle-verify") {
      // ★[W4-B · 결함 7] cycle-verify 는 GUI 에서 판정할 수 없다 — Allow/Deny 를 내린다
      //  (W-4 기만 버튼 재도입 금지). 근거·분류 우선순위는 feedclass.ts(classifyPendingFeed)
      //  주석 참조: GUI Allow 는 operator 토큰 경로(pane 미귀속·resolver_surface=None)라
      //  cycle-agent 의 영수증 검증(cys.rs cycle_receipt_ok — resolver==지정 검증자 대조)이
      //  거부한다 — 항목만 소모되고 clear 는 실행되지 않으며, 검증자의 정상 reply 기회도
      //  사라진다. ∴ '지정 검증자 pane 에서만 판정 가능' 안내 + 목록 정리 전용 치우기만
      //  남긴다(무동작 pending 금지 — daemon-detected 부류와 동일 처방).
      //  ※ 점프 버튼은 두지 않는다: item.surface_id 는 **cycle 대상** surface 지 판정
      //  주체(지정 검증자 pane)가 아니다 — 거기로 점프시키면 또 다른 오도가 된다.
      const note = document.createElement("div");
      note.className = "fi-meta";
      note.textContent = CYCLE_VERIFY_NOTE;
      const actions = document.createElement("div");
      actions.className = "fi-actions";
      const dismiss = document.createElement("button");
      dismiss.textContent = "알림 치우기";
      dismiss.title = CYCLE_VERIFY_DISMISS_TITLE;
      wireFeedDismiss(dismiss, item.request_id);
      actions.append(dismiss);
      el.append(note, actions);
    } else if (item.status === "pending") {
      const actions = document.createElement("div");
      actions.className = "fi-actions";
      const btns: HTMLButtonElement[] = [];
      for (const [label, decision, cls] of [["Allow", "allow", "allow"], ["Deny", "deny", "deny"]] as const) {
        const btn = document.createElement("button");
        btn.className = cls;
        btn.textContent = label;
        btn.addEventListener("click", async () => {
          // ★GUI 오퍼레이터 승인(오너 2026-07-15 · 리뷰어1 R1 반영): in-flight 동안 두 버튼 비활성
          // (이중클릭·상반 결정 경합 차단), finally에서 전 경로(성공 포함) 재활성 후 재렌더로 일원화
          // — 기존 .catch(() => {}) 은폐 제거 + 실패 시 사유 토스트.
          btns.forEach((b) => (b.disabled = true));
          try {
            // ★CEO 승격 Allow 결함 수리(오너 2026-07-15 + 적대검증 D-2/3/4): 이 요청을 만든 cys-dept
            // 대기자는 데몬 재시작 등으로 죽어 pending 고아가 되며, feed_reply만으론 승격이 집행되지
            // 않았다(먹통). ①머신 kind(ceo-promote-request)로만 라우팅 — 제목 정규식은 정보성 알림
            // ("보류/대기")에도 매칭돼 오탐(D-3). ②승격을 feed_reply보다 **먼저** 집행 — 실패 시 항목을
            // pending으로 남겨 재시도 가능(D-4). ③promote-ceo가 미승격 PENDING이면 exit 5→Err→실패
            // 표시(가짜 "완료" 토스트 차단·D-2).
            const isCeoPromote = item.kind === "ceo-promote-request";
            if (isCeoPromote && decision === "allow") {
              try {
                const r = (await invoke("approve_ceo_promotion")) as string;
                try {
                  await invoke("feed_reply", { requestId: item.request_id, decision });
                  toast("watchdog", "✅ CEO 승격 완료", r || "기본 데몬 master를 CEO로 승격했습니다.");
                } catch (e) {
                  // 승격은 성공, feed 항목 해소만 실패(pending 잔존) — 두 사실을 모두 표시.
                  toast("health", "CEO 승격 완료 · feed 해소 실패",
                    `승격: ${r || "완료"} / feed: ${feedReplyErrorText(e)}`);
                }
              } catch (e) {
                // 승격 실패 — feed_reply 하지 않음(항목 pending 유지·재시도 가능). 실패 사유 표시.
                toast("health", "CEO 승격 실패", String(e));
              }
            } else {
              try {
                await invoke("feed_reply", { requestId: item.request_id, decision });
              } catch (e) {
                toast("health", decision === "allow" ? "승인 실패" : "거부 실패", feedReplyErrorText(e));
              }
            }
          } finally {
            btns.forEach((b) => (b.disabled = false));
            refreshFeed();
            refreshSidebarStatus(); // 결정 직후 집계 배지 즉시 갱신
          }
        });
        btns.push(btn);
        actions.appendChild(btn);
      }
      el.appendChild(actions);
    } else {
      const d = document.createElement("div");
      d.className = "fi-decision";
      d.textContent = item.status === "timeout" ? "⏱ timeout" : `→ ${item.decision}`;
      el.appendChild(d);
    }
    box.appendChild(el);
  }
}

// ---------- 자동 업데이트 ----------

// invoke 응답의 신뢰 모양 — **명명 타입으로 둔다**(인라인 금지). 아래 checkForUpdate 가
// `as typeof bin` 으로 단언하던 자리에서 TS2339('… does not exist on type never')가 7건 났던
// 원인이 이것이다: `as typeof X` 는 선언 타입이 아니라 **그 지점의 좁혀진 타입**을 가리키는데,
// 바로 위에서 null 로 초기화했으므로 typeof X = null 이 되고 → 대입 후 X 는 null 로 좁혀지며
// → `X && X.version` 의 truthy 분기가 never 가 된다. 명명 별칭은 좁혀지지 않으므로 원래 의도
// (응답을 이 모양으로 신뢰)를 그대로 표현하면서 게이트(bunx tsc -p tsconfig.check.json)를 통과한다.
type BinUpdateInfo = { version: string; current?: string; notes?: string };
type PackUpdateInfo = { pack_version: string; manifest_url: string; binary_too_old: boolean };

let updateAvailable: { version: string; notes?: string } | null = null;
// 무중단 팩 업데이트(check_pack_update) 결과 — 팩만 변경 시 세션·데몬 유지 경로(install_pack_update).
let packUpdateAvailable: PackUpdateInfo | null = null;

/// 업데이트 확인. silent=true면 시작 시 백그라운드 체크(결과 없으면 조용히).
/// 바이너리(check_update·재시작)와 무중단 팩(check_pack_update·세션 유지)을 둘 다 확인해 분기한다.
async function checkForUpdate(silent: boolean) {
  // 1) 바이너리 업데이트(Tauri updater latest.json) — 재시작 경로.
  let bin: BinUpdateInfo | null = null;
  let binCheckFailed = false;
  try {
    bin = (await invoke("check_update")) as BinUpdateInfo | null;
  } catch (e) {
    // ★early-return 안 함(팩 체크는 계속) — 단, 바이너리 상태 불명을 기억해 아래 '최신' 단정을 억제한다.
    binCheckFailed = true;
    if (!silent) toast("health", "업데이트 확인 실패", String(e));
  }
  // 2) 무중단 팩 업데이트(pack-manifest.json) — 세션·데몬 유지 경로. 실패는 조용히(폴링).
  let pack: PackUpdateInfo | null = null;
  let packCheckFailed = false;
  try {
    pack = (await invoke("check_pack_update")) as PackUpdateInfo | null;
  } catch {
    /* 팩 체크 실패(네트워크·부재) = 조용히 무시 */
    packCheckFailed = true;
  }

  // ★fail-safe: 체크가 성공했을 때만 상태를 갱신한다. 일시 네트워크/업데이터 장애로 체크가 실패하면
  // 마지막으로 검증된 상태(있던 업데이트 배지)를 보존한다 — 장애로 배지가 사라져 "업데이트 없음"으로
  // 오인하는 것을 막는다(fresh 성공 시에만 갱신·해제).
  if (!binCheckFailed) {
    updateAvailable = bin && bin.version ? { version: bin.version, notes: bin.notes } : null;
  }
  if (!packCheckFailed) {
    packUpdateAvailable =
      pack && pack.pack_version
        ? { pack_version: pack.pack_version, manifest_url: pack.manifest_url, binary_too_old: pack.binary_too_old }
        : null;
  }

  const badge = document.getElementById("update-badge")!;
  // 분기 판정은 순수 함수(updateplan.ts — 옵션 2·오너 승인 2026-07-14)로 일원화.
  // 기존 4분기 배지·문구는 updateplan.test.ts가 문자열 단위로 핀(회귀 0) — 신설은
  // pack-and-binary(본체+팩 동시·호환 시 팩 무중단을 가리지 않음·T5 불변) 하나뿐이다.
  const plan = updatePlan({
    binVersion: updateAvailable ? updateAvailable.version : null,
    packVersion: packUpdateAvailable ? packUpdateAvailable.pack_version : null,
    binaryTooOld: packUpdateAvailable ? packUpdateAvailable.binary_too_old : false,
    binCheckFailed,
    packCheckFailed,
  });
  if (plan.kind !== "unknown") {
    // unknown = 체크 실패·보존 상태 없음 → 배지 유지('최신' 오단정 금지, 종전 fail-safe).
    badge.hidden = false;
    badge.textContent = plan.badge;
    if (plan.ok) badge.classList.add("ok");
    else badge.classList.remove("ok");
    badge.title = plan.title;
  }
  switch (plan.kind) {
    case "pack-and-binary":
      // ★옵션 2: 팩 무중단이 실행 가능한 액션 — 모달은 팩 하나만(silent 불변식: 모달 금지는
      // silent 경로에만 해당·비silent도 모달 1개 상한), 본체는 토스트로 병행 안내(T5 경로 유지).
      if (!silent) {
        promptPackInstall();
        toast("feed", "🔄 새 본체도 있음", `새 본체 ${updateAvailable!.version} — 상단 Update 버튼으로 패치 설치(재시작·자동 복원)`);
      } else toast("feed", "↻ 무중단 팩 + 새 본체", plan.toastMsg);
      break;
    case "binary":
      // 본체(바이너리) 패치 설치 — 오너 지시(2026-07-15) 재배선(구 T5 홈페이지 전용의 실험적 개정).
      if (!silent) promptBinaryPatch();
      else toast("feed", "🔄 새 본체 버전", plan.toastMsg);
      break;
    case "pack":
      // 팩만 변경 + 바이너리 호환 → 무중단 가능(세션·데몬 생존).
      if (!silent) promptPackInstall();
      else toast("feed", "↻ 무중단 팩 업데이트", plan.toastMsg);
      break;
    case "binary-required":
      // 팩은 있으나 min_binary_version > 설치 바이너리 → 무중단 불가, 본체 업데이트(홈페이지) 필요(T5 정책).
      if (!silent) toast("health", "본체 업데이트 필요", plan.toastMsg);
      else toast("feed", "⚠ 업데이트 있음", plan.toastMsg);
      break;
    case "none":
      // 오너 지시(2026-07-03): 최신 확인 시 숨김 대신 "0" 표시. 중립 스타일(.ok)로 경고색 회피.
      if (!silent) toast("watchdog", "✅ 최신 버전", "최신 버전입니다. 추가 업데이트가 없습니다.");
      break;
    case "unknown":
      break;
  }
}

/// 본체(바이너리) 패치 설치 — 오너 지시(2026-07-15)로 인앱 install_update 재배선(구 T5 홈페이지
/// 전용 정책의 실험적 개정). install_update = drain 저장 신호 → 다운로드·서명검증 → .app 교체 →
/// 데몬 핸드오프 → 앱 재시작(부서·노드는 피닉스·resume으로 자동 복원). 진행 표시는
/// update-progress 리스너("upd-bin" sticky)가 전담한다.
async function promptBinaryPatch() {
  // ★A7(성찰 확정): install_update 는 앱을 교체·재시작한다 — 리셋 실행 중이면 격리 스레드가
  // 중도 사멸해 manifest(복구 지도) 없는 반쪽 격리가 남는다. 완료 래치 상태에서도 무의미하다.
  if (daemonActionBlocked()) return;
  if (!updateAvailable) {
    await checkForUpdate(false);
    return;
  }
  const v = updateAvailable.version;
  const ok = await confirmModal(
    `새 본체 버전 ${v} — 패치 설치`,
    `새 본체(앱) ${v}을 패치 방식으로 설치합니다: 저장(drain) 신호 후 다운로드·서명 검증·교체하고 앱을 ` +
      `재시작합니다. 부서·노드는 재시작 후 자동 복원됩니다(대화 기억 포함). 마지막 미저장분은 손실될 수 ` +
      `있습니다.\n\n지금 설치하시겠습니까? (수동 설치는 홈페이지 www.cysinsight.com)`,
    "설치",
  );
  if (!ok) return;
  try {
    await invoke("install_update", { force: true });
    // 성공 시 백엔드가 app.restart()까지 수행 — 후속 UI 처리 없음(진행은 update-progress 리스너).
  } catch (e) {
    dismissToast("upd-bin");
    toast("health", "패치 설치 실패", String(e));
  }
}

// ── 버전 스큐 세대교체(무중단 rename-swap의 짝) — 메인 + 부서 데몬 ──
// 업데이트 후 구 데몬(lame-duck)이 세션을 보존하는 동안 "데몬 vX ↔ 앱 vY" 스큐를 비차단으로 알린다.
// 강제 재시작 없음(세션 보존 우선). 잃을 세션 0인 노드는 무손실 자동 교대, 세션 있는 노드만 배지+1회 안내.
// ★거버넌스: 부서 교대는 '재기동'일 뿐 CSO 단일소유 생성/폐기 권한을 건드리지 않는다
// (rotate_dept_daemon=cys-dept rotate=데몬 프로세스만 재기동·레지스트리·묘비·CEO 불변).
let rotatingDaemon = false;
// ★[F3] 부서 완전 폐역(purge) 진행 플래그 — purge와 데몬 교대(restart)는 같은 부서 데몬을 동시에 건드리면
// 경합한다. purge는 rotatingDaemon을, restart는 purgingDept를 서로 존중해 상호 배제한다.
let purgingDept = false;
// ★완전 초기화(팩토리 리셋) 진행 플래그 — 리셋은 전 데몬을 죽이므로 restart(부활)·purge와
// 절대 겹치면 안 된다. 세 플래그가 서로를 존중한다(F3 상호 배제 확장).
let factoryResetting = false;
// ★A6(성찰 확정): 리셋 **성공 후 래치**. 성공 시점부터 이 앱 프로세스는 "설치 직후"를 향한
// 반쪽 상태(pack·훅·launchd 부재)다 — 여기서 데몬을 되살리면 pack 없는 유령 데몬이 서고
// 다음 실행 온보딩이 살아있는 데몬 위로 겹쳐 돈다(설계 §4 계약 침식). 종료 전까지 데몬을
// 생성·부활시키는 모든 경로를 영구 차단한다(플래그는 성공 경로에서 절대 해제되지 않는다).
let resetCompleted = false;

// 데몬 생성·부활 액션이 막힌 사유 — 세 진행 플래그·완료 래치를 하나의 안내 문구로.
function daemonActionBlockedMsg(): string {
  if (resetCompleted)
    return "완전 초기화가 끝났습니다 — 앱을 종료하세요. 다시 실행하면 설치 온보딩이 시작됩니다.";
  if (factoryResetting) return "완전 초기화가 진행 중입니다 — 끝날 때까지 기다려 주세요.";
  if (purgingDept) return "부서 완전 삭제가 진행 중입니다 — 잠시 후 다시 시도하세요.";
  return "데몬 교대·재시작이 진행 중입니다 — 잠시 후 다시 시도하세요.";
}

// 데몬을 새로 만들거나 되살리는 액션의 공통 진입 가드(true면 차단·안내 완료).
function daemonActionBlocked(): boolean {
  if (rotatingDaemon || purgingDept || factoryResetting || resetCompleted) {
    toast("feed", "실행 불가", daemonActionBlockedMsg());
    return true;
  }
  return false;
}
let verSkewBadge: HTMLElement | null = null;
let skewNoticeShown = false; // C: 세션당 1회 능동 안내 플래그(스큐 해소 시 리셋)

interface SkewedDept {
  name: string;
  socket: string;
}

// rotate_daemon/rotate_dept_daemon 래퍼 — force=false면 백엔드가 세션>0 시 "live_sessions:N"로 거부(=보류).
// skipDrain: verified 재시작 경로만 true(사전 drain --verify로 저장 확인됨) — 기본 false는 plain drain(회귀 0).
async function rotateMainDaemon(force: boolean, skipDrain = false): Promise<"ok" | "held" | "err"> {
  try {
    await invoke("rotate_daemon", { force, skipDrain });
    return "ok";
  } catch (e) {
    return String(e).includes("live_sessions:") ? "held" : "err";
  }
}
async function rotateDeptDaemon(name: string, force: boolean, skipDrain = false): Promise<"ok" | "held" | "err"> {
  try {
    await invoke("rotate_dept_daemon", { name, force, skipDrain });
    return "ok";
  } catch (e) {
    return String(e).includes("live_sessions:") ? "held" : "err";
  }
}

// 메인+부서 데몬 버전 스큐 감지. 부서 열거=list_depts(레지스트리 SOT — 열린 탭 무관·Windows pipe 포함).
async function detectSkew(
  appVer: string,
): Promise<{ mainSkew: boolean; daemonVer: string; skewedDepts: SkewedDept[] }> {
  let daemonVer = "";
  let mainSkew = false;
  try {
    const st = (await invoke("daemon_status")) as Record<string, unknown>;
    daemonVer = String(st.version ?? "");
    mainSkew = !!(daemonVer && daemonVer !== appVer);
  } catch {
    /* 조회 실패=판정 보류(보수적) */
  }
  // ★F3(리뷰): 부서 열거를 레지스트리 SOT(list_depts)로 — name+socket을 레지스트리에서 직접 얻어
  // deptNameFromSocket(unix 전용 정규식) 의존을 없앤다(Windows named pipe 우회). 죽은 등재 항목은
  // daemon_status(socket) 실패로 skip돼 무해.
  const reg = (await invoke("list_depts").catch(() => ({ depts: {} }))) as {
    depts?: Record<string, { socket?: string }>;
  };
  const skewedDepts: SkewedDept[] = [];
  for (const [name, meta] of Object.entries(reg.depts ?? {})) {
    const socket = meta.socket;
    if (!socket) continue;
    try {
      const st = (await invoke("daemon_status", { socket })) as Record<string, unknown>;
      const dv = String(st.version ?? "");
      if (dv && dv !== appVer) skewedDepts.push({ name, socket });
    } catch {
      /* 죽은/전이 중 부서 소켓 skip(무해) */
    }
  }
  return { mainSkew, daemonVer, skewedDepts };
}

function clearSkewBadge() {
  if (verSkewBadge) {
    verSkewBadge.remove();
    verSkewBadge = null;
  }
  skewNoticeShown = false;
}

// 보류(세션>0) 노드만 배지에 반영(멱등 갱신·이미 있으면 갱신·없으면 생성). 부서 스큐 개수 병기.
function showSkewBadge(
  info: HTMLElement,
  appVer: string,
  heldMain: boolean,
  daemonVer: string,
  heldDepts: SkewedDept[],
) {
  if (!verSkewBadge || !verSkewBadge.isConnected) {
    verSkewBadge = document.createElement("span");
    verSkewBadge.className = "ver-skew-badge";
    info.appendChild(verSkewBadge);
  }
  const suffix = heldDepts.length ? ` (+부서 ${heldDepts.length}개)` : "";
  verSkewBadge.textContent = heldMain
    ? `데몬 v${daemonVer} · 앱 v${appVer}${suffix} — 세션 보존 중`
    : `앱 v${appVer} · 부서 ${heldDepts.length}개 구버전 — 세션 보존 중`;
  verSkewBadge.title =
    "업데이트가 적용됐지만 실행 중인 세션(마스터·워커·부서)을 보존하기 위해 기존 데몬이 계속 봉사합니다.\n" +
    "클릭하면 저장(drain) 후 새 버전으로 순차 교대(메인→부서)하고 세션을 복원합니다.";
  verSkewBadge.onclick = () => void manualRotateSkewed(appVer, heldMain, heldDepts);
}

// 배지 클릭(수동) — 확인 1회 후 force=true로 순차 교대(메인→부서). app.restart 없는 경로라 토스트까지 책임.
async function manualRotateSkewed(appVer: string, heldMain: boolean, heldDepts: SkewedDept[]) {
  // ★A3(성찰 확정): purge·완전 초기화 중에도 막는다 — 이 경로는 rotate_daemon→ensure_daemon 으로
  // cysd 를 **되살리므로**, 리셋 진행 중 클릭되면 격리와 경합하거나 리셋을 반토막으로 중단시킨다.
  if (rotatingDaemon || purgingDept || factoryResetting || resetCompleted) {
    // 리뷰 2R MIN-C: 자동 교대·주기 재검 진행 중 클릭이 조용히 무시돼 "안 눌림"으로 보이던 무피드백 해소.
    toast("feed", "교대 불가", daemonActionBlockedMsg());
    return;
  }
  const nodes = (heldMain ? 1 : 0) + heldDepts.length;
  const ok = await confirmModal(
    `데몬 교대 (새 버전 v${appVer})`,
    `작업 세션이 물려 있는 데몬 ${nodes}개를 새 버전으로 순차 교대(메인→부서)합니다. 저장(drain) 신호 후 ` +
      `교대하고 세션을 복원합니다. 마지막 미저장분은 손실될 수 있습니다.\n\n지금 교대하시겠습니까?`,
    "교대",
  );
  if (!ok) return;
  rotatingDaemon = true;
  stickyToast("rotate-daemon", "feed", "↻ 데몬 교대", `새 버전 v${appVer}로 교대 중… 저장 후 세션을 복원합니다.`);
  try {
    if (heldMain) await invoke("rotate_daemon", { force: true, skipDrain: false });
    // 경미2: rotate_dept_daemon이 반환하는 restore_ok=false(교대 후 부서 노드 복원 실패)를 삼키지 않고 승격.
    let deptRestoreFailed = false;
    for (const d of heldDepts) {
      const info = (await invoke("rotate_dept_daemon", { name: d.name, force: true, skipDrain: false })) as { restore_ok?: boolean };
      if (info?.restore_ok === false) deptRestoreFailed = true;
    }
    dismissToast("rotate-daemon");
    clearSkewBadge();
    if (deptRestoreFailed)
      toast("health", "⚠ 교대 후 부서 복원 실패", `데몬은 v${appVer}로 교대됐으나 일부 부서 노드 복원이 실패했습니다 — 상태를 점검하세요.`);
    else toast("watchdog", "✅ 데몬 교대 완료", `데몬이 v${appVer}로 교대됐습니다. 노드 복원이 진행됩니다.`);
  } catch (e) {
    dismissToast("rotate-daemon");
    toast("health", "데몬 교대 실패", String(e));
  } finally {
    rotatingDaemon = false;
  }
}

// ── drain --verify 결과 타입(cys 코어 결정론 JSON) ──
type DrainVerifyNode = {
  role: string;
  department?: string;
  surface: string;
  outcome: string; // saved | timeout | delivery_failed | unverifiable | skipped_restoring
  detail: string;
  pending_undelivered?: number;
};
type DrainVerifyReport = {
  all_saved: boolean;
  total: number;
  summary: { saved: number; timeout: number; delivery_failed: number; unverifiable: number; skipped_restoring: number };
  nodes: DrainVerifyNode[];
  pending_loss_warning?: { role: string; surface: string; pending_undelivered: number }[];
};

// ★[F2] 검증은 nonce '마커 기입'만 확인한다 — 노드가 마커 앞에서 SESSION_STATE 내용을 실제로 최신화했는지는
// 보증하지 못한다(형식적 순응 한계·내용 최신성은 노드 책임). 라벨을 "마커 확인"으로 완화해 과대주장 금지.
const OUTCOME_LABEL: Record<string, string> = {
  saved: "체크포인트 마커 확인",
  timeout: "마커 미확인(시간초과)",
  delivery_failed: "지시 전달 실패(입력 미제출)",
  unverifiable: "검증 불가(구버전 데몬)",
  skipped_restoring: "복원 중 — 건너뜀",
};

// 부분 실패 노드를 사람이 읽을 리포트로. 마커 확인된 노드는 생략, 미확인만 나열한다.
function drainVerifyReportText(r: DrainVerifyReport): string {
  const bad = r.nodes.filter((n) => n.outcome !== "saved");
  const lines = bad.map(
    (n) => `• ${n.department ? n.department + " / " : ""}${n.role} (${n.surface}): ${OUTCOME_LABEL[n.outcome] ?? n.outcome}`,
  );
  return (
    `${r.total}개 노드 중 ${r.summary.saved}개만 체크포인트 마커가 확인됐습니다.\n\n` +
    `${lines.join("\n")}\n\n` +
    "대화 원문은 재시작 후 트랜스크립트로 복원됩니다. 이 확인은 마커 기입만 보증하며, 위 노드를 포함해 각 노드의 증류 체크포인트(SESSION_STATE·TODO) 내용 최신성은 노드 책임입니다.\n\n" +
    "그래도 지금 재시작하시겠습니까?"
  );
}

// 데몬 재시작 코어 — 메인 + 살아있는 부서를 force 순차 교대한다(종료 → 새 데몬 기동 → 피닉스·resume 복원).
// skipDrain=true면 rotate가 이중 drain을 생략(verified 경로: 사전 drain --verify로 저장 확인됨),
// false면 rotate가 plain drain(기존 거동·폴백). 앱 재시작 없음 — GUI는 새 데몬에 자동 재연결.
// 죽은 부서 소켓은 skip(detectSkew 동형 — 부서 부활은 CSO·피닉스 소유).
async function restartAllDaemons(skipDrain: boolean): Promise<{ failedDepts: string[]; deptRestoreFailed: boolean }> {
  await invoke("rotate_daemon", { force: true, skipDrain });
  // 부서 열거=list_depts(레지스트리 SOT) + daemon_status 생존 확인 — detectSkew 동형(죽은 등재 skip).
  const reg = (await invoke("list_depts").catch(() => ({ depts: {} }))) as {
    depts?: Record<string, { socket?: string }>;
  };
  let deptRestoreFailed = false;
  const failedDepts: string[] = [];
  for (const [name, meta] of Object.entries(reg.depts ?? {})) {
    if (!meta.socket) continue;
    try {
      await invoke("daemon_status", { socket: meta.socket });
    } catch {
      continue; // 죽은/전이 중 부서 소켓 skip(무해)
    }
    try {
      const info = (await invoke("rotate_dept_daemon", { name, force: true, skipDrain })) as { restore_ok?: boolean };
      if (info?.restore_ok === false) deptRestoreFailed = true;
    } catch {
      failedDepts.push(name);
    }
  }
  return { failedDepts, deptRestoreFailed };
}

function restartResultToast(failedDepts: string[], deptRestoreFailed: boolean) {
  if (failedDepts.length)
    toast("health", "⚠ 일부 부서 재시작 실패", `메인 데몬은 재시작됐으나 부서 교대가 실패했습니다: ${failedDepts.join(", ")} — 상태를 점검하세요.`);
  else if (deptRestoreFailed)
    toast("health", "⚠ 재시작 후 부서 복원 실패", "데몬은 재시작됐으나 일부 부서 노드 복원이 실패했습니다 — 상태를 점검하세요.");
  else toast("watchdog", "✅ 데몬 재시작 완료", "데몬이 다시 시작됐습니다. 부서·노드 복원이 진행됩니다.");
}

// ── 상시 "↻ 재시작" 버튼(초보자용) — "저장 검증 후 자동 재시작" 흐름 ──
// 1) 확인 모달[저장 후 재시작/취소] → 2) drain --verify(feature-detect)로 노드별 체크포인트 저장 검증
// → 3) green(all_saved)이면 자동 진행(skipDrain=true), 부분 실패면 노드별 리포트+[그래도 재시작/취소].
// cys 코어가 --verify를 미지원하면(구버전) plain drain 폴백(skipDrain=false)+경고. '무손실' 표현 금지 —
// 대화 원문은 트랜스크립트 복원, 이 기능은 증류 체크포인트(SESSION_STATE·TODO) 최신성만 보증한다.
async function manualRestartAllDaemons() {
  // ★[F3]+A6: purge·완전 초기화 진행 중이거나 **초기화 완료 래치**가 걸렸으면 재시작을 막는다
  // (완료 후 재시작은 pack 없는 유령 데몬을 세워 "설치 직후" 계약을 무음 침식한다).
  if (daemonActionBlocked()) return;
  const ok = await confirmModal(
    "데몬 재시작",
    "재시작 전에 각 노드의 체크포인트(SESSION_STATE·TODO) 저장을 먼저 검증합니다. 검증이 끝나면 데몬(메인+부서)을 " +
      "다시 켜고 부서·노드의 대화 기억을 트랜스크립트로 복원합니다.\n\n지금 저장을 검증하고 재시작하시겠습니까?",
    "저장 후 재시작",
  );
  if (!ok) return;
  rotatingDaemon = true;
  try {
    // 1) 저장 검증(drain --verify) — feature-detect. 미지원/실패 시 plain drain 폴백.
    stickyToast("restart-daemon", "feed", "↻ 저장 검증", "재시작 전 노드 체크포인트(SESSION_STATE)를 검증하는 중…");
    let verify: DrainVerifyReport | null = null;
    // [F5] 폴백 사유 분기: "unsupported"(구버전 미지원) vs "verify_failed"(크래시/하드캡). 둘 다 plain
    // drain 폴백(skipDrain=false)이나 UI 문구는 정직하게 다르게 표기한다("무손실" 표현 없음).
    let fallback: ReturnType<typeof classifyDrainVerifyFallback> = null;
    try {
      verify = (await invoke("drain_verify", { timeout: 20 })) as DrainVerifyReport;
    } catch (e) {
      fallback = classifyDrainVerifyFallback(String(e));
      if (!fallback) throw e; // 알 수 없는 에러는 상위 catch로
    }
    if (fallback) {
      dismissToast("restart-daemon");
      const t = drainVerifyFallbackToast(fallback);
      toast("health", t.title, t.body);
      stickyToast("restart-daemon", "feed", "↻ 데몬 재시작", "저장 후 데몬을 다시 시작하는 중… 부서·노드를 자동 복원합니다.");
      const { failedDepts, deptRestoreFailed } = await restartAllDaemons(false);
      dismissToast("restart-daemon");
      restartResultToast(failedDepts, deptRestoreFailed);
      return;
    }
    // 2) 부분 실패면 노드별 리포트 + [그래도 재시작/취소]. green(all_saved)이면 자동 진행.
    if (verify && !verify.all_saved) {
      dismissToast("restart-daemon");
      const proceed = await confirmModal("일부 노드 저장 미확인", drainVerifyReportText(verify), "그래도 재시작");
      if (!proceed) {
        rotatingDaemon = false;
        return;
      }
    }
    // 3) verified 재시작 — 사전 검증했으므로 rotate는 이중 drain 생략(skipDrain=true).
    stickyToast("restart-daemon", "feed", "↻ 데몬 재시작", "저장 검증 완료 — 데몬을 다시 시작하고 노드를 복원하는 중…");
    const { failedDepts, deptRestoreFailed } = await restartAllDaemons(true);
    dismissToast("restart-daemon");
    // 재시작 창 큐 보존[A3-F3]: 인메모리 미배달 push는 재시작에 유실된다 — 정직하게 고지(무음 유실 금지).
    const pendingLost = (verify?.pending_loss_warning ?? []).reduce((a, p) => a + (p.pending_undelivered || 0), 0);
    if (pendingLost > 0)
      toast("health", "미배달 push 유실", `재시작으로 미배달 큐 ${pendingLost}건이 유실됩니다(대화 원문은 트랜스크립트로 복원).`);
    restartResultToast(failedDepts, deptRestoreFailed);
  } catch (e) {
    dismissToast("restart-daemon");
    toast("health", "데몬 재시작 실패", String(e));
  } finally {
    rotatingDaemon = false;
  }
}

// 시작 시 1회 + 5분 주기(B) — 스큐 재검·배지 멱등 갱신·무손실 자동 교대·1회 능동 안내(C).
async function checkVersionSkew() {
  if (rotatingDaemon || purgingDept || factoryResetting || resetCompleted) return; // 교대·purge·초기화 진행/완료 중 중복 발동 방지(주기 타이머·수동 클릭) [F3+A6]
  let appVer = "";
  try {
    appVer = (await invoke("app_version")) as string;
  } catch {
    return;
  }
  if (!appVer) return;
  const info = document.getElementById("daemon-info");
  if (!info) return;
  const { mainSkew, daemonVer, skewedDepts } = await detectSkew(appVer);
  if (!mainSkew && skewedDepts.length === 0) {
    clearSkewBadge();
    return;
  }
  // 무손실 자동 교대(세션 0인 노드만 — 백엔드 게이트가 force=false로 판정). 보류(세션>0·"held")만 남긴다.
  // ★F3(리뷰): "held"(세션 보유 보류)뿐 아니라 "err"(카운트·교대 실패)도 배지 대상에 포함 —
  // 스큐가 사용자에게 계속 보이게(구 배지 가시성 보존). 실패는 다음 tick 재검·재시도로 자가 교정된다.
  // 참고: F1로 카운트 실패는 "live_sessions:unknown"→래퍼가 "held" 분류, "err"는 그 외 교대 실패.
  let heldMain = false;
  const heldDepts: SkewedDept[] = [];
  rotatingDaemon = true;
  try {
    if (mainSkew) {
      const r = await rotateMainDaemon(false);
      if (r === "held" || r === "err") heldMain = true;
    }
    for (const d of skewedDepts) {
      const r = await rotateDeptDaemon(d.name, false);
      if (r === "held" || r === "err") heldDepts.push(d);
    }
  } finally {
    rotatingDaemon = false;
  }
  if (!heldMain && heldDepts.length === 0) {
    clearSkewBadge(); // 전부 무손실 자동 교대됨 — 배지 없음
    return;
  }
  showSkewBadge(info, appVer, heldMain, daemonVer, heldDepts);
  if (!skewNoticeShown) {
    // C: 자동 교대가 보류/실패로 남을 때 1회 안내(sticky 아님 — 8초 auto-dismiss)
    skewNoticeShown = true;
    toast("feed", "새 버전 준비", `새 버전 v${appVer} 준비 — 상태바 배지를 눌러 저장 후 교대하세요.`);
  }
}

/// 무중단 팩 설치 — install_pack_update(세션·데몬 생존, app.restart 없음) 호출.
/// 진행/완료/경고는 pack-progress·pack-updated·update-warning 리스너가 표시한다(아래 startup).
/// ★"재시작" 확인 다이얼로그를 띄우지 않는다 — 세션이 죽지 않는 게 바이너리 경로와의 핵심 차이.
async function promptPackInstall() {
  // ★A7: 팩 설치는 격리로 이동 중인 ~/.cys/pack 을 재생성한다 — 리셋 진행/완료 중 금지.
  if (daemonActionBlocked()) return;
  if (!packUpdateAvailable) {
    await checkForUpdate(false);
    return;
  }
  const pv = packUpdateAvailable.pack_version;
  // 지속형 토스트: pack-progress 리스너가 갱신하고 pack-updated/update-warning이 dismiss한다.
  stickyToast("upd-pack", "feed", "↻ 무중단 팩 업데이트", `팩 ${pv} 적용 중… 세션·데몬 유지(재시작 없음).`);
  try {
    await invoke("install_pack_update", { manifestUrl: packUpdateAvailable.manifest_url });
    // 성공(또는 degraded)은 pack-updated/update-warning 리스너가 후속 처리(sticky도 거기서 dismiss).
  } catch (e) {
    dismissToast("upd-pack"); // 완료 이벤트 없이 reject된 경로 — 진행 토스트를 내린다.
    // 백엔드가 update-error도 emit하지만, join/실행 단계 실패는 emit 없이 reject되므로 여기서 표시.
    toast("health", "팩 업데이트 실패", String(e));
  }
}

/// Update 버튼 디스패처 — 가용 업데이트 종류에 따라 경로를 고른다.
/// 본체(바이너리)=패치 설치(오너 2026-07-15 재배선·재시작+자동복원) → 무중단 팩 → 미확인 시 수동 재확인.
async function onUpdateButton() {
  if (updateAvailable) return promptBinaryPatch();
  if (packUpdateAvailable && !packUpdateAvailable.binary_too_old) return promptPackInstall();
  return checkForUpdate(false);
}

/// 간단한 확인 모달 (WKWebView confirm 회피). resolve(true/false).
// ───────── 07 Command Palette (⌘K) — 순수 DOM 오버레이 + fuzzy + 액션 큐레이션 ─────────
// 흡수: 팔레트 메커니즘(모달·fuzzy·키 라우팅)=webview primitive. 액션 큐레이션(역할 점프·재기동·60% cycle·feed 승인)=cysjavis 처방 solution.
// org_status Tauri 커맨드(src-tauri/main.rs:171)·기존 setFocus/confirmModal/send_input/feed_list/feed_reply 재사용. 데몬 무변경.

// 팔레트 1개 행 — cmux 액션 스키마(title/subtitle/keywords/confirm) adapt.
interface PaletteItem {
  id: string; // 안정 키(중복 dedupe·테스트용). 예: "node:<socket>#<sid>", "act:restart-cso"
  title: string; // 표시 라벨(역할/제목/액션명)
  subtitle?: string; // 보조 설명(surface_ref·idle·context_pct 등)
  keywords?: string; // 추가 검색어(role 별칭·한글/영문 동의어). title+subtitle+keywords가 매칭 대상
  action: () => void | Promise<void>;
  confirm?: { title: string; body: string }; // 있으면 실행 전 confirmModal 통과 요구(파괴적 액션)
}

// org.status surface 1개의 webview 타입(필요 필드만 — 데몬 핸들러 handlers.rs org.status arm와 일치)
interface OrgSurface {
  surface_id: number;
  surface_ref: string; // "surface:N"
  role: string | null;
  title: string | null;
  idle_secs: number;
  agent: string | null;
  agent_alive: boolean | null;
  status: { state: string; context_pct: number | null; task: string | null; age_secs: number } | null;
}

// 쿼리 문자가 순서대로 부분 등장하면 매치. 점수 = 연속 매치 보너스 + 시작 보너스(낮을수록 우위는 -로 정렬).
// 반환 null = 비매치. 공백 쿼리는 전부 매치(score 0). 의존성 0(서브시퀀스 매처 자체 구현).
function fuzzyScore(query: string, text: string): number | null {
  const q = query.toLowerCase().trim();
  if (q === "") return 0;
  const t = text.toLowerCase();
  let qi = 0,
    score = 0,
    run = 0,
    prevIdx = -1;
  for (let ti = 0; ti < t.length && qi < q.length; ti++) {
    if (t[ti] === q[qi]) {
      run = prevIdx === ti - 1 ? run + 1 : 0;
      score += 1 + run * 2 + (ti === 0 ? 3 : 0); // 연속·선두 가중
      prevIdx = ti;
      qi++;
    }
  }
  return qi === q.length ? -score : null; // 전부 매치해야 통과, 음수=좋을수록 작음
}

function filterPalette(items: PaletteItem[], query: string): PaletteItem[] {
  const scored: { it: PaletteItem; s: number }[] = [];
  for (const it of items) {
    const hay = `${it.title} ${it.subtitle ?? ""} ${it.keywords ?? ""}`;
    const s = fuzzyScore(query, hay);
    if (s !== null) scored.push({ it, s });
  }
  scored.sort((a, b) => a.s - b.s); // 음수 점수 오름차순 = 높은 매치 우선
  return scored.map((x) => x.it);
}

// 점프: 다른 ws일 수 있으므로 ws 전환 2단계(ws-tab mousedown 레퍼런스 차용). setFocus(기존)는 활성 ws의 pane만 본다.
function jumpToSurface(sid: number, socket?: string) {
  const wsIdx = workspaces.findIndex(
    (w) => (w.socket ?? undefined) === (socket ?? undefined) && collectSids(w.tree).includes(sid),
  );
  if (wsIdx >= 0 && wsIdx !== activeWs) {
    activeWs = wsIdx;
    render();
  }
  setFocus(sid); // 현재 활성 ws의 pane만 잡으므로 전환 후 호출
}

// ★#4-b: 부서 워크스페이스(socket)로 전환한다 — surface 를 특정하지 않는 판(jumpToSurface 형제).
// 승인 Feed 의 '이 부서로 이동'이 쓴다. 결과를 **세 갈래로 사실대로** 돌려 호출부가 오안내하지
// 않게 한다 — 조용히 아무 일도 안 하거나 틀린 사유를 말하면 '눌러도 안 되는 버튼'이 된다.
//   "switched" = 그 워크스페이스가 활성이다(이미 활성이었던 경우 포함 — 결과 상태가 같다)
//   "pending"  = 부서 데몬 기동 중인 placeholder 로 전환했다(★F6② — '닫힌 탭'이 **아니다**)
//   "missing"  = 그 socket 의 탭이 실제로 없다(레지스트리 잔재)
function switchToWorkspaceBySocket(socket: string): DeptSwitchOutcome {
  // 판정(어느 탭이며 세 갈래 중 무엇인가)은 `deptlabel.ts` 의 순수 `pickDeptWorkspace` 가
  // 한다 — 여기 남는 것은 **부작용**(활성 전환·재그림·포커스)뿐이다. 판정을 순수부로 뺀
  // 이유는 이 함수가 모듈-private 이라 핀이 닿지 못했기 때문이다(deptlabel.test.ts 가 박제).
  const { outcome, index } = pickDeptWorkspace(workspaces, socket);
  if (outcome === "missing") return outcome;
  if (index !== activeWs) {
    activeWs = index;
    render();
    // pending placeholder 에는 pane 이 없다(collectSids=[]) — setFocus 는 자연히 생략된다.
    const first = collectSids(current().tree)[0];
    if (first != null) setFocus(first); // ws-tab mousedown 과 동일 2단계(전환 후 포커스)
  }
  return outcome;
}

// 60% cycle: hot 노드를 순차 점프(모듈 전역 cursor로 라운드로빈).
let hotCycleCursor = 0;
function cycleHotNodes(hot: OrgSurface[], socket?: string) {
  if (hot.length === 0) return;
  const s = hot[hotCycleCursor % hot.length];
  hotCycleCursor++;
  jumpToSurface(s.surface_id, socket);
  toast("feed", "60% cycle", `${s.role} · ctx ${s.status?.context_pct}% (${hotCycleCursor % hot.length || hot.length}/${hot.length})`);
}

// 재기동: role의 첫 surface로 명령+개행 주입(send_input human=true 재사용, data에 "\n"으로 원자 제출 — 계약 변경 금지).
// ★R5 machineOrigin: 이 명령문은 UI 코드가 만든 것이므로 배달 원장에 기록돼야 한다(자동 제출까지
// 하므로 대상 pane 의 훅이 그대로 프롬프트로 본다 — 표식이 없으면 오너 임무로 기록된다).
async function restartNode(role: string, cmd: string, surfaces: OrgSurface[], socket?: string) {
  const target = surfaces.find((s) => s.role === role && !(s.status?.state === "offline"));
  if (!target) {
    toast("watchdog", "재기동 실패", `${role} 노드 없음`);
    return;
  }
  jumpToSurface(target.surface_id, socket);
  await invoke("send_input", {
    socket: socket ?? null,
    surfaceId: target.surface_id,
    data: cmd + "\n",
    machineOrigin: true,
  });
}

// feed 승인(팔레트 액션): **대상이 확정된 뒤에만** 노출한다 — 아래 buildPaletteItems 가
// feed_list 로 실제 대상을 뽑아 confirm 본문에 kind·title·request_id 를 박고, 그 항목의
// request_id 를 이 함수에 넘긴다.
//
// ★종전 설계의 결함 3종(적대검증 2R major — 전부 이 라운드에서 수리):
//  ① **맹목 승인**: 인자 없이 호출돼 실행 시점에 스스로 [0] 을 골랐고 confirm 본문은 "가장
//    오래된 pending 요청을 Allow 합니다." 뿐이었다 — 오너는 **무엇을 승인하는지 모른 채**
//    눌렀다. 팔레트 조회와 실행 사이에 새 항목이 끼어들면 대상이 바뀌기까지 했다.
//  ② **데몬 감지 항목 오승인**: 데몬이 화면 패턴으로 올린 approval 은 waiter 가 없어 Allow 가
//    앱에 아무 영향도 못 준다(그래서 CC 패널에서는 Allow 를 없애고 '점프+치우기'로 바꿨다).
//    그런데 이 액션은 같은 항목을 그대로 allow 로 소각해, 패널에서 없앤 기만 버튼이 팔레트에
//    살아 있었다. 이제 아래 게이트가 그 항목을 **대상에서 제외**한다(패널과 동일 술어 재사용).
//  ③ **소켓 불일치**: 노출 게이트는 활성 ws 소켓의 org_status.feed.pending 이었는데 실제
//    feed_reply 는 **기본 데몬** 고정이다(src-tauri feed_reply = default_socket()). 부서 ws 에서
//    누르면 무관한 본부 항목이 승인됐다. 이제 게이트도 실행과 같은 기본 데몬 feed_list 다.
async function approveFeedItem(requestId: string) {
  try {
    await invoke("feed_reply", { requestId, decision: "allow" });
    toast("feed", "✅ feed 승인", requestId);
  } catch (e) {
    // 종전엔 await 결과를 버려 실패가 무음이었다(거부 시 unhandled rejection) — 사유를 표시한다.
    toast("health", "feed 승인 실패", feedReplyErrorText(e));
  } finally {
    refreshFeed();
    refreshSidebarStatus(); // 승인 직후 집계 배지 즉시 갱신
  }
}

// org.status로 노드 행 생성 + 빌트인 액션 행 추가. socket = 활성 ws socket(1차: 단일 소켓).
async function buildPaletteItems(): Promise<PaletteItem[]> {
  const items: PaletteItem[] = [];
  const sock = current()?.socket; // undefined=기본 데몬
  let org: { surfaces?: OrgSurface[]; feed?: { pending: number } } = {};
  try {
    org = (await invoke("org_status", { socket: sock ?? null })) as { surfaces?: OrgSurface[]; feed?: { pending: number } };
  } catch {
    /* 데몬 미응답시 노드행 생략 — 빌트인 액션은 항상 표시 */
  }

  // ── (1) 노드 점프 행 ──
  for (const s of org.surfaces ?? []) {
    const role = s.role ?? "";
    const ctx = s.status?.context_pct;
    const label = `${role || "(no role)"} · ${s.title ?? s.surface_ref}`;
    const sub =
      `${s.surface_ref} · idle ${s.idle_secs}s` +
      (ctx != null ? ` · ctx ${ctx}%` : "") +
      (s.status?.task ? ` · ${s.status.task}` : "");
    items.push({
      id: `node:${sock ?? ""}#${s.surface_id}`,
      title: `점프 → ${label}`,
      subtitle: sub,
      keywords: `jump goto ${role} ${s.surface_ref} ${s.title ?? ""}`,
      action: () => jumpToSurface(s.surface_id, sock),
    });
  }

  // ── (2) 60% 노드 cycle ──
  const hot = (org.surfaces ?? []).filter((s) => (s.status?.context_pct ?? 0) >= 60);
  if (hot.length > 0) {
    items.push({
      id: "act:cycle-60",
      title: `60% 노드 cycle (${hot.length})`,
      subtitle: hot.map((s) => `${s.role}·${s.status?.context_pct}%`).join(", "),
      keywords: "cycle context 60 hot 컨텍스트 순환",
      action: () => cycleHotNodes(hot, sock),
    });
  }

  // ── (3) 노드 재기동(명령 주입) — role별 처방. 파괴적이므로 confirm. ──
  const RESTART: Record<string, string> = {
    cso: "cys launch-agent --role cso --agent claude",
    worker: "cys launch-agent --role worker --agent claude",
    "reviewer-gemini": "agy --dangerously-skip-permissions",
    "reviewer-codex": "codex --dangerously-bypass-approvals-and-sandbox",
  };
  for (const [role, cmd] of Object.entries(RESTART)) {
    items.push({
      id: `act:restart-${role}`,
      title: `재기동 → ${role}`,
      subtitle: cmd,
      keywords: `restart relaunch reboot 재기동 ${role}`,
      confirm: { title: `${role} 재기동`, body: `${role} 노드에 다음 명령을 주입합니다:\n${cmd}` },
      action: () => restartNode(role, cmd, org.surfaces ?? [], sock),
    });
  }

  // ── (4) feed 승인(가장 오래된 응답 가능 pending Allow) ──
  // ★게이트를 org_status(활성 ws 소켓 집계)에서 **기본 데몬 feed_list** 로 바꿨다 —
  //   feed_reply 가 기본 데몬 고정이므로(src-tauri feed_reply = default_socket()) 게이트와
  //   실행의 소켓이 같아야 한다. 종전엔 부서 ws 를 보고 있는데 본부 항목이 승인됐다.
  // ★대상을 **여기서 확정**해 confirm 본문에 kind·title·request_id 를 적는다(맹목 승인 제거).
  //   조회~실행 사이에 새 항목이 끼어들어도 승인 대상은 여기서 고른 그 항목 하나다.
  // ★데몬 감지 항목은 제외한다 — Allow 가 앱에 닿지 않아(waiter 없음) 사용자를 속이는 버튼이
  //   되기 때문이고, CC 패널이 같은 이유로 이미 Allow 를 없앴다(술어 공유 =
  //   classifyPendingFeed · feedclass.ts). 그 항목의 처리 경로는 패널의 '점프 / 알림 치우기'다.
  // ★W4-B(결함 7): cycle-verify 도 제외한다 — 판정자는 지정 검증자 pane 뿐이고, GUI/팔레트
  //   Allow 는 영수증(resolver) 없는 소모가 되어 cycle 을 안전 중단시킨다(기만 버튼 동일 계급 —
  //   근거는 feedclass.ts 주석). "standard" 만 승인 가능하다.
  const feedPending = ((await invoke("feed_list", { status: "pending" }).catch(() => null)) as
    | { items: FeedItem[] }
    | null)?.items ?? [];
  // feed.list 는 삽입순(handlers.rs items.iter()) → [0] = 가장 오래된.
  const approvable = feedPending.filter(
    (i) => i.status === "pending" && classifyPendingFeed(i) === "standard",
  );
  const oldestApprovable = approvable[0];
  if (oldestApprovable) {
    const rid = oldestApprovable.request_id;
    items.push({
      id: "act:feed-approve",
      title: `feed 승인 (응답 가능 ${approvable.length})`,
      subtitle: `${oldestApprovable.kind} · ${oldestApprovable.title}`,
      keywords: "feed approve allow 승인 피드 대기",
      confirm: {
        title: "feed 승인",
        body:
          `다음 요청 1건을 Allow 합니다(기본 데몬):\n` +
          `· kind: ${oldestApprovable.kind}\n` +
          `· 제목: ${oldestApprovable.title}\n` +
          `· request_id: ${rid}\n` +
          (oldestApprovable.surface_id != null ? `· surface: ${oldestApprovable.surface_id}\n` : "") +
          `\n본문: ${oldestApprovable.body.slice(0, 300)}`,
      },
      action: () => approveFeedItem(rid),
    });
  }

  // ── (4-b) ★R8(WP-2): CEO 승격 대기 해소 — cys-dept PENDING(부트 게이트 보류)의 즉시 경로.
  // ── (4-c) ★D4(v4 · 결정 D4): CEO 승격 재실행(템플릿 전진 적용) — 노출 게이트=[.pre-ceo 존재
  // ∧ md≠라이브 CEO_TEMPLATE](ceo_promotion_drift · 파일 실측). 템플릿 전진 릴리스 후 구본화된
  // 승격본을 갱신하는 오너 GUI 경로(종전엔 CLI promote-ceo 재실행이 유일 — 막다른 흐름 제거).
  // 둘 다 온디맨드 조회(팔레트 열 때만 — 신규 타이머 0)·상호 배타 노출(pending 우선 — 결정
  // 로직은 selfdiag.ceoPaletteEntries 순수 함수에 고정, 회귀 핀 selfdiag.test.ts).
  const [ceoPend, ceoDrift] = await Promise.all([
    invoke("ceo_pending").catch(() => false),
    invoke("ceo_promotion_drift").catch(() => false),
  ]);
  for (const ceoEntry of ceoPaletteEntries({ pending: ceoPend === true, drift: ceoDrift === true })) {
    if (ceoEntry === "pending") {
      // 대기형은 오너 동의 게이트(feed --wait) 경유.
      items.push({
        id: "act:ceo-promote",
        title: "CEO 승격 진행 (대기 중)",
        subtitle: "부서가 존재·base 부트 완료 — 동의 게이트(feed)를 거쳐 승격합니다",
        keywords: "ceo promote 승격 pending 대기",
        confirm: { title: "CEO 승격", body: "기본 데몬 master를 CEO로 승격합니다(동의 요청이 feed에 뜹니다 · .pre-ceo 백업으로 가역)." },
        action: async () => {
          try {
            const r = (await invoke("promote_pending_ceo")) as string;
            toast("feed", "CEO 승격 처리", r || "완료");
          } catch (e) {
            toast("health", "CEO 승격 실패", String(e));
          }
        },
      });
    } else {
      // 재실행=approve_ceo_promotion(promote-ceo consented) 재사용 — _swap 이 기존 .pre-ceo 를
      // 보존한 채 새 템플릿만 md 에 적용(가역성 불파괴). 실패는 exit 5 truthful 표시 관례(:3963).
      items.push({
        id: "act:ceo-repromote",
        title: "CEO 승격 재실행 (템플릿 전진 적용)",
        subtitle: "릴리스 전진으로 승격본이 구본화됨(md≠라이브 CEO_TEMPLATE) — 새 템플릿을 재적용합니다",
        keywords: "ceo promote 승격 재실행 템플릿 전진 drift repromote",
        confirm: {
          title: "CEO 승격 재실행",
          body: "새 CEO_TEMPLATE를 MASTER_DIRECTIVE.md에 재적용합니다(.pre-ceo 백업은 보존 — 부서 0개 시 자동 강등 가역성 유지).",
        },
        action: async () => {
          try {
            const r = (await invoke("approve_ceo_promotion")) as string;
            toast("watchdog", "✅ CEO 승격 재실행 완료", r || "새 템플릿을 적용했습니다.");
          } catch (e) {
            toast("health", "CEO 승격 재실행 실패", String(e));
          }
        },
      });
    }
  }

  // ── (5) 빌트인 webview 액션(정적) ──
  items.push(
    { id: "act:new-tab", title: "새 탭", keywords: "new tab 탭", action: () => actionNew() },
    { id: "act:split-row", title: "가로 분할", keywords: "split row 분할", action: () => actionSplit("row") },
    { id: "act:split-col", title: "세로 분할", keywords: "split col 분할", action: () => actionSplit("col") },
    { id: "act:close", title: "패널 닫기", keywords: "close 닫기", action: () => actionClose() },
    { id: "act:equalize", title: "패널 균등화", keywords: "equalize 균등", action: () => actionEqualize() },
    { id: "act:cc", title: "Control Center 토글", keywords: "control center dashboard 대시보드", action: () => setCcOpen(!ccOpen) },
    { id: "act:feed-panel", title: "승인 Feed 탭 열기", keywords: "feed panel 피드 패널 승인 control center", action: () => openFeed() },
    { id: "act:dept", title: "부서 워크스페이스 추가 (독립 부서장·전용 데몬)", keywords: "dept workspace 부서 부서장 master", action: () => { if (daemonActionBlocked()) return; void addDeptWorkspace(); } },
  );
  return items;
}

let paletteOpen = false;
// 팔레트 모달 렌더 + 키보드. 패턴=showCtxMenu(window capture + 닫을 때 removeEventListener) + confirmModal 합성.
async function openPalette() {
  if (paletteOpen) return;
  paletteOpen = true;
  const all = await buildPaletteItems(); // 데몬 1콜
  let filtered = filterPalette(all, "");
  let sel = 0;

  const ov = document.createElement("div");
  ov.className = "palette-overlay";
  ov.innerHTML = `<div class="palette"><input class="palette-input" placeholder="노드·역할·액션 검색…" /><div class="palette-list"></div></div>`;
  const input = ov.querySelector(".palette-input") as HTMLInputElement;
  const list = ov.querySelector(".palette-list") as HTMLElement;

  const close = () => {
    paletteOpen = false;
    ov.remove();
    window.removeEventListener("keydown", onKey, true);
  };
  const renderRows = () => {
    list.innerHTML = "";
    filtered.slice(0, 50).forEach((it, i) => {
      const row = document.createElement("div");
      row.className = "palette-item" + (i === sel ? " sel" : "");
      const t = document.createElement("div");
      t.className = "pi-title";
      t.textContent = it.title; // textContent — XSS 가드(쿼리·노드 title)
      row.appendChild(t);
      if (it.subtitle) {
        const s = document.createElement("div");
        s.className = "pi-sub";
        s.textContent = it.subtitle;
        row.appendChild(s);
      }
      row.addEventListener("mousedown", (e) => {
        e.preventDefault();
        run(it);
      });
      list.appendChild(row);
    });
  };
  const run = async (it: PaletteItem) => {
    close(); // confirm 모달(z 1000)이 팔레트(z 1600) 아래로 가려지지 않게 먼저 닫음
    if (it.confirm && !(await confirmModal(it.confirm.title, it.confirm.body))) return;
    await it.action();
  };
  const onKey = (e: KeyboardEvent) => {
    if (e.isComposing || e.keyCode === 229) return; // 07: IME 조합 중 Enter가 액션 오발화 방지(적대검증 교정)
    if (e.key === "Escape") {
      e.preventDefault();
      close();
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      sel = Math.min(sel + 1, filtered.length - 1);
      renderRows();
      list.children[sel]?.scrollIntoView({ block: "nearest" });
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      sel = Math.max(sel - 1, 0);
      renderRows();
      list.children[sel]?.scrollIntoView({ block: "nearest" });
    } else if (e.key === "Enter") {
      e.preventDefault();
      if (filtered[sel]) run(filtered[sel]);
    }
  };
  input.addEventListener("input", () => {
    filtered = filterPalette(all, input.value);
    sel = 0;
    renderRows();
  });
  ov.addEventListener("mousedown", (e) => {
    if (e.target === ov) close();
  });
  window.addEventListener("keydown", onKey, true); // capture — xterm/모달 위에서 화살표/Enter 가로채기
  document.body.appendChild(ov);
  renderRows();
  input.focus();
}

// ★확인 버튼 라벨 매개변수화(오너 2026-07-15 실보고): 업데이트 창용 "설치" 하드코딩이 모든
// 확인 창에 노출(완전 삭제 창의 확인 버튼이 "설치"로 표시). 호출부가 동작 동사를 지정한다.
/// `noLabel` — 거절 버튼 라벨(기본 "아니오"). 완료 보고형 모달에서 "아니오"는 의미가 어긋나
/// "나중에" 처럼 상황에 맞는 말이 필요하다(P0-4 시뮬레이션 지적). 본문은 길어질 수 있어 스크롤한다.
function confirmModal(
  title: string,
  body: string,
  yesLabel = "확인",
  noLabel = "아니오",
): Promise<boolean> {
  return new Promise((resolve) => {
    const ov = document.createElement("div");
    ov.className = "modal-overlay";
    ov.innerHTML =
      `<div class="modal"><h3></h3><p style="white-space:pre-wrap;max-height:52vh;overflow-y:auto"></p>` +
      `<div class="modal-btns"><button class="modal-no"></button>` +
      `<button class="modal-yes"></button></div></div>`;
    (ov.querySelector("h3") as HTMLElement).textContent = title;
    (ov.querySelector("p") as HTMLElement).textContent = body;
    (ov.querySelector(".modal-yes") as HTMLElement).textContent = yesLabel;
    (ov.querySelector(".modal-no") as HTMLElement).textContent = noLabel;
    const done = (v: boolean) => {
      ov.remove();
      resolve(v);
    };
    ov.querySelector(".modal-yes")!.addEventListener("click", () => done(true));
    ov.querySelector(".modal-no")!.addEventListener("click", () => done(false));
    ov.addEventListener("click", (e) => {
      if (e.target === ov) done(false);
    });
    document.body.appendChild(ov);
  });
}

/// D6 제품 모드 입력 모달 (WKWebView prompt 회피·순수 DOM) — 본문 원고/주제 붙여넣기. resolve(text|null).
/// HITL 미리보기·신뢰선 라벨 보존(게이트 건너뛰기 금지). 빈 입력·취소는 null.
function inputModal(title: string, label: string, placeholder: string): Promise<string | null> {
  return new Promise((resolve) => {
    const ov = document.createElement("div");
    ov.className = "modal-overlay";
    ov.innerHTML =
      `<div class="modal"><h3></h3><p class="modal-label"></p>` +
      `<textarea class="modal-input" rows="8"></textarea>` +
      `<div class="modal-trust">⚠ 산출물은 "AI 보조 생성 · 오너 검수 전"입니다. 외부 공유 전 검수를 받으세요.</div>` +
      `<div class="modal-btns"><button class="modal-no">취소</button>` +
      `<button class="modal-yes">진행</button></div></div>`;
    (ov.querySelector("h3") as HTMLElement).textContent = title;
    (ov.querySelector(".modal-label") as HTMLElement).textContent = label;
    const ta = ov.querySelector(".modal-input") as HTMLTextAreaElement;
    ta.placeholder = placeholder;
    const done = (v: string | null) => {
      ov.remove();
      resolve(v);
    };
    ov.querySelector(".modal-yes")!.addEventListener("click", () => done(ta.value.trim() || null));
    ov.querySelector(".modal-no")!.addEventListener("click", () => done(null));
    ov.addEventListener("click", (e) => {
      if (e.target === ov) done(null);
    });
    document.body.appendChild(ov);
    setTimeout(() => ta.focus(), 50);
  });
}

// 다중 필드 HITL 모달 — 카탈로그 entry.fields([{key,label,placeholder,multiline}])를 각 입력으로 렌더.
// 결과는 "key: value" 줄들로 합쳐 기존 userInput 자리에 사용(빈 필드 생략). 취소·전부 빈값=null.
function fieldsModal(title: string, label: string, fields: any[]): Promise<string | null> {
  return new Promise((resolve) => {
    const ov = document.createElement("div");
    ov.className = "modal-overlay";
    const rows = fields
      .map((f, i) => {
        const ph = ccEsc(String(f.placeholder ?? ""));
        const lb = ccEsc(String(f.label ?? f.key ?? ""));
        const inp = f.multiline
          ? `<textarea class="modal-input cc-field" data-i="${i}" rows="4" placeholder="${ph}"></textarea>`
          : `<input class="modal-input cc-field" data-i="${i}" type="text" placeholder="${ph}" style="min-height:auto" />`;
        return `<p class="modal-label">${lb}</p>${inp}`;
      })
      .join("");
    ov.innerHTML =
      `<div class="modal"><h3></h3><p class="modal-label modal-head"></p>${rows}` +
      `<div class="modal-trust">⚠ 산출물은 "AI 보조 생성 · 오너 검수 전"입니다. 외부 공유 전 검수를 받으세요.</div>` +
      `<div class="modal-btns"><button class="modal-no">취소</button>` +
      `<button class="modal-yes">진행</button></div></div>`;
    (ov.querySelector("h3") as HTMLElement).textContent = title;
    (ov.querySelector(".modal-head") as HTMLElement).textContent = label;
    const done = (v: string | null) => {
      ov.remove();
      resolve(v);
    };
    ov.querySelector(".modal-yes")!.addEventListener("click", () => {
      const parts: string[] = [];
      ov.querySelectorAll<HTMLInputElement | HTMLTextAreaElement>(".cc-field").forEach((el) => {
        const f = fields[Number(el.dataset.i)];
        const val = el.value.trim();
        if (val) parts.push(`${f.key ?? f.label ?? "field"}: ${val}`);
      });
      done(parts.length ? parts.join("\n") : null);
    });
    ov.querySelector(".modal-no")!.addEventListener("click", () => done(null));
    ov.addEventListener("click", (e) => {
      if (e.target === ov) done(null);
    });
    document.body.appendChild(ov);
    setTimeout(() => (ov.querySelector(".cc-field") as HTMLElement | null)?.focus(), 50);
  });
}

// ★기능2: 부서 완전 폐역 확인 — 부서명 타이핑 일치 시에만 "완전 삭제" 활성. 크기·최종 mtime·CEO 강등·
// 격리 복구를 정직 고지한다(비가역처럼 보이나 실제는 격리 후 약 14일 소거 — 오도 금지). teardown-only 인
// 기존 탭 삭제(2-click 대체 close)와 별개 경로. resolve(true)=진행.
function purgeConfirmModal(
  name: string,
  info: { sizeHuman: string; mtime: string; isLast: boolean; exists: boolean },
): Promise<boolean> {
  return new Promise((resolve) => {
    const ov = document.createElement("div");
    ov.className = "modal-overlay";
    const notice = [
      info.exists
        ? `대화기억(state) 크기 ${info.sizeHuman} · 최종 수정 ${info.mtime}`
        : "대화기억(state) 디렉토리 없음(격리 대상 없음)",
      // ★D2a(purge-safety 2026-07-16): 작업 폴더(cwd)는 격리하지 않는다 — 전 부서 cwd=$HOME(공유)
      // 현실에서 "작업물 격리" 고지는 홈 격리 약속이 되는 오도였다. GUI는 --purge-workdir 미요청.
      "삭제되는 것: 부서 데몬 종료 + 대화기억·pack 격리 + 재시작 부활 영구 차단(묘비 존치). 작업 폴더(cwd)는 보존됩니다.",
      info.isLast ? "⚠ 이 부서가 마지막입니다 — 삭제 시 CEO가 표준 master로 강등됩니다." : "",
      "복구: 즉시 삭제가 아니라 ~/.local/state/cys-trash/ 로 격리 보관되어 되돌릴 수 있고, 약 14일 후 자동 소거됩니다.",
      `계속하려면 아래에 부서명 "${name}" 을 정확히 입력하세요.`,
    ]
      .filter(Boolean)
      .join("\n\n");
    ov.innerHTML =
      `<div class="modal"><h3></h3><p class="modal-label" style="white-space:pre-wrap"></p>` +
      `<input class="modal-input" type="text" />` +
      `<div class="modal-hint" aria-live="polite"></div>` +
      `<div class="modal-btns"><button class="modal-no">취소</button>` +
      `<button class="modal-yes" disabled>완전 삭제</button></div></div>`;
    (ov.querySelector("h3") as HTMLElement).textContent = `부서 "${name}" 완전 삭제(부활 차단)`;
    (ov.querySelector(".modal-label") as HTMLElement).textContent = notice;
    const inp = ov.querySelector(".modal-input") as HTMLInputElement;
    const yes = ov.querySelector(".modal-yes") as HTMLButtonElement;
    const hint = ov.querySelector(".modal-hint") as HTMLElement;
    // ★D3b(purge-safety 2026-07-16): macOS 자동 대문자화가 소문자 입력을 재교정해 정확 재입력조차
    // 불일치가 되던 실사고 차단 — 확인 입력엔 자동 교정 3종을 끈다(계약=PURGE_INPUT_GUARDS).
    for (const [k, v] of PURGE_INPUT_GUARDS) inp.setAttribute(k, v);
    inp.placeholder = name;
    inp.addEventListener("input", () => {
      // ★D3c: 불일치는 침묵이 아니라 사유를 말한다(비활성 버튼 무반응 오인 방지).
      yes.disabled = !purgeNameMatches(inp.value, name);
      hint.textContent = purgeMismatchHint(inp.value, name);
    });
    let onKey: ((e: KeyboardEvent) => void) | null = null;
    const done = (v: boolean) => {
      if (onKey) document.removeEventListener("keydown", onKey, true);
      ov.remove();
      resolve(v);
    };
    // Esc = 취소. 파괴적 다이얼로그가 "닫히지 않는 창"으로 보이지 않게(초보 시뮬레이션 지적).
    onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        done(false);
      }
    };
    document.addEventListener("keydown", onKey, true);
    yes.addEventListener("click", () => {
      if (!yes.disabled) done(true);
    });
    ov.querySelector(".modal-no")!.addEventListener("click", () => done(false));
    ov.addEventListener("click", (e) => {
      if (e.target === ov) done(false);
    });
    document.body.appendChild(ov);
    setTimeout(() => inp.focus(), 50);
  });
}

// ★기능2: 부서 완전 폐역 실행 — 프리뷰(크기·mtime·마지막여부) 조회 → 타이핑 확인 → purge → 탭 제거.
// 기존 close(2-click 대체) 경로와 별개: 이건 대화기억까지 격리하고 부활을 영구 차단한다(javis_org destroy).
async function purgeDept(ws: Workspace) {
  if (!ws.socket) return;
  // ★[F3] 데몬 교대(restart)·완전 초기화가 진행/완료 상태면 같은 부서 데몬 경합을 피해 거부·안내.
  if (daemonActionBlocked()) return;
  let info: {
    name?: string;
    size_bytes?: number;
    mtime_secs?: number;
    is_last?: boolean;
    exists?: boolean;
  } = {};
  try {
    info = (await invoke("dept_purge_preview_by_socket", { socket: ws.socket })) as typeof info;
  } catch (e) {
    toast("watchdog", "삭제 프리뷰 실패", `${e} — 삭제를 중단합니다. 다시 시도해 주세요.`);
    return;
  }
  const nm = info.name || ws.name || UNTITLED;
  const bytes = Number(info.size_bytes || 0);
  const sizeHuman =
    bytes >= 1e9
      ? (bytes / 1e9).toFixed(1) + " GB"
      : bytes >= 1e6
        ? (bytes / 1e6).toFixed(1) + " MB"
        : bytes >= 1e3
          ? (bytes / 1e3).toFixed(1) + " KB"
          : bytes + " B";
  const mtime = info.mtime_secs ? new Date(info.mtime_secs * 1000).toLocaleString() : "?";
  const ok = await purgeConfirmModal(nm, {
    sizeHuman,
    mtime,
    isLast: !!info.is_last,
    exists: !!info.exists,
  });
  if (!ok) return;
  // ★[F3-R TOCTOU] 진입 가드는 모달 '전'이라, 부서명 타이핑 모달이 열려 있던 수초간 checkVersionSkew
  // 타이머/수동 restart가 rotatingDaemon을 잡았을 수 있다. 모달 확인 직후·집행 직전에 재확인한다.
  if (rotatingDaemon) {
    toast("feed", "작업 진행 중", "데몬 재시작이 진행 중입니다 — 잠시 후 다시 시도하세요.");
    return;
  }
  // ★[F3] 데몬 폐역 구간 동안 restart를 배제(manualRestartAllDaemons·checkVersionSkew가 purgingDept 존중).
  purgingDept = true;
  try {
    // ★D2b(purge-safety 2026-07-16): 실패는 8초 자동소멸 toast가 아니라 sticky — 실사고에서 사용자가
    // 실패 사실 자체를 인지하지 못했다("눌러도 무반응"). 재시도 진입 시 이전 실패는 갱신/해소된다.
    const failId = `purge-fail-${ws.socket}`;
    try {
      await invoke("purge_dept_daemon_by_socket", { socket: ws.socket });
    } catch (e) {
      stickyToast(failId, "watchdog", "부서 완전 삭제 실패", `${nm}: ${e} — 삭제되지 않았습니다.`);
      return;
    }
    dismissToast(failId);
    toast("watchdog", "부서 완전 삭제 완료", `${nm} — 대화기억은 격리 보관(복구 가능)·재시작 부활 차단.`);
    // 프론트 pane·탭 정리(데몬은 이미 down — close_surface 실패는 관용).
    for (const sid of collectSids(ws.tree)) {
      await invoke("close_surface", { socket: ws.socket, surfaceId: sid }).catch(() => {});
      destroyPaneRuntime(sid, ws.socket);
    }
    const i = workspaces.indexOf(ws);
    if (i < 0) {
      render();
      return;
    }
    workspaces.splice(i, 1);
    if (workspaces.length === 0) {
      await addWorkspace();
    } else {
      if (i < activeWs) activeWs -= 1;
      activeWs = Math.min(activeWs, workspaces.length - 1);
    }
    render();
  } finally {
    purgingDept = false;
  }
}

// ★완전 초기화(팩토리 리셋) 확인 — 고정 문구 타이핑 일치 시에만 활성(resetconfirm.ts 순수 판정).
// purgeConfirmModal 규약 계승: textContent 주입(XSS-safe)·자동교정 차단·불일치 사유 표시·정직 고지.
function factoryResetConfirmModal(info: ResetPreview): Promise<boolean> {
  return new Promise((resolve) => {
    const ov = document.createElement("div");
    ov.className = "modal-overlay";
    ov.innerHTML =
      `<div class="modal"><h3></h3>` +
      // 고지문이 길어졌으므로(경로·강조·안내) 본문만 스크롤시킨다 — 확인 입력·버튼은 항상 보인다.
      `<p class="modal-label" style="white-space:pre-wrap;max-height:46vh;overflow-y:auto"></p>` +
      `<input class="modal-input" type="text" />` +
      `<div class="modal-hint" aria-live="polite"></div>` +
      `<div class="modal-btns"><button class="modal-no">취소</button>` +
      `<button class="modal-yes" disabled>완전 초기화</button></div></div>`;
    (ov.querySelector("h3") as HTMLElement).textContent = "완전 초기화(팩토리 리셋) — 설치 초기 상태로";
    (ov.querySelector(".modal-label") as HTMLElement).textContent = resetNoticeLines(info).join("\n\n");
    const inp = ov.querySelector(".modal-input") as HTMLInputElement;
    const yes = ov.querySelector(".modal-yes") as HTMLButtonElement;
    const hint = ov.querySelector(".modal-hint") as HTMLElement;
    for (const [k, v] of PURGE_INPUT_GUARDS) inp.setAttribute(k, v);
    inp.placeholder = RESET_PHRASE;
    inp.addEventListener("input", () => {
      yes.disabled = !resetPhraseMatches(inp.value);
      hint.textContent = resetMismatchHint(inp.value);
    });
    const done = (v: boolean) => {
      ov.remove();
      resolve(v);
    };
    yes.addEventListener("click", () => {
      if (!yes.disabled) done(true);
    });
    ov.querySelector(".modal-no")!.addEventListener("click", () => done(false));
    ov.addEventListener("click", (e) => {
      if (e.target === ov) done(false);
    });
    document.body.appendChild(ov);
    setTimeout(() => inp.focus(), 50);
  });
}

// ★완전 초기화 실행 — 프리뷰(쓰기 0) → 문구 확인 → factory_reset_execute(코어=cys::factory_reset:
// 데몬 전멸 하드 게이트 → cys-trash 격리+manifest → 훅·스킬링크 해제) → 종료 안내.
// 완료 후 데몬이 없으므로 앱은 반쪽 상태 — 곧장 종료를 권한다(재실행 시 설치 온보딩).
async function factoryResetFlow() {
  if (daemonActionBlocked()) return;
  let info: {
    quarantine_count?: number;
    total_bytes?: number;
    trash_dir?: string;
    quarantine?: { path: string; label: string; size_bytes: number; outside_state?: boolean }[];
    kept?: { path: string; label: string }[];
    strip_profiles?: number;
    report_only?: string[];
    live_sessions?: number;
    dept_count?: number;
    trash_root_ready?: boolean;
    trash_root_error?: string | null;
    interrupted_prior?: string[];
  } = {};
  // ★P0-2: 프리뷰는 전 트리 재귀 stat 이라 수 초 걸린다 — 무피드백이면 "버튼이 안 눌렸다"로
  // 보여 재클릭을 부른다(백엔드는 spawn_blocking 이라 창은 굳지 않는다).
  stickyToast("reset-preview", "feed", "⟲ 격리 대상 계산 중", "무엇이 지워지는지 확인하는 중…");
  try {
    info = (await invoke("factory_reset_preview", {})) as typeof info;
  } catch (e) {
    dismissToast("reset-preview");
    toast("watchdog", "초기화 프리뷰 실패", `${e} — 초기화를 중단합니다. 다시 시도해 주세요.`);
    return;
  }
  dismissToast("reset-preview");
  // ★P0-6: 격리 폴더를 못 쓰는 상태면 **모달을 띄우지 않고** 사전 거부한다(데몬 무접촉).
  if (info.trash_root_ready === false) {
    toast(
      "watchdog",
      "초기화를 시작할 수 없습니다",
      `${info.trash_root_error ?? "격리 폴더를 쓸 수 없습니다"} — 해결한 뒤 다시 시도하세요.`,
    );
    return;
  }
  const preview: ResetPreview = {
    quarantineCount: info.quarantine_count ?? 0,
    totalBytes: Number(info.total_bytes ?? 0),
    keptCount: (info.kept ?? []).length,
    stripProfiles: info.strip_profiles ?? 0,
    trashDir: info.trash_dir ?? "~/.local/state/cys-trash",
    items: info.quarantine ?? [],
    reportOnly: info.report_only ?? [],
    liveSessions: info.live_sessions ?? 0,
    deptCount: info.dept_count ?? 0,
    interruptedPrior: info.interrupted_prior ?? [],
  };
  const ok = await factoryResetConfirmModal(preview);
  if (!ok) return;
  // TOCTOU 재확인(purgeDept와 동일 근거): 모달이 열려 있던 동안 restart/purge가 시작됐을 수 있다.
  if (rotatingDaemon || purgingDept) {
    toast("feed", "작업 진행 중", "데몬 재시작 또는 부서 삭제가 진행 중입니다 — 잠시 후 다시 시도하세요.");
    return;
  }
  factoryResetting = true;
  const failId = "reset-fail";
  try {
    stickyToast("factory-reset", "watchdog", "⟲ 완전 초기화 중", "데몬 정지·격리 진행 중… 앱을 닫지 마세요.");
    let rep: ResetResult;
    try {
      // purgeLicense/purgeLocal=false — 라이선스와 직접 만든 오버레이(~/.cys/local)는 보존이 기본이다
      // (CLI 는 --purge-license·--purge-local 로 선택 격리 가능·모달 고지문과 동일 계약).
      rep = (await invoke("factory_reset_execute", { purgeLicense: false, purgeLocal: false })) as typeof rep;
    } catch (e) {
      dismissToast("factory-reset");
      stickyToast(failId, "watchdog", "완전 초기화 실패", `${e} — 아무것도(또는 일부만) 변경되지 않았을 수 있습니다. 재시도하거나 cys factory-reset --plan 으로 상태를 확인하세요.`);
      return;
    }
    dismissToast("factory-reset");
    // ★A6(성찰 확정): 여기부터 이 앱 프로세스는 되돌릴 수 없는 반쪽 상태다(데몬·pack·훅 부재).
    // 부분 실패든 성공이든 **격리가 시작된 이상** 데몬 소생 경로를 영구 차단한다(래치는 해제되지 않는다).
    resetCompleted = true;
    // ★P0-4: 실패·부활은 토스트(60초 수명)로만 알리면 완료 모달 뒤에 가려지고 앱을 끄면
    // 영영 사라진다. **모달 제목·본문 자체**가 결과에서 파생되도록 하고(정면 노출),
    // 디스크의 REPORT.txt 경로를 함께 안내한다. 토스트는 보조로만 남긴다.
    if (rep.ok === false) {
      const fails = (rep.failed ?? []).map((f) => `${f.path}: ${f.error}`).join("\n");
      stickyToast(failId, "watchdog", "완전 초기화 부분 실패", `일부 항목이 이동되지 않았습니다:\n${fails}\n격리 보관함: ${rep.trash_dir ?? ""}`);
    } else {
      dismissToast(failId);
    }
    if (rep.revived_warning) {
      stickyToast("reset-revived", "health", "⚠ 초기화 중 데몬 부활", rep.revived_warning);
    }
    // ★P1-2: "되돌릴 수 있습니다"를 실제로 손에 쥐여 준다 — 격리 폴더를 파일 관리자로 연다.
    // 초보가 터미널 없이도 무엇이 보관됐는지(REPORT.txt 포함) 눈으로 확인할 수 있는 유일한 경로.
    if (rep.trash_dir) {
      toast(
        "feed",
        "격리 보관함",
        `${rep.trash_dir} — 클릭하면 폴더를 엽니다(REPORT.txt 에 요약, --undo 로 복구).`,
        () => void invoke("reveal_path", { path: rep.trash_dir }).catch(() => {}),
      );
    }
    const quit = await confirmModal(resetResultTitle(rep), resetResultBody(rep), "앱 종료", "나중에");
    if (quit) {
      // ★P1-3: 화면 저장값(레이아웃·테마·핀)은 WebView 저장소에 있어, 앱이 살아 있는 동안
      // 파일 격리만으로는 지워지지 않는다(Windows 는 rename 자체가 실패해 항상 남는다).
      // 종료 직전 여기서 직접 비우면 "이연" 여부와 무관하게 다음 실행이 초기 화면이 된다.
      try {
        localStorage.clear();
      } catch {
        /* 저장소 접근 실패는 종료를 막지 않는다 */
      }
      await invoke("factory_reset_quit_app", {}).catch(() => {});
    }
  } finally {
    factoryResetting = false;
  }
}

// ---------- toasts (daemon push events) ----------

// ★T-0147-3(2026-07-30): 모든 토스트는 종류 불문 유한 수명을 갖는다(정책=toastttl.ts).
// 소멸해도 내용은 알람 이력(alarmHistory → Control Center '알람' 탭)에 남으므로
// main.ts:4878 주석의 실사고("실패를 인지 못함")는 이력 + 수동 × + 만료 배너로 대체 방어한다.
let alarmHistory: AlarmRecord[] = [];

function recordAlarm(category: string, name: string, detail: string, id?: string) {
  alarmHistory = pushAlarm(alarmHistory, { ts: Date.now(), category, name, detail, id });
  if (ccOpen && ccTab === "alarms") renderAlarmHistory();
}

// 우상단 × — 자동 소멸을 기다리지 않고 즉시 치울 수 있는 수동 경로(sticky는 id로 정리).
function addToastCloseButton(el: HTMLElement, id?: string) {
  const x = document.createElement("button");
  x.className = "toast-x";
  x.type = "button";
  x.title = "닫기";
  x.textContent = "×";
  x.addEventListener("click", (e) => {
    e.stopPropagation();
    if (id) dismissToast(id);
    else el.remove();
  });
  el.appendChild(x);
}

/// `onClick` — 토스트 본문을 눌렀을 때의 동작(선택). 완전 초기화 완료 후 격리 폴더를 여는
/// 것처럼 **행동으로 이어지는 안내**에만 쓴다(P1-2). 닫기 버튼 클릭과는 분리한다.
function toast(category: string, name: string, detail: string, onClick?: () => void) {
  recordAlarm(category, name, detail);
  const box = document.getElementById("toasts")!;
  const el = document.createElement("div");
  el.className = toastClassName(category); // 등급색 서식의 단일 진실(sticky 와 같은 함수)
  el.innerHTML = `<span class="toast-name"></span><span class="toast-detail"></span>`;
  (el.querySelector(".toast-name") as HTMLElement).textContent = name;
  (el.querySelector(".toast-detail") as HTMLElement).textContent = detail;
  if (onClick) {
    el.style.cursor = "pointer";
    el.addEventListener("click", (e) => {
      if ((e.target as HTMLElement).closest(".toast-x")) return; // 닫기(×)는 제외
      onClick();
    });
  }
  addToastCloseButton(el);
  box.appendChild(el);
  setTimeout(() => el.remove(), toastTtl("volatile").ttlMs);
}

// 지속형(sticky) 토스트 — id로 갱신/제거하고, 갱신마다 TTL 타이머가 리셋된다(debounce).
// 완료·실패 때 dismissToast로 내리는 기존 계약은 그대로 유지되고, 페어가 유실돼도
// TTL이 최후 방어선으로 화면을 정리한다(구 구현은 타이머가 없어 영구 잔존했다).
const stickyToasts = new Map<string, { el: HTMLElement; timer: ReturnType<typeof setTimeout> }>();

function stickyToast(id: string, category: string, name: string, detail: string) {
  recordAlarm(category, name, detail, id);
  const box = document.getElementById("toasts")!;
  const prev = stickyToasts.get(id);
  const plan = toastTimerPlan("sticky", id, !!prev);
  if (prev && plan.clearPrevious) clearTimeout(prev.timer);
  let el = prev?.el;
  if (!el) {
    el = document.createElement("div");
    el.innerHTML = `<span class="toast-name"></span><span class="toast-detail"></span>`;
    addToastCloseButton(el, id);
    box.appendChild(el);
  }
  // (MINOR-N4) 등급색은 **낼 때마다** 다시 못박는다. 예전에는 생성 시점에 한 번만 정해져,
  // 같은 id 로 실패(watchdog) → 성공(system)이 오면 본문만 '✅'로 바뀌고 테두리는 경고색 그대로
  // 남았다 — 등급을 표시하는 유일한 장치가 거짓말을 한다. 자식 노드(본문·닫기 버튼)는 그대로다.
  el.className = toastClassName(category);
  (el.querySelector(".toast-name") as HTMLElement).textContent = name;
  (el.querySelector(".toast-detail") as HTMLElement).textContent = detail;
  const timer = setTimeout(() => {
    dismissToast(id);
    // 고위험 실패(purge-fail-* 등)는 조용히 사라지지 않는다 — OS 배너로 1회 보강(D2b 계승).
    if (needsExpiryBanner(id)) {
      const b = expiryBannerText(name, detail);
      osBanner(b.title, b.body);
    }
  }, plan.ttlMs);
  stickyToasts.set(id, { el, timer });
}

function dismissToast(id: string) {
  const t = stickyToasts.get(id);
  if (t) {
    clearTimeout(t.timer); // 타이머 짝 해제 — 없으면 재사용 id에서 유령 만료가 남는다
    t.el.remove();
    stickyToasts.delete(id);
  }
}

// 알람 이력 탭 — 소멸한 토스트를 세션 내에서 다시 볼 수 있는 수용처(최신순).
function renderAlarmHistory() {
  const box = document.getElementById("cc-alarm-list");
  if (!box) return;
  box.textContent = "";
  if (alarmHistory.length === 0) {
    const empty = document.createElement("div");
    empty.className = "cc-board-note";
    empty.textContent = "표시된 알람이 없습니다.";
    box.appendChild(empty);
    return;
  }
  for (const a of alarmHistory) {
    const row = document.createElement("div");
    row.className = `alarm-item ${a.category}`;
    const meta = document.createElement("div");
    meta.className = "al-meta";
    meta.textContent = `${formatAlarmTime(a.ts)} · ${a.category}${a.id ? ` · ${a.id}` : ""}`;
    const title = document.createElement("div");
    title.className = "al-title";
    title.textContent = a.name;
    const body = document.createElement("div");
    body.className = "al-body";
    body.textContent = a.detail;
    row.append(meta, title, body);
    box.appendChild(row);
  }
}

// OS 네이티브 배너(B4): 채팅창 밖에서도 고우선 이벤트 포착. 권한 거부·미지원은 무해(try/catch).
async function osBanner(title: string, body: string) {
  try {
    let granted = await isPermissionGranted();
    if (!granted) granted = (await requestPermission()) === "granted";
    if (granted) sendNotification({ title, body });
  } catch {
    /* 권한 거부·플러그인 미지원 — 무해 */
  }
}

function onDaemonEvent(event: Record<string, unknown>) {
  const name = String(event.name ?? "");
  const category = String(event.category ?? "");
  const payload = (event.payload ?? {}) as Record<string, unknown>;
  const sid = event.surface_id;

  // --- name-우선 전용 처리(B1) : name 매칭이 category 폴백보다 우선 ---
  if (name === "approval.request") {
    toast("approval", "⚠ 승인 대기", `${payload.role ?? ""} ${payload.surface_ref ?? ""} — ${String(payload.excerpt ?? "").slice(0, 100)}`);
    osBanner("⚠ 승인 대기", `${payload.role ?? ""} ${payload.surface_ref ?? ""} — ${String(payload.excerpt ?? "").slice(0, 100)}`); // B4 OS 배너(고우선)
    // 자동 화면전환 없음 — 페인 승인 프롬프트는 master 즉각 자동승인 관할.
    // 토스트·OS 배너·사이드바 배지로만 알린다(feed.item.created의 유예 경로와 정합).
    refreshFeed();
    refreshSidebarStatus(); // 사이드바 ⚠ 배지 갱신 (B3)
    return;
  }
  if (name === "approval.stalled") {
    // master가 stall 임계(기본 5분) 내 처리하지 못한 승인 = 사람 개입 필요 신호 —
    // 이때만 화면을 전환한다(승인 UX 원칙: 알림과 포커스 강탈의 분리, escalation 짝).
    toast("approval", "⚠ 승인 방치", `${payload.surface_ref ?? ""} ${String(payload.title ?? "").slice(0, 80)} — ${payload.age_secs}s 경과`);
    osBanner("⚠ 승인 방치 — 사람 확인 필요", `${payload.surface_ref ?? ""} ${String(payload.title ?? "").slice(0, 80)}`);
    openFeed();
    refreshFeed();
    refreshSidebarStatus();
    return;
  }
  if (name === "context.threshold") {
    toast("threshold", `🔋 컨텍스트 ${payload.context_pct}%`, `${payload.role ?? ""} ${payload.surface_ref ?? ""} ≥ ${payload.threshold}% — ${payload.action ?? ""}`);
    if (Number(payload.context_pct ?? 0) >= 80)
      osBanner(`🔋 컨텍스트 ${payload.context_pct}%`, `${payload.role ?? ""} ${payload.surface_ref ?? ""} ≥ ${payload.threshold}% — ${payload.action ?? ""}`); // B4 OS 배너(≥80만)
    refreshSidebarStatus();
    return;
  }
  if (name === "pane.idle") {
    toast("idle", "💤 노드 유휴", `surface:${sid} — ${payload.idle_seconds}s 무출력`);
    refreshSidebarStatus();
    return;
  }
  if (name === "master.idle") {
    // ★G2 결함 8(W3-B): v2 데몬의 정보성 침묵 신호 — idle 은 alert 가 아니다.
    //   ① OS 배너 없음·화면 전환 없음(id 접두 "master-idle:"은 BANNER_ON_EXPIRY_PREFIXES
    //      비매칭 = 만료 시에도 배너 0 — toastttl.ts).
    //   ② 무한 토스트 금지: stickyToast(role별 안정 id)로 dedupe — 데몬측 디바운스
    //      (기본 300s)와 무관하게 화면에는 role당 1장만 갱신되고 TTL 로 유한 소멸한다.
    //      (BLOCKER 실측: category 폴백 레인의 무dedupe 토스트가 5분마다 영구 적층 —
    //       category="info"는 아래 폴백 3종(health/watchdog/feed) 어디에도 없어 낙하하지
    //       않지만, name-first 핸들러가 없으면 운영자 가시성이 0이 된다. 이 핸들러가
    //       '조용한 배지+알람 기록' 층위로 가시성을 보존한다 — 설계 rationale.)
    //   ③ 알람 이력: stickyToast→recordAlarm(id) — 같은 id는 최신 1건으로 합쳐져
    //      이력 링버퍼를 잠식하지 않는다(pushAlarm coalesce).
    const role = String(payload.role ?? "master");
    stickyToast(
      `master-idle:${event.socket_slug ?? ""}:${role}`,
      "idle",
      "💤 master 유휴",
      `surface:${sid} ${role} — ${payload.idle_secs}s 무출력(임계 ${payload.threshold_secs}s)`,
    );
    refreshSidebarStatus();
    return;
  }
  if (name === "agent.exited") {
    toast("alert", "❌ 에이전트 사망", `surface:${sid} ${payload.role ?? ""}`);
    osBanner("❌ 에이전트 사망", `surface:${sid} ${payload.role ?? ""}`); // B4 OS 배너(고우선)
    refreshSidebarStatus();
    // ★정합기 리셋 훅(스펙 D4 ②): 앱 즉사(SIGKILL — 복원 시퀀스 없음)로 유출·잔존한 트래킹을
    // 소등한다. 조준은 socket_slug **실해석 성공** pane 만(:5194 선례 — slug 부재·미해석 시
    // 기본 데몬 폴백 금지, 생략: 타 부서 동일 sid pane 의 정합기 오파괴 방지).
    // 에포크 가드: 이벤트 채널(cys-event)과 출력 스트림은 순서 보장이 없다 — 수신 시점 세대를
    // 캡처하고 한 태스크 양보 뒤 비교해, 그 사이(딜리버리 스큐 창) 새 앱의 명시 DECSET/alt
    // 전이가 관측됐으면 장부 소거를 생략(새 앱 상태 보호)하고 상수 소등 주입만 멱등 수행한다.
    // 관측 카운터는 필터의 세대 번호(trackfilter.generation)로 구현돼 있다.
    const exSock = event.socket_slug ? socketForSlug.get(String(event.socket_slug)) : undefined;
    if (exSock && sid != null) {
      const rt = panes.get(paneKey(Number(sid), exSock));
      if (rt) {
        const gen = rt.trackFilter.generation();
        window.setTimeout(() => {
          const live = panes.get(paneKey(Number(sid), exSock));
          if (live !== rt) return; // pane 이 이미 파괴·교체됨 — dispose 된 term 에 write 금지
          rt.trackFilter.clearLedgerIfGeneration(gen); // 세대 변화 시 장부 보존(소거 생략)
          rt.term.write(MOUSE_ALL_OFF); // 상수 소등 주입은 항상 멱등(1049l 미포함·필터 우회)
        }, 0);
      }
    }
    return;
  }
  if (name === "master.deadman") {
    // v2(G2)는 role·axis 등 additive 필드를 싣고, v1 은 {reason,idle_secs}뿐 — payload.role
    // 폴백이 양 버전을 모두 흡수한다(governance.rs). reason 은 verbatim 표시라 신규값
    // ("shell process dead"/"agent process dead" 등)도 자연 수용 — 핸들러 무변경(W3-B).
    toast("alert", "🚨 master 무응답(deadman)", `surface:${sid} ${payload.role ?? ""} ${payload.reason ?? ""}`);
    osBanner("🚨 master 무응답(deadman)", `surface:${sid} ${payload.reason ?? ""}`); // B4 OS 배너(고우선)
    return;
  }
  if (name === "status.changed" || name === "task.changed") {
    if (name === "status.changed") refreshSidebarStatus(); // toast 없음(빈도 높음) — 사이드바만
    // Tasks Control Center 실시간 갱신: 부서(socket_slug)×노드(surface_id) 셀 패치. 폴링 없이 즉시.
    const slug = event.socket_slug ? String(event.socket_slug) : "";
    if (slug && sid != null) upsertTaskCell(slug, Number(sid), payload);
    return;
  }
  if (name === "osc.notify") {
    toast("osc", `🔔 ${payload.title ?? "알림"}`, `surface:${sid} — ${String(payload.body ?? "").slice(0, 120)}`);
    return;
  }

  if (category === "health") {
    toast("health", `⚠ ${name}`, `surface:${sid} rule=${payload.rule} — ${String(payload.line ?? "").slice(0, 120)}`);
  } else if (category === "watchdog") {
    const detail =
      name === "watchdog.duplicate_procs"
        ? `중복 서버 ${payload.count}개: ${String(payload.cmdline ?? "").slice(0, 80)}`
        : JSON.stringify(payload).slice(0, 120);
    toast("watchdog", `🐕 ${name}`, detail);
  } else if (category === "feed") {
    if (name === "feed.item.created") {
      toast("feed", "📥 승인 요청", String(payload.title ?? ""));
      // 즉시 전환하지 않는다 — master/CEO 자동 승인 유예 후에도 pending인 항목만
      // 사람 개입 필요로 보고 전환한다(자동 승인분은 무전환).
      // W3.4: auto_route 항목은 90초 기본 + CEO 활성 동적 연장, 비대상 wait 항목은 30초.
      const autoRoute = payload.auto_route === true;
      if (payload.wait === true || autoRoute)
        scheduleFeedSwitchIfStillPending(String(payload.request_id ?? ""), autoRoute);
    }
    refreshFeed();
    refreshSidebarStatus(); // 피드 이벤트 시 집계 배지 갱신(멀티부서 정합)
  } else if (name === "surface.exited" || name === "surface.closed" || name === "surface.reaped") {
    // 종료 즉시 죽은 pane 자동 제거 (A안) — 데몬 reap을 기다리지 않는다. 멱등.
    // 멀티마스터 F4: 출처 데몬을 socket_slug로 특정해 그 부서 pane만 제거(타 부서 같은 sid 보호).
    const sock = event.socket_slug ? socketForSlug.get(String(event.socket_slug)) : undefined;
    if (event.socket_slug && !sock) return; // slug 명시됐는데 미해결 → 기본 데몬 폴백 금지(타부서 동일 sid 오제거 방지)
    removeDeadPane(Number(sid), sock);
  }
}

// ---------- startup / session restore ----------

async function start() {
  const info = document.getElementById("daemon-info")!;
  // ★T2 안전모드 pull(emit-before-listen 레이스 회피): 백엔드가 비정규 실행 위치(translocation/DMG 등)
  // 면 데몬이 뜨지 않아 아래 daemon-ready await 가 영원히 안 풀린다. 그 await **전에** 백엔드를 직접
  // 조회해(데몬 무관 순수 커맨드) 안내를 확정 표시한다. translocation-blocked 이벤트는 벨트앤서스펜더
  // 로 유지되며, 같은 토스트 id("safe-mode")라 중복 발화해도 dedupe 된다.
  try {
    const guidance = await invoke("boot_verdict");
    if (typeof guidance === "string" && guidance) {
      stickyToast("safe-mode", "health", "안전모드 — 설치 위치를 옮겨 주세요", guidance);
    }
  } catch {
    /* 비-macOS·조회 실패는 정상 부트로 흘려보냄(안전모드는 macOS 전용 게이트) */
  }
  // ★설치본 무결성 pull(ATOMIC-1 짝 · 2026-08-01 "손상되었기 때문에 열 수 없습니다" 실사고):
  // 정규 위치에 설치됐는데도 구성요소가 빠진 '반쪽 번들'이면 어떤 파일이 없는지와 재설치 절차를
  // 알린다. 안전모드와 **별개 토스트**다 — 원인도 처방도 다르므로 같은 제목에 섞으면 오히려
  // 사용자를 헤매게 한다("위치를 옮기라"고 해도 해결되지 않는 고장이다). bundle-damaged 이벤트는
  // 벨트앤서스펜더(같은 id 라 중복 발화해도 dedupe).
  //
  // ★봉인 파손 합산(F3 격차1 · SEAL-DIAG): 이 pull 은 구조 결손만이 아니라 **캐시된 코드서명
  // 봉인 파손 판정**도 백엔드가 합산해 돌려준다(src-tauri bundle_integrity → merge_integrity_pull).
  // 봉인 자가진단의 push(bundle-damaged emit)는 아래 listen 등록 **전**에 나가면 유실되는데
  // (emit-before-listen 레이스 — 리로드·기동 타이밍), 이 pull 이 그 유실을 기계적으로 회수한다.
  // ★중복 토스트 금지: push 가 먼저(또는 나중에) 와도 토스트 id 가 "bundle-damaged" 하나라
  // stickyToast 가 같은 엘리먼트를 재사용한다(같은 id 재호출 = 갱신·TTL 리셋 — 화면엔 한 줄).
  // 둘 다 결론이 "재설치"라 어느 쪽 문구가 남아도 안내는 어긋나지 않는다.
  try {
    const damage = await invoke("bundle_integrity");
    if (typeof damage === "string" && damage) {
      stickyToast("bundle-damaged", "health", "설치본이 온전하지 않습니다 — 재설치 필요", damage);
    }
  } catch {
    /* 비-macOS·번들 밖 실행은 해당 없음 */
  }
  await new Promise<void>((resolve) => {
    listen("daemon-ready", () => resolve());
    listen("daemon-error", (e) => {
      info.textContent = `daemon error: ${e.payload}`;
    });
    // ★신선 머신 부트 수리 짝(오너 2026-07-15): 백엔드 재시도 4회째 발화 — 상단바 텍스트는
    // 초보자가 놓치므로 sticky 토스트로 로그인 항목 승인 안내(데몬이 뜨면 daemon-ready가 진행).
    listen("daemon-retry-hint", () => {
      stickyToast(
        "daemon-hint",
        "health",
        "데몬 시작 대기 중",
        "백그라운드 서비스(cysd) 시작을 기다리고 있습니다. 계속 이 상태면: 시스템 설정 → 일반 → 로그인 항목에서 cys 관련 항목을 허용해 주세요. 허용 즉시 자동으로 연결됩니다.",
      );
    });
    listen("daemon-ready", () => dismissToast("daemon-hint"));
    const probe = setInterval(async () => {
      try {
        await invoke("daemon_status");
        clearInterval(probe);
        resolve();
      } catch {
        /* not yet */
      }
    }, 300);
  });

  const status = (await invoke("daemon_status")) as Record<string, unknown>;
  info.textContent = `daemon pid=${status.daemon_pid} sock=${status.socket_path}`;

  // 버전 스큐 세대교체(메인 + 부서 데몬) — 시작 1회 + 5분 주기 재검(B). 무중단 rename-swap의 짝으로
  // 구 데몬(lame-duck) 스큐를 비차단 배지로 알리고, 잃을 세션 0인 노드는 무손실 자동 교대한다.
  // 상세=checkVersionSkew(감지·배지 멱등·자동 교대·1회 능동 안내). 배지는 부가 기능이라 실패해도 시작 무영향.
  void checkVersionSkew();
  setInterval(() => void checkVersionSkew(), 5 * 60_000);

  await listen("daemon-event", (e) => onDaemonEvent(e.payload as Record<string, unknown>));

  // ── 파일 드래그&드롭 → 드롭한 pane의 PTY에 경로 주입(iTerm2 동작) ──
  // dragDropEnabled 기본 활성이라 Tauri가 OS 드롭을 가로채 tauri://drag-drop로 준다(HTML5 drop 미발화).
  // payload.position=물리 픽셀. 전역 listen은 target=Any라 창 라벨로 emit된 이 이벤트를 수신한다
  // (검증: tauri 2.11 event/listener.rs match_any_or_filter — listener.target==Any면 emit 타겟 무관 매칭).
  // F2: 트리 드래그와 동일 파이프라인(injectPathsToPane) — 재검증·스트리밍 가드·@멘션 형식 공유.
  // 직격 실패 시 무동작+토스트(포커스 pane 폴백 오배달 금지).
  await listen("tauri://drag-drop", (e) => {
    const p = (e.payload ?? {}) as { paths?: string[]; position?: { x: number; y: number } };
    const paths = p.paths ?? [];
    if (!paths.length) return;
    const rt = paneAtPointStrict(p.position);
    if (!rt) {
      if (panes.size > 0) toast("watchdog", "드롭 취소", "pane 위에 놓아야 삽입됩니다");
      return;
    }
    void injectPathsToPane(rt, paths);
  });

  // 바이너리 업데이트 진행률(install_update가 emit). chunk=이번 청크 바이트(누적 아님), total=전체(Option→null 가능).
  // ★재활성(오너 2026-07-15): promptBinaryPatch가 install_update를 다시 호출한다 — 이 리스너가
  //   "upd-bin" sticky 진행 토스트를 전담(backend install_update 주석과 짝).
  let updDownloaded = 0;
  await listen("update-progress", (e) => {
    const p = (e.payload ?? {}) as { phase?: string; chunk?: number; total?: number };
    const mb = (n: number) => (n / 1048576).toFixed(1);
    if (p.phase === "download") {
      if (p.chunk === undefined) {
        // chunk 없는 첫 download 이벤트 = 시작 신호 → 누적 카운터 리셋
        updDownloaded = 0;
        stickyToast("upd-bin", "feed", "⬇ 업데이트 설치", "다운로드 시작…");
        return;
      }
      updDownloaded += p.chunk;
      if (p.total && p.total > 0) {
        const pct = Math.floor((updDownloaded / p.total) * 100);
        stickyToast("upd-bin", "feed", "⬇ 업데이트 설치", `다운로드 중 ${mb(updDownloaded)} / ${mb(p.total)} MB (${pct}%)`);
      } else {
        stickyToast("upd-bin", "feed", "⬇ 업데이트 설치", `다운로드 중 ${mb(updDownloaded)} MB`);
      }
    } else if (p.phase === "drain") {
      stickyToast("upd-bin", "feed", "⬇ 업데이트 설치", "세션 정리 중…");
    } else if (p.phase === "handoff") {
      stickyToast("upd-bin", "feed", "⬇ 업데이트 설치", "재시작 준비 중…");
    }
  });

  // 무중단 팩 업데이트 진행 피드백(install_pack_update가 emit). ★app.restart 없음 — 세션 유지된 채 적용.
  await listen("pack-progress", (e) => {
    const p = (e.payload ?? {}) as { phase?: string };
    if (p.phase === "start")
      stickyToast("upd-pack", "feed", "🔄 무중단 적용 중", "서명검증 → 다운로드 → 원자적 팩 교체 → 노드 reinject…");
  });
  await listen("pack-updated", (e) => {
    const p = (e.payload ?? {}) as { pack_version?: string; reinject_failed?: number; reinject_deferred?: number };
    packUpdateAvailable = null;
    dismissToast("upd-pack"); // 진행 토스트를 내리고 아래 완료 토스트로 교대.
    const badge = document.getElementById("update-badge")!;
    if (!updateAvailable) badge.hidden = true; // 바이너리 업데이트가 별도로 남아있지 않으면 배지 해제
    // degraded(reinject 일부 실패/보류)면 '완료' 단정 회피 — 상세는 update-warning이 띄운다(모순 차단).
    const failed = p.reinject_failed ?? 0;
    const deferred = p.reinject_deferred ?? 0;
    if (failed > 0 || deferred > 0) {
      toast(
        "watchdog",
        "✅ 팩 디스크 반영 완료",
        `팩 ${p.pack_version ?? ""} 적용 — 세션 유지(재시작 없음). 일부 노드 reinject 보류/실패는 다음 폴링에서 재시도.`,
      );
    } else {
      toast(
        "watchdog",
        "✅ 팩 업데이트 완료",
        `팩 ${p.pack_version ?? ""} 적용 — 세션 유지·노드 reinject 완료(재시작 없음).`,
      );
    }
  });
  await listen("update-warning", (e) => {
    const p = (e.payload ?? {}) as { message?: string };
    dismissToast("upd-pack"); // 진행 토스트를 내리고 아래 경고 토스트로 교대.
    toast("health", "⚠ 팩 일부 미각성", p.message ?? "디스크 팩은 갱신됐으나 일부 노드 reinject 보류/실패(라이브 유지).");
  });

  // (T4) 업데이트 후 조직 복원 진행(restore-progress·spawn_org_restore emit) — '직원 복귀 중' 가시화.
  // ★TCC 처방(오너 2026-07-15): macOS 폴더 권한 거부 감지 → 안내(EPERM 실사고 — CLI 자식은
  // 팝업 없이 조용히 거부되므로 GUI가 유일한 안내 주체다).
  await listen("perm-warning", (e) => {
    const p = (e.payload ?? {}) as { folder?: string };
    const f = p.folder === "Documents" ? "문서" : "데스크탑";
    stickyToast(
      `perm-${p.folder ?? "folder"}`,
      "health",
      `⚠ macOS ${f} 폴더 접근 차단`,
      `pane 안의 claude 등이 EPERM으로 꺼질 수 있습니다 — 시스템 설정 → 개인정보 보호 및 보안 → 파일 및 폴더(또는 전체 디스크 접근 권한)에서 cys를 허용한 뒤 앱을 재시작하세요.`,
    );
  });
  // 완전 초기화 진행 이벤트 — sticky toast 본문을 단계 상세로 갱신(결과는 invoke 반환이 정본).
  await listen("reset-progress", (e) => {
    const p = (e.payload ?? {}) as { phase?: string; detail?: string };
    if (factoryResetting && p.detail) {
      stickyToast("factory-reset", "watchdog", "⟲ 완전 초기화 중", `[${p.phase ?? ""}] ${p.detail}`);
    }
  });

  await listen("restore-progress", (e) => {
    const p = (e.payload ?? {}) as { phase?: string; hq_ok?: boolean; ok?: number; fail?: number; detail?: string };
    // ★P1-3: 방금 조직을 지운 사용자에게 "직원 복귀 중"은 정반대 신호다. 리셋 진행/완료
    // 상태에서는 복원 토스트를 띄우지 않는다(복원 자체는 백엔드 판단이므로 표시만 억제).
    if (factoryResetting || resetCompleted) return;
    if (p.phase === "start") {
      stickyToast("restore", "feed", "👥 직원 복귀 중", "노드 세션 복원 중… (본부·부서)");
    } else if (p.phase === "done") {
      dismissToast("restore");
      const ok = p.ok ?? 0;
      const fail = p.fail ?? 0;
      // 결함1: 부서가 있어도 본부(HQ) 복원 실패가 묻히지 않게 hq_ok===false를 health로 승격.
      if (p.hq_ok === false) toast("health", "⚠ 본부 복원 실패 포함", `본부 노드 복원 실패 · 부서 성공 ${ok} · 실패 ${fail} — 상태를 점검하세요.`);
      else if (fail > 0) toast("health", "⚠ 직원 복귀 일부 실패", `부서 복원 성공 ${ok} · 실패 ${fail} — 상태를 점검하세요.`);
      else toast("watchdog", "✅ 직원 복귀 완료", `노드 세션 복원 완료 (부서 ${ok}).`);
    } else if (p.phase === "error") {
      dismissToast("restore");
      toast("health", "복원 실패", p.detail ?? "노드 복원 실행에 실패했습니다.");
    }
  });

  // (T4) init-pack 실패 등 backend update-error 가시화 — 이제껏 UI 리스너 부재로 침묵하던 갭 해소.
  await listen("update-error", (e) => {
    const msg = typeof e.payload === "string" ? e.payload : "업데이트 후 처리 중 오류가 발생했습니다.";
    toast("health", "업데이트 경고", msg);
  });

  // ★팀 기동 경고(적대검증 D-8): 마스터는 떴으나 cys boot가 팀(CSO·워커·리뷰어)을 못 세운 경우
  // (claude 미설치 등) 침묵하지 않고 안내 — 종전엔 실패를 삼켜 "팀 0개"를 사용자가 몰랐다.
  await listen("boot-warning", (e) => {
    const msg = typeof e.payload === "string" ? e.payload : "팀 기동에 실패했습니다.";
    stickyToast("boot-warn", "health", "팀 기동 경고", msg);
  });

  // ★T-0147-7 W4(B5·B16): 팀 부트 신호가 3등급으로 타입화됐다 — 경고(위) / 정보 / 경로 강등.
  // 종전엔 정상 상황(다른 boot 진행 중·부서 티켓 부재)도 '기동 실패' 경고로 나갔고(P3-B16 위경보),
  // 1차 경로 강등(python 부재 → cys boot 직접)은 **아무 신호도 없었다**(조용한 강등). 둘을 분리한다.
  // 두 토스트 모두 sticky 기본 TTL(60s)로 자동 소멸하고 알람 이력에 남는다(T-0147-3 계약 준수).
  await listen("boot-info", (e) => {
    const msg = typeof e.payload === "string" ? e.payload : "팀 기동 상태 안내입니다.";
    stickyToast("boot-info", "health", "팀 기동 안내", msg);
  });
  await listen("boot-degraded", (e) => {
    const msg =
      typeof e.payload === "string"
        ? e.payload
        : "팀 부트 1차 경로를 쓸 수 없어 직접 호출로 강등했습니다.";
    stickyToast("boot-degrade", "health", "팀 부트 경로 강등", msg);
  });

  // ★T2 안전모드(translocation/비정규 경로): 앱이 임시/비정규 위치에서 실행돼 데몬·launchd·팩 등록을
  // 전부 skip 한 경우(백엔드 조기 반환) 침묵하지 않고 설치 복구 절차를 sticky 로 안내한다. 이 경로에선
  // daemon-ready 가 오지 않아 상단바가 대기 상태로 남으므로, sticky 안내가 유일한 사용자 신호다.
  await listen("translocation-blocked", (e) => {
    const msg =
      typeof e.payload === "string"
        ? e.payload
        : "cys.app을 응용 프로그램(Applications) 폴더로 옮긴 뒤 다시 열어 주세요.";
    stickyToast("safe-mode", "health", "안전모드 — 설치 위치를 옮겨 주세요", msg);
  });

  // ★반쪽 번들 알림(ATOMIC-1 짝): 기동 자기점검과 **업데이트 설치 후 검증** 양쪽이 이 이벤트를 쏜다.
  // 후자는 재시작을 중단하고 이 안내를 띄운다 — 깨진 번들로 재시작하면 다음 기동을 Gatekeeper 가
  // 막아 사용자가 원인 없는 "손상되었기 때문에 열 수 없습니다"만 보게 되기 때문이다.
  // SEAL-DIAG 봉인 파손 emit 도 이 채널로 온다 — listen 등록 전에 나간 emit 은 위 start() 의
  // bundle_integrity pull 캐시 합산이 회수하고(F3 격차1), 여기 도착분과는 같은 토스트 id 로
  // dedupe 된다(stickyToast 같은 id 재호출 = 갱신 — 중복 토스트 없음).
  await listen("bundle-damaged", (e) => {
    const msg =
      typeof e.payload === "string"
        ? e.payload
        : "cys 설치본의 일부 구성요소가 빠졌습니다. 최신 DMG로 재설치해 주세요.";
    stickyToast("bundle-damaged", "health", "설치본이 온전하지 않습니다 — 재설치 필요", msg);
  });
  // ★[F3 재-pull] listen 등록 **직후** 1회 재-pull: start() 의 기동 pull 과 위 listen 등록
  // 사이 창에서 나간 emit 은 양쪽 다 놓친다(pull 이후·listen 이전 — emit-before-listen 의
  // 잔여 격차). 백엔드 캐시(SEAL_BROKEN_CACHE 합산 bundle_integrity)를 등록 직후 한 번 더
  // 읽어 그 창을 봉합한다. 같은 토스트 id("bundle-damaged")라 push/기동 pull 과 중복돼도
  // stickyToast dedupe 가 흡수한다(화면엔 한 줄). src-tauri 변경 불요 — 기존 커맨드 재사용.
  try {
    const damage = await invoke("bundle_integrity");
    if (typeof damage === "string" && damage) {
      stickyToast("bundle-damaged", "health", "설치본이 온전하지 않습니다 — 재설치 필요", damage);
    }
  } catch {
    /* 비-macOS·번들 밖 실행은 해당 없음 */
  }

  // 시작 시 + 6시간마다 백그라운드 업데이트 확인 (조용히 — 있으면 badge·toast)
  checkForUpdate(true);
  setInterval(() => checkForUpdate(true), 6 * 3600 * 1000);

  // 테스트 전용(패치 채널 E2E — 오너 2026-07-15): CYS_AUTOTEST_PATCH_INSTALL=1 env 기동이면 기동
  // 직후 패치 설치를 무클릭 자동 발화(Finder 런칭엔 env 부재 → 프로덕션 무영향). install_update가
  // 자체적으로 업데이트를 재확인하므로 updateAvailable 상태에 의존하지 않는다.
  (async () => {
    try {
      if ((await invoke("autotest_patch_install")) === true) {
        stickyToast("upd-bin", "feed", "⬇ 패치 설치(자동 테스트)", "패치 업데이트 확인·설치 중…");
        await invoke("install_update", { force: true });
      }
    } catch (e) {
      dismissToast("upd-bin");
      toast("health", "자동 테스트 패치 실패", String(e));
    }
  })();

  // Session restore (멀티마스터 F4): 저장본 먼저 로드(ws.socket 포함) → 부서 데몬 확보를 list 대조보다
  // 선행 → 소켓별 대조. 데몬 일시 미가동 ws는 보존(영구 삭제 방지, 검증 mustFix).
  try {
    const saved = JSON.parse(localStorage.getItem(LAYOUT_KEY) ?? "null");
    if (saved && Array.isArray(saved.workspaces)) {
      workspaces = saved.workspaces;
      groups = Array.isArray(saved.groups) ? saved.groups : []; // 06: 하위호환 — 옛 저장본엔 groups 없음
      activeWs = saved.active ?? 0;
      wsCounter = saved.counter ?? 1;
      groupCounter = saved.groupCounter ?? 1; // 06
    }
  } catch {
    workspaces = [];
    groups = []; // 06: 손상 저장본 폴백
  }
  for (const ws of workspaces) ws.socket = ws.socket ?? undefined; // 하위호환 마이그레이션(기본 데몬)
  // socket 1:1 수렴 + id 중복 제거(중복 탭 증식 차단) — 복원 적재 직후 단일 게이트.
  workspaces = normalizeWorkspaces(workspaces);
  // 카운터 보정: 신규 id/이름이 항상 기존 최댓값 초과하도록(중복·손상 저장본에도 강건)
  wsCounter = Math.max(wsCounter, 0, ...workspaces.map((w) => w.id)) + 1;
  // 06: 고아 그룹 청소 + groupCounter를 기존 최대 id+1로 보정(중복·손상 저장본에도 강건).
  groups = normalizeGroups(workspaces, groups);
  groupCounter = Math.max(groupCounter, 0, ...groups.map((g) => g.id)) + 1;

  // (order 8) 레지스트리 진실원 대조 — 죽은 socket이면서 레지스트리 미등록인 부서 ws는 유령(옛 테스트
  // 잔재·삭제된 부서)이므로 재-launch 안 하고 드롭. 조회 실패 시엔 보수적으로 전부 보존(기존 동작).
  let registered: Set<string> | null = null;
  // ＋부서 자동화(패치5·§E-4): socket→display_name 맵 — 복원 시 부서 탭 표시명 회복(rename=표시명 레이어).
  const displayBySocket = new Map<string, string>();
  try {
    const reg = (await invoke("list_depts")) as {
      depts?: Record<string, { socket?: string; display_name?: string }>;
    };
    registered = new Set(
      Object.values(reg.depts ?? {})
        .map((v) => v?.socket)
        .filter((s): s is string => !!s),
    );
    for (const e of Object.values(reg.depts ?? {})) {
      if (e?.socket && e.display_name) displayBySocket.set(e.socket, e.display_name);
    }
  } catch {
    registered = null;
  }
  // ★WP-3 리바이버 게이트: base 데몬 dept 묘비 — 삭제-의도 부서 탭은 등재 여부와 무관하게 드롭
  // (reg_remove 무음 실패로 등재가 잔존해도 부활 차단). 조회 실패=null(보수적 보존 — 현행 거동).
  let deptTombs: Set<string> | null = null;
  try {
    deptTombs = new Set((await invoke("dept_tombstones")) as string[]);
  } catch {
    deptTombs = null;
  }

  // 부서 데몬 확보를 list 대조보다 선행 — 미가동이면 cys-dept launch. 실패해도(등록된) ws는 보존.
  const ghosts = new Set<number>();
  for (const ws of workspaces.filter((w) => w.socket)) {
    // ★WP-3+R10: 묘비 검사를 생존 검사보다 **선행** — spawn_org_restore는 업데이트 후에만
    // 실행되므로(적대검증 보조 관찰), teardown 실패로 살아남은 묘비 데몬의 수렴 주체는 매 시작
    // 도는 이 루프다. 묘비+생존이면 탭 드롭+정리 시도(묘비가 부활을 차단하므로 best-effort).
    // 재생성 레이스 안전: 재생성 경로(allocate/create/launch)가 묘비를 선해소하므로 오드롭 없음.
    {
      const dn = deptNameFromSocket(ws.socket);
      if (deptTombs && dn && deptTombs.has(dn)) {
        ghosts.add(ws.id);
        invoke("stop_dept_daemon_by_socket", { socket: ws.socket }).catch(() => {});
        continue;
      }
    }
    let alive = false;
    try {
      await invoke("daemon_status", { socket: ws.socket });
      alive = true;
    } catch {
      alive = false;
    }
    if (alive) continue;
    // 죽은 socket + 레지스트리 미등록 → 유령 → 드롭(재-launch로 부활시키지 않음)
    if (registered && ws.socket && !registered.has(ws.socket)) {
      ghosts.add(ws.id);
      continue;
    }
    // 등록된(또는 레지스트리 미조회) 부서 → 재-launch. ★시나리오4: rename으로 ws.name이 바뀌어도
    // socket(진짜 정체·불변)에서 원래 부서명을 역산해 호출 — '다른 소켓 새 데몬'이 원래 데몬을 고아화하지 않게.
    try {
      const info = (await invoke("launch_dept_daemon", { name: deptNameFromSocket(ws.socket) ?? ws.name })) as { socket: string; socket_slug?: string };
      if (info.socket_slug && info.socket) socketForSlug.set(info.socket_slug, info.socket);
      if (info.socket) ws.socket = info.socket; // 재-launch된 실제 socket 반영(이후 집계·prune·병합 정합)
    } catch {
      /* 데몬 확보 실패 — 등록된 ws는 빈 채 보존(저장본 삭제 금지) */
    }
  }
  if (ghosts.size) workspaces = workspaces.filter((w) => !ghosts.has(w.id));

  // 소켓별 live 집계 — 데몬 미응답(ok=false) 소켓은 판정 보류(죽은 pane 제거 스킵, ws 보존).
  const sockets = [...new Set(workspaces.map((w) => w.socket))];
  const liveBySock = new Map<
    string | undefined,
    { ids: Set<number>; ok: boolean; list: { surface_id: number; title: string }[] }
  >();
  for (const sk of sockets) {
    try {
      const r = (await invoke("list_surfaces", { socket: sk })) as {
        surfaces: { surface_id: number; title: string; exited: boolean }[];
      };
      const liveList = r.surfaces.filter((s) => !s.exited);
      liveBySock.set(sk, { ids: new Set(liveList.map((s) => s.surface_id)), ok: true, list: liveList });
    } catch {
      liveBySock.set(sk, { ids: new Set(), ok: false, list: [] });
    }
  }

  // 죽은 pane 제거 — 데몬 미응답 소켓의 ws는 건드리지 않는다(일시 미가동=영구삭제 방지).
  const activeWsId = workspaces[activeWs]?.id;
  for (const ws of workspaces) {
    const lb = liveBySock.get(ws.socket);
    if (!lb || !lb.ok) continue;
    for (const sid of collectSids(ws.tree)) {
      if (!lb.ids.has(sid)) ws.tree = ws.tree ? replaceNode(ws.tree, sid, () => null) : null;
    }
  }
  // 안 A: 부서 ws는 tree:null(빈 셸 미생성)로 저장될 수 있다 — 데몬이 살아있고 입양할 live surface가
  // 있으면(master 등) 드롭하지 말고 보존한다. 아래 입양 루프(병합)가 그 surface로 tree를 채운다.
  // master 자동기동 제거 후: 비활성 부서가 재-launch로 surface 0개로 올라와도 데몬이 살아있으면(ok===true)
  // 드롭하지 말고 보존한다 — 아래 빈-tree 충전 루프가 plain 셸로 채운다(비활성 부서 탭 소실 방지).
  workspaces = workspaces.filter((ws) => {
    if (ws.tree !== null) return true;
    const lb = liveBySock.get(ws.socket);
    if (lb?.ok === false) return true;
    return ws.socket != null && lb?.ok === true;
  });
  // 구버전 자동 번호 이름("ws N")은 미정 표시로 이행
  for (const ws of workspaces) {
    if (/^ws \d+$/.test(ws.name)) ws.name = UNTITLED;
    // §E-4: 부서 탭 표시명 복원 — 표시명이 비었거나(미정·dept-N 번호) 레지스트리에 display_name 이 있으면
    // 그 표시명으로 회복. 사용자가 의미있게 rename 한 이름(레지스트리와 다른 값)은 덮지 않는다.
    if (ws.socket) {
      const disp = displayBySocket.get(ws.socket);
      if (disp && (ws.name === UNTITLED || ws.name === "…" || /^dept-\d+$/.test(ws.name))) {
        ws.name = disp;
      }
    }
  }
  if (workspaces.length === 0) {
    workspaces = [{ id: wsCounter++, name: UNTITLED, tree: null }];
  }
  const restoredIdx = workspaces.findIndex((ws) => ws.id === activeWsId);
  activeWs = restoredIdx >= 0 ? restoredIdx : Math.min(activeWs, workspaces.length - 1);

  // pane 런타임 생성 + 고아(레이아웃에 없는 살아있는 surface)는 같은 소켓 ws에 병합.
  for (const sk of sockets) {
    const lb = liveBySock.get(sk);
    if (!lb || !lb.ok) continue;
    const ws = workspaces.find((w) => (w.socket ?? undefined) === (sk ?? undefined));
    for (const s of lb.list) {
      await makePane(s.surface_id, s.title, sk);
      if (ws && !collectSids(ws.tree).includes(s.surface_id)) {
        ws.tree = ws.tree
          ? { type: "split", dir: "row", a: ws.tree, b: { type: "pane", sid: s.surface_id } }
          : { type: "pane", sid: s.surface_id };
      }
    }
  }
  // master 자동기동 제거 후: 데몬은 살아있으나(ok===true) 입양할 surface가 0개인 부서 ws(비활성 부서가
  // 재-launch된 경우)는 위 병합 루프가 못 채운다 — plain 셸 1개로 충전해 빈 탭 소실/고아 placeholder 방지.
  for (const ws of workspaces) {
    if (ws.tree || ws.socket == null || liveBySock.get(ws.socket)?.ok !== true) continue;
    const sid = await newSurface(null, ws.socket);
    ws.tree = { type: "pane", sid };
  }
  if (!current().tree) {
    // 복원 시 current()가 미응답(ok===false) 부서 ws일 수 있다(필터의 ok===false 절로 보존·activeWs가 선택,
    // 충전 루프는 ok!==true라 스킵) — 죽은 부서 socket에 newSurface하면 backend가 reject해 복원이 깨진다.
    // 기본 데몬(socket undefined·상시 가용)으로 폴백해 빈 화면/미처리 rejection을 막는다(정상 경로 불변).
    let sid: number;
    try {
      sid = await newSurface(null, current().socket);
    } catch {
      sid = await newSurface(null, undefined);
    }
    current().tree = { type: "pane", sid };
  }
  render();
  const first = collectSids(current().tree)[0];
  if (first != null) setFocus(first);
  refreshFeed();
  started = true; // 복원 완료 — 이 시점부터 인터벌 자동 입양 허용
  refreshPaneTitles();
  // 사이드바 노드 신호(B3): 시작 1회 + 10s idle 폴백(이벤트 구동은 onDaemonEvent에서). CC 5s 폴링보다 가벼움.
  refreshSidebarStatus();
  setInterval(refreshSidebarStatus, 10000);
}

// ---------- ui wiring ----------

document.getElementById("btn-new")!.addEventListener("click", actionNew);
document.getElementById("btn-split-h")!.addEventListener("click", () => actionSplit("row"));
document.getElementById("btn-split-v")!.addEventListener("click", () => actionSplit("col"));
document.getElementById("btn-equalize")!.addEventListener("click", actionEqualize);
document.getElementById("btn-close")!.addEventListener("click", actionClose);
document.getElementById("btn-files")!.addEventListener("click", () => setFtOpen(!ftOpen));
document.getElementById("btn-ft-close")!.addEventListener("click", () => setFtOpen(false));
document.getElementById("btn-cc")!.addEventListener("click", () => setCcOpen(!ccOpen));
document.getElementById("btn-cc-close")!.addEventListener("click", () => setCcOpen(false));
document.getElementById("btn-cc-density")!.addEventListener("click", () =>
  applyCcDensity(ccDensity === "glance" ? "ops" : "glance"),
);
document.getElementById("btn-cc-glance-face")!.addEventListener("click", () =>
  applyGlanceFace(ccGlanceFace === "tasks" ? "live" : "tasks"),
);
// 셸 cys 설치/해제 — index.html에서 hidden으로 시작하고 macOS에서만 연다(D2 플랫폼 게이팅).
// Rust의 non-macOS Err 는 심층방어로 남아 있고, 이 줄은 "보이는데 안 되는 버튼"을 없애는 앞단이다.
// 요소 참조는 `?.`로 둔다 — 버튼이 다시 삭제되더라도(가역 계약) 모듈 로드가 통째로 죽지 않게.
if (IS_MACOS) {
  const cliBtn = document.getElementById("btn-install-cli");
  if (cliBtn) cliBtn.hidden = false;
  applyCliButtonView(); // 조회 전 기본 라벨(=HTML 초기값과 동일) 확정 — 상태는 CC 열 때 채운다
}
document.getElementById("btn-install-cli")?.addEventListener("click", async () => {
  const b = document.getElementById("btn-install-cli") as HTMLButtonElement | null;
  if (!b || b.disabled) return;
  // ★(I2) 클릭 분기는 **라벨을 만든 그 판정**을 그대로 쓴다. cliStatus.button 만 보면
  // '다시 설치'라고 적힌 버튼이 해제를 집행하는 창이 열린다(사용자가 본 것과 다른 행동).
  const wantUninstall = cliButtonIntent(cliStatus.button, cliLastInstall) === "uninstall";
  // ★(MAJOR-3 · 8R) 재진입 차단을 **첫 await 앞**으로 올린다.
  //
  // 예전에는 이 줄이 확인 모달 뒤에 있었다. 그래서 해제 경로만, 모달을 await 하는 동안 버튼이
  // 살아 있었다 — 오버레이(.modal-overlay · position:fixed·inset:0)가 **마우스**는 가리지만
  // 포커스는 방금 누른 이 버튼에 그대로 남고, 전역 키 핸들러는 수식키 없는 입력을 그냥 흘려보낸다
  // (이 파일 말미 `if (!mod) return`). 그래서 Enter/Space 한 번이면 핸들러에 다시 들어와 확인 창이
  // 둘 뜨고, 둘 다 승인하면 `uninstall_cli_from_path` 가 **동시에 두 번** 나간다(승격 프롬프트도 둘).
  // 설치 경로는 첫 await 가 invoke 라 원래도 막혀 있었지만, **같은 가드를 경로마다 다른 자리에
  // 두는 것** 자체가 이 어긋남의 원인이었다 — 자리를 하나로 통일한다.
  b.disabled = true; // in-flight 이중 진입 차단(확인 모달·승격 프롬프트가 떠 있는 동안)
  // 해제는 root 소유 심링크를 지우는 비가역에 가까운 행위 — 클릭 즉시 집행하지 않고 확인을 먼저 받는다.
  // alert()/confirm()은 이 WKWebView에서 억제된다는 실측(B-11)이 있어 순수 DOM confirmModal을 쓴다.
  // (I3②) 확인 창에 **현재 상태**를 함께 실어 준다 — 남의 파일 고지(notes)와 잔존 백업본(backups).
  // 문구를 읽어 분류하지 않는다: 분기의 근거는 두 배열의 길이뿐이고, 내용은 옮기거나(notes)
  // 경로에서 만든다(backups — 백엔드는 사실만, 표현은 UI 소유).
  if (wantUninstall) {
    // ★(MAJOR-2 · 7R) linkState 도 함께 넘긴다 — 확인 창에 실리는 백업본 줄이 '그 자리가 지금
    // 비어 있는가' 를 알아야 파괴적인 'sudo mv' 를 내지 않는다(자리가 차 있으면 명령 대신
    // "해제하면 앱이 되돌립니다" 라고 말한다).
    const c = uninstallConfirmText(cliStatus.notes, cliStatus.backups, cliStatus.linkState);
    if (!(await confirmModal(c.title, c.body, c.yes, c.no))) {
      // 취소 — 집행한 것이 없으니 재조회도 하지 않는다(쓸데없이 로그인 셸을 띄우지 않는다).
      b.disabled = false;
      return;
    }
  }
  // ★(G2) 결과 알림은 **이 액션당 정확히 하나**다. 예전에는 여기서 결과 토스트를 내고, finally 의
  // 재조회가 곧바로 상시 고지 토스트를 또 냈다 — 백업이 일어난 설치 1클릭이 서로 다른 문장의
  // sticky 두 개(cli-install + cli-status-notes)로 같은 사실을 말했다. 계획을 들고 있다가
  // 재조회 뒤에 한 번만 낸다(그래야 접어 넣는 고지 줄이 **방금 재조회한 최신 사실**이 된다).
  let plan: ToastPlan | null = null;
  try {
    // 응답 판독은 read*Report(unknown → 계약 모양)가 한다 — `as T ?? {}` 캐스트로 모양을 가정하면
    // Rust 와 어긋난 채 조용히 통과한다(MAJOR-5 의 본체가 정확히 그 캐스트였다).
    if (wantUninstall) {
      const rep = readUninstallReport(await invoke("uninstall_cli_from_path"));
      // 해제가 실측으로 확인됐으면 설치 래치도 함께 푼다 — 지운 뒤에 '다시 설치'가 남으면
      // 그것 또한 라벨과 상태의 어긋남이다(래치는 잔상이지 상태가 아니다).
      if (rep.ok) cliLastInstall = null;
      // (R1) 남은 링크의 복구 명령('sudo rm <경로>')은 **UI 가 조립한다** — 백엔드는 5R 부터
      // 사실만 보낸다. 조립의 후보는 직전 상태 조회가 준 두 링크 경로이고, 그 값을 못 읽었으면
      // 빈 문자열이라 후보가 사라진다(없는 경로를 지목하지 않는다).
      plan = uninstallResultToast(rep, [cliStatus.cysLink, cliStatus.cysdLink]);
    } else {
      const rep = readInstallReport(await invoke("install_cli_to_path"));
      // installed 가 아니면(그림자화·확인 불가) 다음 라벨은 '해제'가 아니라 '다시 설치'다.
      cliLastInstall = normalizeInstallStatus(rep.status);
      plan = installResultToast(rep);
    }
  } catch (e) {
    // 실패(취소·승격 거부·Err)는 볼 것이 남은 결과다 — 60초 sticky 로 낸다.
    // ★실패도 같은 통로로 낸다(G1 대칭): 승격이 거부돼도 이미 옮겨진 원본이 남아 있을 수 있고,
    // 그 사실은 실패 알림에도 실려야 한다 — 실패 경로만 덜 말하면 그것이 곧 자기보고 미도달이다.
    plan = {
      category: "watchdog",
      title: wantUninstall ? "셸 cys 해제 실패" : "셸 설치 실패",
      body: String(e),
      sticky: true,
      id: wantUninstall ? UNINSTALL_TOAST_ID : INSTALL_TOAST_ID,
    };
  } finally {
    // (MINOR-11) 재조회를 **기다린 뒤** 활성화한다. fire-and-forget 이면 버튼이 먼저 살아나
    // 라벨이 아직 이전 상태(예: 방금 설치했는데 '설치')인 창이 열리고, 그 창에서 누르면
    // 사용자가 본 라벨과 다른 행동이 나간다. refreshCliInstallState 는 내부에서 삼키므로 throw 없음.
    // 단, 재조회가 영영 돌아오지 않는 환경(응답 없는 invoke)에서 버튼이 **영구 비활성**으로
    // 죽는 것은 더 나쁘다 — 3초 상한을 걸고 그 뒤에는 라벨이 늦더라도 버튼을 돌려준다.
    await Promise.race([
      // 액션 직후 1회 재조회로 라벨 갱신(폴링 아님). 상시 고지 토스트는 여기서 내지 않는다 —
      // 아래에서 결과 토스트 하나로 접어 넣는다(G2).
      // ★(MAJOR-3) force: 이 재조회만은 중복 억제를 건너뛴다 — CC 를 열 때 시작된 프로브가 아직
      // 떠 있으면 그 답은 **액션 이전의 사실**이라, 그것으로 대신하면 라벨·고지 줄이 낡는다.
      refreshCliInstallState({ notice: false, force: true }),
      new Promise<void>((resolve) => setTimeout(resolve, 3000)),
    ]);
    if (plan) showCliToast(withCliNotice(plan, cliNoticeLines(cliStatus)));
    b.disabled = false;
  }
});
document.querySelectorAll("#cc-tabs .cc-tab").forEach((b) =>
  b.addEventListener("click", () => setCcTab((b as HTMLElement).dataset.view as typeof ccTab)),
);
document.querySelectorAll("#cc-eff-win .cc-win").forEach((b) =>
  b.addEventListener("click", () => {
    ccEffWindow = (b as HTMLElement).dataset.window!;
    document.querySelectorAll("#cc-eff-win .cc-win").forEach((x) => x.classList.toggle("active", x === b));
    refreshEfficiency();
  }),
);
document.querySelectorAll("#cc-skills-win .cc-win").forEach((b) =>
  b.addEventListener("click", () => {
    ccSkillsWindow = (b as HTMLElement).dataset.window!;
    document.querySelectorAll("#cc-skills-win .cc-win").forEach((x) => x.classList.toggle("active", x === b));
    refreshSkills();
  }),
);
document.querySelectorAll("#cc-sessions-win .cc-win[data-window]").forEach((b) =>
  b.addEventListener("click", () => {
    ccSessionsWindow = (b as HTMLElement).dataset.window!;
    document.querySelectorAll("#cc-sessions-win .cc-win[data-window]").forEach((x) => x.classList.toggle("active", x === b));
    refreshSessions();
  }),
);
document.getElementById("cc-sessions-star-filter")!.addEventListener("click", (e) => {
  ccSessionsStarOnly = !ccSessionsStarOnly;
  (e.currentTarget as HTMLElement).classList.toggle("active", ccSessionsStarOnly);
  refreshSessions();
});
document.getElementById("cc-sessions-redact")!.addEventListener("click", (e) => {
  ccSessionsRedact = !ccSessionsRedact;
  (e.currentTarget as HTMLElement).classList.toggle("active", ccSessionsRedact);
  ccSessionSelected = null;
  refreshSessions();
});
// 계정 라벨 가림 토글 — 상태 영속 + refreshControlCenter로 KPI·계정 섹션 재렌더(라벨→해시).
const acctRedactBtn = document.getElementById("btn-cc-acct-redact")!;
acctRedactBtn.classList.toggle("active", ccAcctRedact); // 복원 상태 반영
acctRedactBtn.addEventListener("click", (e) => {
  ccAcctRedact = !ccAcctRedact;
  localStorage.setItem("cys-cc-acct-redact", ccAcctRedact ? "1" : "0");
  (e.currentTarget as HTMLElement).classList.toggle("active", ccAcctRedact);
  refreshControlCenter();
});
// 스킬 보드 검색 — 카탈로그 버튼 필터(재fetch 없이 renderBoardDomains 재렌더).
document.getElementById("cc-board-search")!.addEventListener("input", (e) => {
  ccBoardSearch = (e.currentTarget as HTMLInputElement).value;
  renderBoardDomains();
});
document.getElementById("btn-update")!.addEventListener("click", () => onUpdateButton());
document.getElementById("btn-restart-daemon")!.addEventListener("click", () => void manualRestartAllDaemons());
document.getElementById("btn-factory-reset")!.addEventListener("click", () => void factoryResetFlow());
document.getElementById("btn-theme")!.addEventListener("click", (e) =>
  openThemePopover(e.currentTarget as HTMLElement),
);
// 역할 분리(오너 2026-06-29 결정): "새 워크스페이스"(btn-ws-new) = 기본/현재 데몬의 일반 워크스페이스
// (addWorkspace) — 부서가 아니다. 격리 부서 데몬 생성은 "+부서"(btn-ws-dept→addDeptWorkspace) 전담.
// 새 ws를 master로 선언 시 공유 데몬 claim 충돌은 데몬 레벨 claim_denied(cysd handlers.rs·kill 없음)가
// 비파괴 방어한다(생태계 죽지 않음·거부만). guard-master-claim(Fix2') 부트 자동발동 배선은 별건(헌법 토큰).
document.getElementById("btn-ws-new")!.addEventListener("click", () => {
  if (daemonActionBlocked()) return; // ★P1-3: 리셋 진행/완료 중 새 워크스페이스 차단(무반응 금지)
  void addWorkspace();
});

// (2026-08-20 P2) ▶CEO/▶부서장/셸설치 버튼 3종 제거 — 기동 경로는 pane 마스터 선언(role-bootstrap 훅 체인)·cys launch-agent·phoenix 복원으로 잔존.
// Rust 커맨드(start_master 등)는 존치(git log 참조). 버튼 복원은 HTML 2줄+핸들러 재추가로 가역.
// (2026-08-25) ★셸설치 버튼만 복원 — 제거 사유였던 결함 4종을 함께 고쳤다: ①플랫폼 게이팅 없음
// (macOS 양성 판정 시에만 hidden 해제) ②그림자화·검증실패를 "설치 완료"로 오보고(status 3분류 →
// 등급 분리) ③해제 경로 부재(같은 버튼이 상태 2종을 겸함 + confirmModal 확인) ④검증 무기한 대기.
// 배선은 #btn-cc-glance-face 리스너 직후, 판정은 clipath.ts(순수·clipath.test.ts). ▶CEO/▶부서장
// 두 버튼은 **복원 대상이 아니다** — 위 이원 경로가 정본이다.

// ★R8(WP-2): 시작 시 1회 CEO PENDING 고지 — cys-dept 알림이 가리키는 실존 컨트롤(팔레트
// "CEO 승격 진행")로 안내. 폴링 없음(시작 1회+팔레트 온디맨드 — WINAUDIT 타이머 증식 방지).
(async () => {
  try {
    if (await invoke("ceo_pending")) {
      toast("feed", "CEO 승격 대기 중", "부서가 존재합니다 — base 부트 완료 후 명령 팔레트의 'CEO 승격 진행'으로 승인할 수 있습니다.");
    }
  } catch {
    /* 데몬 미가동 등 — 침묵(다음 시작·팔레트에서 재확인) */
  }
})();

// ---------- 사이드바 폭 드래그 + 글자 배율 (오너 요청 2026-07-12) ----------
// 폭·배율은 CSS 변수(--wsbar-w/--wsbar-font)가 진실원, localStorage 영속. 클램프 산식=wsbar.ts.
// pane 재렌더는 이중 안전: 각 pane의 ResizeObserver(→fitPane)가 폭 변화에 자동 발화하고,
// 드래그 종료 시 refitAllPanes()로 전 pane 강제 재적합+xterm 재렌더를 한 번 더 보장한다.
let wsbarW = clampWsbarWidth(Number(localStorage.getItem("cys-wsbar-w")) || WSBAR_W_DEFAULT);
let wsbarFont = clampWsbarFont(Number(localStorage.getItem("cys-wsbar-font")) || 1);
function applyWsbarVars() {
  document.documentElement.style.setProperty("--wsbar-w", `${wsbarW}px`);
  document.documentElement.style.setProperty("--wsbar-font", String(wsbarFont));
}
applyWsbarVars(); // 마운트 시 저장값 복원

function refitAllPanes() {
  for (const rt of panes.values()) {
    fitPane(rt); // 숨김/미배치 pane은 fitPane 내부 가드가 거른다
    rt.term.refresh(0, rt.term.rows - 1); // PTY rows/cols 불변이어도 화면 재렌더 보장
  }
}

const wsbarDrag = document.getElementById("wsbar-drag");
wsbarDrag?.addEventListener("mousedown", (e0: MouseEvent) => {
  e0.preventDefault();
  const startX = e0.clientX, startW = wsbarW;
  document.body.classList.add("wsbar-resizing");
  const move = (e: MouseEvent) => {
    wsbarW = clampWsbarWidth(startW + (e.clientX - startX));
    applyWsbarVars(); // 드래그 중 실시간 반영 — pane ResizeObserver가 연속 refit(60ms 디바운스)
  };
  const up = () => {
    window.removeEventListener("mousemove", move, true);
    window.removeEventListener("mouseup", up, true);
    document.body.classList.remove("wsbar-resizing");
    localStorage.setItem("cys-wsbar-w", String(wsbarW));
    refitAllPanes();
  };
  window.addEventListener("mousemove", move, true);
  window.addEventListener("mouseup", up, true);
});
wsbarDrag?.addEventListener("dblclick", () => {
  wsbarW = WSBAR_W_DEFAULT;
  applyWsbarVars();
  localStorage.setItem("cys-wsbar-w", String(wsbarW));
  refitAllPanes();
});

function applyWsbarFontStep(dir: number) {
  wsbarFont = clampWsbarFont(wsbarFont + dir * WSBAR_FONT_STEP);
  applyWsbarVars();
  localStorage.setItem("cys-wsbar-font", String(wsbarFont));
}
document.getElementById("btn-ws-font-minus")?.addEventListener("click", () => applyWsbarFontStep(-1));
document.getElementById("btn-ws-font-plus")?.addEventListener("click", () => applyWsbarFontStep(+1));
// 멀티마스터 F4 + ＋부서 자동화(패치5): 새 부서(독립 데몬) workspace 런칭. 부서 번호는 백엔드가 확정.
const deptBtn = document.getElementById("btn-ws-dept") as HTMLButtonElement | null;
// 부서 런칭 실행(공통) — placeholder 탭·in-flight 버튼 가드. catalogKey=undefined → 레거시 dept-N.
// ⑤(gemini R2): invoke 실패 reject 를 try/catch 로 받아 토스트+버튼 disabled 해제(버튼 freeze 방지).
// ①(gemini R2 ★BLOCKER): create exit code 별 분기 — exit5(account dir 미존재=계정누수)는 레거시 폴백 절대 금지.
async function launchDept(catalogKey?: string) {
  if (daemonActionBlocked()) return; // ★A4: 리셋 진행/완료 중 부서 데몬 spawn 차단
  if (!deptBtn || deptBtn.disabled) return; // 연타 차단 — in-flight launch 중 재실행 방지
  const prevLabel = deptBtn.textContent;
  deptBtn.disabled = true;
  deptBtn.textContent = "…"; // 진행 표시 — launch await 동안(placeholder 탭은 즉시 보임)
  let fallbackLegacy = false;
  try {
    await addDeptWorkspace(catalogKey);
  } catch (e) {
    // main.rs 가 create 실패를 'dept-create:<code>:<stderr>' 로 전달(레거시 allocate 실패는 평문).
    const msg = String(e);
    const m = /^dept-create:(-?\d+):/.exec(msg);
    const code = m ? parseInt(m[1], 10) : null;
    if (code === 5) {
      // ★보안: account dir 미존재 = 계정 격리 불가 → 비격리 레거시 dept-N 으로 우회 금지(계정누수 차단)·하드 에러.
      toast("watchdog", "부서 생성 차단(계정 격리 불가)", "account dir 미존재 — 레거시 폴백 금지(보안). 카탈로그 account 경로 점검.");
    } else if (code === 4) {
      // 카탈로그에 정의되지 않은 키 → 에러(레거시 폴백 안 함 — 의도치 않은 무명 부서 방지).
      toast("watchdog", "부서 생성 실패(카탈로그 키)", "카탈로그 미정의 부서 — 레거시 폴백 안 함.");
    } else if (code === 3) {
      // 카탈로그 파일 부재(비격리 위험 없음·번호만) → 레거시 dept-N 허용.
      toast("watchdog", "카탈로그 없음", "레거시 dept-N 으로 생성합니다.");
      fallbackLegacy = true;
    } else {
      toast("watchdog", "부서 런칭 실패", msg);
    }
  } finally {
    deptBtn.disabled = false; // 버튼 freeze 방지 — 성공/실패 무관 항상 해제
    deptBtn.textContent = prevLabel;
  }
  // exit3(카탈로그 부재)만 레거시 폴백 — 버튼 재활성 후 호출해 disabled 가드 통과(exit4/5 는 폴백 없음).
  if (fallbackLegacy) await launchDept(undefined);
}
// 클릭 → 부서 선택 팝업(카탈로그 미사용 부서 + 레거시 dept-N). 선택 후 부서 데몬 런칭.
deptBtn?.addEventListener("click", async () => {
  if (deptBtn.disabled) return; // 연타 차단
  if (!started) return; // ★시나리오3: 복원 진행 중 발급 금지(레지스트리 미확정 윈도우 회피)
  // 현재 열린 부서 탭의 mission_key 집계 → '미사용 부서'만 제시. 레지스트리 socket↔mission_key 대조(데몬 호출 없음·경량).
  const openSockets = new Set(workspaces.map((w) => w.socket).filter((s): s is string => !!s));
  const runningKeys = new Set<string>();
  try {
    const reg = (await invoke("list_depts")) as {
      depts?: Record<string, { socket?: string; mission_key?: string }>;
    };
    for (const e of Object.values(reg.depts ?? {})) {
      if (e?.mission_key && e.socket && openSockets.has(e.socket)) runningKeys.add(e.mission_key);
    }
  } catch {
    /* 레지스트리 미조회 — 필터 없이 전체 제시 */
  }
  let cat: { departments?: Record<string, { display?: string; mission_key?: string }> } = {};
  try {
    cat = (await invoke("read_dept_catalog")) as typeof cat;
  } catch {
    cat = {};
  }
  const items: { label: string; action: () => void }[] = [];
  for (const [key, d] of Object.entries(cat.departments ?? {})) {
    if (d.mission_key && runningKeys.has(d.mission_key)) continue; // 미사용 부서만
    items.push({ label: d.display ?? key, action: () => launchDept(key) });
  }
  items.push({ label: "직접 입력(레거시 dept-N)", action: () => launchDept(undefined) });
  // 미사용 부서가 하나도 없어 레거시만 남으면(카탈로그 부재/손상 OR 6부서 전부 가동중) 팝업 없이 바로 레거시
  // — '버튼 한 번' 유지(클릭 추가 0)·버튼 브릭 방지. 단 카탈로그엔 부서가 있는데 전부 가동중이면
  //   침묵 생성이 혼란스러우므로 토스트로 사유를 알린다(클릭은 여전히 한 번).
  if (items.length === 1) {
    if (Object.keys(cat.departments ?? {}).length > 0) {
      toast("watchdog", "모든 부서 가동 중", "레거시 dept-N 워크스페이스를 생성합니다.");
    }
    launchDept(undefined);
    return;
  }
  const r = deptBtn.getBoundingClientRect();
  showCtxMenu(r.left, r.bottom, items);
});

window.addEventListener("keydown", (e) => {
  if (e.isComposing || e.keyCode === 229) return; // IME 조합 중 무시
  if (paletteOpen) return; // 07: 팔레트 열림 중 전역 단축키 누수 차단(검색 타이핑이 ⌘W/T/D/G 발화 방지 · 적대검증 교정)
  // ★P1-3: 확인 모달이 떠 있으면 전역 단축키가 **뒤의 세션을 건드리지 못하게** 막는다.
  // 시뮬레이션 지적: 파괴적 다이얼로그를 닫으려던 사용자의 ⌘W 가 확인 없이 뒤 pane 을 죽였다.
  if (document.querySelector(".modal-overlay")) return;
  const mod = e.metaKey || e.ctrlKey;
  if (!mod) return;
  if (e.key === "k") {
    // 07: ⌘K — Command Palette 기동(미사용 키, 충돌 없음)
    e.preventDefault();
    openPalette();
    return;
  }
  if (e.key === "t") {
    e.preventDefault();
    actionNew();
  } else if (e.key === "d" && !e.shiftKey) {
    e.preventDefault();
    actionSplit("row");
  } else if ((e.key === "D" || e.key === "d") && e.shiftKey) {
    e.preventDefault();
    actionSplit("col");
  } else if (e.key === "w") {
    e.preventDefault();
    actionClose();
  } else if (e.key === "=" || e.key === "+") {
    e.preventDefault();
    ccOpen ? applyPanelZoom(+1) : applyZoom(+1);
  } else if (e.key === "-") {
    e.preventDefault();
    ccOpen ? applyPanelZoom(-1) : applyZoom(-1);
  } else if (e.key === "0") {
    e.preventDefault();
    ccOpen ? applyPanelZoom(null) : applyZoom(null);
  } else if (e.key === "g" && ccOpen) {
    // HUD-5: ⌘G로 Glance↔Ops 전환(CC 열린 동안만 — 일반 ⌘G와 충돌 회피)
    e.preventDefault();
    applyCcDensity(ccDensity === "glance" ? "ops" : "glance");
  }
});

start().catch((e) => {
  document.getElementById("daemon-info")!.textContent = `startup failed: ${e}`;
});
