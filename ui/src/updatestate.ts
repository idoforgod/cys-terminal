// 업데이트 단일 상태 · 단일 보기 — 순수 모듈 (U9 · 0.14.41 · 설계 §3 U9).
//
// 오너 증상: "업데이트 배지에 숫자가 떠 있는데 눌러 보면 「최신입니다」". 원인은 셋이 겹친 것이었다:
//   ① .badge 에 [hidden] 짝이 없어 숨긴 배지가 보였다(style.css — 별도 수정)
//   ② 배지·클릭·토스트가 서로 다른 원천(전역 두 개 + 즉석 판정)을 봤다(R3 — ↻ 배지인데 클릭은 본체 창)
//   ③ 확인 실패·판독 불가를 '최신'으로 접거나 조용히 삼켰다(R2·R4 · no-op 설치를 "완료" R5)
// 이 모듈은 ②③ 을 구조로 막는다: 상태(UpdateState)는 하나, 배지·창·토스트는 deriveUpdateView 한 함수의
// 출력만 그린다. '최신'이라는 말은 본체·팩 두 확인이 **모두 성공**했을 때만 나온다.
//
// 계약: 최상위 부수효과 0(상수 정의만) · DOM/Tauri 무관 · 구형 WKWebView 비호환 문법 0
// (WORKER 공통 규약 6의 금지 목록 — 정규식 후방탐색·배열 음수 인덱스 메서드·역방향 탐색·전역 깊은 복제·
// 자체 속성 정적 검사 — 를 쓰지 않는다. 금지 토큰 자체를 주석에도 적지 않는다: 텍스트 스캐너 오탐 방지).
import { updatePlan } from "./updateplan";

export type BinGood =
  | { s: "none"; current: string; at: number }
  | { s: "available"; version: string; current: string; notes?: string; at: number };
export type BinComp = { s: "unchecked" } | BinGood | { s: "failed"; error: string; at: number; lastGood?: BinGood };

export type PackGood =
  | { s: "none"; disk: string; remote: string; at: number }
  | { s: "available"; version: string; disk: string; manifestUrl: string; at: number }
  | { s: "binary-too-old"; version: string; disk: string; minBinary: string; manifestUrl: string; at: number }
  | { s: "channel-refused"; version: string; disk: string; at: number };
export type PackComp =
  | { s: "unchecked" }
  | PackGood
  | { s: "manifest-unreadable"; detail: string; at: number }
  | { s: "disk-unknown"; reason: string; detail: string; remote: string; at: number }
  | { s: "failed"; error: string; at: number; lastGood?: PackGood };

export type UpdateState = { bin: BinComp; pack: PackComp; checking: boolean };
export const INITIAL_UPDATE_STATE: UpdateState = { bin: { s: "unchecked" }, pack: { s: "unchecked" }, checking: false };

export type CheckResult = { ok: true; value: unknown } | { ok: false; error: string };

const str = (v: unknown): string => (typeof v === "string" ? v : "");

function isBinGood(c: BinComp): c is BinGood {
  return c.s === "none" || c.s === "available";
}
function isPackGood(c: PackComp): c is PackGood {
  return c.s === "none" || c.s === "available" || c.s === "binary-too-old" || c.s === "channel-refused";
}

/** check_update 결과 → 본체 상태. null = 최신(업데이터가 '같거나 낮음'을 확인) · 실패는 직전 검증 보존. */
export function binFromCheck(prev: BinComp, r: CheckResult, current: string, now: number): BinComp {
  const lastGood = isBinGood(prev) ? prev : prev.s === "failed" ? prev.lastGood : undefined;
  const fail = (error: string): BinComp =>
    lastGood ? { s: "failed", error, at: now, lastGood } : { s: "failed", error, at: now };
  if (!r.ok) return fail(r.error);
  const v = r.value;
  if (v === null) return { s: "none", current, at: now };
  if (typeof v === "object" && v !== null && str((v as { version?: unknown }).version)) {
    const o = v as { version?: unknown; current?: unknown; notes?: unknown };
    const out: BinComp = { s: "available", version: str(o.version), current: str(o.current) || current, at: now };
    if (typeof o.notes === "string") out.notes = o.notes;
    return out;
  }
  return fail("응답 형식 불명");
}

/** check_pack_update 의 타입 있는 응답 → 팩 상태. 모르는 모양은 '최신'이 아니라 실패. */
export function packFromBackend(v: unknown, now: number): PackComp {
  const bad: PackComp = { s: "failed", error: "응답 형식 불명", at: now };
  if (typeof v !== "object" || v === null) return bad;
  const o = v as Record<string, unknown>;
  const version = str(o.pack_version);
  const disk = str(o.disk_version);
  switch (o.status) {
    case "none":
      return { s: "none", disk, remote: version, at: now };
    case "available":
      if (!version) return bad;
      return { s: "available", version, disk, manifestUrl: str(o.manifest_url), at: now };
    case "binary-too-old":
      if (!version) return bad;
      return {
        s: "binary-too-old",
        version,
        disk,
        minBinary: str(o.min_binary_version),
        manifestUrl: str(o.manifest_url),
        at: now,
      };
    case "channel-refused":
      return { s: "channel-refused", version, disk, at: now };
    case "manifest-unreadable":
      return { s: "manifest-unreadable", detail: str(o.detail), at: now };
    case "disk-unknown":
      return { s: "disk-unknown", reason: str(o.reason), detail: str(o.detail), remote: version, at: now };
    default:
      return bad;
  }
}

export function packFromCheck(prev: PackComp, r: CheckResult, now: number): PackComp {
  if (r.ok) return packFromBackend(r.value, now);
  const lastGood = isPackGood(prev) ? prev : prev.s === "failed" ? prev.lastGood : undefined;
  return lastGood ? { s: "failed", error: r.error, at: now, lastGood } : { s: "failed", error: r.error, at: now };
}

/** pack-updated(디스크 반영 성공) → 그 버전이 디스크에 있다 = 팩 최신. */
export function packAfterInstalled(version: string, now: number): PackComp {
  return { s: "none", disk: version, remote: version, at: now };
}

/** pack-uptodate(CLI no-op) → 브리지가 확인한 경우(disk_parse=ok ∧ disk ≥ remote)만 최신, 아니면 불명. */
export function packAfterUpToDate(p: unknown, now: number): PackComp {
  const o = (typeof p === "object" && p !== null ? p : {}) as Record<string, unknown>;
  const disk = str(o.disk_version);
  const remote = str(o.remote_version);
  if (o.confirmed === true) return { s: "none", disk, remote, at: now };
  return { s: "disk-unknown", reason: "pack-version-unreadable", detail: disk, remote, at: now };
}

/** 설치할 수 있는 본체 업데이트(실패 중이면 직전 검증 기준 — fail-safe 보존 · stale=true). */
export function binActionable(st: UpdateState): { version: string; stale: boolean } | null {
  const b = st.bin;
  if (b.s === "available") return { version: b.version, stale: false };
  if (b.s === "failed" && b.lastGood && b.lastGood.s === "available") return { version: b.lastGood.version, stale: true };
  return null;
}

/** 설치할 수 있는(무중단 호환) 팩 업데이트. */
export function packActionable(st: UpdateState): { version: string; manifestUrl: string; stale: boolean } | null {
  const p = st.pack;
  if (p.s === "available") return { version: p.version, manifestUrl: p.manifestUrl, stale: false };
  if (p.s === "failed" && p.lastGood && p.lastGood.s === "available")
    return { version: p.lastGood.version, manifestUrl: p.lastGood.manifestUrl, stale: true };
  return null;
}

function packTooOld(st: UpdateState): { version: string; stale: boolean } | null {
  const p = st.pack;
  if (p.s === "binary-too-old") return { version: p.version, stale: false };
  if (p.s === "failed" && p.lastGood && p.lastGood.s === "binary-too-old") return { version: p.lastGood.version, stale: true };
  return null;
}

function packChannelRefused(st: UpdateState): { version: string } | null {
  const p = st.pack;
  if (p.s === "channel-refused") return { version: p.version };
  if (p.s === "failed" && p.lastGood && p.lastGood.s === "channel-refused") return { version: p.lastGood.version };
  return null;
}

export type UpdateAction = "pack-install" | "bin-install";
export type UpdateRow = { key: "bin" | "pack"; label: string; text: string; tone: "ok" | "alert" | "warn" | "muted" };
export type UpdateView = {
  badge: { text: string; tone: "alert" | "ok" | "warn"; title: string; hidden: boolean };
  headline: string;
  isLatest: boolean;
  rows: UpdateRow[];
  meta: string;
  actions: UpdateAction[];
  /** silent(시작·6시간) 확인에서만 띄우는 토스트 — 설치할 것이 있을 때만(종전 문구 유지). */
  silentToast: { title: string; msg: string } | null;
};

const DISK_REASON: Record<string, string> = {
  "pack-version-missing": "설치된 팩 버전 기록(.pack-version)이 없습니다",
  "pack-version-unreadable": "설치된 팩 버전을 읽지 못했습니다",
  "pack-state-corrupt": "팩 상태 파일(.pack-state.json)이 손상됐습니다 — cys pack-repair-channel 로 복구",
  "pack-state-mismatch": "팩 상태 기록이 설치 버전과 어긋납니다 — cys init-pack 또는 cys pack-repair-channel",
};

function vtag(v: string): string {
  return v ? `v${v}` : "버전 불명";
}

function binRow(b: BinComp, checking: boolean, fmt: (ms: number) => string): UpdateRow {
  const label = "본체(앱)";
  switch (b.s) {
    case "unchecked":
      return { key: "bin", label, text: checking ? "확인 중…" : "아직 확인 전", tone: "muted" };
    case "none":
      return { key: "bin", label, text: `${vtag(b.current)} — 최신`, tone: "ok" };
    case "available":
      return {
        key: "bin",
        label,
        text: `${vtag(b.current)} → v${b.version} — [본체 패치 설치]로 설치(재시작 후 자동 복원)`,
        tone: "alert",
      };
    case "failed": {
      let t = `확인 실패 — ${b.error}`;
      if (b.lastGood?.s === "available") t += ` · 직전 확인(${fmt(b.lastGood.at)}) 기준 새 본체 v${b.lastGood.version}`;
      else if (b.lastGood?.s === "none") t += ` · 직전 확인(${fmt(b.lastGood.at)})에는 최신`;
      return { key: "bin", label, text: t, tone: b.lastGood?.s === "available" ? "alert" : "warn" };
    }
  }
}

function packGoodText(p: PackGood): string {
  switch (p.s) {
    case "none":
      return p.disk ? `${p.disk} — 최신` : "최신";
    case "available":
      return `${p.disk || "?"} → ${p.version} — [팩 무중단 적용]으로 적용(재시작 없음·세션 유지)`;
    case "binary-too-old":
      return (
        `${p.disk || "?"} → ${p.version} — 더 새 본체(${p.minBinary ? "v" + p.minBinary + " 이상" : "최신"})가 필요합니다. ` +
        "본체를 먼저 업데이트하세요(홈페이지 www.cysinsight.com)"
      );
    case "channel-refused":
      return `pro 채널 팩 사용 중 — 공개 팩 ${p.version}은 적용하지 않습니다(pro→free 전환은 cys pack-downgrade-to-free 전용)`;
  }
}

function packRow(p: PackComp, checking: boolean, fmt: (ms: number) => string): UpdateRow {
  const label = "팩(지침·스킬)";
  switch (p.s) {
    case "unchecked":
      return { key: "pack", label, text: checking ? "확인 중…" : "아직 확인 전", tone: "muted" };
    case "none":
      return { key: "pack", label, text: packGoodText(p), tone: "ok" };
    case "available":
    case "binary-too-old":
      return { key: "pack", label, text: packGoodText(p), tone: "alert" };
    case "channel-refused":
      return { key: "pack", label, text: packGoodText(p), tone: "warn" };
    case "manifest-unreadable":
      return {
        key: "pack",
        label,
        text: `팩 정보(매니페스트)를 해석하지 못했습니다 — 최신 여부를 알 수 없음${p.detail ? ` (${p.detail})` : ""}`,
        tone: "warn",
      };
    case "disk-unknown":
      return {
        key: "pack",
        label,
        text:
          `${DISK_REASON[p.reason] ?? "설치된 팩 상태를 알 수 없습니다"} — 최신 여부를 알 수 없음` +
          (p.detail ? ` (${p.detail})` : ""),
        tone: "warn",
      };
    case "failed": {
      let t = `확인 실패 — ${p.error}`;
      const g = p.lastGood;
      if (g) {
        if (g.s === "available") t += ` · 직전 확인(${fmt(g.at)}) 기준 새 팩 ${g.version}`;
        else if (g.s === "binary-too-old") t += ` · 직전 확인(${fmt(g.at)}) 기준 새 팩 ${g.version}(본체 먼저 필요)`;
        else if (g.s === "none") t += ` · 직전 확인(${fmt(g.at)})에는 최신`;
        else t += ` · 직전 확인(${fmt(g.at)}): pro 채널`;
      }
      return { key: "pack", label, text: t, tone: g && (g.s === "available" || g.s === "binary-too-old") ? "alert" : "warn" };
    }
  }
}

/** 사람이 읽는 '확인 실패/불명' 사유(배지 title 용). */
function unknownReasons(st: UpdateState): string[] {
  const out: string[] = [];
  const b = st.bin;
  if (b.s === "failed") out.push(`본체 확인 실패(${b.error})`);
  else if (b.s === "unchecked") out.push("본체 미확인");
  const p = st.pack;
  if (p.s === "failed") out.push(`팩 확인 실패(${p.error})`);
  else if (p.s === "manifest-unreadable") out.push("팩 정보 해석 불가");
  else if (p.s === "disk-unknown") out.push("설치된 팩 상태 불명");
  else if (p.s === "unchecked") out.push("팩 미확인");
  return out;
}

function atOf(c: BinComp | PackComp): number {
  return c.s === "unchecked" ? 0 : c.at;
}

/**
 * 상태 → 보기(배지·창·토스트). 배지 규칙(설계 §3 U9 · 반박 D6 — 숫자 금지, 기호만):
 *   설치 가능(팩 우선 ↻ · 본체 !) > 막힘(본체 필요·채널 거부 !) > 최신(✓ · 두 확인 모두 성공) >
 *   불명(? · 실패·해석 불가·디스크 불명) · 첫 확인 중 '…' · 첫 확인 전 숨김.
 * 다시 확인 중(checking)에는 직전 결과를 그대로 보여 주고 title 에만 '다시 확인 중…'을 붙인다.
 */
export function deriveUpdateView(st: UpdateState, fmt: (ms: number) => string): UpdateView {
  const bin = binActionable(st);
  const pack = packActionable(st);
  const tooOld = packTooOld(st);
  const refused = packChannelRefused(st);
  const isLatest = st.bin.s === "none" && st.pack.s === "none";
  const nothingKnown = st.bin.s === "unchecked" && st.pack.s === "unchecked";
  const rows = [binRow(st.bin, st.checking, fmt), packRow(st.pack, st.checking, fmt)];
  const lastAt = Math.max(atOf(st.bin), atOf(st.pack));
  const meta = (lastAt > 0 ? `마지막 확인 ${fmt(lastAt)}` : "아직 확인 전") + (st.checking ? " · 다시 확인 중…" : "");
  const actions: UpdateAction[] = [];
  if (pack) actions.push("pack-install");
  if (bin) actions.push("bin-install");
  const again = st.checking ? " · 다시 확인 중…" : "";

  // 종전 5분기 판정(updateplan.ts — 문구 핀 유지)을 그대로 쓴다. 실패 플래그는 '검증된 상태가 아님'.
  const plan = updatePlan({
    binVersion: bin ? bin.version : null,
    packVersion: pack ? pack.version : tooOld ? tooOld.version : null,
    binaryTooOld: !pack && !!tooOld,
    binCheckFailed: !(st.bin.s === "none" || st.bin.s === "available"),
    packCheckFailed: !(st.pack.s === "none" || st.pack.s === "available" || st.pack.s === "binary-too-old"),
  });
  const stale = (bin && bin.stale) || (pack && pack.stale) || (!pack && tooOld && tooOld.stale);
  const staleNote = stale ? " · 직전 확인 기준(이번 확인 실패)" : "";

  if (plan.kind === "pack-and-binary" || plan.kind === "binary" || plan.kind === "pack" || plan.kind === "binary-required") {
    const toastTitle: Record<string, string> = {
      "pack-and-binary": "↻ 무중단 팩 + 새 본체",
      binary: "🔄 새 본체 버전",
      pack: "↻ 무중단 팩 업데이트",
      "binary-required": "⚠ 업데이트 있음",
    };
    return {
      badge: { text: plan.badge, tone: "alert", title: plan.title + staleNote + again, hidden: false },
      headline:
        actions.length > 0
          ? `설치할 수 있는 업데이트 ${actions.length}건`
          : "업데이트가 있지만 앱 안에서 바로 설치할 수 없습니다",
      isLatest: false,
      rows,
      meta,
      actions,
      silentToast: { title: toastTitle[plan.kind], msg: plan.toastMsg },
    };
  }
  if (refused) {
    return {
      badge: { text: "!", tone: "alert", title: "팩: pro 채널 — 공개 팩은 적용 대상 아님(눌러서 자세히)" + again, hidden: false },
      headline: "업데이트가 있지만 앱 안에서 바로 설치할 수 없습니다",
      isLatest: false,
      rows,
      meta,
      actions,
      silentToast: null,
    };
  }
  if (isLatest) {
    const b = st.bin.s === "none" ? st.bin.current : "";
    const p = st.pack.s === "none" ? st.pack.disk : "";
    return {
      badge: {
        text: "✓",
        tone: "ok",
        title: `최신 — 본체 ${vtag(b)} · 팩 ${p || "최신"} (확인 ${fmt(lastAt)})` + again,
        hidden: false,
      },
      headline: "최신입니다 — 대기 중인 업데이트 없음",
      isLatest: true,
      rows,
      meta,
      actions,
      silentToast: null,
    };
  }
  if (nothingKnown) {
    return {
      badge: { text: "…", tone: "ok", title: st.checking ? "업데이트 확인 중…" : "", hidden: !st.checking },
      headline: st.checking ? "확인 중…" : "아직 확인 전",
      isLatest: false,
      rows,
      meta,
      actions,
      silentToast: null,
    };
  }
  if (st.checking && (st.bin.s === "unchecked" || st.pack.s === "unchecked")) {
    return {
      badge: { text: "…", tone: "ok", title: "업데이트 확인 중…", hidden: false },
      headline: "확인 중…",
      isLatest: false,
      rows,
      meta,
      actions,
      silentToast: null,
    };
  }
  return {
    badge: {
      text: "?",
      tone: "warn",
      title: `최신 여부 확인 불가 — ${unknownReasons(st).join(" · ")} (눌러서 자세히)` + again,
      hidden: false,
    },
    headline: "확인 실패 — 최신 여부를 알 수 없습니다",
    isLatest: false,
    rows,
    meta,
    actions,
    silentToast: null,
  };
}
