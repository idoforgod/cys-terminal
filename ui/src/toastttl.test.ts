// toastttl.ts 순수 로직 회귀 테스트 (bun test — 신규 의존성 0).
//
// ★T-0147-3: "모든 에러 알람은 종류 불문 일정 시간이 지나면 꺼진다" + "정보는 이력에 남는다"의
// 두 축을 각각 고정한다. ①TTL 정책(종류·id별 수명·무한 잔존 0) ②갱신 리셋 규칙(진행 중 소멸 0)
// ③이력 링버퍼(cap·최신순·같은 id 합침) ④고위험 만료의 OS 배너 보강.
// ★(0.14.43 · J2) '업데이트가 설치되지 않았습니다' 알림(update-not-installed)은 안내용 수명(10분)과 만료 배너를 받는다 —
//   1회만 나오는 알림이라 60초 만에 사라지면 사용자가 실패 자체를 모르고 지나간다(아래 전용 describe).
import { describe, it, expect } from "bun:test";
import { readFileSync } from "node:fs";
import {
  VOLATILE_TTL_MS,
  STICKY_TTL_MS,
  PROGRESS_TTL_MS,
  GUIDE_TTL_MS,
  ALARM_HISTORY_CAP,
  toastTtl,
  toastTimerPlan,
  needsExpiryBanner,
  expiryBannerText,
  pushAlarm,
  formatAlarmTime,
  type AlarmRecord,
} from "./toastttl";
import { PERM_TOAST_PREFIX } from "./folderaccess";
import { UPDATE_FAILED_TOAST_ID } from "./updatenotice";

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
    // (0.14.41 · U14) perm-* 는 아래 전용 테스트(10분)로 옮겼다 — 여전히 유한 수명이다.
    for (const id of ["boot-warn", "safe-mode", "purge-fail-/tmp/x.sock"]) {
      expect(toastTtl("sticky", id).ttlMs).toBe(STICKY_TTL_MS);
    }
  });
  it("장기 진행형 sticky는 3분(중간 갱신 없이 60초를 넘겨도 진행 중 소멸 없음)", () => {
    expect(PROGRESS_TTL_MS).toBe(180000);
    for (const id of ["restore", "rotate-daemon", "transfer", "upd-bin", "upd-pack", "restart-daemon", "daemon-hint"]) {
      expect(toastTtl("sticky", id).ttlMs).toBe(PROGRESS_TTL_MS);
    }
  });
  it("★(0.14.41 · U14) 폴더 접근 안내(perm-*)는 10분 — 설정 화면을 따라가는 동안 사라지지 않는다", () => {
    // 60초면 사람이 시스템 설정의 여러 단계를 따라가는 동안 안내가 먼저 사라졌다(반박 M4).
    // 여전히 유한하다(오너 요구 = 종류 불문 소멸) — 무한 불변식 테스트가 이 id 도 함께 잰다.
    for (const id of ["perm-Desktop", "perm-Documents", "perm-seat-Desktop", "perm-seat-/Volumes/X"]) {
      expect(toastTtl("sticky", id).ttlMs).toBe(600_000);
    }
    expect(GUIDE_TTL_MS).toBe(600_000);
    // 접두는 문구 SOT(folderaccess.ts)의 토스트 id 접두와 같다.
    expect(toastTtl("sticky", `${PERM_TOAST_PREFIX}Desktop`).ttlMs).toBe(GUIDE_TTL_MS);
    // 접두가 우연히 겹치는 다른 id 는 연장하지 않는다(정확히 "perm-" 접두만).
    expect(toastTtl("sticky", "permission").ttlMs).toBe(STICKY_TTL_MS);
    expect(toastTtl("volatile", "perm-Desktop").ttlMs).toBe(VOLATILE_TTL_MS);
  });
  it("어떤 조합도 무한(0·Infinity)이 아니다 — 오너 요구의 하드 불변식", () => {
    const ids = [undefined, "boot-warn", "safe-mode", "restore", "purge-fail-x", "unknown-id", "perm-Desktop", "perm-seat-x", UPDATE_FAILED_TOAST_ID, "update-not-installed-x"];
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

describe("★(0.14.43 · J2) 업데이트 미설치 알림(update-not-installed) — 안내용 수명 10분 · 만료 시 OS 배너 1회", () => {
  it("toastTtl(sticky, update-not-installed) = GUIDE_TTL_MS(10분) — 앱을 다시 연 직후 1분 안에 못 봐도 사라지지 않는다", () => {
    expect(UPDATE_FAILED_TOAST_ID).toBe("update-not-installed");
    expect(toastTtl("sticky", UPDATE_FAILED_TOAST_ID).ttlMs).toBe(GUIDE_TTL_MS);
    expect(toastTtl("sticky", "update-not-installed").ttlMs).toBe(600_000);
    // 같은 id 로 다시 띄워도(갱신) 같은 수명이고 이전 타이머를 걷는다
    expect(toastTimerPlan("sticky", UPDATE_FAILED_TOAST_ID, true)).toEqual({ ttlMs: GUIDE_TTL_MS, clearPrevious: true });
    expect(toastTimerPlan("sticky", UPDATE_FAILED_TOAST_ID, false)).toEqual({ ttlMs: GUIDE_TTL_MS, clearPrevious: false });
    // 10분도 유한하다(오너 요구 = 종류 불문 소멸)
    expect(Number.isFinite(GUIDE_TTL_MS) && GUIDE_TTL_MS > STICKY_TTL_MS).toBe(true);
  });

  it("정확 일치다 — 이름이 비슷한 id 는 연장하지 않는다(접두·부분·대소문자 불일치)", () => {
    for (const id of ["update-not-installed-x", "x-update-not-installed", "update-not-installed ", " update-not-installed", "Update-Not-Installed", "update-not", "not-installed", "update-not-installed:1"]) {
      expect({ id, ttl: toastTtl("sticky", id).ttlMs }).toEqual({ id, ttl: STICKY_TTL_MS });
    }
    expect(toastTtl("sticky", "").ttlMs).toBe(STICKY_TTL_MS);
    expect(toastTtl("sticky", undefined).ttlMs).toBe(STICKY_TTL_MS);
  });

  it("다른 id 의 기존 수명은 그대로다(진행형 3분 · perm- 10분 · 나머지 60초 · volatile 8초)", () => {
    for (const id of ["upd-bin", "upd-pack", "restore", "rotate-daemon", "restart-daemon", "transfer", "daemon-hint"]) {
      expect({ id, ttl: toastTtl("sticky", id).ttlMs }).toEqual({ id, ttl: PROGRESS_TTL_MS });
    }
    for (const id of ["perm-x", "perm-Desktop", "perm-seat-/Volumes/X", `${PERM_TOAST_PREFIX}Documents`]) {
      expect({ id, ttl: toastTtl("sticky", id).ttlMs }).toEqual({ id, ttl: GUIDE_TTL_MS });
    }
    for (const id of ["purge-fail-1", "boot-warn", "safe-mode", "permission", "bundle-damaged", "claude-missing"]) {
      expect({ id, ttl: toastTtl("sticky", id).ttlMs }).toEqual({ id, ttl: STICKY_TTL_MS });
    }
    expect(toastTtl("volatile", UPDATE_FAILED_TOAST_ID).ttlMs).toBe(VOLATILE_TTL_MS); // volatile 은 id 를 무시한다
    expect(toastTtl("volatile", "perm-x").ttlMs).toBe(VOLATILE_TTL_MS);
  });

  it("needsExpiryBanner(update-not-installed) = true — 1회만 나오는 알림이라 만료 때 OS 배너로 한 번 더 알린다", () => {
    expect(needsExpiryBanner(UPDATE_FAILED_TOAST_ID)).toBe(true);
    expect(needsExpiryBanner("update-not-installed")).toBe(true);
  });

  it("배너 보강의 범위는 늘지 않았다 — 이름이 비슷한 id·기존 일반 sticky 는 false · purge-fail- 접두는 그대로 true", () => {
    for (const id of ["update-not-installed-x", "x-update-not-installed", "Update-Not-Installed", "update-not", "upd-bin", "upd-pack", "perm-x", "perm-Desktop", "boot-warn", "bundle-damaged", ""]) {
      expect({ id, banner: needsExpiryBanner(id) }).toEqual({ id, banner: false });
    }
    for (const id of ["purge-fail-1", "purge-fail-/Users/x/.cys/run/dept-1.sock", "purge-fail-"]) {
      expect({ id, banner: needsExpiryBanner(id) }).toEqual({ id, banner: true });
    }
  });

  it("만료 배너 문구는 이 알림에도 '자동 닫힘 + 알람 탭에서 재조회'를 명시한다", () => {
    const t = expiryBannerText("업데이트가 설치되지 않았습니다", "0.14.43 업데이트가 설치되지 않았습니다 — 지금 버전은 0.14.42 그대로입니다.");
    expect(t.title).toBe("⚠ 업데이트가 설치되지 않았습니다");
    expect(t.body).toContain("0.14.42 그대로입니다");
    expect(t.body).toContain("알람");
  });

  it("★id 문자열 두 곳이 같다 — updatenotice.ts 의 UPDATE_FAILED_TOAST_ID 와 toastttl.ts 의 안내용 수명·만료 배너 정확 일치 목록(모듈 결합 없이 값만 같게 둔다)", () => {
    const read = (rel: string) => readFileSync(new URL(rel, import.meta.url), "utf-8");
    const ttl = read("./toastttl.ts");
    const notice = read("./updatenotice.ts");
    // 정의처: updatenotice.ts 에 상수가 정확히 한 번 정의된다
    const def = notice.match(/export const UPDATE_FAILED_TOAST_ID = "([^"]*)";/);
    expect(def === null ? null : def[1]).toBe(UPDATE_FAILED_TOAST_ID);
    // toastttl.ts 의 두 정확 일치 목록은 그 값 하나씩이다
    const listOf = (name: string): string[] => {
      const m = ttl.match(new RegExp(`const ${name} = \\[([^\\]]*)\\] as const;`));
      expect({ 목록: name, 존재: m !== null }).toEqual({ 목록: name, 존재: true });
      return (m![1].match(/"([^"]*)"/g) ?? []).map((x) => x.slice(1, -1));
    };
    expect(listOf("GUIDE_STICKY_IDS")).toEqual([UPDATE_FAILED_TOAST_ID]);
    expect(listOf("BANNER_ON_EXPIRY_IDS")).toEqual([UPDATE_FAILED_TOAST_ID]);
    // 모듈 결합 금지 — toastttl.ts 는 updatenotice.ts 를 import 하지 않는다(값만 같게 둔다)
    expect(/from\s+["']\.\/updatenotice["']/.test(ttl)).toBe(false);
  });

  it("★main.ts 가 이 알림을 그 상수 id 로 띄운다 — stickyToast(UPDATE_FAILED_TOAST_ID, …) · 같은 id 의 리터럴 0 · 그 id 는 10분+배너를 받는다", () => {
    const main = readFileSync(new URL("./main.ts", import.meta.url), "utf-8")
      .replace(/\/\*[\s\S]*?\*\//g, "")
      .split("\n")
      .map((l) => (l.trimStart().startsWith("//") ? "" : l.replace(/\s\/\/.*$/, "")))
      .join("\n");
    expect(main.includes('stickyToast(UPDATE_FAILED_TOAST_ID, "health", plan.title, plan.body)')).toBe(true);
    expect(main.includes('"update-not-installed"')).toBe(false);
    expect(/import\s*\{[^}]*\bUPDATE_FAILED_TOAST_ID\b[^}]*\}\s*from\s*["']\.\/updatenotice["']/.test(main)).toBe(true);
    // 그 상수 값으로 실제 수명·배너 판정이 나온다(두 곳이 어긋나면 여기서 갈린다)
    expect(toastTtl("sticky", UPDATE_FAILED_TOAST_ID).ttlMs).toBe(GUIDE_TTL_MS);
    expect(needsExpiryBanner(UPDATE_FAILED_TOAST_ID)).toBe(true);
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
