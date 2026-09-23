import { describe, expect, test } from "bun:test";
import {
  INITIAL_UPDATE_STATE,
  binFromCheck,
  packFromCheck,
  packFromBackend,
  packAfterInstalled,
  packAfterUpToDate,
  binActionable,
  packActionable,
  deriveUpdateView,
  type BinComp,
  type PackComp,
  type UpdateState,
} from "./updatestate";

// U9(0.14.41) — 배지·클릭 창·토스트가 **한 상태**에서 파생된 **한 보기**만 그린다.
// 오너 증상: "업데이트 숫자가 떠 있는데 눌러 보면 「최신입니다」". 이 표들은 그 어긋남이 구조적으로
// 다시 나올 수 없음을 고정한다 — '최신'은 본체·팩 두 확인이 **모두 성공**했을 때만 나온다.

const T = (ms: number) => `T${ms}`;
const at = 1000;

const BINS: BinComp[] = [
  { s: "unchecked" },
  { s: "none", current: "0.14.40", at },
  { s: "available", version: "0.14.41", current: "0.14.40", at },
  { s: "failed", error: "네트워크", at },
  { s: "failed", error: "네트워크", at, lastGood: { s: "none", current: "0.14.40", at: 500 } },
  { s: "failed", error: "네트워크", at, lastGood: { s: "available", version: "0.14.41", current: "0.14.40", at: 500 } },
];
const PACKS: PackComp[] = [
  { s: "unchecked" },
  { s: "none", disk: "0.14.40", remote: "0.14.40", at },
  { s: "available", version: "0.14.41", disk: "0.14.40", manifestUrl: "u", at },
  { s: "binary-too-old", version: "0.14.42", disk: "0.14.40", minBinary: "0.14.42", manifestUrl: "u", at },
  { s: "channel-refused", version: "0.14.41", disk: "0.14.40", at },
  { s: "manifest-unreadable", detail: "expected value", at },
  { s: "disk-unknown", reason: "pack-version-missing", detail: "", remote: "0.14.40", at },
  { s: "disk-unknown", reason: "pack-state-corrupt", detail: "파싱 실패", remote: "0.14.40", at },
  { s: "failed", error: "curl", at },
  { s: "failed", error: "curl", at, lastGood: { s: "none", disk: "0.14.40", remote: "0.14.40", at: 500 } },
  {
    s: "failed",
    error: "curl",
    at,
    lastGood: { s: "available", version: "0.14.41", disk: "0.14.40", manifestUrl: "u", at: 500 },
  },
];
function* allStates(): Generator<UpdateState> {
  for (const bin of BINS) for (const pack of PACKS) for (const checking of [false, true]) yield { bin, pack, checking };
}

describe("deriveUpdateView — 전 조합 속성(배지·창·'최신'의 단일 원천)", () => {
  test("★'최신'(isLatest·✓·「최신입니다」)은 bin=none ∧ pack=none 일 때만", () => {
    let n = 0;
    for (const st of allStates()) {
      const v = deriveUpdateView(st, T);
      const both = st.bin.s === "none" && st.pack.s === "none";
      expect(v.isLatest).toBe(both);
      expect(v.badge.text === "✓").toBe(both);
      expect(v.headline.includes("최신입니다")).toBe(both);
      n++;
    }
    expect(n).toBe(BINS.length * PACKS.length * 2);
  });

  test("배지 글자는 기호뿐 — 숫자(0·1·2) 금지(Control Center 숫자 배지와의 오인 차단 · 반박 D6)", () => {
    for (const st of allStates()) {
      const v = deriveUpdateView(st, T);
      expect(["", "!", "↻", "✓", "?", "…"]).toContain(v.badge.text);
      expect(/\d/.test(v.badge.text)).toBe(false);
      if (v.badge.hidden) expect(v.badge.text === "" || v.badge.text === "…").toBe(true);
    }
  });

  test("창의 설치 버튼 = 설치 가능 판정과 정확히 일치 · 팩이 있으면 팩 버튼이 먼저(옵션 2 의도)", () => {
    for (const st of allStates()) {
      const v = deriveUpdateView(st, T);
      const want: string[] = [];
      if (packActionable(st)) want.push("pack-install");
      if (binActionable(st)) want.push("bin-install");
      expect(v.actions).toEqual(want);
      // 설치할 것이 있으면 배지는 경고(빨강) — 창을 열면 그 버튼이 반드시 있다.
      if (v.actions.length > 0) {
        expect(v.badge.tone).toBe("alert");
        expect(["!", "↻"]).toContain(v.badge.text);
      }
      // 빨간 배지인데 창에 설치 버튼도 막힘 사유도 없는 조합 금지(배지·창 불일치 = 오너 증상의 모양).
      if (v.badge.tone === "alert" && v.actions.length === 0) {
        const blocked = [st.pack, st.pack.s === "failed" ? st.pack.lastGood : undefined].some(
          (p) => p && (p.s === "binary-too-old" || p.s === "channel-refused"),
        );
        expect(blocked).toBe(true);
      }
    }
  });

  test("창의 두 행(본체·팩)은 항상 있고, 확인 실패·불명은 '?' + '알 수 없' 문구", () => {
    for (const st of allStates()) {
      const v = deriveUpdateView(st, T);
      expect(v.rows.map((r) => r.key)).toEqual(["bin", "pack"]);
      if (v.badge.text === "?") {
        expect(v.badge.tone).toBe("warn");
        expect(v.headline).toContain("알 수 없");
      }
    }
  });
});

describe("deriveUpdateView — 대표 상태", () => {
  test("첫 확인 전: 숨김 · 확인 중: 중립 '…'", () => {
    const v0 = deriveUpdateView(INITIAL_UPDATE_STATE, T);
    expect(v0.badge.hidden).toBe(true);
    const v1 = deriveUpdateView({ ...INITIAL_UPDATE_STATE, checking: true }, T);
    expect([v1.badge.hidden, v1.badge.text, v1.badge.tone]).toEqual([false, "…", "ok"]);
    expect(v1.headline).toContain("확인 중");
  });

  test("둘 다 최신 → ✓(중립) · 창에 두 버전과 확인 시각", () => {
    const v = deriveUpdateView(
      { bin: BINS[1], pack: PACKS[1], checking: false },
      T,
    );
    expect([v.badge.hidden, v.badge.text, v.badge.tone]).toEqual([false, "✓", "ok"]);
    expect(v.headline).toContain("최신입니다");
    expect(v.rows[0].text).toContain("0.14.40");
    expect(v.rows[1].text).toContain("0.14.40");
    expect(v.meta).toContain("T1000");
    expect(v.actions).toEqual([]);
    expect(v.silentToast).toBeNull();
  });

  test("본체+팩 동시 → ↻ · 창 버튼은 팩 먼저 · silent 토스트는 종전 문구(updatePlan)", () => {
    const v = deriveUpdateView({ bin: BINS[2], pack: PACKS[2], checking: false }, T);
    expect(v.badge.text).toBe("↻");
    expect(v.actions).toEqual(["pack-install", "bin-install"]);
    expect(v.silentToast?.title).toBe("↻ 무중단 팩 + 새 본체");
    expect(v.silentToast?.msg).toContain("무중단 적용(재시작 없음)");
  });

  test("본체만 → ! · 본체 버튼만 · 팩 최신 행", () => {
    const v = deriveUpdateView({ bin: BINS[2], pack: PACKS[1], checking: false }, T);
    expect(v.badge.text).toBe("!");
    expect(v.actions).toEqual(["bin-install"]);
    expect(v.rows[0].text).toContain("0.14.41");
    expect(v.silentToast?.title).toBe("🔄 새 본체 버전");
  });

  test("팩만 본체 필요(binary-too-old) → ! · 설치 버튼 없음 · 본체 먼저 안내", () => {
    const v = deriveUpdateView({ bin: BINS[1], pack: PACKS[3], checking: false }, T);
    expect(v.badge.text).toBe("!");
    expect(v.actions).toEqual([]);
    expect(v.rows[1].text).toContain("본체");
    expect(v.silentToast?.title).toBe("⚠ 업데이트 있음");
  });

  test("pro 채널 거부 → ! 지만 경보 아님(warn) · 설치 버튼 없음(누르면 CLI 가 거부하던 경로 차단 · R7) · '업데이트가 있지만' 금지(리뷰1 F1 회귀)", () => {
    const v = deriveUpdateView({ bin: BINS[1], pack: PACKS[4], checking: false }, T);
    // ★F1: v0.14.40 에서 pro 사용자는 회색 중립을 봤다. 이 상태를 빨강 alert 로 되돌리는
    // 뮤테이션(tone 만 alert 로 바꿔도)은 오너 재현 증상("배지는 뭔가 있다는데 눌러 보면 할 게
    // 없다")을 되살린다 — 이 단언이 잡는다. 텍스트는 '막힘' 계열 기호 !을 유지하되 톤은 warn.
    expect(v.badge.text).toBe("!");
    expect(v.badge.tone).toBe("warn");
    expect(v.actions).toEqual([]);
    expect(v.headline).not.toContain("업데이트가 있지만");
    expect(v.rows[1].text).toContain("pro");
  });

  test("팩 매니페스트 해석 불가·디스크 불명·확인 실패 → ? — '최신' 금지(R2·R4)", () => {
    for (const p of [PACKS[5], PACKS[6], PACKS[7], PACKS[8], PACKS[9]]) {
      const v = deriveUpdateView({ bin: BINS[1], pack: p, checking: false }, T);
      expect(v.badge.text).toBe("?");
      expect(v.isLatest).toBe(false);
      expect(v.silentToast).toBeNull(); // silent 폴링은 조용히 — 배지 '?'와 창이 설명한다
    }
  });

  test("확인 실패 + 직전 검증된 새 팩 → 설치 버튼 유지(fail-safe) · '직전 확인' 명시", () => {
    const v = deriveUpdateView({ bin: BINS[1], pack: PACKS[10], checking: false }, T);
    expect(v.badge.text).toBe("↻");
    expect(v.actions).toEqual(["pack-install"]);
    expect(v.rows[1].text).toContain("직전 확인");
    expect(v.badge.title).toContain("직전 확인");
  });

  test("다시 확인 중에도 직전 결과를 유지해 보여 준다(배지 깜빡임·'…' 고착 방지)", () => {
    const v = deriveUpdateView({ bin: BINS[2], pack: PACKS[1], checking: true }, T);
    expect(v.badge.text).toBe("!");
    expect(v.badge.title).toContain("다시 확인 중");
  });
});

describe("상태 전이 — invoke 결과 → 상태", () => {
  test("binFromCheck: null=최신 · {version}=있음 · 실패=failed(+직전 검증 보존)", () => {
    expect(binFromCheck({ s: "unchecked" }, { ok: true, value: null }, "0.14.40", 7)).toEqual({
      s: "none",
      current: "0.14.40",
      at: 7,
    });
    expect(
      binFromCheck({ s: "unchecked" }, { ok: true, value: { version: "0.14.41", current: "0.14.40", notes: "n" } }, "x", 7),
    ).toEqual({ s: "available", version: "0.14.41", current: "0.14.40", notes: "n", at: 7 });
    const good: BinComp = { s: "available", version: "0.14.41", current: "0.14.40", at: 3 };
    const f1 = binFromCheck(good, { ok: false, error: "e1" }, "0.14.40", 9);
    expect(f1).toEqual({ s: "failed", error: "e1", at: 9, lastGood: good });
    // 연속 실패에도 마지막 검증 상태는 그대로(실패가 실패를 lastGood 로 삼지 않는다).
    expect(binFromCheck(f1, { ok: false, error: "e2" }, "0.14.40", 11)).toEqual({
      s: "failed",
      error: "e2",
      at: 11,
      lastGood: good,
    });
    // 모양이 이상한 응답은 '최신'이 아니라 실패로.
    expect(binFromCheck({ s: "unchecked" }, { ok: true, value: { nope: 1 } }, "0.14.40", 7).s).toBe("failed");
  });

  test("packFromBackend: 타입 있는 status 전부 · 모르는 모양 = 실패(최신으로 접지 않는다)", () => {
    expect(packFromBackend({ status: "none", pack_version: "1.0.0", disk_version: "1.0.0" }, 5)).toEqual({
      s: "none",
      disk: "1.0.0",
      remote: "1.0.0",
      at: 5,
    });
    expect(
      packFromBackend(
        { status: "available", pack_version: "2.0.0", disk_version: "1.0.0", manifest_url: "u", binary_too_old: false },
        5,
      ),
    ).toEqual({ s: "available", version: "2.0.0", disk: "1.0.0", manifestUrl: "u", at: 5 });
    expect(
      packFromBackend(
        { status: "binary-too-old", pack_version: "2.0.0", disk_version: "1.0.0", min_binary_version: "9.0.0", manifest_url: "u" },
        5,
      ),
    ).toEqual({ s: "binary-too-old", version: "2.0.0", disk: "1.0.0", minBinary: "9.0.0", manifestUrl: "u", at: 5 });
    expect(packFromBackend({ status: "channel-refused", pack_version: "2.0.0", disk_version: "1.0.0" }, 5).s).toBe(
      "channel-refused",
    );
    expect(packFromBackend({ status: "manifest-unreadable", detail: "d" }, 5)).toEqual({
      s: "manifest-unreadable",
      detail: "d",
      at: 5,
    });
    expect(
      packFromBackend({ status: "disk-unknown", reason: "pack-state-mismatch", detail: "d", pack_version: "1.0.0" }, 5),
    ).toEqual({ s: "disk-unknown", reason: "pack-state-mismatch", detail: "d", remote: "1.0.0", at: 5 });
    for (const junk of [null, undefined, 3, "x", {}, { status: "weird" }, { status: "available" }]) {
      expect(packFromBackend(junk, 5).s).toBe("failed");
    }
  });

  test("packFromCheck: 실패는 직전 검증 상태를 lastGood 로 보존", () => {
    const good: PackComp = { s: "available", version: "2.0.0", disk: "1.0.0", manifestUrl: "u", at: 1 };
    const f = packFromCheck(good, { ok: false, error: "curl" }, 9);
    expect(f).toEqual({ s: "failed", error: "curl", at: 9, lastGood: good });
    // 해석 불가·디스크 불명은 '검증된 상태'가 아니므로 lastGood 가 되지 않는다.
    const unk: PackComp = { s: "manifest-unreadable", detail: "d", at: 1 };
    expect(packFromCheck(unk, { ok: false, error: "curl" }, 9)).toEqual({ s: "failed", error: "curl", at: 9 });
  });

  test("팩 설치 완료 → 최신 · no-op(pack-uptodate) → 확인된 경우만 최신, 아니면 불명", () => {
    expect(packAfterInstalled("2.0.0", 4)).toEqual({ s: "none", disk: "2.0.0", remote: "2.0.0", at: 4 });
    expect(packAfterUpToDate({ confirmed: true, disk_version: "2.0.0", remote_version: "2.0.0" }, 4)).toEqual({
      s: "none",
      disk: "2.0.0",
      remote: "2.0.0",
      at: 4,
    });
    const u = packAfterUpToDate({ confirmed: false, disk_version: "-", remote_version: "2.0.0" }, 4);
    expect(u.s).toBe("disk-unknown");
    expect(packAfterUpToDate(null, 4).s).toBe("disk-unknown");
  });
});
