import { describe, expect, test } from "bun:test";
import { updatePlan } from "./updateplan";

const base = { binCheckFailed: false, packCheckFailed: false, binaryTooOld: false };

describe("updatePlan — 옵션 2 분기 판정(문자열 핀 = 회귀 0 증명)", () => {
  test("★신설: 본체+팩 동시 + 호환 → 팩 무중단을 가리지 않는다", () => {
    const p = updatePlan({ ...base, binVersion: "0.12.57", packVersion: "0.12.58" });
    expect(p.kind).toBe("pack-and-binary");
    expect(p.badge).toBe("↻");
    expect(p.title).toBe("팩 0.12.58 무중단 적용 가능 (새 본체 0.12.57은 패치 설치)");
    expect(p.toastMsg).toContain("무중단 적용(재시작 없음)");
    expect(p.toastMsg).toContain("패치 설치"); // T5 개정(오너 2026-07-15) — 본체 인앱 패치 안내
  });

  test("본체+팩 동시 + 비호환 → 종전대로 본체 필요(가림이 정당한 케이스)", () => {
    const p = updatePlan({ ...base, binVersion: "0.12.57", packVersion: "0.12.58", binaryTooOld: true });
    // binVersion 존재가 우선 — 본체 안내(팩은 어차피 min_binary 미달).
    expect(p.kind).toBe("binary");
  });

  test("본체만 → 패치 설치 문구(T5 개정 — 오너 2026-07-15)", () => {
    const p = updatePlan({ ...base, binVersion: "0.12.57", packVersion: null });
    expect(p.kind).toBe("binary");
    expect(p.badge).toBe("!");
    expect(p.title).toBe("새 본체 버전 0.12.57 (Update 버튼으로 패치 설치)");
    expect(p.toastMsg).toBe("새 본체 0.12.57 — 상단 Update 버튼으로 패치 설치(재시작·자동 복원)");
  });

  test("팩만 + 호환 → 종전 무중단 문구 그대로(회귀 0)", () => {
    const p = updatePlan({ ...base, binVersion: null, packVersion: "0.12.58" });
    expect(p.kind).toBe("pack");
    expect(p.badge).toBe("↻");
    expect(p.title).toBe("팩 0.12.58 (무중단·세션 유지)");
    expect(p.toastMsg).toBe("팩 0.12.58 — 상단 Update(재시작 없음)");
  });

  test("팩만 + 비호환 → 종전 본체 필요 문구 그대로(회귀 0)", () => {
    const p = updatePlan({ ...base, binVersion: null, packVersion: "0.12.58", binaryTooOld: true });
    expect(p.kind).toBe("binary-required");
    expect(p.badge).toBe("!");
    expect(p.title).toBe("팩 0.12.58: 본체 업데이트 필요 (홈페이지에서 다운로드)");
    expect(p.toastMsg).toBe(
      "새 팩 0.12.58은 더 새로운 본체를 요구합니다 — 홈페이지(www.cysinsight.com)에서 본체 업데이트 후 적용됩니다.",
    );
  });

  // ★의도적 계약 변경(U9 · 0.14.41 · 설계 §3 U9): 최신 표시 '0' → '✓'. 오너 증상("업데이트 숫자 vs
  // 「최신입니다」")의 한 뿌리가 둥근 알약 '0'을 알림 개수로 읽는 것이었다. 2026-07-03 지시의 목적
  // ('확인 전'과 '확인됐고 대기 0건'의 구별)은 그대로 지킨다 — 확인 전은 숨김/'…', 확인 뒤는 '✓'.
  test("업데이트 없음 + 양쪽 체크 성공 → '✓' 배지(숫자 금지 · U9)", () => {
    const p = updatePlan({ ...base, binVersion: null, packVersion: null });
    expect(p.kind).toBe("none");
    expect(p.badge).toBe("✓");
    expect(p.ok).toBe(true);
    expect(p.title).toBe("최신 버전 — 대기 중인 업데이트 없음");
  });

  test("업데이트 없음 + 체크 실패 → unknown = 배지 유지(fail-safe 종전)", () => {
    for (const f of [
      { binCheckFailed: true, packCheckFailed: false },
      { binCheckFailed: false, packCheckFailed: true },
      { binCheckFailed: true, packCheckFailed: true },
    ]) {
      const p = updatePlan({ ...base, ...f, binVersion: null, packVersion: null });
      expect(p.kind).toBe("unknown");
      expect(p.badge).toBe("");
    }
  });

  test("체크 실패여도 보존 상태가 있으면 그 상태로 판정(fail-safe 보존 종전)", () => {
    const p = updatePlan({ ...base, binCheckFailed: true, binVersion: "0.12.57", packVersion: null });
    expect(p.kind).toBe("binary");
  });
});
