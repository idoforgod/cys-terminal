// U6(0.14.41 · 리뷰1 수정 라운드) 피드백 작성 창의 **수명 동작** — 순수 함수(DOM·Tauri 는 인자로
// 받는다 · 최상위 부수효과 0). feedbackmodal.ts 가 이 함수들을 실제로 쓰는지는 feedbackwiring.test.ts
// ④ 가 소스로 대조한다(리뷰1 §3 변이 M5·M7·M11·M13·M14 가 저장소 안 테스트 없이는 GREEN 이던 자리).
import { shouldReclaimFocus } from "./modalguard";

/** makeEscHandler 가 받는 최소 키 이벤트 모양(KeyboardEvent 는 구조적으로 만족한다). */
export interface EscKeyLike {
  key: string;
  isComposing: boolean;
  keyCode: number;
  preventDefault(): void;
  stopPropagation(): void;
}

/**
 * Esc = 닫기 요청(리뷰 minor #4 · M14). 두 경우엔 손대지 않는다 — ① IME 조합 중(`isComposing` 또는
 * 구형 WebKit 의 `keyCode===229`)의 Esc 는 조합 취소이지 창 닫기가 아니다. ② 이 창 위에 다른 모달이
 * 떠 있으면(온보딩 심문 등) 그 창의 Esc 다 — `isTopLayer()` 가 false 면 가로채지도 삼키지도 않는다.
 */
export function makeEscHandler(isTopLayer: () => boolean, onClose: () => void): (e: EscKeyLike) => void {
  return (e) => {
    if (e.key !== "Escape") return;
    if (e.isComposing || e.keyCode === 229) return;
    if (!isTopLayer()) return;
    onClose();
    e.preventDefault();
    e.stopPropagation();
  };
}

/** makeFocusReclaimer 가 받는 최소 포커스 이벤트 모양(FocusEvent 는 구조적으로 만족한다). */
export interface FocusInLike {
  target: unknown;
}

/**
 * focusin 되찾기(M5 · 반박 D2 이중 방어) — setFocus 가드가 뚫리는 경로가 남아도, 포커스가 이 모달
 * 층 밖(뒤에 가려진 pane 의 xterm 등)으로 나가면 창의 기본 칸으로 즉시 되찾는다. 창을 닫은 뒤에는
 * 손대지 않는다(pane 으로 돌아가는 포커스와 다투지 않게).
 */
export function makeFocusReclaimer(deps: {
  closed: () => boolean;
  home: () => { focus(): void };
}): (e: FocusInLike) => void {
  return (e) => {
    if (deps.closed()) return;
    if (!shouldReclaimFocus(e.target)) return;
    try {
      deps.home().focus();
    } catch {
      /* 되찾기는 방어선일 뿐 — 실패해도 창은 그대로다 */
    }
  };
}

type Un = () => void;
type ListenFn = (name: string, handler: (e: { payload: unknown }) => void) => Promise<Un>;

/**
 * 창 수명 구독(D13) — `sub()` 은 즉시 구독을 걸고, 창이 닫혀도(`dispose()`) 늦게 풀린 구독을 놓치지
 * 않는다(M11: 늦게 풀린 해제 함수는 도착 즉시 스스로를 해제한다). 해제 함수가 던져도 나머지 해제와
 * 창 닫기는 계속된다. 구독 자체가 실패(거부·동기 예외)해도 창을 죽이지 않고 실패한 이름으로 알린다.
 */
export function scopedListener(listen: ListenFn): { sub: (name: string, h: (e: { payload: unknown }) => void, onFail?: (name: string) => void) => void; dispose: () => void } {
  let disposed = false;
  const uns: Un[] = [];
  const sub = (name: string, h: (e: { payload: unknown }) => void, onFail?: (name: string) => void) => {
    try {
      listen(name, h)
        .then((un) => {
          if (disposed) {
            try {
              un();
            } catch {
              /* 이미 늦었다 — 그래도 해제는 마친다 */
            }
            return;
          }
          uns.push(un);
        })
        .catch(() => onFail?.(name));
    } catch {
      onFail?.(name);
    }
  };
  const dispose = () => {
    disposed = true;
    for (const un of uns.splice(0)) {
      try {
        un();
      } catch {
        /* 하나가 던져도 나머지 해제는 계속된다 */
      }
    }
  };
  return { sub, dispose };
}

/**
 * 버리기 = 초안 삭제(M7) — 줄 선 첨부 작업(`job`)이 끝난 **뒤에** 초안 폴더를 지운다(Rust 잠금과
 * 이중 방어). 묶음을 이미 만들었으면(`bundled`) 지우지 않는다(사용자가 아직 메일에 첨부하지 않았을
 * 수 있다). 초안을 만든 적이 없으면(`draft` 가 없음) 아무것도 하지 않는다. 어느 단계가 실패해도
 * (첨부 실패·초안 생성 실패·삭제 실패) 줄은 깨지지 않는다.
 */
export async function discardDraftAfter(
  job: Promise<unknown>,
  state: { draft: Promise<string> | null; bundled: boolean },
  invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown>,
): Promise<void> {
  try {
    await job;
  } catch {
    /* 줄 선 작업이 실패해도 초안 정리는 시도한다 */
  }
  if (state.bundled) return;
  let id: string | null = null;
  try {
    id = state.draft ? await state.draft : null;
  } catch {
    id = null;
  }
  if (!id) return;
  try {
    await invoke("feedback_discard", { id });
  } catch {
    /* 초안이 없거나 이미 지워졌다 — 완전 초기화 인벤토리가 마지막 그물 */
  }
}

/** openBundleForMail 이 필요로 하는 최소 보고서 모양. */
export interface MailableReport {
  id: string;
  attachments: unknown[];
  include_diag: boolean;
}

/**
 * 보내기 뒤 여는 순서(M13) — 끌어다 넣을 파일(첨부 또는 진단)이 있으면 폴더를 먼저, 메일 창을
 * 나중에 연다(나중에 뜬 창이 앞에 온다). 넣을 파일이 없으면 폴더는 열지 않고 메일만 연다. 폴더
 * 열기가 실패해도 메일은 연다 — 실패 사유는 각각 돌려준다(성공은 null).
 */
export async function openBundleForMail(
  report: MailableReport,
  tryInvoke: (cmd: string, args: Record<string, unknown>) => Promise<string | null>,
): Promise<{ folderErr: string | null; mailErr: string | null }> {
  const hasFiles = report.attachments.length > 0 || report.include_diag;
  const folderErr = hasFiles ? await tryInvoke("feedback_reveal", { id: report.id }) : null;
  const mailErr = await tryInvoke("feedback_open_mail", { id: report.id });
  return { folderErr, mailErr };
}
