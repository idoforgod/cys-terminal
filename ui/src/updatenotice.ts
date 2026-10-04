// ui/src/updatenotice.ts — 업데이트가 '설치되지 않았을 때' 사람이 읽는 안내 문구와 재시작 뒤 판정의 해석(0.14.43 · J2).
//
// 왜 필요한가: Windows 의 '스마트 앱 컨트롤'이 코드 서명도 평판도 없는 설치 파일을 막아도 인앱 업데이트는 아무 말이 없었다. 업데이터 플러그인이
// 설치기를 띄운 뒤 반환값을 보지 않고 곧바로 앱을 끝내므로(tauri-plugin-updater 2.10.1) 실패를 알릴 코드가 실행되지 않고, 사용자가
// 앱을 다시 열어도 버전만 그대로였다. 백엔드(src-tauri)가 설치 직전에 '시도 기록'을 남기고 다시 뜬 앱에서 한 번 판정해 돌려주면
// (update_attempt_report) 이 모듈이 그 응답을 화면이 할 일로 옮기고, 설치 전에는 스마트 앱 컨트롤이 켜진 PC 에 사실을 미리 알린다.
//
// 이 모듈은 문구와 해석만 한다 — 화면·IPC·저장소·타이머를 모른다(main.ts 가 배선·렌더를 한다).
// ★문구는 **잰 것만 단정**하고 사실만 말한다(W11/FINDINGS): 스마트 앱 컨트롤이 켜진 PC 에서 막히는 것은 Windows 가 신뢰하는 코드 서명도 평판도 없는 파일이다 —
//   서명 없는 7-Zip 은 평판으로 허용됐다. 이 문구는 0.14.43 앱 안에 굳어 **다음 버전으로 올릴 때** 화면에 나오는데 그 설치 파일이 서명될지는 지금 모른다.
//   그래서 "서명 없는 프로그램은 실행되지 않는다"·"이 설치 파일에는 코드 서명이 없다" 같은 단정은 쓰지 않고 조건문("…없으면")으로 적는다
//   (updatenotice.test.ts 가 옛 단정의 부재를 핀한다).
//   스마트 앱 컨트롤을 끄라는 지시도, 끄는 방법도 어디에도 없다 — 그 판단은 사용자 몫이다(낱말로 핀한다).
//   ★WU(아래 installerLaunchFailure)의 4551 문구만 확인할 곳과 선택의 결과(보호 수준을 낮춘다 · Windows 업데이트 상태에 따라 다시 켜지 못할 수 있다)를 사실로 적는다 — 지시는 아니다.
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

/**
 * 설치 전 안내 문단(스마트 앱 컨트롤이 **켜짐**일 때만) — 확인 창 본문 끝에 한 줄 띄우고 붙는다. 두 판이 있고 **"막히면 …" 문장만 다르다**:
 *  · 기본(확인 실행 켜짐 · `CYS_UPDATE_CHECKED_LAUNCH` 미설정 또는 0 이 아님) — 막히면 앱은 닫히지 않은 채 그 사실을 알린다.
 *  · 확인 실행 꺼짐(`CYS_UPDATE_CHECKED_LAUNCH=0` — 종전 경로) — 막혀도 앱이 알림 없이 닫힌다. 판은 백엔드가 알려 주는 값(update_checked_launch_enabled)으로 고른다.
 * 새 설치 파일의 서명 여부는 이 앱이 받기 전에 알 수 없다 — 그래서 막힌다고 단정하지 않고 "…없으면"으로 적는다(0.14.43 까지의 설치 파일에 서명이 없다는 것만 단정한다).
 */
const SAC_PREFLIGHT_HEAD =
  "이 PC 는 Windows '스마트 앱 컨트롤'이 켜져 있습니다. " +
  "새 설치 파일에 Windows 가 신뢰하는 코드 서명이나 평판이 없으면 Windows 가 실행을 막습니다(0.14.43 까지의 cys 설치 파일에는 코드 서명이 없습니다). ";
const SAC_PREFLIGHT_IF_CHECKED = "막히면 업데이트는 설치되지 않고, 이 앱은 닫히지 않은 채 그 사실을 알려 드립니다. ";
const SAC_PREFLIGHT_IF_UNCHECKED = "막히면 업데이트는 설치되지 않고, 지금 설정(CYS_UPDATE_CHECKED_LAUNCH=0)에서는 이 앱이 알림 없이 닫힙니다. ";
const SAC_PREFLIGHT_TAIL =
  "그 경우 홈페이지에서 받은 설치 파일도 같은 이유로 막힙니다. " +
  "스마트 앱 컨트롤이 켜져 있는 동안에는 이 앱을 닫으면 다시 열 때도 막힐 수 있습니다.";
const SAC_PREFLIGHT_TEXT = SAC_PREFLIGHT_HEAD + SAC_PREFLIGHT_IF_CHECKED + SAC_PREFLIGHT_TAIL;
const SAC_PREFLIGHT_TEXT_UNCHECKED = SAC_PREFLIGHT_HEAD + SAC_PREFLIGHT_IF_UNCHECKED + SAC_PREFLIGHT_TAIL;

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
 * `checkedLaunch`(가산 · 생략해도 된다): 확인 실행이 켜져 있는지(백엔드 update_checked_launch_enabled). **정확히 `false` 일 때만** 확인 실행 꺼짐 판
 * (막히면 앱이 알림 없이 닫힌다)을 돌려주고, 그 밖(true·생략·null·모르는 값)은 기본 판이다 — 조회 실패·시간 초과도 기본값(켜짐)으로 본다.
 */
export function sacPreflightText(sac: string | null | undefined, checkedLaunch?: boolean | null): string | null {
  if (sac !== "on") return null;
  return checkedLaunch === false ? SAC_PREFLIGHT_TEXT_UNCHECKED : SAC_PREFLIGHT_TEXT;
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
 *  · windows — 원인 후보(설치가 끝나지 않았거나 받는 중에 앱이 닫힘 · Windows 가 설치 파일의 실행을 막았을 수도 있음) · 확인 위치(이벤트 뷰어의
 *    CodeIntegrity 3033·3077 · 설치 폴더의 설치기 실패 기록 cys-install-failure.txt) · 받은 설치 파일이 저장되는 임시 폴더 이름 ·
 *    (스마트 앱 컨트롤이 켜짐/평가 모드이면 그 사실 한 줄). 끄라는 말은 없다.
 *  · 그 밖 — 홈페이지에서 설치 파일을 받아 직접 설치해 달라는 안내.
 */
export function updateFailedNotice(r: UpdateFailedInput): { title: string; body: string } {
  const from = version(r.from);
  const to = version(r.to);
  const lines: string[] = [`${to} 업데이트가 설치되지 않았습니다 — 지금 버전은 ${from} 그대로입니다.`];
  if (typeof r.os === "string" && r.os.trim().toLowerCase() === "windows") {
    // 설치기의 실패 기록 `cys-install-failure.txt` 는 설치기 훅(src-tauri/nsis-hooks.nsh cys_post_fail)이 `$INSTDIR` — 설치 폴더(기본 %LOCALAPPDATA%\cys) — 에 쓴다.
    lines.push(
      "설치가 끝나지 않았거나 받는 중에 앱이 닫혔을 수 있습니다. Windows 가 설치 파일의 실행을 막았을 수도 있습니다(스마트 앱 컨트롤 · Defender). " +
        "확인: 이벤트 뷰어 → 응용 프로그램 및 서비스 로그 → Microsoft → Windows → CodeIntegrity → Operational 의 이벤트 3033·3077, " +
        "설치 폴더(보통 %LOCALAPPDATA%\\cys)의 cys-install-failure.txt(설치기의 실행 파일 교체·검증이 실패하면 이 파일이 남습니다). " +
        `받은 설치 파일은 임시 폴더의 cys-${to}-updater-… 아래에 저장됩니다.`,
    );
    if (r.sac === "on") {
      lines.push("이 PC 의 스마트 앱 컨트롤: 켜짐 — 켜져 있는 동안은 Windows 가 신뢰하는 코드 서명이나 평판이 없는 설치 파일이 수동 설치에서도 막힙니다.");
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

// ── (0.14.43 · WU) 설치 파일 실행이 막혔을 때의 알림 ─────────────────────────────────────────────────────────────────
//
// 윈도우 인앱 업데이트가 설치 파일을 띄우지 못하면(스마트 앱 컨트롤 등 앱 제어 정책이 코드 서명도 평판도 없는 파일의 실행을 막음) 백엔드는 앱을 닫지 않고
// 오류 `installer_launch_failed:<os_code>:<shell_ret>` 를 돌려준다. J2 의 알림은 앱이 **다시 뜬 뒤** 한 번 알리는 경로이고, 이쪽은 앱이 **살아 있는 채**
// 바로 알리는 경로다 — 같은 사실(업데이트가 설치되지 않았다)이라 같은 알림 자리(토스트 id)를 쓴다. 이 함수는 문구만 만든다(화면은 main.ts).
// ★문구는 사실만 말한다 — 막힌 이유(코드별)·업데이트가 설치되지 않았고 지금 버전이 그대로 실행 중이며 앱이 닫히지 않았다는 것·확인할 곳.
//   J2 의 원칙(사실만 · 지시 없음)은 그대로다: 4551 문구의 마지막 줄은 확인할 곳과 선택의 결과를 적을 뿐 무엇을 하라고 말하지 않는다(판단은 사용자 몫).

/**
 * 설치 파일 실행 실패 알림의 sticky 토스트 id — J2 '설치되지 않았습니다' 알림(`UPDATE_FAILED_TOAST_ID`)과 **같은 id 값**이다.
 * 같은 사실이 한 자리에 뜨고(두 알림이 겹치지 않는다), toastttl.ts 가 이 값에 거는 안내용 수명(10분)·만료 시 OS 배너 1회 규칙이 그대로 적용된다.
 */
export const INSTALLER_LAUNCH_FAILED_TOAST_ID = UPDATE_FAILED_TOAST_ID;

/** 백엔드 오류 문자열의 꼴(Rust `LaunchError` 의 Display) — 자릿수를 묶어 이상하게 큰 수를 거른다. 앞뒤 공백·다른 접두는 이 꼴이 아니다. */
const LAUNCH_FAILED_RE = /^installer_launch_failed:(\d{1,10}):(-?\d{1,10})$/;

/**
 * 설치 파일 실행 실패 오류 → 사람이 읽는 제목·본문. `err` 가 `installer_launch_failed:<os_code>:<shell_ret>` 꼴이 아니면 null(= 종전 토스트 그대로).
 *  · 4551(앱 제어 정책 차단 — 스마트 앱 컨트롤 등) · 5(접근 거부) · 2·3(설치 파일 없음) · 225(보안 프로그램이 위험으로 판정) · 1223(취소) · 그 밖(코드 표시).
 *  · 모든 본문이 같은 문장을 나눠 갖는다: 업데이트는 설치되지 않았고 지금 버전(`current`)이 그대로 실행 중이며 앱은 닫히지 않았다.
 *  · `current`·`target` 은 화면에 올릴 수 있게 다듬는다(제어문자 제거·길이 제한·비면 "?") — 오류 문자열에서 오는 값은 숫자뿐이다.
 */
export function installerLaunchFailure(err: string, current: string, target: string): { title: string; body: string } | null {
  if (typeof err !== "string") return null;
  const m = LAUNCH_FAILED_RE.exec(err);
  if (m === null) return null;
  const code = Number(m[1]);
  const ret = Number(m[2]);
  const cur = version(current);
  const tgt = version(target);
  const kept = `업데이트는 설치되지 않았고 지금 버전(${cur})이 그대로 실행 중입니다. 앱은 닫히지 않았습니다.`;
  const make = (title: string, lines: string[]): { title: string; body: string } => ({ title, body: lines.join("\n") });
  switch (code) {
    case 4551:
      return make("설치 파일 실행이 차단되었습니다", [
        `Windows 의 앱 제어 정책(스마트 앱 컨트롤 등)이 새 버전(${tgt}) 설치 파일의 실행을 막았습니다(오류 4551).`,
        kept,
        "스마트 앱 컨트롤이 켜진 PC 에서는 Windows 가 신뢰하는 코드 서명이나 평판이 없는 프로그램이 실행되지 않습니다.",
        "스마트 앱 컨트롤이 켜져 있는 동안에는 이 앱을 닫으면 다시 열 때도 막힐 수 있으니, 작업을 마치기 전에는 앱을 닫지 마세요.",
        "확인하는 곳: Windows 보안 → 앱 및 브라우저 컨트롤 → 스마트 앱 컨트롤. " +
          "스마트 앱 컨트롤을 사용하지 않도록 바꾸면 설치할 수 있지만 PC 의 보호 수준을 낮추는 선택이며, Windows 업데이트 상태에 따라 다시 켜지 못할 수 있습니다.",
      ]);
    case 5:
      return make("설치 파일 실행이 거부되었습니다(오류 5)", [
        `Windows 가 새 버전(${tgt}) 설치 파일의 실행을 거부했습니다(오류 5).`,
        kept,
        "보안 프로그램이나 폴더 접근 권한이 임시 폴더의 설치 파일 실행을 막았을 수 있습니다.",
      ]);
    case 2:
    case 3:
      return make("설치 파일을 찾을 수 없습니다", [
        `방금 받은 새 버전(${tgt}) 설치 파일을 임시 폴더에서 찾지 못했습니다(오류 ${code}).`,
        kept,
        "보안 프로그램이 설치 파일을 격리했거나 지웠을 수 있습니다.",
      ]);
    case 225:
      return make("보안 프로그램이 설치 파일을 위험으로 판정했습니다", [
        `보안 프로그램이 새 버전(${tgt}) 설치 파일을 위험한 파일로 판정해 실행을 막았습니다(오류 225).`,
        kept,
        "사용 중인 보안 프로그램의 검사·격리 기록을 확인해 주세요.",
      ]);
    case 1223:
      return make("설치 파일 실행이 취소되었습니다", [
        `새 버전(${tgt}) 설치 파일 실행이 취소되었습니다(오류 1223).`,
        kept,
        "다시 설치하려면 업데이트를 다시 시작해 주세요.",
      ]);
    default:
      return make(`설치 파일을 실행하지 못했습니다(오류 ${code})`, [
        `Windows 가 새 버전(${tgt}) 설치 파일을 실행하지 못했습니다(오류 ${code} · 셸 반환값 ${ret}).`,
        kept,
      ]);
  }
}
