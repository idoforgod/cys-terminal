// cys UI — xterm.js panes over the cysd socket (thin client).
// 세션 영속은 구조로 해결: 세션(PTY)은 데몬 소유, UI는 attach만 한다.

import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { isPermissionGranted, requestPermission, sendNotification } from "@tauri-apps/plugin-notification";

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
  titleEl: HTMLElement;
  usageEl: HTMLElement;
  term: Terminal;
  fit: FitAddon;
  unlisten: (() => void)[];
  observer: ResizeObserver;
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
let ccTab: "live" | "eff" | "skills" | "sessions" | "weekly" | "learn" | "board" | "tasks" | "feed" = "live";
let ccEffWindow = "today";
let ccSkillsWindow = "today";
let ccSessionsWindow = "7d";
let ccSessionsStarOnly = false;
let ccSessionsRedact = false;
let ccSessionSelected: string | null = null;

// HUD-5: 밀도 모드 — 비기술자 Glance(오늘 큰 글씨) ↔ 엔지니어 Ops(6탭). body class 1개가 진실원.
type CcDensity = "ops" | "glance";
let ccDensity: CcDensity =
  (localStorage.getItem("cys-cc-density") as CcDensity) === "glance" ? "glance" : "ops";
// Tasks Control Center: Glance 모드 안에서 보여줄 면(Live=시스템부하 ↔ tasks=부서 업무) — 오너 선택.
let ccGlanceFace: "live" | "tasks" =
  localStorage.getItem("cys-cc-glance-face") === "tasks" ? "tasks" : "live";
// 마지막 org_fleet 스냅샷 — 실시간 이벤트(task.changed/status.changed)가 셀 단위로 패치한다.
let lastFleet: any = null;

const CC_ROLE_COLOR: Record<string, string> = {
  master: "#3b82f6", cso: "#8b5cf6", worker: "#00e676",
  "reviewer-gemini": "#ffa726", "reviewer-codex": "#00d4ff",
};
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

function setCcOpen(open: boolean) {
  ccOpen = open;
  document.getElementById("cc-panel")!.hidden = !open;
  if (open) {
    applyCcDensity(ccDensity); // 저장된 밀도 모드 복원(class·버튼 라벨)
    refreshControlCenter();
    refreshHw();
    tickCc();
    if (ccTimer == null) ccTimer = setInterval(refreshControlCenter, 5000) as unknown as number;
    if (ccHwTimer == null) ccHwTimer = setInterval(refreshHw, 2000) as unknown as number;
    if (ccClockTimer == null) ccClockTimer = setInterval(tickCc, 1000) as unknown as number;
  } else {
    if (ccTimer != null) { clearInterval(ccTimer); ccTimer = null; }
    if (ccHwTimer != null) { clearInterval(ccHwTimer); ccHwTimer = null; }
    if (ccClockTimer != null) { clearInterval(ccClockTimer); ccClockTimer = null; }
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

function setCcTab(view: "live" | "eff" | "skills" | "sessions" | "weekly" | "learn" | "board" | "tasks" | "feed") {
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
  if (view === "feed") refreshFeed();
}

// D5: 스킬 버튼 보드 — 카탈로그 큐레이션 렌더 + 일회용 워커 실행 + 산출물 회수(터미널 입력 0회).
async function refreshBoard() {
  const cat = (await invoke("read_board_catalog").catch(() => ({ domains: [], actions: [] }))) as any;
  const host = document.getElementById("cc-board-domains")!;
  host.innerHTML = "";
  for (const d of cat.domains ?? []) {
    const sec = document.createElement("div");
    sec.className = "cc-board-domain";
    sec.innerHTML = `<div class="cc-board-domain-h">${ccEsc(d.label ?? d.id ?? "")}</div>`;
    const wrap = document.createElement("div");
    wrap.className = "cc-board-btns";
    for (const s of d.skills ?? []) {
      if ((s.acl ?? 1) > 1) continue; // 비기술자: acl≤1만 (민감/위험 스킬은 카탈로그 미포함=암묵 차단)
      const b = document.createElement("button");
      b.className = "cc-board-btn";
      b.textContent = s.label ?? s.name;
      b.title = `${s.scope ?? ""}${s.gate === "hitl" ? " · 미리보기 확인 필요" : ""}`;
      b.onclick = () => runSkillButton(s);
      wrap.appendChild(b);
    }
    sec.appendChild(wrap);
    host.appendChild(sec);
  }
  // SB-4: actions(write-a-skill 등) 1급 노출 — 도메인과 동일 실행 경로(신규 인프라 0)
  const actions = cat.actions ?? [];
  if (actions.length) {
    const sec = document.createElement("div");
    sec.className = "cc-board-domain";
    sec.innerHTML = `<div class="cc-board-domain-h">도구</div>`;
    const wrap = document.createElement("div");
    wrap.className = "cc-board-btns";
    for (const a of actions) {
      if ((a.acl ?? 1) > 1) continue;
      const b = document.createElement("button");
      b.className = "cc-board-btn";
      b.textContent = a.label ?? a.name;
      b.onclick = () =>
        runSkillButton({
          name: a.name,
          label: a.label ?? a.name,
          scope: "새 스킬 만들기 (write-a-skill — 일상 워크플로우를 스킬로 codify)",
          success: "SKILL.md 4칸 본문 생성·트리거 명확",
          gate: "hitl",
        });
      wrap.appendChild(b);
    }
    sec.appendChild(wrap);
    host.appendChild(sec);
  }
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
    b.addEventListener("click", () => invoke("open_path", { path: b.dataset.path }).catch(() => {})),
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
    let userInput = "";
    if (s.gate === "hitl") {
      // D6 제품 모드: 본문 원고/주제 입력 모달(HITL 보존·신뢰선 라벨·게이트 건너뛰기 금지)
      const got = await inputModal(
        s.label ?? s.name,
        s.scope ?? "내용을 입력하세요",
        "여기에 본문 원고나 주제를 붙여넣으세요…",
      );
      if (got === null) return; // 취소
      userInput = got;
    }
    // ★무계약 차단: task-prompt 티켓을 먼저 생성(javis_orchestra 경유). UI는 ticket 텍스트만 받는다.
    const scope = userInput ? `${s.scope ?? ""} · 입력 원고: ${userInput}` : s.scope ?? "";
    const ticket = (await invoke("make_ticket", {
      task: s.label ?? s.name,
      scope,
      success: s.success ?? "",
      to: "worker",
    })) as string;
    await invoke("run_skill", { name: s.name, ticket, agent: "claude", closeAfter: null });
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
          return `<div class="cc-learn-row"><span class="cc-learn-round">${ccEsc(k)}</span><span class="cc-learn-verdict" style="color:${vColor[v] ?? "inherit"}">${ccEsc(v)}</span><span class="cc-learn-meta">저장 ${r?.stored?.length ?? 0} · harness ${r?.harness?.length ?? 0}</span></div>`;
        })
        .join("")
    : `<div class="cc-empty">학습 라운드 기록 없음 — RSI 라운드(javis_rsi.py checkpoint)가 기록을 남기면 여기 표시됩니다</div>`;

  const ribbons: string[] = [];
  for (const k of keys) for (const h of rounds[k]?.harness ?? []) ribbons.push(`${k}: ${h.retention ?? "?"}`);
  document.getElementById("cc-learn-retention")!.innerHTML = ribbons.length
    ? ribbons.map((t) => `<span class="cc-learn-ribbon ${t.includes("keep") ? "keep" : "rollback"}" title="retention: keep=개선 채택 유지 / rollback=회귀로 되돌림">${ccEsc(t)}</span>`).join("")
    : `<div class="cc-empty">채택/롤백 기록 없음</div>`;

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
    invoke("open_path", { path: ev }).catch(() => toast("watchdog", "근거 파일 없음", ev));
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
  document.getElementById("cc-kpi")!.innerHTML = ["5h", "7d"]
    .map((lab) => {
      const w = agg[lab];
      const used = w ? Math.round(w.used) : 0;
      const name = lab === "5h" ? "세션 (5h)" : "주간 (7d)";
      const tip = lab === "5h" ? "최근 5시간 rate limit 사용률 (전 노드 최대값)" : "최근 7일 rate limit 사용률 (전 노드 최대값)";
      return `<div class="cc-card ${sevClass(used, 60, 80)}" title="${tip}"><div class="cc-card-val">${used}%</div><div class="cc-card-reset">${w ? ccReset(lab, w.reset) : ""}</div><div class="cc-card-name">${name}</div></div>`;
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
      const w = agg[lab];
      const used = w ? Math.round(w.used) : 0;
      const name = lab === "5h" ? "세션" : "주간";
      return `<div class="cc-tbar"><span class="cc-tbar-lab">${name}</span><span class="cc-tbar-track"><span class="cc-tbar-fill ${sevClass(used, 60, 80)}" style="width:${Math.min(100, used)}%"></span></span><span class="cc-tbar-pct">${used}%</span><span class="cc-tbar-reset">${w ? ccReset(lab, w.reset) : ""}</span></div>`;
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

// Control Center 본문 전용 zoom — 터미널 fontSize와 분리(배율 단위).
// WebKit `zoom`을 #cc-body에만 적용(host #cc-panel은 fixed라 zoom 시 위치/스크롤 회귀 → 본문만 확대,
// sticky 헤더·탭은 1.0x 유지). 사이드바(ft/feed)는 터미널 작업공간 폭이라 zoom 비대상(터미널 fit 회귀 방지).
let panelZoom = Math.min(2, Math.max(0.6, Number(localStorage.getItem("cys-panel-zoom")) || 1)); // NaN·범위밖 방어
function applyPanelZoomVar() {
  document.documentElement.style.setProperty("--panel-zoom", String(panelZoom));
}
applyPanelZoomVar(); // 마운트 시 저장된 배율 복원
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
let pendingApprovals = 0; // org.status feed.pending 집계
const root = document.getElementById("root")!;

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

// surface도 번호 대신 이름 — 기본 자동 제목("surface N"·빈 문자열)이면 현재 디렉토리 경로 표시.
const isAutoTitle = (t: string | null | undefined) => !t || /^surface \d+$/.test(t);
const paneTitle = (title: string | null | undefined, liveCwd?: string | null) =>
  isAutoTitle(title) ? liveCwd || "…" : (title as string);

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
        await makePane(s.surface_id, s.title, sk);
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
  header.append(titleEl, usageEl, closeBtn);
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
    // ★Windows: Latin 등폭폰트(Cascadia Mono/Consolas)를 CJK 폰트보다 앞에 둔다. 아니면 Menlo/SF Mono
    //   부재 시 xterm가 셀 폭을 CJK 전각폰트(Noto Sans KR)로 측정해 Latin 글자가 넓게 벌어진다(자간 이상).
    fontFamily: "Menlo, 'SF Mono', 'Cascadia Mono', Consolas, 'Apple SD Gothic Neo', 'Malgun Gothic', 'Noto Sans KR', monospace",
    fontSize,
    theme: { background: "#0d1117", foreground: "#c9d1d9" },
    scrollback: 5000,
  });
  const fit = new FitAddon();
  term.loadAddon(fit);
  term.open(termHost);

  // WKWebView IME(한글 등 CJK) 조합 가드: 조합 중 keydown(keyCode 229/isComposing)을
  // xterm이 일반 키로 처리하면 자모가 분리 입력된다 — 조합 완성분만 onData로 흐르게 차단.
  term.attachCustomKeyEventHandler((e) => {
    if (e.isComposing || e.keyCode === 229) return false;
    // ★붙여넣기(F2): Ctrl/Cmd+V·Ctrl+Shift+V 를 xterm이 \x16(literal)로 삼키지 않게 false 반환 →
    // 브라우저 네이티브 paste 이벤트가 발화되고 아래 paste 리스너가 클립보드를 PTY로 보낸다.
    // (WebView2에서 xterm 기본 붙여넣기가 안 먹던 문제 — permission 불요의 clipboardData 경로.)
    if ((e.ctrlKey || e.metaKey) && (e.key === "v" || e.key === "V")) return false;
    return true;
  });

  // 전송 직렬화 체인: 빠른 타자에서 비동기 IPC 호출이 경주하면 도착 순서가 뒤집힌다 —
  // promise 체인으로 같은 pane의 모든 입력을 발사 순서대로 보장한다.
  let sendChain: Promise<unknown> = Promise.resolve();
  const sendRaw = (data: string) => {
    sendChain = sendChain
      .then(() => invoke("send_input", { socket, surfaceId: sid, data }))
      .catch(() => {});
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
      if (text) term.paste(text);
    },
    true,
  );

  // ── WKWebView 한글 IME 조합 상태 머신 ──────────────────────────────────
  // WKWebView는 composition 이벤트 없이 ①음절 첫 자모를 insertText로 커밋(xterm이 즉시
  // 전송해버림 = 자모 유출) ②조합 진행을 insertReplacementText로 value 치환(xterm 미인지 =
  // 완성 글자 유실)한다. 여기서 자모 유출을 차단하고 음절 확정 시 완성 글자만 보낸다.
  let pendingHangul = "";
  // 자모(31xx·11xx) + 완성형 음절(AC00-D7A3) — ★멀티문자 허용(2026-06-13): 고속 입력에서
  // IME가 여러 음절을 한 insertText로 병합 커밋하는데, 단일 문자만 인정하면 그 묶음이
  // input 핸들러에서 무시되고 onData(고속에서 발화 비결정)도 못 받쳐 통째로 유실된다
  // — "4자 치면 2자" 절반 유실의 주 경로.
  const isHangulText = (t: string) => /^[\u3131-\u318E\u1100-\u11FF\uAC00-\uD7A3]+$/.test(t);
  // Incomplete jamo only (composing lead/medial \u2014 excludes complete syllables AC00-D7A3).
  // Used by onData to detect whether a leftover `pendingHangul` is the composing jamo of the
  // same syllable xterm just completed, so it can be dropped instead of leaked ahead of it.
  const isIncompleteJamo = (t: string) => /^[\u3131-\u318E\u1100-\u11FF]+$/.test(t);
  // IME 계측(사람 단계 재현용): localStorage.cysImeDebug="1" 설정 시 이벤트 시퀀스를
  // /tmp/cys-ime.log에 기록 — 유실 경로를 결정론으로 확정하는 채널. 평시 비용 0.
  const imeDbg = localStorage.getItem("cysImeDebug") === "1";
  const dbg = (line: string) => {
    if (imeDbg) invoke("log_ime", { line: `[s${sid}] ${line}` }).catch(() => {});
  };
  const flushPending = (why: string) => {
    if (pendingHangul) {
      dbg(`FLUSH(${why}) "${pendingHangul}"`);
      sendRaw(pendingHangul);
      pendingHangul = "";
    }
  };

  term.onData((data) => {
    // 한글 음절: 이 WebKit은 조합을 표준 composition inputType(insertFromComposition)으로
    // 확정하고, 그때 xterm이 완성 음절을 onData로 정확히 1회 발화한다 — 차단 없이 그대로
    // PTY로 보낸다. (구 isHangulText 차단은 insertText 기반 상태머신을 전제했으나, 실기기
    // WebKit은 insertCompositionText/insertFromComposition을 보내 그 머신이 작동한 적이
    // 없었고, 차단만 살아 순수 한글 음절을 통째로 유실시켰다 — "너는 마스터다"→"는 다".)
    // ★한글 초성 이중방출 차단: 이 WebKit은 input(insertText, 초성 자모)로 pending에 초성을
    // 버퍼한 뒤, 같은 음절을 composition 경로로 onData(완성 음절)까지 발화한다. 여기서 무조건
    // flushPending하면 버퍼된 초성이 완성 음절 앞으로 새어 "ㅎ한"이 된다. → pending이 '조합중
    // 자모'(완성 음절 아님)이고 지금 data가 한글 음절이면, 그 자모는 이 음절의 조합중 상태이므로
    // 폐기한다(유출 금지). pending이 완성형(진짜 직전 음절)이면 기존대로 flush해 순서 보존.
    if (pendingHangul && isIncompleteJamo(pendingHangul) && isHangulText(data)) {
      dbg(`DROP-pending "${pendingHangul}" (초성 조합중) superseded by onData "${data}"`);
      pendingHangul = "";
    } else {
      flushPending("onData"); // (no-op 안전장치: 잔여 pending 있으면 순서 보존 후 전송)
    }
    sendRaw(data);
  });

  // ★F 수정: 위 IME 조합 상태 머신은 macOS WKWebView 전용 우회다. Windows WebView2 등 Chromium
  // 계열은 xterm.js 네이티브 composition이 완성 음절을 onData로 정확히 1회 발화하므로, 이 우회를
  // 함께 켜면 input 핸들러가 pendingHangul에 버퍼한 글자를 onData의 flushPending이 보내고
  // sendRaw(data)가 다시 보내 **이중 전송**된다("너"→"너너" 전 글자 중복 — Windows 실측).
  // ∴ WKWebView(AppleWebKit·비-Chromium)에서만 상태 머신을 붙인다(macOS 회귀0).
  const _ua = navigator.userAgent;
  const isWKWebView = /AppleWebKit/.test(_ua) && !/Chrome|Chromium|Edg\//.test(_ua);
  if (isWKWebView) {
    const ta = term.textarea;
    if (ta) {
      ta.addEventListener("input", (e) => {
        const ie = e as InputEvent;
        dbg(`input ${ie.inputType} data="${ie.data ?? "∅"}" pending="${pendingHangul}"`);
        if (ie.inputType === "insertText" && ie.data && isHangulText(ie.data)) {
          // 직전 조합 확정 후 새 커밋을 '수정 가능 창'(pending)에 둔다. 병합 커밋
          // (2음절+)은 마지막 음절만 수정 창에 — 앞 음절들은 확정분이므로 즉시 전송
          // (replacement 재조합은 마지막 음절 단위로 온다).
          flushPending("insertText");
          if (ie.data.length > 1) {
            dbg(`SEND(multi-head) "${ie.data.slice(0, -1)}"`);
            sendRaw(ie.data.slice(0, -1));
          }
          pendingHangul = ie.data.slice(-1);
        } else if (ie.inputType === "insertReplacementText" && ie.data) {
          if (pendingHangul) {
            pendingHangul = ie.data; // 조합 갱신 (하→한)
          } else {
            // 이미 전송된 직전 음절의 교정 — PTY 동기화: 백스페이스+재전송
            dbg(`SEND(repl-sync) DEL+"${ie.data}"`);
            sendRaw("\x7f" + ie.data);
          }
        } else if (ie.inputType === "deleteContentBackward" && pendingHangul) {
          // 멀티 pending(병합 커밋 잔여)이면 마지막 글자만 — IME 부분 재조합 대응
          pendingHangul = pendingHangul.slice(0, -1);
          dbg(`del-backward pending="${pendingHangul}"`);
        }
      });
      ta.addEventListener("keydown", (e) => {
        if (imeDbg && e.keyCode !== 229) dbg(`keydown ${e.key}`);
        // 일반 키(Enter·Space·화살표 등, IME 처리중 229 제외) 직전에 조합 확정
        if (e.keyCode !== 229) flushPending("keydown");
        // 조합 중이 아닐 때 textarea 잔여 value 정리 (IME value 누적 방지)
        if (e.keyCode !== 229 && !pendingHangul && ta.value.length > 64) {
          (ta as HTMLTextAreaElement).value = "";
        }
      });
      ta.addEventListener("blur", () => flushPending("blur"));
    }
  }
  el.addEventListener("mousedown", () => setFocus(sid));
  term.textarea?.addEventListener("focus", () => setFocus(sid));

  // attach 먼저 — 백엔드가 (소켓 slug, surface_id) 이벤트명을 만들어 반환한다(단일 진실, UI 재계산 금지).
  const ev = (await invoke("attach_surface", { socket, surfaceId: sid })) as {
    output_event: string;
    exited_event: string;
  };
  const un1 = await listen(ev.output_event, (e) => {
    term.write(b64ToBytes(e.payload as string));
  });
  const un2 = await listen(ev.exited_event, () => {
    term.write("\r\n\x1b[31m[surface exited]\x1b[0m\r\n");
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

  const rt: PaneRuntime = { sid, socket, el, termHost, titleEl, usageEl, term, fit, unlisten: [un1, un2], observer };
  panes.set(paneKey(sid, socket), rt);
  return rt;
}

/// Fit only when actually laid out — a detached/hidden pane must not shrink the PTY.
function fitPane(rt: PaneRuntime) {
  if (rt.termHost.offsetWidth < 60 || rt.termHost.offsetHeight < 40) return;
  rt.fit.fit();
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
    const over = (document.elementFromPoint(e.clientX, e.clientY) as HTMLElement | null)?.closest(
      ".pane",
    ) as HTMLElement | null;
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
    if (dragging && target) movePane(sid, target.sid, target.side);
  };
  window.addEventListener("mousemove", move, true);
  window.addEventListener("mouseup", up, true);
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

// ---------- render ----------

function render() {
  for (const rt of panes.values()) rt.el.remove();
  root.innerHTML = "";
  const tree = current()?.tree;
  if (tree) root.appendChild(renderNode(tree));
  renderWsTabs();
  requestAnimationFrame(() => {
    for (const sid of collectSids(current()?.tree ?? null)) {
      const rt = panes.get(paneKey(sid, current()?.socket));
      if (rt) fitPane(rt);
    }
  });
  saveLayout();
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

// ---------- 정렬: 역할(role) 기반 고정 배치 ----------
// 현재 워크스페이스의 살아있는 surface를 역할별 표준 자리로 재배치한다:
//   · 왼쪽 끝 컬럼  = master(위) / cso(아래), 세로 5:1
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
  // 왼쪽 끝: master(위) / cso(아래) = 5:1 (누락 시 있는 쪽이 컬럼 전체)
  if (master != null && cso != null) columns.push({ type: "split", dir: "col", ratio: 5 / 6, a: pane(master), b: pane(cso) });
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
  for (const sock of sockets) {
    try {
      const r = (await invoke("org_status", { socket: sock })) as {
        surfaces?: any[];
        feed?: { pending?: number };
      };
      pend += r.feed?.pending ?? 0;
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
  tab.style.borderLeftColor = color; // ws 고유색은 좌측 바 (사이드바 항목 식별)
  const titleRow = document.createElement("div");
  titleRow.className = "ws-title-row";
  const label = document.createElement("span");
  label.className = "ws-name";
  label.textContent = ws.name;
  const close = document.createElement("span");
  close.className = "ws-close";
  close.textContent = "×";
  close.title = "워크스페이스 닫기 (surface 전부 종료)";
  titleRow.append(label, close);
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
    const i = workspaces.indexOf(ws); // 그룹 재배열로 시각 순서≠배열 순서 — 실시간 위치로 전환
    if (i !== activeWs) {
      activeWs = i;
      render();
      const first = collectSids(current().tree)[0];
      if (first != null) setFocus(first);
    }
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
    ]);
  });
  close.addEventListener("click", async () => {
    // WKWebView에서 confirm()은 무동작 — 2-click 확인 패턴 사용
    if (close.dataset.arm !== "1") {
      close.dataset.arm = "1";
      close.innerHTML = TRASH_SVG;
      close.classList.add("close-armed");
      close.title = "한 번 더 누르면 삭제";
      setTimeout(() => {
        close.dataset.arm = "";
        close.textContent = "×";
        close.classList.remove("close-armed");
        close.title = "워크스페이스 닫기 (surface 전부 종료)";
      }, 2500);
      return;
    }
    for (const sid of collectSids(ws.tree)) {
      await invoke("close_surface", { socket: ws.socket, surfaceId: sid }).catch(() => {});
      destroyPaneRuntime(sid, ws.socket);
    }
    const i = workspaces.indexOf(ws); // 캡처된 idx는 stale일 수 있음 — 실시간 위치로 식별
    if (i < 0) { render(); return; } // 이미 제거된 ws 재클릭 — no-op
    workspaces.splice(i, 1);
    // 부서 데몬 teardown은 '그 socket을 쓰는 마지막 탭'일 때만(중복 탭 잔존 시 다른 탭 보호)
    const stillUsed = ws.socket && workspaces.some((w) => w.socket === ws.socket);
    // socket 기준 teardown(order 8) — ws rename으로 name↔socket이 끊겨도 정확히 종료.
    if (ws.socket && !stillUsed) await invoke("stop_dept_daemon_by_socket", { socket: ws.socket }).catch(() => {});
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
function showCtxMenu(x: number, y: number, items: { label: string; action: () => void }[]) {
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
    row.className = "ctx-item";
    row.textContent = it.label;
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

async function addWorkspace(): Promise<Workspace> {
  const sid = await newSurface();
  const ws: Workspace = { id: wsCounter++, name: UNTITLED, tree: { type: "pane", sid } };
  workspaces.push(ws);
  activeWs = workspaces.length - 1;
  render();
  setFocus(sid);
  return ws;
}

// 부서 socket 경로(~/.local/state/cys-dept-<name>/cys.sock)에서 원래 부서명 역산.
// rename으로 ws.name이 바뀌어도 socket은 불변이므로, 재-launch가 '다른 소켓 새 데몬'을 만들어
// 원래 데몬을 고아화하는 것을 막는다(시나리오4).
function deptNameFromSocket(sock: string | undefined): string | null {
  const m = /\/cys-dept-(.+?)\/cys\.sock$/.exec(sock ?? "");
  return m ? m[1] : null;
}

// 멀티마스터 F4: 새 '부서 workspace' 런칭 = 새 부서 데몬 spawn(cys-dept launch 단일 진입점).
// 첫 부서가 생기면 백엔드(cys-dept)가 기본 데몬을 CEO로 자동 승격한다.
// ① 표시 지연(안 C): 무거운 launch await(최대 ~12s) '전에' placeholder 탭을 즉시 render — 체감 지연 0.
// ② 고아 방지(안 A): 빈 newSurface를 만들지 않는다. cys-dept가 띄우는 role=master surface가
//    refreshPaneTitles 자동입양으로 '첫 pane'이 되게 한다(빈 셸 미생성 → 고아 0).
async function addDeptWorkspace(catalogKey?: string): Promise<Workspace> {
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
    // 안 A(C4 더블 surface 해소): cys-dept(create=javis_boot_node·allocate=자동각성)가 부서장 role=master
    // surface를 띄우므로 UI는 plain 셸을 직접 만들지 않는다. socket 확정 + pending 해제 → refreshPaneTitles
    // 자동입양이 그 master를 '첫 pane'으로 채운다(rolePri master=0 → 좌측·focus). 빈 셸 0·더블 surface 0.
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

// 같은 ws의 첫 surface가 지금 있는 경로 — 새 surface의 시작 경로로 쓴다 (이후 이동은 사용자 몫)
async function firstPaneCwd(): Promise<string | null> {
  const first = collectSids(current().tree)[0];
  if (first == null) return null;
  try {
    const r = (await invoke("list_surfaces", { socket: current()?.socket })) as {
      surfaces: { surface_id: number; live_cwd: string | null }[];
    };
    return r.surfaces.find((s) => s.surface_id === first)?.live_cwd ?? null;
  } catch {
    return null;
  }
}

async function actionNew() {
  if (current()?.pending) return; // 부서 데몬 준비 중(빈 socket placeholder) — surface 생성 금지(기본 데몬 고아 차단)
  const sid = await newSurface(await firstPaneCwd(), current().socket);
  const ws = current();
  ws.tree = ws.tree
    ? { type: "split", dir: "row", a: ws.tree, b: { type: "pane", sid } }
    : { type: "pane", sid };
  render();
  setFocus(sid);
}

async function actionSplit(dir: "row" | "col") {
  const ws = current();
  // stale focusedSid 검증 — 트리에 없는 대상을 분할하면 replaceNode가 무음 no-op 되어
  // 보이지 않는 고아 surface(살아있는 PTY)가 생긴다
  if (focusedSid == null || !ws.tree || !collectSids(ws.tree).includes(focusedSid)) {
    return actionNew();
  }
  const target = focusedSid;
  const sid = await newSurface(await firstPaneCwd(), ws.socket);
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
}

// 승인 Feed는 Control Center의 '승인 Feed' 탭으로 편입됨(독립 패널 폐기).
// 여는 동작 = CC 패널 오픈 + 탭 활성(setCcTab이 refreshFeed 호출).
function openFeed() {
  if (!ccOpen) setCcOpen(true);
  setCcTab("feed");
}

// ---------- file tree (오른쪽 섹션 — 선택한 surface의 폴더 탐색) ----------

let ftOpen = false;
let ftRoot: string | null = null;
const ftExpanded = new Set<string>(); // 펼쳐진 하위 폴더 경로

function setFtOpen(open: boolean) {
  ftOpen = open;
  document.getElementById("ft-panel")!.hidden = !open;
  if (open) updateFtRoot(); // pane 폭 변화는 ResizeObserver가 자동 보정
}

// 포커스된 surface의 현재 경로를 트리 루트로 — 포커스 이동·cd 모두 추적
async function updateFtRoot() {
  if (!ftOpen || focusedSid == null) return;
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

async function renderFileTree() {
  const body = document.getElementById("ft-body")!;
  const label = document.getElementById("ft-root-label")!;
  if (!ftRoot) {
    body.innerHTML = "";
    label.textContent = "파일";
    return;
  }
  label.textContent = ftRoot.split("/").pop() || ftRoot;
  label.title = ftRoot;
  const frag = await buildDirNodes(ftRoot, 0);
  body.innerHTML = "";
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
      if (ent.is_dir) {
        if (ftExpanded.has(full)) ftExpanded.delete(full);
        else ftExpanded.add(full);
        renderFileTree();
      } else {
        invoke("open_path", { path: full }).catch(() => {}); // 시스템 기본 앱으로 열기
      }
    });
    frag.appendChild(row);
    if (ent.is_dir && ftExpanded.has(full)) frag.appendChild(await buildDirNodes(full, depth + 1));
  }
  return frag;
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
    box.textContent = "(비어 있음)";
    return;
  }
  for (const item of items.slice(0, 50)) {
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
    if (item.status === "pending") {
      const actions = document.createElement("div");
      actions.className = "fi-actions";
      for (const [label, decision, cls] of [["Allow", "allow", "allow"], ["Deny", "deny", "deny"]] as const) {
        const btn = document.createElement("button");
        btn.className = cls;
        btn.textContent = label;
        btn.addEventListener("click", async () => {
          await invoke("feed_reply", { requestId: item.request_id, decision }).catch(() => {});
          refreshFeed();
          refreshSidebarStatus(); // 결정 직후 집계 배지 즉시 갱신
        });
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

let updateAvailable: { version: string; notes?: string } | null = null;
// 무중단 팩 업데이트(check_pack_update) 결과 — 팩만 변경 시 세션·데몬 유지 경로(install_pack_update).
let packUpdateAvailable: { pack_version: string; manifest_url: string; binary_too_old: boolean } | null = null;

/// 업데이트 확인. silent=true면 시작 시 백그라운드 체크(결과 없으면 조용히).
/// 바이너리(check_update·재시작)와 무중단 팩(check_pack_update·세션 유지)을 둘 다 확인해 분기한다.
async function checkForUpdate(silent: boolean) {
  // 1) 바이너리 업데이트(Tauri updater latest.json) — 재시작 경로.
  let bin: { version: string; current?: string; notes?: string } | null = null;
  let binCheckFailed = false;
  try {
    bin = (await invoke("check_update")) as typeof bin;
  } catch (e) {
    // ★early-return 안 함(팩 체크는 계속) — 단, 바이너리 상태 불명을 기억해 아래 '최신' 단정을 억제한다.
    binCheckFailed = true;
    if (!silent) toast("health", "업데이트 확인 실패", String(e));
  }
  // 2) 무중단 팩 업데이트(pack-manifest.json) — 세션·데몬 유지 경로. 실패는 조용히(폴링).
  let pack: { pack_version: string; manifest_url: string; binary_too_old: boolean } | null = null;
  let packCheckFailed = false;
  try {
    pack = (await invoke("check_pack_update")) as typeof pack;
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
  if (updateAvailable) {
    // 바이너리 우선 — 재시작이 팩도 함께 반영(DESIGN '둘 다 → 바이너리 우선').
    badge.hidden = false;
    badge.textContent = "!";
    badge.classList.remove("ok");
    badge.title = `새 버전 ${updateAvailable.version} (재시작 필요)`;
    if (!silent) promptInstall();
    else toast("feed", "🔄 업데이트 있음", `새 버전 ${updateAvailable.version} — 상단 Update(재시작)`);
  } else if (packUpdateAvailable && !packUpdateAvailable.binary_too_old) {
    // 팩만 변경 + 바이너리 호환 → 무중단 가능(세션·데몬 생존).
    badge.hidden = false;
    badge.textContent = "↻";
    badge.classList.remove("ok");
    badge.title = `팩 ${packUpdateAvailable.pack_version} (무중단·세션 유지)`;
    if (!silent) promptPackInstall();
    else
      toast("feed", "↻ 무중단 팩 업데이트", `팩 ${packUpdateAvailable.pack_version} — 상단 Update(재시작 없음)`);
  } else if (packUpdateAvailable && packUpdateAvailable.binary_too_old) {
    // 팩은 있으나 min_binary_version > 설치 바이너리 → 무중단 불가, 바이너리 업데이트 필요.
    badge.hidden = false;
    badge.textContent = "!";
    badge.classList.remove("ok");
    badge.title = `팩 ${packUpdateAvailable.pack_version}: 바이너리 업데이트 필요`;
    const msg = `새 팩 ${packUpdateAvailable.pack_version}은 더 새로운 바이너리를 요구합니다 — 바이너리 업데이트(재시작) 후 적용됩니다.`;
    if (!silent) toast("health", "바이너리 업데이트 필요", msg);
    else toast("feed", "⚠ 업데이트 있음", msg);
  } else {
    // ★fail-safe: 양쪽 체크가 모두 성공적으로 '없음'을 확인했을 때만 상태를 갱신한다. 장애(체크 실패)
    // 시엔 마지막 검증 상태(배지)를 유지한다 — 일시 장애로 "최신" 오단정하지 않게.
    if (!binCheckFailed && !packCheckFailed) {
      // 오너 지시(2026-07-03): 최신 확인 시 숨김 대신 "0" 표시 — "확인이 끝났고 대기 업데이트
      // 0건"을 명시(숨김은 '아직 확인 전'과 구별 불가였다). 중립 스타일(.ok)로 경고색 회피.
      badge.hidden = false;
      badge.textContent = "0";
      badge.classList.add("ok");
      badge.title = "최신 버전 — 대기 중인 업데이트 없음";
    }
    // 어느 한쪽이라도 체크 실패면 상태 불명 — '이미 최신' 단정 금지(바이너리·팩 둘 다 성공 확인 시에만).
    if (!silent && !binCheckFailed && !packCheckFailed) toast("watchdog", "✅ 최신 버전", "최신 버전입니다. 추가 업데이트가 없습니다.");
  }
}

/// 설치 확인 + 데몬 핸드오프 정책(세션 없으면 자동·있으면 확인).
async function promptInstall() {
  if (!updateAvailable) {
    await checkForUpdate(false);
    return;
  }
  const v = updateAvailable.version;
  const sessions = (await invoke("live_session_count").catch(() => 0)) as number;
  let force = false;
  if (sessions > 0) {
    // WKWebView는 confirm 지원이 불안정 → 커스텀 모달
    const ok = await confirmModal(
      `새 버전 ${v} 설치`,
      `현재 작업 세션 ${sessions}개가 데몬에 물려 있습니다. 업데이트는 데몬을 재시작하므로 ` +
        `이 세션들이 종료됩니다.\n\n그래도 지금 설치하시겠습니까? (아니오: 세션을 정리한 뒤 다시 시도)`,
    );
    if (!ok) return;
    force = true;
  }
  toast("feed", "⬇ 업데이트 설치", `버전 ${v} 다운로드 중… 완료 후 자동 재시작됩니다.`);
  try {
    await invoke("install_update", { force });
    // 성공 시 app.restart()로 프로세스가 교체되므로 이 줄에 도달하지 않는다.
  } catch (e) {
    const msg = String(e);
    if (msg.includes("live_sessions:")) {
      // 가드에 막힘(force 미적용 경로) — 다시 확인 흐름으로
      await promptInstall();
    } else {
      toast("health", "업데이트 설치 실패", msg);
    }
  }
}

/// 무중단 팩 설치 — install_pack_update(세션·데몬 생존, app.restart 없음) 호출.
/// 진행/완료/경고는 pack-progress·pack-updated·update-warning 리스너가 표시한다(아래 startup).
/// ★"재시작" 확인 다이얼로그를 띄우지 않는다 — 세션이 죽지 않는 게 바이너리 경로와의 핵심 차이.
async function promptPackInstall() {
  if (!packUpdateAvailable) {
    await checkForUpdate(false);
    return;
  }
  const pv = packUpdateAvailable.pack_version;
  toast("feed", "↻ 무중단 팩 업데이트", `팩 ${pv} 적용 중… 세션·데몬 유지(재시작 없음).`);
  try {
    await invoke("install_pack_update", { manifestUrl: packUpdateAvailable.manifest_url });
    // 성공(또는 degraded)은 pack-updated/update-warning 리스너가 후속 처리.
  } catch (e) {
    // 백엔드가 update-error도 emit하지만, join/실행 단계 실패는 emit 없이 reject되므로 여기서 표시.
    toast("health", "팩 업데이트 실패", String(e));
  }
}

/// Update 버튼 디스패처 — 가용 업데이트 종류에 따라 경로를 고른다.
/// 바이너리 우선(재시작이 팩 포함) → 무중단 팩 → 미확인/바이너리 필요 시 수동 재확인.
async function onUpdateButton() {
  if (updateAvailable) return promptInstall();
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
async function restartNode(role: string, cmd: string, surfaces: OrgSurface[], socket?: string) {
  const target = surfaces.find((s) => s.role === role && !(s.status?.state === "offline"));
  if (!target) {
    toast("watchdog", "재기동 실패", `${role} 노드 없음`);
    return;
  }
  jumpToSurface(target.surface_id, socket);
  await invoke("send_input", { socket: socket ?? null, surfaceId: target.surface_id, data: cmd + "\n" });
}

// feed 승인: feed_list로 request_id 획득(org.status엔 count만) → 가장 오래된 pending Allow.
async function approveOldestFeed() {
  const r = (await invoke("feed_list", { status: "pending" }).catch(() => null)) as { items: FeedItem[] } | null;
  const pending = (r?.items ?? []).filter((i) => i.status === "pending");
  if (pending.length === 0) {
    toast("feed", "feed 승인", "대기 요청 없음");
    return;
  }
  const oldest = pending[0]; // feed.list는 삽입순(handlers.rs items.iter()) → [0]=가장 오래된
  await invoke("feed_reply", { requestId: oldest.request_id, decision: "allow" });
  refreshFeed();
  refreshSidebarStatus(); // 승인 직후 집계 배지 즉시 갱신
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

  // ── (4) feed 승인(가장 오래된 pending Allow) ──
  if ((org.feed?.pending ?? 0) > 0) {
    items.push({
      id: "act:feed-approve",
      title: `feed 승인 (대기 ${org.feed!.pending})`,
      subtitle: "가장 오래된 pending 요청 Allow",
      keywords: "feed approve allow 승인 피드 대기",
      confirm: { title: "feed 승인", body: "가장 오래된 pending 요청을 Allow 합니다." },
      action: () => approveOldestFeed(),
    });
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
    { id: "act:dept", title: "부서 워크스페이스 추가", keywords: "dept workspace 부서", action: () => addDeptWorkspace() },
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

function confirmModal(title: string, body: string): Promise<boolean> {
  return new Promise((resolve) => {
    const ov = document.createElement("div");
    ov.className = "modal-overlay";
    ov.innerHTML =
      `<div class="modal"><h3></h3><p></p>` +
      `<div class="modal-btns"><button class="modal-no">아니오</button>` +
      `<button class="modal-yes">설치</button></div></div>`;
    (ov.querySelector("h3") as HTMLElement).textContent = title;
    (ov.querySelector("p") as HTMLElement).textContent = body;
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

// ---------- toasts (daemon push events) ----------

function toast(category: string, name: string, detail: string) {
  const box = document.getElementById("toasts")!;
  const el = document.createElement("div");
  el.className = `toast ${category}`;
  el.innerHTML = `<span class="toast-name"></span><span class="toast-detail"></span>`;
  (el.querySelector(".toast-name") as HTMLElement).textContent = name;
  (el.querySelector(".toast-detail") as HTMLElement).textContent = detail;
  box.appendChild(el);
  setTimeout(() => el.remove(), 8000);
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
    openFeed(); // 승인은 즉시 승인 Feed 탭 오픈 (feed.item.created의 wait 경로와 정합)
    refreshFeed();
    refreshSidebarStatus(); // 사이드바 ⚠ 배지 갱신 (B3)
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
  if (name === "agent.exited") {
    toast("alert", "❌ 에이전트 사망", `surface:${sid} ${payload.role ?? ""}`);
    osBanner("❌ 에이전트 사망", `surface:${sid} ${payload.role ?? ""}`); // B4 OS 배너(고우선)
    refreshSidebarStatus();
    return;
  }
  if (name === "master.deadman") {
    // 페이로드는 {reason,idle_secs}만 — role 키 없음(governance.rs). payload.role 폴백 안전.
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
      if (payload.wait === true) openFeed();
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
  await new Promise<void>((resolve) => {
    listen("daemon-ready", () => resolve());
    listen("daemon-error", (e) => {
      info.textContent = `daemon error: ${e.payload}`;
    });
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

  // ★P2 버전 스큐 배지(무중단 rename-swap의 짝): 업데이트 후 구 데몬(lame-duck)이 세션을 계속
  // 보존하는 동안 "데몬 vX ↔ 앱 vY" 스큐를 비차단으로 알린다. 강제 재시작 없음 — 세션 보존이 우선.
  try {
    const appVer = (await invoke("app_version")) as string;
    const daemonVer = String(status.version ?? "");
    if (daemonVer && appVer && daemonVer !== appVer) {
      const badge = document.createElement("span");
      badge.className = "ver-skew-badge";
      badge.textContent = `데몬 v${daemonVer} · 앱 v${appVer} — 세션 보존 중`;
      badge.title =
        "업데이트가 적용됐지만 실행 중인 세션(마스터·워커·부서)을 보존하기 위해 기존 데몬이 계속 봉사합니다.\n모든 작업이 유휴일 때 데몬을 재시작하면 새 버전으로 교대됩니다. 세션은 죽지 않습니다.";
      info.appendChild(badge);
    }
  } catch {
    /* 배지는 부가 기능 — 실패해도 시작을 막지 않는다 */
  }

  await listen("daemon-event", (e) => onDaemonEvent(e.payload as Record<string, unknown>));

  // 무중단 팩 업데이트 진행 피드백(install_pack_update가 emit). ★app.restart 없음 — 세션 유지된 채 적용.
  await listen("pack-progress", (e) => {
    const p = (e.payload ?? {}) as { phase?: string };
    if (p.phase === "start")
      toast("feed", "🔄 무중단 적용 중", "서명검증 → 다운로드 → 원자적 팩 교체 → 노드 reinject…");
  });
  await listen("pack-updated", (e) => {
    const p = (e.payload ?? {}) as { pack_version?: string; reinject_failed?: number; reinject_deferred?: number };
    packUpdateAvailable = null;
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
    toast("health", "⚠ 팩 일부 미각성", p.message ?? "디스크 팩은 갱신됐으나 일부 노드 reinject 보류/실패(라이브 유지).");
  });

  // 시작 시 + 6시간마다 백그라운드 업데이트 확인 (조용히 — 있으면 badge·toast)
  checkForUpdate(true);
  setInterval(() => checkForUpdate(true), 6 * 3600 * 1000);

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

  // 부서 데몬 확보를 list 대조보다 선행 — 미가동이면 cys-dept launch. 실패해도(등록된) ws는 보존.
  const ghosts = new Set<number>();
  for (const ws of workspaces.filter((w) => w.socket)) {
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
document.getElementById("btn-install-cli")?.addEventListener("click", async () => {
  try {
    const r = (await invoke("install_cli_to_path")) as {
      cys_link: string; cysd_link: string; shadowed_by: string | null; warnings: string[];
    };
    // B-11: alert()는 WKWebView에서 억제될 수 있음(confirm() 무동작 실측과 동계열) — toast로 통일
    let msg = `${r.cys_link} · ${r.cysd_link} — 새 터미널에서 'cys' 사용 가능`;
    if (r.warnings?.length) msg += ` ⚠ ${r.warnings.join(" ⚠ ")}`;
    toast("system", "셸 설치 완료", msg);
  } catch (e) {
    toast("watchdog", "셸 설치 실패", String(e));
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
document.getElementById("btn-update")!.addEventListener("click", () => onUpdateButton());
// 역할 분리(오너 2026-06-29 결정): "새 워크스페이스"(btn-ws-new) = 기본/현재 데몬의 일반 워크스페이스
// (addWorkspace) — 부서가 아니다. 격리 부서 데몬 생성은 "+부서"(btn-ws-dept→addDeptWorkspace) 전담.
// 새 ws를 master로 선언 시 공유 데몬 claim 충돌은 데몬 레벨 claim_denied(cysd handlers.rs·kill 없음)가
// 비파괴 방어한다(생태계 죽지 않음·거부만). guard-master-claim(Fix2') 부트 자동발동 배선은 별건(헌법 토큰).
document.getElementById("btn-ws-new")!.addEventListener("click", () => addWorkspace());
// 멀티마스터 F4 + ＋부서 자동화(패치5): 새 부서(독립 데몬) workspace 런칭. 부서 번호는 백엔드가 확정.
const deptBtn = document.getElementById("btn-ws-dept") as HTMLButtonElement | null;
// 부서 런칭 실행(공통) — placeholder 탭·in-flight 버튼 가드. catalogKey=undefined → 레거시 dept-N.
// ⑤(gemini R2): invoke 실패 reject 를 try/catch 로 받아 토스트+버튼 disabled 해제(버튼 freeze 방지).
// ①(gemini R2 ★BLOCKER): create exit code 별 분기 — exit5(account dir 미존재=계정누수)는 레거시 폴백 절대 금지.
async function launchDept(catalogKey?: string) {
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
