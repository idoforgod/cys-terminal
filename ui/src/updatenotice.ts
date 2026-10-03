// ui/src/updatenotice.ts — 업데이트가 '설치되지 않았을 때' 사람이 읽는 안내 문구와 재시작 뒤 판정의 해석(0.14.43 · J2).
//
// 왜 필요한가: Windows 의 '스마트 앱 컨트롤'이 서명 없는 설치 파일을 막아도 인앱 업데이트는 아무 말이 없었다. 업데이터 플러그인이
// 설치기를 띄운 뒤 반환값을 보지 않고 곧바로 앱을 끝내므로(tauri-plugin-updater 2.10.1) 실패를 알릴 코드가 실행되지 않고, 사용자가
// 앱을 다시 열어도 버전만 그대로였다. 백엔드(src-tauri)가 설치 직전에 '시도 기록'을 남기고 다시 뜬 앱에서 한 번 판정해 돌려주면
// (update_attempt_report) 이 모듈이 그 응답을 화면이 할 일로 옮기고, 설치 전에는 스마트 앱 컨트롤이 켜진 PC 에 사실을 미리 알린다.
//
// 이 모듈은 문구와 해석만 한다 — 화면·IPC·저장소·타이머를 모른다(main.ts 가 배선·렌더를 한다).
// ★문구는 **사실만** 말한다. 스마트 앱 컨트롤을 끄라는 지시도, 끄는 방법도 어디에도 없다 — 그 판단은 사용자 몫이다
//   (updatenotice.test.ts 가 낱말로 핀한다).
// ★백엔드 응답은 **신뢰할 수 없는 데이터**로 다룬다 — 모양부터 의심하고, 화면에 올릴 문자열은 제어문자를 걷고 길이를 자른다.
//   렌더는 main.ts 가 stickyToast·확인 창(textContent)으로만 한다(HTML 삽입 없음).
//
// ★이 모듈의 불변식(updatenotice.test.ts 가 핀으로 고정 — starvednotice.ts 와 같다):
//   · 최상위 부수효과 0 — 선언(export/const/function/interface/type)만. 저장소·문서 객체·창 객체·타이머·IPC 접근 0
//     (main.js 는 번들 하나라 여기서 평가 중 예외가 나면 앱 전체가 백지가 된다).
//   · 구형 WKWebView 가 파싱하지 못하는 문법 0 — 정규식 뒤돌아보기·배열 끝 인덱스 접근·뒤에서 찾기·구조 복제·소유 판정 정적 메서드·전체 치환 계열.

/** 업데이트 미설치 알림의 제목. */
export const UPDATE_FAILED_TITLE = "업데이트가 설치되지 않았습니다";

/**
 * 업데이트 미설치 알림의 sticky 토스트 id — main.ts 의 stickyToast 호출이 이 상수를 쓴다(id 문자열의 정의처는 여기 하나).
 * 수명(10분)·만료 시 OS 배너 보강은 toastttl.ts 가 이 id 에 건다 — 그쪽은 모듈 결합을 피하려 **같은 값을 따로 적어 둔다**
 * (toastttl.test.ts 가 두 값을 함께 잰다 — 한쪽만 바뀌면 이 알림이 60초 만에 조용히 사라지는 기본 수명으로 되돌아간다).
 */
export const UPDATE_FAILED_TOAST_ID = "update-not-installed";

/** 판정 보류(`pending`) 뒤 다시 당기기 전에 더 기다리는 여유(초) — 백엔드가 말한 대기 시간이 막 끝난 경계에서 또 보류가 나오지 않게. */
export const UPDATE_ATTEMPT_RETRY_SLACK_SECS = 5;
/** 백엔드가 `wait_secs` 를 주지 않았거나 읽을 수 없을 때의 대기(초) — 판정 보류 창(90초) 전체. */
export const UPDATE_ATTEMPT_WAIT_DEFAULT_SECS = 90;
/** `wait_secs` 의 상한(초) — 이상한 큰 값이 타이머 한계(약 24.8일)를 넘어 즉시 발화로 뒤집히는 것을 막는다. */
export const UPDATE_ATTEMPT_WAIT_MAX_SECS = 300;
/** 알림에 싣는 버전 문자열의 글자 수 상한(코드 포인트) — 이상한 값이 알림을 도배하지 않게. */
const VERSION_MAX = 40;

/** 설치 전 안내 문단(스마트 앱 컨트롤이 **켜짐**일 때만) — 확인 창 본문 끝에 한 줄 띄우고 붙는다. */
const SAC_PREFLIGHT_TEXT =
  "이 PC 는 Windows '스마트 앱 컨트롤'이 켜져 있습니다. 지금 cys 설치 파일에는 코드 서명이 없어 Windows 가 설치를 막을 수 있습니다. " +
  "막히면 경고 없이 지금 버전이 그대로 남고, 앱을 다시 열면 '설치되지 않았습니다' 알림이 뜹니다. " +
  "스마트 앱 컨트롤이 켜진 PC 에서는 홈페이지에서 받은 설치 파일도 같은 이유로 막힙니다.";

/** 제어문자·줄바꿈·양방향 제어·제로폭 문자 — 알림 한 줄을 속이거나 깨뜨릴 수 있는 것들. */
const INVISIBLE = /[\u{0}-\u{1f}\u{7f}-\u{9f}\u{200b}-\u{200f}\u{2028}\u{2029}\u{202a}-\u{202e}\u{2066}-\u{2069}\u{feff}]/gu;

/** 버전 문자열 하나를 화면에 올릴 수 있게 다듬는다 — 문자열이 아니거나 비면 "?"(모른다고 말한다). */
function version(v: unknown): string {
  if (typeof v !== "string") return "?";
  const s = v.replace(INVISIBLE, " ").replace(/\s+/g, " ").trim();
  if (s === "") return "?";
  const cs = Array.from(s);
  return cs.length > VERSION_MAX ? cs.slice(0, VERSION_MAX - 1).join("") + "…" : s;
}

/**
 * 패치 설치 확인 창에 덧붙일 사전 안내 — 스마트 앱 컨트롤이 `"on"`(켜짐)일 때만 문단을 돌려준다.
 * `"off"`·`"eval"`(평가 모드 — 차단하지 않는다)·모르는 값·null·undefined·빈 문자열은 null(= 본문은 종전과 바이트 동일).
 * 설치를 막는 문구가 아니다 — 사실을 알릴 뿐이고 사용자가 계속할지 정한다.
 */
export function sacPreflightText(sac: string | null | undefined): string | null {
  return sac === "on" ? SAC_PREFLIGHT_TEXT : null;
}

/** 설치되지 않은 업데이트의 알림 입력 — 백엔드 `update_attempt_report` 의 `failed` 응답에서 온다. */
export interface UpdateFailedInput {
  /** 설치 직전에 실행 중이던(지금도 그대로인) 버전. */
  from: string;
  /** 설치하려던 버전. */
  to: string;
  /** 실행 OS — "windows" | "macos" | "linux"(백엔드 `std::env::consts::OS`). 없으면 윈도우 아님으로 본다. */
  os?: string;
  /** 윈도우 스마트 앱 컨트롤 상태 — "on" | "off" | "eval" | null(조회 못 함·윈도우 아님). */
  sac?: string | null;
}

/**
 * '업데이트가 설치되지 않았습니다' 알림의 제목·본문. 첫 줄은 모든 OS 공통이고, 그 아래는 OS 별이다:
 *  · windows — 서명 없는 설치 파일을 Windows 가 막았을 수 있다는 사실 · 확인 위치(이벤트 뷰어의 CodeIntegrity 3033·3077) · 설치기가
 *    풀리는 임시 폴더 이름 · (스마트 앱 컨트롤이 켜짐/평가 모드이면 그 사실 한 줄). 끄라는 말은 없다.
 *  · 그 밖 — 홈페이지에서 설치 파일을 받아 직접 설치해 달라는 안내.
 */
export function updateFailedNotice(r: UpdateFailedInput): { title: string; body: string } {
  const from = version(r.from);
  const to = version(r.to);
  const lines: string[] = [`${to} 업데이트가 설치되지 않았습니다 — 지금 버전은 ${from} 그대로입니다.`];
  if (typeof r.os === "string" && r.os.trim().toLowerCase() === "windows") {
    lines.push(
      "Windows 가 서명 없는 설치 파일을 막았을 수 있습니다(스마트 앱 컨트롤 · Defender). " +
        "확인: 이벤트 뷰어 → 응용 프로그램 및 서비스 로그 → Microsoft → Windows → CodeIntegrity → Operational 의 이벤트 3033·3077. " +
        `설치기는 임시 폴더의 cys-${to}-updater-… 아래에 풀립니다.`,
    );
    if (r.sac === "on") {
      lines.push("이 PC 의 스마트 앱 컨트롤: 켜짐 — 켜져 있는 동안은 서명 없는 설치 파일이 수동 설치에서도 막힙니다.");
    } else if (r.sac === "eval") {
      lines.push("이 PC 의 스마트 앱 컨트롤: 평가 모드(차단하지 않음) — 다른 원인(Defender 등)을 확인해 주세요.");
    }
  } else {
    lines.push("홈페이지(www.cysinsight.com)에서 설치 파일을 받아 직접 설치해 주세요.");
  }
  return { title: UPDATE_FAILED_TITLE, body: lines.join("\n") };
}

/** 백엔드 응답을 화면이 할 일로 옮긴 결과. */
export type UpdateAttemptPlan =
  /** 알릴 것도 기다릴 것도 없다(응답 null·모르는 모양). */
  | { kind: "none" }
  /** 설치기가 아직 도는 중일 수 있다 — `delayMs` 뒤에 **한 번만** 다시 당긴다(그때도 보류면 더 하지 않는다). */
  | { kind: "retry"; delayMs: number }
  /** 업데이트가 설치되지 않았다 — 이 제목·본문으로 알린다. */
  | { kind: "notice"; title: string; body: string };

/**
 * `update_attempt_report` 응답 → 화면이 할 일.
 *  · `{failed: true, from, to, os, sac}` → 알림(notice)
 *  · `{pending: true, wait_secs}` → `(wait_secs + 5)` 초 뒤 재-pull(retry) — `wait_secs` 가 숫자가 아니거나 음수면 90초, 300초로 상한
 *  · 그 밖(null·모르는 모양·배열) → none
 * 둘 다 참이면(있을 수 없는 응답) 알림이 이긴다.
 */
export function planUpdateAttemptReport(r: unknown): UpdateAttemptPlan {
  if (typeof r !== "object" || r === null || Array.isArray(r)) return { kind: "none" };
  const o = r as Record<string, unknown>;
  if (o.failed === true) {
    const n = updateFailedNotice({
      from: o.from as string,
      to: o.to as string,
      os: typeof o.os === "string" ? o.os : undefined,
      sac: typeof o.sac === "string" ? o.sac : null,
    });
    return { kind: "notice", title: n.title, body: n.body };
  }
  if (o.pending === true) {
    const w = o.wait_secs;
    const secs =
      typeof w === "number" && Number.isFinite(w) && w >= 0
        ? Math.min(Math.ceil(w), UPDATE_ATTEMPT_WAIT_MAX_SECS)
        : UPDATE_ATTEMPT_WAIT_DEFAULT_SECS;
    return { kind: "retry", delayMs: (secs + UPDATE_ATTEMPT_RETRY_SLACK_SECS) * 1000 };
  }
  return { kind: "none" };
}
