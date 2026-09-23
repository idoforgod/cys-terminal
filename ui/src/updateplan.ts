// 업데이트 분기 판정 — 순수 함수 (옵션 2 · 오너 승인 2026-07-14).
// 목적: "본체+팩 동시 존재 & 바이너리 호환"일 때 팩 무중단 경로가 본체 알림에
// 가려지던 간극을 닫는다(pack-and-binary 분기 신설).
// ★T5 개정(오너 2026-07-15 실험): 본체 인앱 패치 설치(install_update) 재배선 —
// binary·pack-and-binary 문구가 "홈페이지 다운로드"에서 "Update 버튼 패치 설치"로
// 바뀌었고 updateplan.test.ts 핀도 함께 갱신됐다(의도적 계약 변경).
// 계약: ①분기 구조(5분기)는 불변 ②silent 경로는 토스트만
// (모달 금지 — 시작 시 자동 체크가 온보딩 화면에 끼어들지 않게, 성찰 불변식).

export type UpdateInputs = {
  binVersion: string | null; // 새 본체 버전(없으면 null) — fail-safe 보존 상태 기준
  packVersion: string | null; // 새 팩 버전(없으면 null)
  binaryTooOld: boolean; // 팩 min_binary_version > 설치 바이너리
  binCheckFailed: boolean;
  packCheckFailed: boolean;
};

export type UpdatePlan = {
  kind: "pack-and-binary" | "binary" | "pack" | "binary-required" | "none" | "unknown";
  badge: string; // 배지 텍스트("" = 배지 갱신 안 함·유지)
  ok: boolean; // 중립 스타일(.ok) 여부
  title: string; // 배지 title
  toastMsg: string; // silent 토스트 본문(없으면 "")
};

export function updatePlan(i: UpdateInputs): UpdatePlan {
  if (i.binVersion && i.packVersion && !i.binaryTooOld) {
    // ★옵션 2: 팩 무중단을 가리지 않는다 — 팩은 지금(↻·재시작 없음), 본체는 패치 설치(T5 개정).
    return {
      kind: "pack-and-binary",
      badge: "↻",
      ok: false,
      title: `팩 ${i.packVersion} 무중단 적용 가능 (새 본체 ${i.binVersion}은 패치 설치)`,
      toastMsg:
        `팩 ${i.packVersion}은 상단 Update로 무중단 적용(재시작 없음) · ` +
        `새 본체 ${i.binVersion}은 Update 버튼으로 패치 설치(재시작·자동 복원)`,
    };
  }
  if (i.binVersion) {
    // 본체 패치 설치(T5 개정 — 오너 2026-07-15): Update 버튼 클릭 시 인앱 패치 설치.
    return {
      kind: "binary",
      badge: "!",
      ok: false,
      title: `새 본체 버전 ${i.binVersion} (Update 버튼으로 패치 설치)`,
      toastMsg: `새 본체 ${i.binVersion} — 상단 Update 버튼으로 패치 설치(재시작·자동 복원)`,
    };
  }
  if (i.packVersion && !i.binaryTooOld) {
    return {
      kind: "pack",
      badge: "↻",
      ok: false,
      title: `팩 ${i.packVersion} (무중단·세션 유지)`,
      toastMsg: `팩 ${i.packVersion} — 상단 Update(재시작 없음)`,
    };
  }
  if (i.packVersion && i.binaryTooOld) {
    return {
      kind: "binary-required",
      badge: "!",
      ok: false,
      title: `팩 ${i.packVersion}: 본체 업데이트 필요 (홈페이지에서 다운로드)`,
      toastMsg:
        `새 팩 ${i.packVersion}은 더 새로운 본체를 요구합니다 — ` +
        `홈페이지(www.cysinsight.com)에서 본체 업데이트 후 적용됩니다.`,
    };
  }
  if (!i.binCheckFailed && !i.packCheckFailed) {
    // ★U9(0.14.41 · 의도적 계약 변경): '0' → '✓'. 둥근 알약 '0'이 알림 개수로 읽혀 "숫자가 떠 있는데
    // 눌러 보면 최신"의 한 뿌리가 됐다. '확인 전'(숨김/…)과 '확인됐고 0건'(✓)의 구별(2026-07-03 지시 목적)은 유지.
    return {
      kind: "none",
      badge: "✓",
      ok: true,
      title: "최신 버전 — 대기 중인 업데이트 없음",
      toastMsg: "",
    };
  }
  // 체크 실패 + 보존 상태 없음 = 상태 불명 — 배지 유지('최신' 오단정 금지, 종전 fail-safe).
  return { kind: "unknown", badge: "", ok: false, title: "", toastMsg: "" };
}
