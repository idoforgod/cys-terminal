// toastttl.ts 순수 로직 회귀 테스트 (bun test — 신규 의존성 0).
//
// ★T-0147-3: "모든 에러 알람은 종류 불문 일정 시간이 지나면 꺼진다" + "정보는 이력에 남는다"의
// 두 축을 각각 고정한다. ①TTL 정책(종류·id별 수명·무한 잔존 0) ②갱신 리셋 규칙(진행 중 소멸 0)
// ③이력 링버퍼(cap·최신순·같은 id 합침) ④고위험 만료의 OS 배너 보강.
import { describe, it, expect } from "bun:test";
import {
  VOLATILE_TTL_MS,
  STICKY_TTL_MS,
  PROGRESS_TTL_MS,
  ALARM_HISTORY_CAP,
  toastTtl,
  toastTimerPlan,
  needsExpiryBanner,
  expiryBannerText,
  pushAlarm,
  formatAlarmTime,
  type AlarmRecord,
} from "./toastttl";

describe("toastTtl — 종류 불문 유한 수명", () => {
  it("volatile은 구 하드코딩 8초를 승계(회귀 0)", () => {
    expect(VOLATILE_TTL_MS).toBe(8000);
    expect(toastTtl("volatile").ttlMs).toBe(8000);
  });
  it("volatile은 id를 무시한다(익명 토스트)", () => {
    expect(toastTtl("volatile", "upd-bin").ttlMs).toBe(VOLATILE_TTL_MS);
  });
  it("sticky 기본은 60초 — 구 구현의 '영구 잔존'을 대체", () => {
    expect(STICKY_TTL_MS).toBe(60000);
    expect(toastTtl("sticky").ttlMs).toBe(60000);
    // 실제로 영구 잔존했던 id들이 모두 유한 수명을 받는다
    for (const id of ["boot-warn", "safe-mode", "perm-Documents", "purge-fail-/tmp/x.sock"]) {
      expect(toastTtl("sticky", id).ttlMs).toBe(STICKY_TTL_MS);
    }
  });
  it("장기 진행형 sticky는 3분(중간 갱신 없이 60초를 넘겨도 진행 중 소멸 없음)", () => {
    expect(PROGRESS_TTL_MS).toBe(180000);
    for (const id of ["restore", "rotate-daemon", "transfer", "upd-bin", "upd-pack", "restart-daemon", "daemon-hint"]) {
      expect(toastTtl("sticky", id).ttlMs).toBe(PROGRESS_TTL_MS);
    }
  });
  it("어떤 조합도 무한(0·Infinity)이 아니다 — 오너 요구의 하드 불변식", () => {
    const ids = [undefined, "boot-warn", "safe-mode", "restore", "purge-fail-x", "unknown-id"];
    for (const kind of ["volatile", "sticky"] as const) {
      for (const id of ids) {
        const { ttlMs } = toastTtl(kind, id);
        expect(Number.isFinite(ttlMs)).toBe(true);
        expect(ttlMs).toBeGreaterThan(0);
      }
    }
  });
});

describe("toastTimerPlan — 갱신 시 리셋(debounce) 규칙", () => {
  it("같은 id 갱신이면 이전 타이머를 걷고 새 수명을 준다", () => {
    const p = toastTimerPlan("sticky", "upd-bin", true);
    expect(p.clearPrevious).toBe(true);
    expect(p.ttlMs).toBe(PROGRESS_TTL_MS);
  });
  it("첫 표시(기존 없음)는 걷을 타이머가 없다", () => {
    expect(toastTimerPlan("sticky", "boot-warn", false)).toEqual({
      ttlMs: STICKY_TTL_MS,
      clearPrevious: false,
    });
  });
  it("volatile은 매 호출이 새 엘리먼트 — 리셋 대상이 없다", () => {
    expect(toastTimerPlan("volatile", undefined, true).clearPrevious).toBe(false);
  });
  it("hadExisting 기본값은 false(호출측 실수 시 안전한 쪽)", () => {
    expect(toastTimerPlan("sticky", "restore").clearPrevious).toBe(false);
  });
});

describe("needsExpiryBanner / expiryBannerText — 고위험 실패의 만료 보강", () => {
  it("purge-fail-* 는 조용히 사라지지 않는다(D2b purge-safety 계승)", () => {
    expect(needsExpiryBanner("purge-fail-/Users/x/.cys/run/dept-1.sock")).toBe(true);
  });
  it("일반 sticky는 배너 보강 없음(배너 남용 방지)", () => {
    for (const id of ["boot-warn", "safe-mode", "restore", "upd-bin", "perm-Desktop"]) {
      expect(needsExpiryBanner(id)).toBe(false);
    }
  });
  it("배너 문구는 '자동 닫힘 + 이력에서 재조회'를 명시한다", () => {
    const t = expiryBannerText("부서 완전 삭제 실패", "dept-1: EACCES — 삭제되지 않았습니다.");
    expect(t.title).toContain("부서 완전 삭제 실패");
    expect(t.body).toContain("dept-1: EACCES");
    expect(t.body).toContain("알람");
  });
});

const rec = (over: Partial<AlarmRecord> = {}): AlarmRecord => ({
  ts: 1_700_000_000_000,
  category: "watchdog",
  name: "n",
  detail: "d",
  ...over,
});

describe("pushAlarm — 이력 링버퍼(정보 소실 방지 장치)", () => {
  it("최신이 index 0(최신순 렌더 그대로)", () => {
    const r1 = rec({ name: "old" });
    const r2 = rec({ name: "new" });
    const ring = pushAlarm(pushAlarm([], r1), r2);
    expect(ring.map((r) => r.name)).toEqual(["new", "old"]);
  });
  it("입력 배열을 변형하지 않는다(순수)", () => {
    const base = [rec({ name: "a" })];
    const out = pushAlarm(base, rec({ name: "b" }));
    expect(base.length).toBe(1);
    expect(out.length).toBe(2);
  });
  it("cap을 넘으면 가장 오래된 것부터 버린다", () => {
    let ring: AlarmRecord[] = [];
    for (let i = 0; i < 250; i++) ring = pushAlarm(ring, rec({ name: `n${i}`, ts: i }));
    expect(ring.length).toBe(ALARM_HISTORY_CAP);
    expect(ring[0].name).toBe("n249");
    expect(ring[ALARM_HISTORY_CAP - 1].name).toBe(`n${250 - ALARM_HISTORY_CAP}`);
  });
  it("cap 기본값은 200", () => {
    expect(ALARM_HISTORY_CAP).toBe(200);
  });
  it("같은 id는 최신 1건으로 합쳐진다 — 진행률 갱신이 이력을 잠식하지 않음", () => {
    let ring: AlarmRecord[] = [];
    for (let i = 1; i <= 50; i++) {
      ring = pushAlarm(ring, rec({ id: "upd-bin", detail: `${i}%`, ts: i }));
    }
    expect(ring.length).toBe(1);
    expect(ring[0].detail).toBe("50%");
  });
  it("id 있는 갱신은 앞선 실패 알람을 밀어내지 않는다", () => {
    let ring: AlarmRecord[] = [];
    ring = pushAlarm(ring, rec({ id: "upd-bin", detail: "1%" }));
    ring = pushAlarm(ring, rec({ name: "부서 완전 삭제 실패" }));
    ring = pushAlarm(ring, rec({ id: "upd-bin", detail: "2%" }));
    // 최신 진행률 1건 + 실패 1건만 남고, 이전 진행률 항목은 사라진다
    expect(ring.map((r) => r.detail)).toEqual(["2%", "d"]);
    expect(ring.some((r) => r.name === "부서 완전 삭제 실패")).toBe(true);
  });
  it("id 없는 volatile은 합치지 않고 매 건 쌓는다(반복 실패 횟수 보존)", () => {
    let ring: AlarmRecord[] = [];
    for (let i = 0; i < 3; i++) ring = pushAlarm(ring, rec({ detail: "같은 실패" }));
    expect(ring.length).toBe(3);
  });
  it("cap 0/음수는 최소 1건으로 보정(빈 링 반환 금지)", () => {
    expect(pushAlarm([], rec(), 0).length).toBe(1);
    expect(pushAlarm([], rec(), -5).length).toBe(1);
  });
});

describe("formatAlarmTime", () => {
  it("HH:MM:SS 2자리 0패딩", () => {
    const d = new Date(2026, 6, 30, 9, 5, 3);
    expect(formatAlarmTime(d.getTime())).toBe("09:05:03");
  });
});
