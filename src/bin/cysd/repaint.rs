//! ★(0.14.45 · F2) 화면 사본 재동기 — **다시 그리기 요청**(PTY 크기를 한 줄 흔든다 · stdin 0 바이트).
//!
//! 【왜】 데몬의 vt100 화면 사본은 좌석 출력만으로 만들어진다. Claude Code 는 상대 이동으로 **고친 곳만** 다시 그리고,
//!   유휴 좌석은 프롬프트 줄을 다시 그리지 않는다. 그래서 사본이 한 번 어긋나면(파서 패닉 격리가 빈 파서로 갈았다 ·
//!   청크가 버려졌다) 커서 행에서 마커를 영영 못 찾고 큐가 `prompt_unknown`·`input_pending`(판독 불가)으로 굶는다
//!   (0.14.45 윈도우 11 제보 — master 16분 · worker 의 빈 프롬프트). 사람이 직접 send + Return 을 하면 풀렸던 것은
//!   그것이 다시 그리기를 일으켰기 때문이다. 여기서는 **키를 한 바이트도 쓰지 않고** 같은 효과를 낸다 — PTY 를
//!   (rows-1, cols) 로 줄였다가 ~500ms 뒤 (rows, cols) 로 되돌리면 SIGWINCH(유닉스)·ConPTY 크기 변경(윈도우)을 받은
//!   TUI 가 화면 전체를 다시 그린다. 화면 사본(vt100 파서)의 크기도 같이 맞춘다.
//!
//! 【어느 변을 흔드는가 — 높이만, 줄였다가 되돌린다 (A3 · 윈도우 ConPTY 숙고)】
//!   · 무엇이든 크기가 바뀌면 node 는 `process.stdout` 에 `resize` 를 낸다(유닉스 SIGWINCH · 윈도우 libuv 의
//!     WINDOW_BUFFER_SIZE 이벤트 — rows·cols 어느 쪽이 달라져도). Ink 는 그 이벤트에 **프레임 전체를 다시 그린다**
//!     (classic 은 log-update 가 지난 프레임 줄을 지우고 다시 쓴다 · fullscreen 은 화면을 다시 칠한다). 그래서
//!     너비가 아니라 **높이**를 흔들어도 다시 그리기는 같은 효과다.
//!   · 너비(cols)를 흔들면 윈도우 conhost(ConPTY)는 **버퍼 전체를 재줄바꿈(reflow)** 하고 바뀐 줄을 클라이언트에
//!     다시 내보낸다 — 긴 대화일수록 많은 줄이 재방송되고 그 줄들이 클라이언트 스크롤백(데몬 줄 버퍼·GUI xterm)에
//!     **중복으로 쌓인다**(ConPTY 의 알려진 동작). 높이만 바꾸면 재줄바꿈이 없다 — 되돌릴 때 가려졌던 **한 줄**이
//!     다시 드러나 재방송될 뿐이다(그 한 줄도 A2 반향 제외 창이 룰·색인에서 가린다).
//!   · 줄였다가 되돌린다(rows-1 → rows), 늘렸다가 되돌리지 않는다(rows+1 → rows): 늘리면 그 순간 PTY 가 GUI 의 실제
//!     화면보다 한 줄 크다 — TUI 가 그 여분 행에 그리면 xterm 은 범위 밖 커서 이동을 맨 아래 행으로 접고 줄바꿈이
//!     화면을 밀어 올린다(GUI 스크롤백에 찌꺼기 줄). 줄이면 모든 출력이 언제나 실제 화면 안에 머문다. 되돌릴 때
//!     conhost 가 한 줄을 다시 드러내는 것은 두 방향이 같으므로 줄이기 쪽이 손실이 더 적다.
//!   · 최소 크기: rows ≥ 3(줄여도 2행 — Ink 의 '터미널이 너무 작다' 경로를 밟지 않는다) · cols ≥ 2.
//!
//! 【치명위험 렌즈】
//!   ① 폭주 없음 — 좌석당 동시 1건(`repaint_in_flight` · Drop 가드가 패닉에도 내린다) · 좌석당 최소 간격 300초 ·
//!      복구되지 않은 연속 요청은 간격을 두 배씩(최대 16배 = 80분) 늘린다. **판독 가능한** 화면(준비 판정 또는 커서
//!      행 관측)을 한 번이라도 보면 간격이 원래대로 돌아간다 — 다른 사유의 막힘(대체 화면·선택기 행 등)은 시계만
//!      지우고 배수는 유지한다(A4). 다시 그려진 화면은 옛 줄의 재방송이라 흔들기 시작부터 되돌린 뒤 3초까지 건강
//!      룰·회상 색인을 타지 않는다(A2 · `Surface::repaint_echo_until`).
//!   ② clear 게이트 무관 — stdin 에 아무것도 쓰지 않는다(입력줄 계수·초안·clear 가드 경로를 건드리지 않는다).
//!   ③ 데몬 생존 — 크기 변경은 틱·reader 스레드 밖 전용 스레드에서 하고, 오류는 기록하고 건너뛴다(패닉·unwrap 0).
//!      크기 변경은 좌석 `resize_serial` 락 아래에서 두 단계(master → parser)를 한 번에 한다 — GUI `surface.resize`
//!      ([`apply_resize`])와 같은 락이라 '줄이기'·'대조+되돌리기' 가 바깥 변경과 엇갈리지 않는다(A1). 잠든 사이(settle)
//!      에는 락을 놓고, 그 사이 바깥 변경(세대 증가)이 있었으면 되돌리지 않는다. 파서 `set_size` 는 `catch_unwind`
//!      로 감싸 파서 락이 독(poison)에 들지 않게 한다(A6).
//!   ④ 사람 화면 — 마커를 아는 에이전트 좌석(Claude Code 등 — 크기 변경을 견디는 TUI)에서, 대체 화면(전체화면 앱)이
//!      아닐 때만 부른다(호출부 조건). ★정직 고지(A7): 대체 화면 판정은 **파서 사본**에서 나온다 — 파서 패닉 격리가
//!      사본을 빈 파서로 갈았으면 그 사본은 대체 화면이 아니라고 읽히므로, 패닉 직후에는 전체화면 앱도 흔들릴 수 있다.
//!      남는 방어는 호출부의 마커 좌석 조건 하나다(맨 셸·마커 미선언 어댑터는 여기 오지 않는다). 그 좌석의 TUI 는
//!      크기 변경이 정상 입력(사람이 창을 끄는 것과 같다)이라 손실은 한 번의 다시 그리기다.
//!   끄기: `CYS_SCREEN_REPAINT_NUDGE=0`.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;

use crate::state::{now_epoch, Daemon, Surface};

/// 판독 불가가 이만큼 이어지면 요청한다(초) — 파서 패닉이 없을 때.
pub(crate) const REPAINT_UNREADABLE_SECS: u64 = 60;
/// 마지막 요청 뒤 새 파서 패닉이 있었으면 이만큼만 기다린다(초) — 패닉 직후의 빈 사본은 스스로 채워질 수도 있다.
pub(crate) const REPAINT_AFTER_PANIC_SECS: u64 = 10;
/// 좌석당 최소 간격(초).
pub(crate) const REPAINT_MIN_INTERVAL_SECS: u64 = 300;
/// 복구되지 않은 연속 요청의 간격 배수 상한(2^4 = 16배).
const REPAINT_BACKOFF_MAX_SHIFT: u32 = 4;
/// 줄였다가 되돌리기까지의 간격(ms) — TUI 가 첫 크기 변경을 읽을 시간.
pub(crate) const REPAINT_SETTLE_MS: u64 = 500;
/// (A2) 되돌린 뒤 반향 제외 창이 이어지는 시간(ms) — 다시 그려진 화면이 reader 를 지나 줄 버퍼에 닿을 시간.
pub(crate) const REPAINT_ECHO_AFTER_MS: u64 = 3_000;
/// (A2) 흔들기 시작 때 거는 반향 제외 창의 상한(초) — Drop 가드가 끝에서 [`REPAINT_ECHO_AFTER_MS`] 로 줄인다.
/// 스레드가 어떤 이유로 오래 끌려도 이 시간 뒤에는 룰·색인이 되살아난다(실패 방향 = 룰 복귀).
pub(crate) const REPAINT_ECHO_CAP_SECS: u64 = 60;
/// (A5) 판독 불가 관측 사이의 틈이 이보다 크면 시계를 다시 세운다(초) — 틱 간격(5초)의 3배. 관측이 끊겼다 돌아온
/// 좌석(잠든 랩톱 · 틱 지연)이 옛 시계로 곧장 요청하지 않게 한다(실패 방향 = 요청 지연).
pub(crate) const REPAINT_OBS_STALE_SECS: u64 = 3 * crate::governance::WATCHDOG_INTERVAL_SECS;

/// 좌석별 재동기 상태(leaf 락 · 다른 락을 쥔 채 잡지 않는다).
#[derive(Debug, Default)]
pub(crate) struct RepaintState {
    /// 이 좌석이 판독 불가(막힘 ∧ 커서 행 마커 없음)로 처음 관측된 시각 — 판독 가능·다른 사유를 보면 지운다.
    pub(crate) unreadable_since: Option<Instant>,
    /// (A5) 마지막 관측 시각(어떤 관측이든) — 판독 불가 관측 사이의 틈이 [`REPAINT_OBS_STALE_SECS`] 를 넘으면 시계를 다시 세운다.
    pub(crate) last_observed: Option<Instant>,
    /// 마지막 요청 시각(단조 시계 · 간격 판정용).
    pub(crate) last_request: Option<Instant>,
    /// 마지막 요청 시각(epoch 초 · 진단 표기용).
    pub(crate) last_request_epoch: Option<f64>,
    /// 판독 가능한 화면을 다시 보기 전까지 낸 연속 요청 수(간격 배수의 지수).
    pub(crate) unrecovered: u32,
    /// 마지막 요청 때의 좌석 파서 패닉 누계 — 이보다 크면 '새 패닉' 이다.
    pub(crate) panics_seen: u64,
}

/// 요청 사유 — 이벤트 `screen.repaint_requested` 의 `reason`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RepaintReason {
    /// 마지막 요청 뒤 화면 파서 패닉이 있었고 판독 불가가 [`REPAINT_AFTER_PANIC_SECS`] 이상.
    ParserPanic,
    /// 판독 불가가 [`REPAINT_UNREADABLE_SECS`] 이상.
    ScreenUnreadable,
}

impl RepaintReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            RepaintReason::ParserPanic => "parser_panic",
            RepaintReason::ScreenUnreadable => "screen_unreadable",
        }
    }
}

/// 큐 틱이 마커 좌석에서 본 화면의 분류 — [`note_queue_screen`] 의 입력.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScreenObs {
    /// 막힘 사유가 `prompt_unknown`·`input_pending` 이고 커서 행에 마커가 없다(화면 판독 불가) — 시계가 간다.
    Unreadable,
    /// 판독 가능 — 준비 판정(`Ready`) 또는 커서 행을 읽었다(`obs.line.is_some()`). 시계와 미복구 배수를 지운다(A4).
    Readable,
    /// 그 밖(대체 화면 · 선택기 행 · 발행 중 프레임 · 다른 사유의 막힘) — 시계만 지우고 미복구 배수는 유지한다(A4).
    Other,
}

/// [`repaint_due`] 의 입력(순수 판정 재료).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RepaintFacts {
    /// 노브(`CYS_SCREEN_REPAINT_NUDGE` 가 `0` 이 아니다).
    pub(crate) enabled: bool,
    /// 이 좌석에 진행 중인 요청이 있다.
    pub(crate) in_flight: bool,
    /// 판독 불가가 이어진 초 — `None` 이면 지금 판독 불가가 아니다(또는 해당 없음).
    pub(crate) unreadable_secs: Option<u64>,
    /// 마지막 요청 뒤(요청이 없었으면 기동 뒤) 이 좌석에 파서 패닉이 있었다.
    pub(crate) new_panic: bool,
    /// 마지막 요청에서 지난 초 — `None` 이면 요청한 적 없다.
    pub(crate) since_last_request_secs: Option<u64>,
    /// 복구되지 않은 연속 요청 수.
    pub(crate) unrecovered: u32,
}

/// 좌석당 다음 요청까지의 최소 간격(초) — 복구되지 않은 연속 요청마다 두 배(상한 16배).
pub(crate) fn repaint_interval_secs(unrecovered: u32) -> u64 {
    REPAINT_MIN_INTERVAL_SECS << unrecovered.min(REPAINT_BACKOFF_MAX_SHIFT)
}

/// 다시 그리기를 **지금** 요청할 것인가(순수). 첫 거부가 답이다:
///   끔 · 진행 중 → 없음 / 판독 불가 아님 → 없음 / 간격 미달 → 없음 / 새 패닉 ∧ 10초 → `ParserPanic` / 60초 → `ScreenUnreadable`.
/// 실패 방향: 판정 재료가 모호하면(판독 불가 관측이 끊기면) 요청하지 않는다(= 종전 동작 · 사람 처방).
pub(crate) fn repaint_due(f: &RepaintFacts) -> Option<RepaintReason> {
    if !f.enabled || f.in_flight {
        return None;
    }
    let unreadable = f.unreadable_secs?;
    if let Some(since) = f.since_last_request_secs {
        if since < repaint_interval_secs(f.unrecovered) {
            return None;
        }
    }
    if f.new_panic && unreadable >= REPAINT_AFTER_PANIC_SECS {
        return Some(RepaintReason::ParserPanic);
    }
    if unreadable >= REPAINT_UNREADABLE_SECS {
        return Some(RepaintReason::ScreenUnreadable);
    }
    None
}

/// (A5 · 순수) 판독 불가 시계를 다시 세워야 하는가 — 직전 관측에서 [`REPAINT_OBS_STALE_SECS`] 보다 긴 틈이 있었다.
/// 직전 관측이 없으면(기동 직후 · 시험이 시계를 손으로 둔 경우) 다시 세우지 않는다.
pub(crate) fn clock_is_stale(last_observed: Option<Instant>, now: Instant) -> bool {
    last_observed.is_some_and(|t| now.saturating_duration_since(t).as_secs() > REPAINT_OBS_STALE_SECS)
}

/// 노브 — `CYS_SCREEN_REPAINT_NUDGE=0` 이면 끈다(기본 켬). 검체는 H 노브 덮개로 주입한다.
pub(crate) fn repaint_nudge_enabled() -> bool {
    crate::governance::h_knob("CYS_SCREEN_REPAINT_NUDGE").is_none_or(|v| v.trim() != "0")
}

/// 큐 틱이 마커 좌석을 관측할 때마다 부른다(분류는 [`ScreenObs`] — 호출부가 가린다). 판독 불가가 아니면 시계를 지우고,
/// 판독 가능이면 미복구 배수도 지운다. 요청이 도래하면 전용 스레드로 크기 흔들기를 띄우고 이벤트를 낸다. 반환 = 이번에 요청했는가.
/// 락: 좌석 `repaint`(leaf)만 잠깐 쥔다 — 이벤트 발행·스레드 생성은 그 락을 놓은 뒤다.
pub(crate) fn note_queue_screen(daemon: &Arc<Daemon>, s: &Arc<Surface>, obs: ScreenObs) -> bool {
    let now = Instant::now();
    let panics = s.parser_panics.load(Ordering::Relaxed);
    let (reason, unreadable_secs, attempt) = {
        let mut st = s.repaint.lock().unwrap_or_else(|e| e.into_inner());
        match obs {
            ScreenObs::Readable => {
                st.unreadable_since = None;
                st.unrecovered = 0;
                st.last_observed = Some(now);
                return false;
            }
            ScreenObs::Other => {
                st.unreadable_since = None;
                st.last_observed = Some(now);
                return false;
            }
            ScreenObs::Unreadable => {}
        }
        // (A5) 관측이 끊겼다 돌아왔으면 옛 시계를 버린다 — 틈 동안의 화면은 본 적이 없다.
        if st.unreadable_since.is_some() && clock_is_stale(st.last_observed, now) {
            st.unreadable_since = Some(now);
        }
        st.last_observed = Some(now);
        let since = *st.unreadable_since.get_or_insert(now);
        let unreadable_secs = now.saturating_duration_since(since).as_secs();
        let facts = RepaintFacts {
            enabled: repaint_nudge_enabled(),
            in_flight: s.repaint_in_flight.load(Ordering::Acquire),
            unreadable_secs: Some(unreadable_secs),
            new_panic: panics > st.panics_seen,
            since_last_request_secs: st.last_request.map(|t| now.saturating_duration_since(t).as_secs()),
            unrecovered: st.unrecovered,
        };
        let Some(reason) = repaint_due(&facts) else {
            return false;
        };
        // 진행 중 표식은 상태 락 안에서 세운다(같은 좌석 이중 요청 차단 · 아래 스레드의 Drop 가드가 끝에서 내린다).
        if s.repaint_in_flight.swap(true, Ordering::AcqRel) {
            return false;
        }
        st.last_request = Some(now);
        st.last_request_epoch = Some(now_epoch());
        st.unrecovered = st.unrecovered.saturating_add(1);
        st.panics_seen = panics;
        (reason, unreadable_secs, st.unrecovered)
    };
    let (rows, cols) = s.parser.lock().unwrap_or_else(|e| e.into_inner()).screen().size();
    daemon.bus.publish(
        "screen.repaint_requested",
        "surface",
        Some(s.id),
        json!({
            "surface_id": s.id,
            "surface_ref": cys::surface_ref(s.id),
            "reason": reason.as_str(),
            "unreadable_secs": unreadable_secs,
            "parser_panics": panics,
            "attempt": attempt,
            "rows": rows,
            "cols": cols,
            "next_min_interval_secs": repaint_interval_secs(attempt),
            "note": "화면 사본이 어긋나 PTY 높이를 한 줄 줄였다 되돌려 다시 그리기를 요청한다(키 입력 없음)",
        }),
    );
    // (A2) 반향 제외 창은 스레드를 띄우기 **전에** 연다(상한 60초 · 스레드가 늦게 돌아도 틈이 없다). 끝은 Drop 가드가 마감한다.
    open_echo_window(s);
    let seat = Arc::clone(s);
    let spawned = std::thread::Builder::new()
        .name(format!("cysd-repaint-{}", s.id))
        .spawn(move || {
            // (A6) Drop 가드 — 정상 종료·오류·패닉 어느 경로로 끝나도 진행 중 표식을 내리고 반향 창을 3초로 마감한다.
            let _guard = InFlightGuard(&seat);
            if let Err(e) = nudge_resize(&seat, Duration::from_millis(REPAINT_SETTLE_MS)) {
                eprintln!("[cysd] surface {} 다시 그리기 요청(크기 흔들기) 건너뜀: {e}", seat.id);
            }
        });
    if let Err(e) = spawned {
        *s.repaint_echo_until.lock().unwrap_or_else(|e| e.into_inner()) = None;
        s.repaint_in_flight.store(false, Ordering::Release);
        eprintln!("[cysd] surface {} 다시 그리기 스레드 생성 실패 — 건너뜀: {e}", s.id);
    }
    true
}

/// (A2) 반향 제외 창을 상한([`REPAINT_ECHO_CAP_SECS`])으로 연다 — 흔들기가 어떤 이유로 오래 끌려도 이 시간 뒤에는 룰·색인이 되살아난다.
pub(crate) fn open_echo_window(s: &Surface) {
    *s.repaint_echo_until.lock().unwrap_or_else(|e| e.into_inner()) =
        Some(Instant::now() + Duration::from_secs(REPAINT_ECHO_CAP_SECS));
}

/// (A6) 흔들기 스레드의 수명 가드 — 떨어질 때(정상·오류·**패닉 unwinding 포함**) 진행 중 표식을 내리고 반향 창을
/// '지금 + [`REPAINT_ECHO_AFTER_MS`]' 로 마감한다.
pub(crate) struct InFlightGuard<'a>(pub(crate) &'a Surface);

impl Drop for InFlightGuard<'_> {
    fn drop(&mut self) {
        *self.0.repaint_echo_until.lock().unwrap_or_else(|e| e.into_inner()) =
            Some(Instant::now() + Duration::from_millis(REPAINT_ECHO_AFTER_MS));
        self.0.repaint_in_flight.store(false, Ordering::Release);
    }
}

/// 크기 흔들기의 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NudgeOutcome {
    /// 줄였다가 되돌렸다.
    Restored,
    /// 줄인 사이에 다른 누가(GUI `surface.resize`) 크기를 바꿨다 — 그 크기를 존중하고 되돌리지 않았다.
    SupersededByOtherResize,
}

/// (A1) 바깥(GUI `surface.resize`)의 크기 변경 — `resize_serial` 락 아래에서 세대를 올리고 PTY · 파서를 한 번에 맞춘다.
/// 핸들러는 이 함수만 부른다(두 단계를 따로 하면 재동기 스레드와 엇갈린다). 실패는 `Err`(PTY 는 바뀌지 않았다 · 파서
/// 실패는 PTY 만 바뀐 상태 — 그 사유를 돌려준다).
pub(crate) fn apply_resize(s: &Surface, rows: u16, cols: u16) -> Result<(), String> {
    let mut serial = s.resize_serial.lock().unwrap_or_else(|e| e.into_inner());
    *serial = serial.wrapping_add(1);
    resize_pty(s, rows, cols)?;
    set_parser_size(s, rows, cols)
}

/// PTY 를 (rows-1, cols) 로 줄였다가 `settle` 뒤 (rows, cols) 로 되돌린다 — 파서 크기도 같이 맞춘다(높이만 · 사유는 모듈 doc A3).
/// 두 단계(줄이기 · 대조+되돌리기)는 각각 `resize_serial` 락 아래에서 하고 잠든 사이에는 놓는다. 그 사이 바깥 변경(세대
/// 증가)이 있었으면 되돌리지 않는다. 오류는 `Err` 로 돌려준다(패닉 없음).
pub(crate) fn nudge_resize(s: &Surface, settle: Duration) -> Result<NudgeOutcome, String> {
    if s.exited.load(Ordering::Relaxed) {
        return Err("좌석이 종료됐다".into());
    }
    let (rows, cols, serial_seen) = {
        let serial = s.resize_serial.lock().unwrap_or_else(|e| e.into_inner());
        let (rows, cols) = s.parser.lock().unwrap_or_else(|e| e.into_inner()).screen().size();
        if rows < 3 || cols < 2 {
            return Err(format!("크기가 너무 작다({rows}x{cols})"));
        }
        let short = rows - 1;
        resize_pty(s, short, cols)?;
        if let Err(e) = set_parser_size(s, short, cols) {
            // 파서만 못 줄였다 — PTY 를 곧장 되돌리고 포기한다(두 크기가 어긋난 채 두지 않는다).
            let _ = resize_pty(s, rows, cols);
            return Err(e);
        }
        (rows, cols, *serial)
    };
    std::thread::sleep(settle);
    let serial = s.resize_serial.lock().unwrap_or_else(|e| e.into_inner());
    if *serial != serial_seen {
        return Ok(NudgeOutcome::SupersededByOtherResize);
    }
    if s.exited.load(Ordering::Relaxed) {
        return Err("되돌리기 전에 좌석이 종료됐다".into());
    }
    resize_pty(s, rows, cols)?;
    set_parser_size(s, rows, cols)?;
    Ok(NudgeOutcome::Restored)
}

fn resize_pty(s: &Surface, rows: u16, cols: u16) -> Result<(), String> {
    s.master
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .resize(portable_pty::PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
        .map_err(|e| format!("PTY 크기 변경 실패({rows}x{cols}): {e}"))
}

/// (A6) 파서 크기 변경 — `catch_unwind` 로 감싼다. 파서 락을 쥔 채 안에서 패닉이 나면 가드가 unwinding 중에 떨어져 락이
/// 독에 들고, 그 뒤 모든 `parser.lock()` 이 `into_inner` 로 독을 삼키며 반쯤 바뀐 파서를 쓰게 된다. 패닉은 `Err` 가 된다.
fn set_parser_size(s: &Surface, rows: u16, cols: u16) -> Result<(), String> {
    let mut p = s.parser.lock().unwrap_or_else(|e| e.into_inner());
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| p.set_size(rows, cols)))
        .map_err(|_| format!("파서 크기 변경 중 패닉({rows}x{cols})"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> RepaintFacts {
        RepaintFacts {
            enabled: true,
            in_flight: false,
            unreadable_secs: Some(REPAINT_UNREADABLE_SECS),
            new_panic: false,
            since_last_request_secs: None,
            unrecovered: 0,
        }
    }

    /// 판정 표 전수 — 첫 거부가 답이다.
    #[test]
    fn repaint_due_table() {
        use RepaintReason::*;
        let cases: Vec<(&str, RepaintFacts, Option<RepaintReason>)> = vec![
            ("기본: 60초 판독 불가", facts(), Some(ScreenUnreadable)),
            ("59초는 아직", RepaintFacts { unreadable_secs: Some(59), ..facts() }, None),
            ("0초", RepaintFacts { unreadable_secs: Some(0), ..facts() }, None),
            ("판독 불가 아님", RepaintFacts { unreadable_secs: None, ..facts() }, None),
            ("판독 불가 아님 + 새 패닉", RepaintFacts { unreadable_secs: None, new_panic: true, ..facts() }, None),
            ("노브 끔", RepaintFacts { enabled: false, ..facts() }, None),
            ("노브 끔 + 새 패닉", RepaintFacts { enabled: false, new_panic: true, ..facts() }, None),
            ("진행 중", RepaintFacts { in_flight: true, ..facts() }, None),
            ("새 패닉 10초", RepaintFacts { new_panic: true, unreadable_secs: Some(10), ..facts() }, Some(ParserPanic)),
            ("새 패닉 9초", RepaintFacts { new_panic: true, unreadable_secs: Some(9), ..facts() }, None),
            ("새 패닉 60초 = 패닉 사유", RepaintFacts { new_panic: true, ..facts() }, Some(ParserPanic)),
            ("간격 미달(299초)", RepaintFacts { since_last_request_secs: Some(299), ..facts() }, None),
            ("간격 미달은 새 패닉도 막는다", RepaintFacts { since_last_request_secs: Some(10), new_panic: true, ..facts() }, None),
            ("간격 도달(300초)", RepaintFacts { since_last_request_secs: Some(300), ..facts() }, Some(ScreenUnreadable)),
            ("미복구 1회 → 600초 필요", RepaintFacts { since_last_request_secs: Some(599), unrecovered: 1, ..facts() }, None),
            ("미복구 1회 · 600초", RepaintFacts { since_last_request_secs: Some(600), unrecovered: 1, ..facts() }, Some(ScreenUnreadable)),
            ("미복구 상한(9회 → 16배)", RepaintFacts { since_last_request_secs: Some(4799), unrecovered: 9, ..facts() }, None),
            ("미복구 상한 도달", RepaintFacts { since_last_request_secs: Some(4800), unrecovered: 9, ..facts() }, Some(ScreenUnreadable)),
            ("u32 최대 미복구도 상한 16배", RepaintFacts { since_last_request_secs: Some(4800), unrecovered: u32::MAX, ..facts() }, Some(ScreenUnreadable)),
        ];
        for (name, f, want) in cases {
            assert_eq!(repaint_due(&f), want, "{name}: {f:?}");
        }
    }

    #[test]
    fn interval_doubles_and_caps() {
        assert_eq!(repaint_interval_secs(0), 300);
        assert_eq!(repaint_interval_secs(1), 600);
        assert_eq!(repaint_interval_secs(4), 4800);
        assert_eq!(repaint_interval_secs(5), 4800);
        assert_eq!(repaint_interval_secs(u32::MAX), 4800);
    }

    /// (A5) 시계 신선도 — 직전 관측이 없으면 신선 · 틈이 15초(틱 5초 × 3)를 넘어야 오래됨.
    #[test]
    fn clock_stale_only_after_three_ticks_gap() {
        assert_eq!(REPAINT_OBS_STALE_SECS, 15);
        let now = Instant::now();
        assert!(!clock_is_stale(None, now), "직전 관측 없음 = 다시 세우지 않는다");
        assert!(!clock_is_stale(Some(now - Duration::from_secs(15)), now), "경계(15초)는 아직");
        assert!(clock_is_stale(Some(now - Duration::from_secs(16)), now));
        assert!(clock_is_stale(Some(now - Duration::from_secs(3600)), now));
    }

    /// 폭주 없음(①) — 같은 좌석을 매 틱(2초) 판독 불가로 1시간 관측해도 요청은 유계다. 순수 판정과 상태 갱신 규칙을
    /// 같은 순서로 흉내 낸다(note_queue_screen 의 상태 전이 = 이 루프). 첫 요청 60초 · 이후 600·1200·2400·4800… 간격.
    #[test]
    fn hour_of_unreadable_ticks_requests_are_bounded() {
        let (mut last_request, mut unrecovered): (Option<u64>, u32) = (None, 0);
        let unreadable_since = 0u64;
        let mut requests = Vec::new();
        for t in (0..3600u64).step_by(2) {
            let f = RepaintFacts {
                enabled: true,
                in_flight: false,
                unreadable_secs: Some(t - unreadable_since),
                new_panic: false,
                since_last_request_secs: last_request.map(|r| t - r),
                unrecovered,
            };
            if repaint_due(&f).is_some() {
                requests.push(t);
                last_request = Some(t);
                unrecovered += 1;
            }
        }
        assert_eq!(requests, vec![60, 660, 1860], "1시간 동안 3회(60 → +600 → +1200)");
    }

    #[cfg(unix)]
    fn seat(tag: &str) -> (std::path::PathBuf, Arc<Daemon>, Arc<Surface>) {
        let dir = std::env::temp_dir().join(format!("cys-repaint-{tag}-{}-{}", std::process::id(), now_epoch() as u64));
        let _ = std::fs::create_dir_all(&dir);
        let daemon = Daemon::new(dir.join("cysd.sock"));
        let s = daemon.create_surface(None, Some("sleep 30".into()), None, None, 24, 80).expect("surface");
        (dir, daemon, s)
    }

    #[cfg(unix)]
    fn pty_size(s: &Surface) -> (u16, u16) {
        let p = s.master.lock().unwrap().get_size().expect("pty size");
        (p.rows, p.cols)
    }

    /// 진행 중 표식·간격 — 실제 좌석으로 `note_queue_screen` 을 두 번 연달아 불러도 요청은 한 번이다. 판독 가능 관측은 시계를 지운다.
    /// 되돌린 뒤 반향 제외 창은 '지금 + 3초' 안에 닫힌다(A2).
    #[cfg(unix)]
    #[test]
    fn note_queue_screen_requests_once_and_restores_size() {
        let (dir, daemon, s) = seat("once");
        // 판독 가능 → 요청 없음 · 시계 없음.
        assert!(!note_queue_screen(&daemon, &s, ScreenObs::Readable));
        assert!(s.repaint.lock().unwrap().unreadable_since.is_none());
        // 첫 판독 불가 관측 = 시계만 선다.
        assert!(!note_queue_screen(&daemon, &s, ScreenObs::Unreadable));
        // 61초 전부터 판독 불가였던 것으로 되돌린다.
        s.repaint.lock().unwrap().unreadable_since = Some(Instant::now() - Duration::from_secs(61));
        let mut rx = daemon.bus.subscribe();
        assert!(note_queue_screen(&daemon, &s, ScreenObs::Unreadable), "60초 판독 불가 = 요청");
        assert!(!note_queue_screen(&daemon, &s, ScreenObs::Unreadable), "곧바로 다시 불러도(진행 중·간격) 요청하지 않는다");
        // 흔들기 시작과 함께 반향 제외 창이 열려 있다(상한 60초).
        assert!(Daemon::repaint_echo_active(&s), "흔들기 중에는 반향 제외 창이 열려 있어야 한다");
        // 이벤트 1건.
        let mut seen = 0;
        while let Ok(ev) = rx.try_recv() {
            if ev["name"] == "screen.repaint_requested" {
                seen += 1;
                assert_eq!(ev["payload"]["reason"], "screen_unreadable");
                assert_eq!(ev["payload"]["attempt"], 1);
            }
        }
        assert_eq!(seen, 1, "이벤트는 한 번");
        // 스레드가 끝나면 진행 중 표식이 내려가고 크기는 원래대로다.
        let dl = Instant::now() + Duration::from_secs(3);
        while s.repaint_in_flight.load(Ordering::Acquire) && Instant::now() < dl {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!s.repaint_in_flight.load(Ordering::Acquire), "진행 중 표식이 내려가야 한다");
        assert_eq!(s.parser.lock().unwrap().screen().size(), (24, 80), "파서 크기 원복");
        assert_eq!(pty_size(&s), (24, 80), "PTY 크기 원복");
        // (A2) 되돌린 뒤 반향 창은 '지금 + 3초' 이내로 마감됐다.
        let until = s.repaint_echo_until.lock().unwrap().expect("반향 창");
        assert!(until > Instant::now() && until <= Instant::now() + Duration::from_millis(REPAINT_ECHO_AFTER_MS));
        assert!(Daemon::repaint_echo_active(&s));
        // 바깥 크기 변경 세대는 흔들기로 오르지 않는다(자기 변경을 바깥 변경으로 세지 않는다).
        assert_eq!(*s.resize_serial.lock().unwrap(), 0);
        // 판독 가능 관측 → 시계·미복구 수 리셋.
        assert!(!note_queue_screen(&daemon, &s, ScreenObs::Readable));
        let st = s.repaint.lock().unwrap();
        assert!(st.unreadable_since.is_none() && st.unrecovered == 0);
        drop(st);
        let _ = s.child.lock().unwrap().kill();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// (A4) 미복구 배수는 **판독 가능** 관측에만 리셋된다 — 다른 사유(대체 화면 등)는 시계만 지운다.
    /// (A5) 관측이 15초 넘게 끊겼다 돌아오면 시계를 다시 세운다 — 옛 시계로 곧장 요청하지 않는다.
    #[cfg(unix)]
    #[test]
    fn backoff_resets_only_on_readable_and_stale_clock_restarts() {
        let (dir, daemon, s) = seat("a4a5");
        {
            let mut st = s.repaint.lock().unwrap();
            st.unrecovered = 2;
            st.unreadable_since = Some(Instant::now() - Duration::from_secs(61));
            st.last_request = Some(Instant::now() - Duration::from_secs(10));
        }
        assert!(!note_queue_screen(&daemon, &s, ScreenObs::Other));
        {
            let st = s.repaint.lock().unwrap();
            assert!(st.unreadable_since.is_none(), "다른 사유 = 시계는 지운다");
            assert_eq!(st.unrecovered, 2, "다른 사유 = 배수는 유지한다(A4)");
        }
        assert!(!note_queue_screen(&daemon, &s, ScreenObs::Readable));
        assert_eq!(s.repaint.lock().unwrap().unrecovered, 0, "판독 가능 = 배수 리셋");
        // (A5) 61초 전부터 판독 불가 + 직전 관측이 20초 전 → 시계를 다시 세우고 요청하지 않는다.
        {
            let mut st = s.repaint.lock().unwrap();
            st.last_request = None;
            st.unreadable_since = Some(Instant::now() - Duration::from_secs(61));
            st.last_observed = Some(Instant::now() - Duration::from_secs(20));
        }
        assert!(!note_queue_screen(&daemon, &s, ScreenObs::Unreadable), "오래된 시계로는 요청하지 않는다");
        let since = s.repaint.lock().unwrap().unreadable_since.expect("시계");
        assert!(since.elapsed() < Duration::from_secs(5), "시계가 다시 섰다");
        assert!(!s.repaint_in_flight.load(Ordering::Acquire));
        // 직전 관측이 14초 전(틈 ≤ 15초)이면 시계를 유지한다 → 요청.
        {
            let mut st = s.repaint.lock().unwrap();
            st.unreadable_since = Some(Instant::now() - Duration::from_secs(61));
            st.last_observed = Some(Instant::now() - Duration::from_secs(14));
        }
        assert!(note_queue_screen(&daemon, &s, ScreenObs::Unreadable), "틈이 작으면 시계 유지 = 요청");
        let dl = Instant::now() + Duration::from_secs(3);
        while s.repaint_in_flight.load(Ordering::Acquire) && Instant::now() < dl {
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = s.child.lock().unwrap().kill();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// (A1) 줄인 사이에 바깥 크기 변경(`apply_resize` = GUI `surface.resize`)이 들어오면 되돌리지 않는다 — PTY · 파서 모두 GUI 크기다.
    /// 흔들기는 높이만 한 줄 줄인다(A3). 너무 작은 화면은 건드리지 않는다.
    #[cfg(unix)]
    #[test]
    fn nudge_respects_concurrent_resize_and_rejects_tiny() {
        let (dir, _daemon, s) = seat("race");
        let s2 = Arc::clone(&s);
        let h = std::thread::spawn(move || nudge_resize(&s2, Duration::from_millis(400)));
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(s.parser.lock().unwrap().screen().size(), (23, 80), "높이만 한 줄 줄인다");
        assert_eq!(pty_size(&s), (23, 80));
        apply_resize(&s, 30, 100).unwrap();
        assert_eq!(h.join().unwrap(), Ok(NudgeOutcome::SupersededByOtherResize));
        assert_eq!(s.parser.lock().unwrap().screen().size(), (30, 100), "GUI 크기 유지(파서)");
        assert_eq!(pty_size(&s), (30, 100), "GUI 크기 유지(PTY)");
        assert_eq!(*s.resize_serial.lock().unwrap(), 1, "바깥 변경 1회 = 세대 1");
        // 바깥 변경이 없으면 되돌린다.
        assert_eq!(nudge_resize(&s, Duration::from_millis(1)), Ok(NudgeOutcome::Restored));
        assert_eq!(s.parser.lock().unwrap().screen().size(), (30, 100));
        assert_eq!(pty_size(&s), (30, 100));
        // 너무 작은 화면은 건드리지 않는다(높이 2행 · 너비 1열).
        s.parser.lock().unwrap().set_size(2, 80);
        assert!(nudge_resize(&s, Duration::from_millis(1)).is_err());
        s.parser.lock().unwrap().set_size(24, 1);
        assert!(nudge_resize(&s, Duration::from_millis(1)).is_err());
        let _ = s.child.lock().unwrap().kill();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// (A1) 바깥 변경이 흔들기의 '줄이기' 와 동시에 와도 직렬화된다 — 어느 순서든 끝 크기는 GUI 크기이거나(추월) 원 크기(복원)이고
    /// PTY 와 파서가 **같다**(어긋난 조합 없음). 20회 반복.
    #[cfg(unix)]
    #[test]
    fn concurrent_resize_never_leaves_pty_and_parser_diverged() {
        let (dir, _daemon, s) = seat("serial");
        for i in 0..20u16 {
            let gui = (30 + i % 3, 100 + i % 5);
            let s2 = Arc::clone(&s);
            let h = std::thread::spawn(move || nudge_resize(&s2, Duration::from_millis(20)));
            if i % 2 == 0 {
                std::thread::yield_now();
            } else {
                std::thread::sleep(Duration::from_millis(10));
            }
            apply_resize(&s, gui.0, gui.1).unwrap();
            let out = h.join().unwrap().expect("nudge");
            let parser = s.parser.lock().unwrap().screen().size();
            assert_eq!(parser, pty_size(&s), "{i}: PTY 와 파서가 어긋났다({out:?})");
            assert_eq!(parser, gui, "{i}: 끝 크기는 GUI 크기({out:?})");
        }
        let _ = s.child.lock().unwrap().kill();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// (A6) 흔들기 스레드가 패닉해도 진행 중 표식은 내려가고 반향 창은 3초로 마감된다(Drop 가드).
    #[cfg(unix)]
    #[test]
    fn in_flight_guard_resets_even_on_panic() {
        let (dir, _daemon, s) = seat("guard");
        s.repaint_in_flight.store(true, Ordering::Release);
        let s2 = Arc::clone(&s);
        open_echo_window(&s);
        let h = std::thread::spawn(move || {
            let _g = InFlightGuard(&s2);
            assert!(Daemon::repaint_echo_active(&s2));
            panic!("흉내 낸 패닉");
        });
        assert!(h.join().is_err(), "스레드는 패닉으로 끝난다");
        assert!(!s.repaint_in_flight.load(Ordering::Acquire), "가드가 표식을 내렸다");
        let until = s.repaint_echo_until.lock().unwrap().expect("반향 창");
        assert!(until <= Instant::now() + Duration::from_millis(REPAINT_ECHO_AFTER_MS));
        // 파서 락은 독에 들지 않았다(set_parser_size 는 catch_unwind — 정상 호출도 Ok).
        assert!(set_parser_size(&s, 24, 80).is_ok());
        assert!(s.parser.lock().is_ok());
        let _ = s.child.lock().unwrap().kill();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
