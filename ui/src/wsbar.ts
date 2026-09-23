// 워크스페이스 사이드바(#wsbar) 폭·글자 배율 계산 (순수 로직 — main.ts에서 분리해 단위테스트 대상).
// 오너 요청 2026-07-12: ①사이드바 폭을 마우스 드래그로 조절 ②글자 크기 조절 — 둘 다 localStorage 영속.
// 폭 하한은 헤더 버튼(A−/A＋/＋)이 잘리지 않는 선, 상한은 터미널 작업공간 잠식 방지.
// (0.14.41 U17: ＋부서는 사이드바 바닥 「전문가용」 칸으로 옮겼다 — 헤더가 좁아지는 쪽이라 하한 176 은 그대로 둔다.)

export const WSBAR_W_MIN = 176; // 140이면 헤더(제목+버튼4 — 당시)가 2단 랩(높이 33→70px) — 랩 없는 실측 하한(성찰 후속 2026-07-12)
export const WSBAR_W_MAX = 520;
export const WSBAR_W_DEFAULT = 216;
export const WSBAR_FONT_MIN = 0.8;
export const WSBAR_FONT_MAX = 2.2;
export const WSBAR_FONT_STEP = 0.1;

export function clampWsbarWidth(w: number): number {
  if (!Number.isFinite(w)) return WSBAR_W_DEFAULT;
  return Math.min(WSBAR_W_MAX, Math.max(WSBAR_W_MIN, Math.round(w)));
}

export function clampWsbarFont(f: number): number {
  if (!Number.isFinite(f) || f <= 0) return 1;
  return +Math.min(WSBAR_FONT_MAX, Math.max(WSBAR_FONT_MIN, f)).toFixed(2);
}
