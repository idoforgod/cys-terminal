// U6(0.14.41) 피드백 1단계 — 순수 판정·문구(DOM·Tauri 무관 · 최상위 부수효과 0).
//
// 1단계 범위(설계 §3 U6): 작성 창 → 이 컴퓨터에 묶음(~/.cys/feedback/<id>/) → 기본 메일 앱을
// 받는 주소·제목·본문 자동으로 열기 + 묶음 폴더 열기. 서버 업로드는 없다(2단계 — 설치 파일과
// 분리된 격리 호스팅이 정해진 뒤).
//
// 상한 값은 Rust(src-tauri/src/feedback.rs)가 **다시** 강제한다 — 여기 값은 사용자에게 먼저
// 알려 주기 위한 사본이고, 두 값의 동일성은 feedback.test.ts '상한 파리티'가 소스로 대조한다.
import { baseName } from "./ftdrop";

export const FEEDBACK_MAX_FILES = 10;
export const FEEDBACK_MAX_FILE_BYTES = 200 * 1024 * 1024;
export const FEEDBACK_MAX_TOTAL_BYTES = 300 * 1024 * 1024;
export const FEEDBACK_MIN_DESC_CHARS = 5;
export const FEEDBACK_MAX_DESC_CHARS = 10000;
/** 파일 고르기·붙여넣기 바이트를 Rust 로 넘기는 원시 IPC 조각 크기(메모리 상한 고정). */
export const FEEDBACK_CHUNK_BYTES = 4 * 1024 * 1024;
/** 흔한 메일 첨부 한도(25MB)보다 조금 아래 — 넘기면 미리 알린다. */
const MAIL_SOFT_LIMIT = 20 * 1024 * 1024;

const IMAGE_EXT = ["png", "jpg", "jpeg", "gif", "webp", "heic"];
const VIDEO_EXT = ["mov", "mp4", "m4v", "webm"];
const TEXT_EXT = ["txt", "log", "json"];
/** 허용 확장자(사진·영상·텍스트). 실행형·스크립트·SVG(스크립트 포함 가능)는 넣지 않는다. */
export const FEEDBACK_ALLOWED_EXT: readonly string[] = [...IMAGE_EXT, ...VIDEO_EXT, ...TEXT_EXT];

export type AttachKind = "image" | "video" | "text";

/** 마지막 점 뒤 확장자(소문자). 경로 구분자는 맥 `/`·윈도우 `\` 둘 다 인식. 없으면 "". */
export function extOf(nameOrPath: string): string {
  const name = baseName(nameOrPath);
  const i = name.lastIndexOf(".");
  if (i <= 0 || i === name.length - 1) return "";
  return name.slice(i + 1).toLowerCase();
}

export function attachmentKind(nameOrPath: string): AttachKind | null {
  const e = extOf(nameOrPath);
  if (IMAGE_EXT.includes(e)) return "image";
  if (VIDEO_EXT.includes(e)) return "video";
  if (TEXT_EXT.includes(e)) return "text";
  return null;
}

export type AttachCheck = { ok: true; kind: AttachKind } | { ok: false; reason: string };

/** 첨부 하나를 더해도 되는가(형식·빈 파일·파일 상한·개수·합계). 이유는 사람이 읽는 문장. */
export function validateAttachment(
  nameOrPath: string,
  size: number,
  current: { count: number; total: number },
): AttachCheck {
  const kind = attachmentKind(nameOrPath);
  if (!kind) {
    const e = extOf(nameOrPath);
    return {
      ok: false,
      reason: `${e ? "." + e : "확장자 없는"} 파일은 첨부할 수 없습니다 — 사진·영상·텍스트(로그)만 받습니다.`,
    };
  }
  if (!(size > 0)) return { ok: false, reason: "빈 파일은 첨부할 수 없습니다." };
  if (size > FEEDBACK_MAX_FILE_BYTES) {
    return { ok: false, reason: `파일 하나는 200MB까지입니다(${formatBytes(size)}).` };
  }
  if (current.count >= FEEDBACK_MAX_FILES) {
    return { ok: false, reason: `첨부는 ${FEEDBACK_MAX_FILES}개까지입니다.` };
  }
  if (current.total + size > FEEDBACK_MAX_TOTAL_BYTES) {
    return { ok: false, reason: "첨부 합계는 300MB까지입니다." };
  }
  return { ok: true, kind };
}

export function formatBytes(n: number): string {
  if (n < 1024) return `${n}B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)}KB`;
  return `${(n / (1024 * 1024)).toFixed(1)}MB`;
}

/** 설명 길이 — 앞뒤 공백을 뺀 글자(코드 포인트) 수. 한글 1자 = 1. */
export function descLength(s: string): number {
  return Array.from(s.trim()).length;
}

type SubmitState = { desc: string; pending: number; submitting: boolean };

/** [메일로 보내기]를 막는 이유(사람이 읽는 문장). 보낼 수 있으면 null. */
export function submitBlockReason(st: SubmitState): string | null {
  if (st.submitting) return "묶음을 만드는 중입니다…";
  if (st.pending > 0) return "첨부 파일을 넣는 중입니다…";
  const n = descLength(st.desc);
  if (n < FEEDBACK_MIN_DESC_CHARS) return `무엇이 있었는지 ${FEEDBACK_MIN_DESC_CHARS}자 이상 적어 주세요.`;
  if (n > FEEDBACK_MAX_DESC_CHARS) return `설명은 ${FEEDBACK_MAX_DESC_CHARS}자까지입니다(지금 ${n}자).`;
  return null;
}

export function canSubmit(st: SubmitState): boolean {
  return submitBlockReason(st) === null;
}

/** 닫을 때 "버릴까요?"를 물을지 — 쓴 것이 있고 아직 묶음을 만들기 전일 때만. */
export function shouldConfirmDiscard(st: { desc: string; attachments: number; done: boolean }): boolean {
  if (st.done) return false;
  return descLength(st.desc) > 0 || st.attachments > 0;
}

/** 붙여넣은 이미지의 확장자 — 이미지 MIME 만(SVG 제외). 아니면 null. */
export function feedbackImageExt(mime: string): "png" | "jpg" | "gif" | "webp" | null {
  const m = mime.toLowerCase();
  if (m === "image/png") return "png";
  if (m === "image/jpeg" || m === "image/jpg") return "jpg";
  if (m === "image/gif") return "gif";
  if (m === "image/webp") return "webp";
  return null;
}

/** [start, end) 조각 목록 — 빈틈·겹침 없이 0..size 를 덮는다. 조각 크기 ≤ 0 이면 빈 계획. */
export function chunkRanges(size: number, chunk: number): Array<[number, number]> {
  const out: Array<[number, number]> = [];
  if (!(chunk > 0) || !(size > 0)) return out;
  for (let a = 0; a < size; a += chunk) out.push([a, Math.min(a + chunk, size)]);
  return out;
}

/** 메일 첨부 한도를 넘길 묶음이면 안내 한 줄, 아니면 null. */
export function mailSizeNote(totalBytes: number): string | null {
  if (totalBytes <= MAIL_SOFT_LIMIT) return null;
  return `첨부 합계 ${formatBytes(totalBytes)} — 보통 메일은 25MB까지 받습니다. 큰 영상은 클라우드 링크(iCloud Mail Drop·Google Drive 등)로 보내 주셔도 됩니다.`;
}

/** 캡처 요령(플랫폼 단축키). */
export function captureTips(platform: "mac" | "win" | "other"): string {
  if (platform === "mac") return "캡처 요령: ⌘⇧4 = 화면 일부 사진 · ⌘⇧5 = 화면 녹화(영상)";
  if (platform === "win") return "캡처 요령: Win+Shift+S = 화면 일부 사진 · 캡처 도구(Snipping Tool)의 녹화 = 영상";
  return "캡처 요령: 운영체제의 화면 캡처 도구로 사진·영상을 찍어 끌어다 놓아 주세요.";
}

/** 개인정보 안내(1단계 — 서버 전송 없음). 보관 기한처럼 오너가 정할 약속은 적지 않는다. */
export const FEEDBACK_PRIVACY_NOTICE =
  "수집 항목: 작성하신 설명, 첨부하신 파일, (동의하신 경우) 앱·운영체제 버전·데몬 응답 여부·화면 수.\n" +
  "터미널 화면 내용·대화 기록·파일 목록은 자동으로 모으지 않습니다.\n" +
  "저장·전달: 먼저 이 컴퓨터의 ~/.cys/feedback 폴더에만 저장됩니다. 앱이 자동으로 보내지 않으며, 메일 앱에서 직접 [보내기]를 누르실 때만 전달됩니다.\n" +
  "목적: cys 오류 확인·개선과 답장.\n" +
  "거부 권리: 진단 정보는 체크를 끄면 넣지 않습니다. 동의하지 않아도 피드백은 보낼 수 있습니다.";
