import { describe, expect, test } from "bun:test";
import {
  bootBannerDecision,
  isFormationKind,
  bannerScopeId,
  normalizeBootWarning,
  formationFeedAction,
  bootWarningPlan,
  PARTIAL_BANNER_TEXT,
  GLOBAL_BOOT_WARN_ID,
} from "./bootbanner";

describe("bootBannerDecision — 배너 수명 판정(kind 문자열 핀 = 회귀 0)", () => {
  test("formation-complete → dismiss(배너 소멸)", () => {
    expect(bootBannerDecision("formation-complete")).toEqual({ action: "dismiss" });
  });

  test("formation-partial → update(문구 갱신·제거 아님)", () => {
    const d = bootBannerDecision("formation-partial");
    expect(d.action).toBe("update");
    if (d.action === "update") expect(d.text).toBe(PARTIAL_BANNER_TEXT);
  });

  test("formation-pending → ignore(미완이므로 배너 유지·무변경)", () => {
    expect(bootBannerDecision("formation-pending")).toEqual({ action: "ignore" });
  });

  test("formation-failed → ignore(실패는 dismiss 하지 않는다 — 오소멸 방지)", () => {
    expect(bootBannerDecision("formation-failed")).toEqual({ action: "ignore" });
  });

  test("구팩 호환 kind 'formation'(상태 미분화) → ignore(오소멸 방지)", () => {
    expect(bootBannerDecision("formation")).toEqual({ action: "ignore" });
  });

  test("비-formation kind(승인 요청 등) → ignore", () => {
    expect(bootBannerDecision("ceo-promote-request")).toEqual({ action: "ignore" });
    expect(bootBannerDecision("bootstrap-fail")).toEqual({ action: "ignore" });
    expect(bootBannerDecision("")).toEqual({ action: "ignore" });
  });
});

describe("isFormationKind — 편성 신호 라우팅 판별", () => {
  test("formation-* 접두는 편성 신호", () => {
    expect(isFormationKind("formation-complete")).toBe(true);
    expect(isFormationKind("formation-partial")).toBe(true);
    expect(isFormationKind("formation-pending")).toBe(true);
    expect(isFormationKind("formation")).toBe(true);
  });

  test("그 외 kind 는 편성 신호 아님(승인 경로 유지)", () => {
    expect(isFormationKind("ceo-promote-request")).toBe(false);
    expect(isFormationKind("notification")).toBe(false);
    expect(isFormationKind("")).toBe(false);
  });
});

describe("bannerScopeId — 소켓 스코프 배너 id", () => {
  test("slug 있으면 boot-warn:<slug>", () => {
    expect(bannerScopeId("abc123")).toBe("boot-warn:abc123");
  });
  test("slug 없으면(구 payload) 전역 boot-warn", () => {
    expect(bannerScopeId(null)).toBe(GLOBAL_BOOT_WARN_ID);
    expect(bannerScopeId("")).toBe(GLOBAL_BOOT_WARN_ID);
    expect(bannerScopeId(undefined)).toBe(GLOBAL_BOOT_WARN_ID);
  });
});

describe("normalizeBootWarning — payload 정규화(신형 객체·구형 문자열)", () => {
  test("신형 {slug,message}", () => {
    expect(normalizeBootWarning({ slug: "s1", message: "hi" })).toEqual({ slug: "s1", message: "hi" });
  });
  test("구형 string → slug null(전역 배너)", () => {
    expect(normalizeBootWarning("legacy msg")).toEqual({ slug: null, message: "legacy msg" });
  });
  test("빈 slug/누락 → null + 폴백 메시지", () => {
    expect(normalizeBootWarning({ slug: "", message: "" }).slug).toBeNull();
    const n = normalizeBootWarning({});
    expect(n.slug).toBeNull();
    expect(n.message.length).toBeGreaterThan(0);
    expect(normalizeBootWarning(null).slug).toBeNull();
  });
});

describe("formationFeedAction — 소켓 스코프(FIX-1 교차 오소멸 차단)", () => {
  test("같은 소켓 complete → 그 소켓 배너 id dismiss", () => {
    expect(formationFeedAction("formation-complete", "base9")).toEqual({
      op: "dismiss",
      bannerId: "boot-warn:base9",
    });
  });

  test("★교차소켓: 부서 소켓 complete 는 부서 배너 id 만 대상(base 배너 미접촉)", () => {
    const deptAct = formationFeedAction("formation-complete", "dept7");
    expect(deptAct).toEqual({ op: "dismiss", bannerId: "boot-warn:dept7" });
    // base 배너 id 와 다르다 → onDaemonEvent 의 dismissToast(bannerId) 가 base 를 못 지운다.
    expect(deptAct.op === "dismiss" && deptAct.bannerId !== "boot-warn:base9").toBe(true);
  });

  test("partial → 그 소켓 배너 update(text 동봉)", () => {
    const a = formationFeedAction("formation-partial", "d1");
    expect(a).toEqual({ op: "update", bannerId: "boot-warn:d1", text: PARTIAL_BANNER_TEXT });
  });

  test("slug 부재 → op none(스코프 불명·오소멸 방지)", () => {
    expect(formationFeedAction("formation-complete", null)).toEqual({ op: "none" });
    expect(formationFeedAction("formation-complete", "")).toEqual({ op: "none" });
  });

  test("pending/failed/구팩 formation → op none(배너 유지)", () => {
    expect(formationFeedAction("formation-pending", "s").op).toBe("none");
    expect(formationFeedAction("formation-failed", "s").op).toBe("none");
    expect(formationFeedAction("formation", "s").op).toBe("none");
  });
});

describe("bootWarningPlan — 엣지 경합 백스톱(FIX-2 불멸 배너 차단)", () => {
  test("★complete 선도착: 그 소켓 최신=complete면 배너 생성 안 함(순서 무관 수렴)", () => {
    expect(bootWarningPlan("s1", "formation-complete")).toEqual({
      create: false,
      bannerId: "boot-warn:s1",
    });
  });

  test("아직 complete 아님(pending/partial/미관측) → 스코프 배너 생성", () => {
    expect(bootWarningPlan("s1", "formation-pending")).toEqual({ create: true, bannerId: "boot-warn:s1" });
    expect(bootWarningPlan("s1", "formation-partial")).toEqual({ create: true, bannerId: "boot-warn:s1" });
    expect(bootWarningPlan("s1", undefined)).toEqual({ create: true, bannerId: "boot-warn:s1" });
  });

  test("교차소켓 무간섭: 다른 소켓이 complete여도 이 소켓 배너는 생성", () => {
    // lastKind 는 호출자가 이 소켓 slug 로 조회하므로, 다른 소켓 complete 는 여기 인자에 안 들어온다.
    expect(bootWarningPlan("s2", undefined).create).toBe(true);
  });

  test("구 payload(slug null) → 전역 배너 생성(백스톱 비적용 — 보수적 잔존)", () => {
    expect(bootWarningPlan(null, "formation-complete")).toEqual({
      create: true,
      bannerId: GLOBAL_BOOT_WARN_ID,
    });
  });
});
