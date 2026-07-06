// WKWebView 한글 IME 조합 판단 로직 — 순수 리듀서.
//
// main.ts의 DOM 이벤트 핸들러(input·keydown·blur·composition·onData)는 이 리듀서에
// 배선만 하고, PTY로 보낼 바이트(send)와 계측(debug)은 전부 actions로만 나온다.
// 순수 함수라 프로필별 이벤트 시퀀스를 결정론으로 재현·회귀 테스트할 수 있다.
//
// 배경(main.ts 원 주석 요약): WKWebView는 표준 composition 이벤트 없이 ①음절 첫 자모를
// insertText로 커밋(xterm이 즉시 전송 = 자모 유출) ②조합 진행을 insertReplacementText로
// value 치환(xterm 미인지 = 완성 글자 유실)한다. 이 리듀서가 자모 유출을 pending에
// 붙들었다가 음절 확정 시 완성 글자만 보낸다.

export interface ImeState {
  /** 조합 중이라 아직 확정 전송하지 않은 한글 음절(들). 보통 1글자, 병합/치환 시 다중. */
  pending: string;
  /**
   * ★유예(defer) 버퍼 (2026-07-06 3차 재발 수리 — 017e20e defer의 리듀서 이식):
   * WKWebView 이중배달 프로필은 한글 키마다 xterm onData(자모/완성음절)와 input 머신
   * (insertText→insertReplacementText) 두 경로가 같은 글자를 모두 배달한다. 0.12.20 업스트림
   * 리듀서 재작성 때 1차 수리(defer)가 소실되어 onData 즉시 send가 부활 — 초성 선유출
   * ("ㅁ마")·음절 복제("다다")가 재발했다. 한글 onData는 여기 buffered 후 DEFER_MS 유예:
   * 같은 글자를 input 머신이 받아가면(exact match) 취소, 아무도 안 받아가면 타임아웃 전송
   * (insertFromComposition만 쓰는 WebKit 변종 유실 0 — 차단이 아닌 유예).
   */
  deferred: string;
}

export type ImeEvent =
  | { kind: "input"; inputType: string; data: string | null }
  | { kind: "keydown"; keyCode: number; key: string }
  | { kind: "compositionstart" }
  | { kind: "compositionupdate" }
  | { kind: "compositionend" }
  /** wk=true(macOS WKWebView 배선)일 때만 한글 defer 경로 활성 — Windows WebView2 등은 종전 즉시 send. */
  | { kind: "onData"; data: string; wk?: boolean }
  /** 배선의 DEFER_MS 타이머 만료 — deferred 잔여분을 전송(머신이 안 받아간 경우). */
  | { kind: "deferTimeout" }
  | { kind: "blur" };

/** send=PTY로 보낼 바이트, debug=cysImeDebug 채널 로그(평시 미출력), armDefer=배선에 유예 타이머 (재)장전 지시. */
export type ImeAction = { send: string } | { debug: string } | { armDefer: true };

export interface ImeResult {
  state: ImeState;
  actions: ImeAction[];
}

export const initialImeState = (): ImeState => ({ pending: "", deferred: "" });

/** 한글 onData 유예 시간(ms) — 이중배달 프로필에서 input 머신 이벤트는 같은 tick에 도착한다(1차 실측). */
export const DEFER_MS = 40;

// 자모(31xx·11xx) + 완성형 음절(AC00-D7A3) — 멀티문자 허용: 고속 입력에서 IME가 여러 음절을
// 한 insertText로 병합 커밋하므로 단일 문자만 인정하면 그 묶음이 통째로 유실된다.
const HANGUL_TEXT = /^[ㄱ-ㆎᄀ-ᇿ가-힣]+$/;
export const isHangulText = (t: string) => HANGUL_TEXT.test(t);

// 조합 중 자모만(호환자모 ㄱ-ㆎ·조합자모 ᄀ-ᇿ) — 완성형 음절(가-힣)은 제외.
// 빈 onData가 도착했을 때 pending이 '아직 조합 중인 자모'인지 판별해, 완성음절 앞으로
// 유출시키지 않고 조합 지속으로 두기 위한 기준.
const INCOMPLETE_JAMO = /^[ㄱ-ㆎᄀ-ᇿ]+$/;
export const isIncompleteJamo = (t: string) => INCOMPLETE_JAMO.test(t);

// 완성형 음절의 초성(호환자모) — pending 홑자모가 deferred 첫 음절의 조합중 초성인지 판별용.
// (예: pending "ㅁ"은 deferred "마"의 초성 → 그 음절에 흡수된 조합중 상태이므로 별도 전송 금지)
const CHOSEONG = ["ㄱ", "ㄲ", "ㄴ", "ㄷ", "ㄸ", "ㄹ", "ㅁ", "ㅂ", "ㅃ", "ㅅ", "ㅆ", "ㅇ", "ㅈ", "ㅉ", "ㅊ", "ㅋ", "ㅌ", "ㅍ", "ㅎ"];
export const isChoseongOf = (jamo: string, syllable: string) => {
  if (jamo.length !== 1 || !syllable) return false;
  const c = syllable.codePointAt(0)! - 0xac00;
  if (c < 0 || c > 11171) return false;
  return CHOSEONG[Math.floor(c / 588)] === jamo;
};

export function imeStep(state: ImeState, event: ImeEvent): ImeResult {
  const actions: ImeAction[] = [];
  let pending = state.pending;
  let deferred = state.deferred;

  const flush = (why: string) => {
    if (pending) {
      actions.push({ debug: `FLUSH(${why}) "${pending}"` });
      actions.push({ send: pending });
      pending = "";
    }
  };
  // input 머신이 같은 글자를 받아갔다 — 유예분 취소(이중배달 차단). exact match 한정:
  // 과잉 취소(≠)는 유실, 과소 취소는 40ms 뒤 복제이므로, 이중배달 프로필이 두 경로에
  // 문자열을 동일하게 배달한다는 1차 실측 불변식에 정확히 맞춘다.
  const cancelDeferredIfTaken = (data: string, why: string) => {
    if (deferred && deferred === data) {
      actions.push({ debug: `DEFER-cancel(${why}) "${deferred}"` });
      deferred = "";
    }
  };
  // 유예분 방출 — pending이 그 첫 음절의 조합중 초성이면 흡수된 상태이므로 폐기(유출 금지),
  // pending이 유예분과 동일하면 이중배달이므로 한쪽만 전송.
  const releaseDeferred = (why: string) => {
    if (!deferred) return;
    if (pending && pending === deferred) {
      actions.push({ debug: `DEFER-dedup(${why}) "${pending}"` });
      pending = "";
    } else if (pending && isChoseongOf(pending, deferred)) {
      actions.push({ debug: `DROP(absorbed-by-deferred) "${pending}"` });
      pending = "";
    }
    actions.push({ debug: `DEFER-send(${why}) "${deferred}"` });
    actions.push({ send: deferred });
    deferred = "";
  };
  // 프로필 불변 규칙: 조합 이벤트가 관측되는 순간, pending 자모는 정의상 그 조합에 흡수된
  // 것이므로 폐기(drop)한다 — flush 금지. 어떤 WebKit 프로필에서도 조합 이벤트 후 pending
  // 자모 전송이 옳은 경우는 없다(혼성 프로필 C의 자모 유출 근본 차단).
  const dropSuperseded = () => {
    if (pending) {
      actions.push({ debug: `DROP(composition-supersede) "${pending}"` });
      pending = "";
    }
  };

  switch (event.kind) {
    case "input": {
      const { inputType, data } = event;
      actions.push({ debug: `input ${inputType} data="${data ?? "∅"}" pending="${pending}"` });
      if (inputType === "insertCompositionText" || inputType === "insertFromComposition") {
        // 표준 composition inputType 관측 → 조합이 pending 자모를 흡수했다 → drop.
        dropSuperseded();
      } else if (inputType === "insertText" && data && isHangulText(data)) {
        // ★머신이 이 글자를 받아간다 — onData가 유예해 둔 동일 글자를 취소(이중배달 차단).
        cancelDeferredIfTaken(data, "insertText");
        // 직전 조합 확정 후 새 커밋을 '수정 가능 창'(pending)에 둔다. 병합 커밋(2음절+)은
        // 마지막 음절만 수정 창에 — 앞 음절들은 확정분이므로 즉시 전송.
        flush("insertText");
        if (data.length > 1) {
          actions.push({ debug: `SEND(multi-head) "${data.slice(0, -1)}"` });
          actions.push({ send: data.slice(0, -1) });
        }
        pending = data.slice(-1);
      } else if (inputType === "insertReplacementText" && data) {
        cancelDeferredIfTaken(data, "insertReplacementText"); // 조합 계속 — 머신 소유(이중배달 차단)
        if (pending) {
          pending = data; // 조합 갱신 (하→한)
        } else {
          // 이미 전송된 직전 음절의 교정 — PTY 동기화: 백스페이스+재전송
          actions.push({ debug: `SEND(repl-sync) DEL+"${data}"` });
          actions.push({ send: "\x7f" + data });
        }
      } else if (inputType === "deleteContentBackward" && pending) {
        // 멀티 pending(병합 커밋 잔여)이면 마지막 글자만 — IME 부분 재조합 대응
        pending = pending.slice(0, -1);
        actions.push({ debug: `del-backward pending="${pending}"` });
      }
      break;
    }
    case "keydown": {
      // 일반 키(Enter·Space·화살표 등, IME 처리중 229 제외) 직전에 조합 확정.
      // 유예분이 남아 있으면(머신이 안 받아간 onData) 먼저 방출 — releaseDeferred가
      // pending과의 중복(dedup)·흡수(초성) 관계를 해소해 경계 유출·복제를 막는다.
      if (event.keyCode !== 229) {
        actions.push({ debug: `keydown ${event.key}` });
        releaseDeferred("keydown");
        flush("keydown");
      }
      break;
    }
    case "compositionstart":
    case "compositionupdate": {
      // 조합 시작/진행 관측 → pending 자모는 이 조합에 흡수됨 → drop.
      actions.push({ debug: event.kind });
      dropSuperseded();
      break;
    }
    case "compositionend": {
      // 확정 완성 음절은 xterm의 onData로 별도 도착하므로 여기서 drop하지 않는다.
      actions.push({ debug: event.kind });
      break;
    }
    case "onData": {
      // ★빈 onData 유출 가드 (2026-07-06 실기기 계측 확정): 사용자가 조합 중 홑자모까지
      // 되돌린 상태(pending='ㅎ' 등)에서 데이터 없는 onData("")가 도착하면, 아래 flush가 그
      // 조합중 자모를 완성음절 앞으로 유출시킨다(실 PTY 유출 5건: ㅇㅈㅇㅎㄱ). 빈 onData는
      // 완성분을 나르지 않으므로 조합중 자모는 flush하지 않고 pending에 그대로 둔다 — 후속
      // 조합/input 이벤트가 흡수(drop)하거나 완성 커밋한다(유출0·유실0). 완성형 pending(가-힣)은
      // 진짜 직전 음절이므로 종전대로 flush(순서 보존). 유예분(deferred)도 건드리지 않는다 —
      // 빈 onData는 아무것도 나르지 않으므로 순서 제약이 없고, 조기 방출은 복제를 만든다.
      if (event.data === "" && isIncompleteJamo(pending)) {
        actions.push({ debug: `onData(empty) keep composing jamo "${pending}"` });
        break;
      }
      // ★한글 onData 유예 (3차 재발 수리 — 017e20e defer의 리듀서 이식, wk 배선 한정):
      // 이중배달 프로필은 같은 글자를 input 머신이 같은 tick에 받아간다(1차 실측) — 즉시
      // send하면 pending flush와 겹쳐 초성 선유출("ㅁ마")·음절 복제("다다")가 된다. 유예 후
      // 머신이 받아가면 취소(cancelDeferredIfTaken), 안 받아가면 타임아웃 전송(유실 0).
      if (event.wk && event.data && isHangulText(event.data)) {
        if (event.data === pending) {
          // 머신이 이미 이 글자를 조합 창(pending)에 소유 — onData 쪽은 이중배달분이므로 폐기.
          actions.push({ debug: `DUP-drop(onData) "${event.data}" (== pending)` });
          break;
        }
        deferred += event.data;
        actions.push({ debug: `DEFER(onData) "${event.data}" buffered="${deferred}"` });
        actions.push({ armDefer: true });
        break;
      }
      // 비한글(또는 비wk 배선): 순서 보존 — 유예분 → pending → 이번 데이터 순으로 전송.
      releaseDeferred("onData-nonhangul");
      // (no-op 안전장치: 잔여 pending 있으면 순서 보존 후 전송) 뒤이어 완성 음절을 그대로 PTY로.
      flush("onData");
      actions.push({ send: event.data });
      break;
    }
    case "deferTimeout": {
      // DEFER_MS 동안 아무 input 머신 이벤트도 이 글자를 받아가지 않았다 —
      // insertFromComposition만 쓰는 WebKit 변종(머신 미작동)이므로 유예분을 그대로 전송.
      releaseDeferred("timeout");
      break;
    }
    case "blur": {
      releaseDeferred("blur");
      flush("blur");
      break;
    }
  }

  return { state: { pending, deferred }, actions };
}
