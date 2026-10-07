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
//!     **중복으로 쌓인다**(ConPTY 의 알려진 동작). 높이만 바꾸면 재줄바꿈이 없다 — 되돌릴 때 가려졌던 줄(기대값은 한
//!     줄)이 다시 드러나 재방송된다(그 줄들도 A2 반향 제외 창이 룰·색인에서 가린다). ★정직 고지: 이 ConPTY 서술은
//!     공개 동작 기록에 근거한 **추론**이고 이 저장소의 PTY 검체는 유닉스 전용이다 — 윈도우 실기(windows-health)에서
//!     재방송 줄 수와 3행 화면(커서가 맨 아랫줄)의 동작은 아직 재지 않았다. 재기 전까지는 '보장' 이 아니라 '선택 근거' 다.
//!   · 줄였다가 되돌린다(rows-1 → rows), 늘렸다가 되돌리지 않는다(rows+1 → rows): 늘리면 그 순간 PTY 가 GUI 의 실제
//!     화면보다 한 줄 크다 — TUI 가 그 여분 행에 그리면 xterm 은 범위 밖 커서 이동을 맨 아래 행으로 접고 줄바꿈이
//!     화면을 밀어 올린다(GUI 스크롤백에 찌꺼기 줄). 줄이면 모든 출력이 언제나 실제 화면 안에 머문다. 되돌릴 때
//!     conhost 가 한 줄을 다시 드러내는 것은 두 방향이 같으므로 줄이기 쪽이 손실이 더 적다.
//!   · 최소 크기: rows ≥ 4(줄여도 3행 · `REPAINT_MIN_ROWS` — Ink 의 '터미널이 너무 작다' 경로와 2행 화면을 보수적으로 피한다) · cols ≥ 2.
//!
//! 【치명위험 렌즈】
//!   ① 폭주 없음 — 좌석당 동시 1건(`repaint_in_flight` · Drop 가드가 패닉에도 내린다) · 좌석당 최소 간격 300초 ·
//!      복구되지 않은 연속 요청은 간격을 두 배씩(최대 16배 = 80분) 늘린다. **판독 가능한** 화면(준비 판정 또는 커서
//!      행 관측)을 한 번이라도 보면 간격이 원래대로 돌아간다 — 다른 사유의 막힘(대체 화면·선택기 행 등)은 시계만
//!      지우고 배수는 유지한다(A4). 다시 그려진 화면은 옛 줄의 재방송이라 흔들기 시작부터 **되돌린 뒤 최소 3초 · 재방송이 이어지면 마지막 출력 + 1초까지**
//!      (상한 60초) 건강 룰·회상 색인을 타지 않는다(A2 · `Surface::repaint_echo` — 최선 노력: 창 안에 시작돼 창 뒤에 끝나는 미완성
//!      줄 하나는 샐 수 있다 · 창은 유한해 영구 봉인은 없다). 창 안에 줄 버퍼로 들어온 줄 번호 구간은 창이 닫힌 뒤에도 남아
//!      `surface.wait_for`·델타 read 가 건너뛴다(성찰 2회차 M1 — 재방송 줄을 `since_line` 뒤의 새 줄로 거짓 일치하지 않는다).
//!      ★정직 고지: 윈도우 classic(주 화면) 좌석에서 ConPTY 가 크기 변경에 얼마나 긴 재방송을 내는지는 실기로 재지 않았다 —
//!      정적 기준 창과 줄 구간 표식은 그 길이에 의존하지 않도록 설계한 것이지 측정 결과가 아니다.
//!   ② clear 게이트 무관 — stdin 에 아무것도 쓰지 않는다(입력줄 계수·초안·clear 가드 경로를 건드리지 않는다).
//!   ③ 데몬 생존 — 크기 변경은 틱·reader 스레드 밖 전용 스레드에서 하고, 오류는 기록하고 건너뛴다(패닉·unwrap 0).
//!      크기 변경은 좌석 `resize_gate` 락 아래에서 두 단계(master → parser)를 한 번에 한다 — GUI `surface.resize`
//!      ([`apply_resize`])와 같은 락이라 '줄이기'·'대조+되돌리기' 가 바깥 변경과 엇갈리지 않는다(A1). 잠든 사이(settle)
//!      에는 락을 놓고, 그 사이 바깥 변경(세대 증가 — PTY 가 실제로 바뀐 뒤에만 오른다)이 있었으면 되돌리지 않는다.
//!      흔들기 중 바깥이 치수를 생략하면 핸들러는 파서의 임시 크기가 아니라 정식 크기(`ResizeGate::nudge_origin`)를 채운다.
//!      파서 `set_size` 는 `catch_unwind` 로 감싸 파서 락이 독(poison)에 들지 않게 하고, 패닉하면 새 파서로 간다(A6).
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
/// (A2 · 성찰 2회차 M1-b) 되돌린 뒤 반향 제외 창이 닫히는 **출력 정적** 하한(ms) — 되돌린 뒤 출력이 이만큼 조용하면 재방송이 끝난 것으로 본다.
/// 종전 고정 3초는 윈도우 classic(주 화면) 좌석의 긴 재방송(ConPTY 가 가려졌던 줄·프레임을 통째로 다시 내보낸다)이 창을 넘길 수 있었다 —
/// 옛 "rate limit"·"Error" 줄이 창 밖에서 룰을 다시 당긴다(폭주 ①). 정적 기준이면 재방송이 길어도 끝난 뒤 1초에 닫히고, 짧으면 더 일찍 닫힌다.
pub(crate) const REPAINT_ECHO_QUIET_MS: u64 = 1_000;
/// (A2 · 2차 검토 MAJOR-1) 되돌린 뒤 창이 **최소한** 열려 있는 시간(ms) — 첫 재방송 청크가 되돌린 뒤 1초 넘게 늦게 와도(TUI 가 크기 변경을 늦게 읽는다) 창 밖으로
/// 떨어지지 않게 하는 바닥. 종전 고정 창과 같은 3초다 — 정적 규칙은 이 바닥 **위에** 더해져 창을 늘릴 뿐 줄이지 않는다.
pub(crate) const REPAINT_ECHO_GRACE_MS: u64 = 3_000;
/// (A2) 흔들기 시작 때 거는 반향 제외 창의 상한(초) — 되돌린 뒤 출력이 끊이지 않아도 이 시간 뒤에는 룰·색인이 되살아난다(실패 방향 = 룰 복귀).
pub(crate) const REPAINT_ECHO_CAP_SECS: u64 = 60;
/// (M1-a) 좌석마다 기억하는 반향 줄 구간의 상한 — 넘으면 가장 오래된 구간부터 잊는다(그 줄들은 보통 스크롤백에서도 이미 밀려났다).
pub(crate) const REPAINT_ECHO_RANGES_MAX: usize = 64;
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

/// ★(A2 · 성찰 2회차 M1) 좌석별 **반향 제외 창** 상태 — `Surface::repaint_echo`(leaf 락 · 다른 락을 쥔 채 잡아도 되지만 이 락을 쥔 채 다른 락은 잡지 않는다).
///
/// 창 = 흔들기 시작(`cap_until` 설정) ~ 되돌린 뒤(`restored_at`) [`REPAINT_ECHO_GRACE_MS`] 가 지나고 **그 뒤 도착한** 출력이 [`REPAINT_ECHO_QUIET_MS`] 이상 조용해진 순간 · 상한 [`REPAINT_ECHO_CAP_SECS`].
/// 창 안에 줄 버퍼로 들어온 줄의 번호 구간(`ranges` · `[start, end)` · 단조 줄 번호 `Surface::line_count` 기준)은 창이 닫힌 **뒤에도** 남아
/// `surface.wait_for`·`surface.read_text since_line` 이 그 줄을 건너뛴다 — 다시 그려진 화면은 **옛 줄의 재방송**이라 `since_line` 뒤의 '새 줄'로
/// 보이지만 새 사실이 아니다(옛 "완료" 표지에 wait_for 가 거짓 일치 · 옛 오류 줄이 델타 소비자에게 새 오류로 보인다).
#[derive(Debug, Default)]
pub(crate) struct RepaintEcho {
    /// 창의 상한 시각 — `None` = 창 없음.
    pub(crate) cap_until: Option<Instant>,
    /// 크기를 되돌린 시각(Drop 가드가 찍는다) — `None` = 아직 흔드는 중(정적 판정 없이 창 유지).
    pub(crate) restored_at: Option<Instant>,
    /// 창 안에서 마지막으로 출력이 도착한 시각(reader 의 청크 단위).
    pub(crate) last_output: Option<Instant>,
    /// 창 안에 들어온 줄 번호 구간(`[start, end)` · 오름차순 · 상한 [`REPAINT_ECHO_RANGES_MAX`]).
    pub(crate) ranges: Vec<(u64, u64)>,
}

impl RepaintEcho {
    /// 순수: `now` 에 창이 열려 있는가. 되돌린 뒤에는 '되돌린 시각 + 바닥 3초' 와 '되돌린 뒤 마지막 출력 + 1초' 가운데 늦은 쪽 전까지만 열려 있고, 어느 경우든 상한을 넘기면 닫힌다.
    pub(crate) fn active_at(&self, now: Instant) -> bool {
        let Some(cap) = self.cap_until else {
            return false;
        };
        if now >= cap {
            return false;
        }
        match self.restored_at {
            None => true,
            Some(restored) => {
                // 바닥(되돌린 뒤 3초) 위에 '되돌린 뒤 도착한 마지막 출력 + 1초' 를 얹는다 — 되돌리기 전 출력은 재방송이 아니라 세지 않는다.
                let mut end = restored + Duration::from_millis(REPAINT_ECHO_GRACE_MS);
                if let Some(o) = self.last_output.filter(|o| *o >= restored) {
                    end = end.max(o + Duration::from_millis(REPAINT_ECHO_QUIET_MS));
                }
                now < end
            }
        }
    }

    /// 창 표식을 지운다(구간은 남긴다 — 닫힌 창의 재방송 줄도 계속 건너뛴다).
    fn clear_window(&mut self) {
        self.cap_until = None;
        self.restored_at = None;
        self.last_output = None;
    }

    /// 순수: 출력이 `now` 에 도착했다 — 창이 열려 있으면 마지막 출력 시각을 찍고 true, 닫혔으면(정적 하한·상한) 표식을 지우고 false.
    pub(crate) fn note_output(&mut self, now: Instant) -> bool {
        if !self.active_at(now) {
            if self.cap_until.is_some() {
                self.clear_window();
            }
            return false;
        }
        self.last_output = Some(now);
        true
    }

    /// 순수: 창 안에 들어온 줄 번호 구간 `[start, end)` 를 기억한다 — 직전 구간과 맞닿으면 잇고, 스크롤백에서 이미 밀려난 구간(`end <= oldest` · 2차 검토 MINOR)을 먼저 잊고,
    /// 그래도 상한을 넘으면 가장 오래된 구간을 잊는다(그 줄은 아직 버퍼에 있을 수 있다 — 최선 노력 · 상한은 메모리 유계를 위한 것).
    pub(crate) fn record_lines(&mut self, start: u64, end: u64, oldest: u64) {
        self.ranges.retain(|&(_, e)| e > oldest);
        if end <= start {
            return;
        }
        match self.ranges.last_mut() {
            Some((_, last_end)) if *last_end == start => *last_end = end,
            Some((_, last_end)) if *last_end > start => *last_end = (*last_end).max(end),
            _ => self.ranges.push((start, end)),
        }
        if self.ranges.len() > REPAINT_ECHO_RANGES_MAX {
            let drop = self.ranges.len() - REPAINT_ECHO_RANGES_MAX;
            self.ranges.drain(..drop);
        }
    }
}

/// 순수: 줄 번호 `line` 이 반향 구간 안인가.
pub(crate) fn line_in_echo(ranges: &[(u64, u64)], line: u64) -> bool {
    ranges.iter().any(|&(s, e)| s <= line && line < e)
}

/// 순수: `surface.wait_for` 의 한 회전 — `start` 번부터 번호가 매겨진 `lines` 에서 패턴에 첫 일치하는 (줄 번호, 줄)을 돌려주되,
/// 반향 구간(`echo` · [`line_in_echo`]) 안의 줄은 **건너뛴다**(재방송된 옛 "완료" 표지에 거짓 일치하지 않는다 — 성찰 2회차 M1-a).
pub(crate) fn wait_for_match(lines: &[String], start: u64, echo: &[(u64, u64)], pattern: &regex::Regex) -> Option<(u64, String)> {
    lines.iter().enumerate().find_map(|(i, line)| {
        let no = start + i as u64;
        (!line_in_echo(echo, no) && pattern.is_match(line)).then(|| (no, line.clone()))
    })
}

/// 좌석의 반향 줄 구간 사본(짧다 — 상한 64) — `surface.wait_for`·델타 read 가 스크롤백 락을 쥔 채 부른다(락 순서: scrollback → repaint_echo · ingest 와 같다).
pub(crate) fn echo_ranges(s: &Surface) -> Vec<(u64, u64)> {
    s.repaint_echo.lock().unwrap_or_else(|e| e.into_inner()).ranges.clone()
}

/// 출력 청크 도착 — reader(`ingest_output`)가 청크마다 한 번 부른다. 창이 열려 있으면 true(이 청크의 완성 줄은 반향이다).
pub(crate) fn echo_note_output(s: &Surface, now: Instant) -> bool {
    s.repaint_echo.lock().unwrap_or_else(|e| e.into_inner()).note_output(now)
}

/// 창 안에 들어온 줄 구간을 기억한다(`ingest_output` 이 스크롤백 락을 쥔 채 부른다 · `oldest` = 스크롤백에 남은 가장 오래된 줄 번호).
pub(crate) fn echo_record_lines(s: &Surface, start: u64, end: u64, oldest: u64) {
    s.repaint_echo.lock().unwrap_or_else(|e| e.into_inner()).record_lines(start, end, oldest);
}

/// 지금 창이 열려 있는가(순수 판정의 지금 판 — 지났으면 표식을 지워 둔다). 건강 룰·회상 색인이 부른다.
pub(crate) fn echo_active(s: &Surface) -> bool {
    let mut e = s.repaint_echo.lock().unwrap_or_else(|e| e.into_inner());
    let active = e.active_at(Instant::now());
    if !active && e.cap_until.is_some() {
        e.clear_window();
    }
    active
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
    last_observed.is_some_and(|t| now.saturating_duration_since(t) > Duration::from_secs(REPAINT_OBS_STALE_SECS))
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
            // (A6) Drop 가드 — 정상 종료·오류·패닉 어느 경로로 끝나도 진행 중 표식을 내리고 반향 창에 되돌린 시각을 찍는다(바닥 3초 · 재방송이 이어지면 연장).
            let _guard = InFlightGuard(&seat);
            if let Err(e) = nudge_resize(&seat, Duration::from_millis(REPAINT_SETTLE_MS)) {
                eprintln!("[cysd] surface {} 다시 그리기 요청(크기 흔들기) 건너뜀: {e}", seat.id);
            }
        });
    if let Err(e) = spawned {
        s.repaint_echo.lock().unwrap_or_else(|e| e.into_inner()).clear_window();
        s.repaint_in_flight.store(false, Ordering::Release);
        eprintln!("[cysd] surface {} 다시 그리기 스레드 생성 실패 — 건너뜀: {e}", s.id);
    }
    true
}

/// (A2) 반향 제외 창을 상한([`REPAINT_ECHO_CAP_SECS`])으로 연다 — 흔들기가 어떤 이유로 오래 끌려도 이 시간 뒤에는 룰·색인이 되살아난다.
/// 되돌린 시각·마지막 출력은 지운다(새 흔들기) · 종전 반향 줄 구간은 남긴다.
pub(crate) fn open_echo_window(s: &Surface) {
    let mut e = s.repaint_echo.lock().unwrap_or_else(|e| e.into_inner());
    e.cap_until = Some(Instant::now() + Duration::from_secs(REPAINT_ECHO_CAP_SECS));
    e.restored_at = None;
    e.last_output = None;
}

/// (A6) 흔들기 스레드의 수명 가드 — 떨어질 때(정상·오류·**패닉 unwinding 포함**) 진행 중 표식을 내리고 반향 창에 '되돌린 시각' 을 찍는다
/// (이후 출력이 [`REPAINT_ECHO_QUIET_MS`] 이상 조용해지면 창이 닫힌다 · 상한은 그대로).
pub(crate) struct InFlightGuard<'a>(pub(crate) &'a Surface);

impl Drop for InFlightGuard<'_> {
    fn drop(&mut self) {
        {
            let mut e = self.0.repaint_echo.lock().unwrap_or_else(|e| e.into_inner());
            if e.cap_until.is_some() {
                e.restored_at = Some(Instant::now());
            }
        }
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

/// (A1) 좌석 크기 변경 관문 — `Surface::resize_gate` 의 내용. `gen` 은 **바깥**(GUI `surface.resize`) 변경의 세대, `nudge_origin` 은
/// 흔들기가 진행 중일 때의 **정식 크기**(줄이기 전 크기 · 되돌릴 목표). 핸들러가 생략된 치수를 채울 때 파서의 임시 크기(rows-1)
/// 대신 이것을 읽어야 흔들린 높이가 GUI 변경에 실려 굳지 않는다(리뷰 지적 #3).
#[derive(Debug, Default)]
pub(crate) struct ResizeGate {
    pub(crate) gen: u64,
    pub(crate) nudge_origin: Option<(u16, u16)>,
}

/// (A1) 바깥(GUI `surface.resize`)이 생략한 치수를 채울 때 쓰는 '지금 정식 크기' — 흔들기 중이면 줄이기 전 크기, 아니면 파서 크기.
pub(crate) fn current_size_for_resize(s: &Surface) -> (u16, u16) {
    let gate = s.resize_gate.lock().unwrap_or_else(|e| e.into_inner());
    match gate.nudge_origin {
        Some(origin) => origin,
        None => s.parser.lock().unwrap_or_else(|e| e.into_inner()).screen().size(),
    }
}

/// (A1) 바깥(GUI `surface.resize`)의 크기 변경 — `resize_gate` 락 아래에서 PTY → 파서를 한 번에 맞춘다. 세대는 **PTY 가 실제로
/// 바뀐 뒤에만** 올린다(실패한 변경이 흔들기의 되돌리기를 취소해 PTY 가 rows-1 에 굳는 경로 차단 — 리뷰 지적 #1). 핸들러는
/// 이 함수만 부른다(두 단계를 따로 하면 재동기 스레드와 엇갈린다). 파서 실패는 PTY 만 바뀐 상태 — 그 사유를 돌려준다
/// (파서는 패닉 시 새 파서로 갈아 PTY 크기에 맞춘다 · `set_parser_size`).
pub(crate) fn apply_resize(s: &Surface, rows: u16, cols: u16) -> Result<(), String> {
    let mut gate = s.resize_gate.lock().unwrap_or_else(|e| e.into_inner());
    resize_pty(s, rows, cols)?;
    gate.gen = gate.gen.wrapping_add(1);
    gate.nudge_origin = None; // 바깥 크기가 새 정식 크기다 — 흔들기는 되돌리지 않는다
    set_parser_size(s, rows, cols)
}

/// PTY 를 (rows-1, cols) 로 줄였다가 `settle` 뒤 (rows, cols) 로 되돌린다 — 파서 크기도 같이 맞춘다(높이만 · 사유는 모듈 doc A3).
/// 두 단계(줄이기 · 대조+되돌리기)는 각각 `resize_gate` 락 아래에서 하고 잠든 사이에는 놓는다. 그 사이 바깥 변경(세대
/// 증가)이 있었으면 되돌리지 않는다. 줄인 뒤 파서를 못 맞추면 PTY 를 곧장 되돌리고 포기한다(두 크기가 어긋난 채 두지 않는다).
/// 오류는 `Err` 로 돌려준다(패닉 없음). 흔들기 중에는 `nudge_origin` 이 정식 크기를 들고 있다(어느 경로로 끝나도 지운다).
pub(crate) fn nudge_resize(s: &Surface, settle: Duration) -> Result<NudgeOutcome, String> {
    if s.exited.load(Ordering::Relaxed) {
        return Err("좌석이 종료됐다".into());
    }
    let (rows, cols, gen_seen) = {
        let mut gate = s.resize_gate.lock().unwrap_or_else(|e| e.into_inner());
        let (rows, cols) = s.parser.lock().unwrap_or_else(|e| e.into_inner()).screen().size();
        if rows < REPAINT_MIN_ROWS || cols < 2 {
            return Err(format!("크기가 너무 작다({rows}x{cols})"));
        }
        let short = rows - 1;
        resize_pty(s, short, cols)?;
        if let Err(e) = set_parser_size(s, short, cols) {
            let _ = resize_pty(s, rows, cols);
            let _ = set_parser_size(s, rows, cols);
            return Err(e);
        }
        gate.nudge_origin = Some((rows, cols));
        (rows, cols, gate.gen)
    };
    std::thread::sleep(settle);
    let mut gate = s.resize_gate.lock().unwrap_or_else(|e| e.into_inner());
    if gate.gen != gen_seen {
        gate.nudge_origin = None;
        return Ok(NudgeOutcome::SupersededByOtherResize);
    }
    if s.exited.load(Ordering::Relaxed) {
        gate.nudge_origin = None;
        return Err("되돌리기 전에 좌석이 종료됐다".into());
    }
    let restored = resize_pty(s, rows, cols).and_then(|()| set_parser_size(s, rows, cols));
    gate.nudge_origin = None;
    restored.map(|()| NudgeOutcome::Restored)
}

/// 흔들 수 있는 최소 높이 — 줄여도 3행이 남는다(Ink 의 '터미널이 너무 작다' 경로와 커서가 맨 아랫줄인 2행 화면을 피한다 · 보수적).
pub(crate) const REPAINT_MIN_ROWS: u16 = 4;

fn resize_pty(s: &Surface, rows: u16, cols: u16) -> Result<(), String> {
    s.master
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .resize(portable_pty::PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
        .map_err(|e| format!("PTY 크기 변경 실패({rows}x{cols}): {e}"))
}

/// (A6) 파서 크기 변경 — `catch_unwind` 로 감싼다. 파서 락을 쥔 채 안에서 패닉이 나면 가드가 unwinding 중에 떨어져 락이
/// 독에 들고, 그 뒤 모든 `parser.lock()` 이 `into_inner` 로 독을 삼키며 반쯤 바뀐 파서를 쓰게 된다. 패닉을 잡으면 **반쯤
/// 바뀐 파서를 그대로 두지 않고** 목표 크기의 새 파서로 간다(reader 의 패닉 격리 `process_chunk_isolated` 와 같은 처방 ·
/// `parser_panics`·`last_parser_panic` 에 남긴다 — 리뷰 지적 #2). 그 뒤 다시 그리기가 사본을 채운다. 패닉은 `Err` 가 된다.
fn set_parser_size(s: &Surface, rows: u16, cols: u16) -> Result<(), String> {
    let mut p = s.parser.lock().unwrap_or_else(|e| e.into_inner());
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| p.set_size(rows, cols))) {
        Ok(()) => Ok(()),
        Err(_) => {
            *p = vt100::Parser::new(rows, cols, crate::state::SCROLLBACK_LINES);
            drop(p);
            s.parser_panics.fetch_add(1, Ordering::Relaxed);
            *s.last_parser_panic.lock().unwrap_or_else(|e| e.into_inner()) = Some(now_epoch());
            Err(format!("파서 크기 변경 중 패닉({rows}x{cols}) — 새 파서로 갈았다"))
        }
    }
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
        assert!(clock_is_stale(Some(now - Duration::from_millis(15_500)), now), "15초를 조금이라도 넘으면 오래됨(초 단위 절삭 없음)");
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
    /// 되돌린 뒤 반향 제외 창은 출력이 1초 조용해지면 닫힌다(A2 · M1-b).
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
        // (A2 · M1-b) 되돌린 시각이 찍혔고 창은 아직 열려 있다(바닥 3초 안) — 출력 없이 바닥이 지나면 닫힌다.
        {
            let e = s.repaint_echo.lock().unwrap();
            assert!(e.restored_at.is_some() && e.cap_until.is_some(), "{e:?}");
            assert!(e.active_at(Instant::now()));
            let later = e.restored_at.unwrap() + Duration::from_millis(REPAINT_ECHO_GRACE_MS);
            assert!(!e.active_at(later), "되돌린 뒤 출력 없이 바닥(3초)이 지나면 창이 닫힌다");
        }
        assert!(Daemon::repaint_echo_active(&s));
        // 바깥 크기 변경 세대는 흔들기로 오르지 않는다(자기 변경을 바깥 변경으로 세지 않는다).
        assert_eq!(s.resize_gate.lock().unwrap().gen, 0);
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
        // 흔들기 중 바깥이 치수를 생략하면 임시 크기(23)가 아니라 정식 크기(24)를 읽는다(리뷰 지적 #3).
        assert_eq!(current_size_for_resize(&s), (24, 80));
        assert_eq!(s.resize_gate.lock().unwrap().nudge_origin, Some((24, 80)));
        apply_resize(&s, 30, 100).unwrap();
        assert_eq!(s.resize_gate.lock().unwrap().nudge_origin, None, "바깥 변경이 정식 크기다");
        assert_eq!(current_size_for_resize(&s), (30, 100));
        assert_eq!(h.join().unwrap(), Ok(NudgeOutcome::SupersededByOtherResize));
        assert_eq!(s.parser.lock().unwrap().screen().size(), (30, 100), "GUI 크기 유지(파서)");
        assert_eq!(pty_size(&s), (30, 100), "GUI 크기 유지(PTY)");
        assert_eq!(s.resize_gate.lock().unwrap().gen, 1, "바깥 변경 1회 = 세대 1");
        // 바깥 변경이 없으면 되돌린다 · 끝나면 정식 크기 표식은 비어 있다.
        assert_eq!(nudge_resize(&s, Duration::from_millis(1)), Ok(NudgeOutcome::Restored));
        assert_eq!(s.parser.lock().unwrap().screen().size(), (30, 100));
        assert_eq!(pty_size(&s), (30, 100));
        assert_eq!(s.resize_gate.lock().unwrap().nudge_origin, None);
        assert_eq!(current_size_for_resize(&s), (30, 100), "흔들기 밖에서는 파서 크기");
        // 너무 작은 화면은 건드리지 않는다(높이 3행 이하 · 너비 1열).
        s.parser.lock().unwrap().set_size(3, 80);
        assert!(nudge_resize(&s, Duration::from_millis(1)).is_err());
        assert_eq!(pty_size(&s), (30, 100), "거부는 PTY 를 건드리지 않는다");
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

    /// (A6) 흔들기 스레드가 패닉해도 진행 중 표식은 내려가고 반향 창에 되돌린 시각이 찍힌다(Drop 가드 · 이후 1초 정적이면 닫힌다).
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
        {
            let e = s.repaint_echo.lock().unwrap();
            let restored = e.restored_at.expect("되돌린 시각");
            assert!(e.active_at(restored) && !e.active_at(restored + Duration::from_millis(REPAINT_ECHO_GRACE_MS)));
        }
        // 파서 락은 독에 들지 않았다(set_parser_size 는 catch_unwind — 정상 호출도 Ok).
        assert!(set_parser_size(&s, 24, 80).is_ok());
        assert!(s.parser.lock().is_ok());
        let _ = s.child.lock().unwrap().kill();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★(성찰 2회차 M1-b · 순수) 반향 창의 끝은 **되돌린 뒤 바닥 3초 위에 '마지막 재방송 + 1초'** 다(고정 3초가 아니다) — 긴 재방송은 창을 늘리고(마지막 출력 + 1초),
    /// 출력이 없으면 되돌린 뒤 1초에 닫히며, 어느 경우든 상한 60초를 넘기지 않는다. 흔드는 중(되돌리기 전)에는 정적과 무관하게 열려 있다.
    #[test]
    fn echo_window_ends_on_quiet_after_restore_capped() {
        let t0 = Instant::now();
        let q = Duration::from_millis(REPAINT_ECHO_QUIET_MS);
        let g = Duration::from_millis(REPAINT_ECHO_GRACE_MS);
        let mut e = RepaintEcho::default();
        assert!(!e.active_at(t0), "창 없음");
        assert!(!e.note_output(t0) && e.ranges.is_empty());
        e.cap_until = Some(t0 + Duration::from_secs(REPAINT_ECHO_CAP_SECS));
        // 흔드는 중 — 출력이 없어도 열려 있다(정적 판정은 되돌린 뒤에만).
        assert!(e.active_at(t0 + Duration::from_secs(5)));
        // 되돌림 — 출력 없이 바닥 3초: 닫힘 · 그 전: 열림(2차 검토 MAJOR-1: 첫 재방송이 1초 넘게 늦어도 창 안).
        let restored = t0 + Duration::from_secs(1);
        e.restored_at = Some(restored);
        assert!(e.active_at(restored + g - Duration::from_millis(1)));
        assert!(!e.active_at(restored + g));
        assert!(e.note_output(restored + Duration::from_millis(2_000)), "되돌린 뒤 2초에 온 첫 재방송 청크는 창 안이다");
        // 되돌리기 전 출력은 재방송이 아니다 — 창을 늘리지 않는다.
        let mut e2 = RepaintEcho::default();
        e2.cap_until = e.cap_until;
        e2.last_output = Some(restored - Duration::from_millis(10));
        e2.restored_at = Some(restored);
        assert!(!e2.active_at(restored + g));
        // 긴 재방송 — 0.5초 간격 출력이 이어지면 창이 따라 늘어난다(바닥 3초를 넘어서도).
        let mut t = restored + Duration::from_millis(2_000);
        for _ in 0..10 {
            t += Duration::from_millis(500);
            assert!(e.note_output(t), "재방송이 이어지는 동안은 창 안이다({t:?})");
        }
        assert!(t > restored + g, "검체 전제: 바닥을 넘겼다");
        assert!(e.active_at(t + q - Duration::from_millis(1)) && !e.active_at(t + q), "마지막 출력 + 1초에 닫힌다");
        // 정적 뒤 도착한 출력은 창 밖 — 표식을 지운다.
        assert!(!e.note_output(t + q), "정적 1초 뒤의 출력은 재방송이 아니다");
        assert!(e.cap_until.is_none() && e.restored_at.is_none() && e.last_output.is_none(), "{e:?}");
        // 상한 — 되돌린 뒤 출력이 끊이지 않아도 60초에 닫힌다(실패 방향 = 룰 복귀).
        let mut e = RepaintEcho::default();
        e.cap_until = Some(t0 + Duration::from_secs(REPAINT_ECHO_CAP_SECS));
        e.restored_at = Some(t0 + Duration::from_secs(1));
        e.last_output = Some(t0 + Duration::from_secs(REPAINT_ECHO_CAP_SECS));
        assert!(e.active_at(t0 + Duration::from_secs(REPAINT_ECHO_CAP_SECS) - Duration::from_millis(1)));
        assert!(!e.active_at(t0 + Duration::from_secs(REPAINT_ECHO_CAP_SECS)));
        // 되돌리기 전에 상한이 지나도 닫힌다.
        let mut e = RepaintEcho::default();
        e.cap_until = Some(t0 + Duration::from_secs(REPAINT_ECHO_CAP_SECS));
        assert!(!e.active_at(t0 + Duration::from_secs(REPAINT_ECHO_CAP_SECS)));
        assert!(!e.note_output(t0 + Duration::from_secs(REPAINT_ECHO_CAP_SECS + 1)) && e.cap_until.is_none());
    }

    /// ★(성찰 2회차 M1-a · 순수) 반향 줄 구간 — 맞닿은 구간은 잇고 · 겹치면 끝을 늘리고 · 빈 구간은 무시 · 스크롤백에서 밀려난 구간은 먼저 잊고 · 그래도 상한 64 를 넘으면
    /// 가장 오래된 것부터 잊는다. 구간 판정 `line_in_echo` 는 `[start, end)` 다. 창이 닫혀도(`clear_window`) 구간은 남는다.
    #[test]
    fn echo_line_ranges_merge_and_cap() {
        let mut e = RepaintEcho::default();
        e.record_lines(10, 10, 0);
        assert!(e.ranges.is_empty(), "빈 구간");
        e.record_lines(10, 13, 0);
        e.record_lines(13, 15, 0); // 맞닿음 → 잇는다
        assert_eq!(e.ranges, vec![(10, 15)]);
        e.record_lines(14, 16, 0); // 겹침 → 끝만 늘린다
        assert_eq!(e.ranges, vec![(10, 16)]);
        e.record_lines(20, 22, 0); // 틈 → 새 구간
        assert_eq!(e.ranges, vec![(10, 16), (20, 22)]);
        // 스크롤백에서 밀려난 구간(end <= oldest)은 잊는다 · 아직 일부라도 남은 구간은 둔다.
        e.record_lines(30, 31, 16);
        assert_eq!(e.ranges, vec![(20, 22), (30, 31)]);
        e.record_lines(40, 41, 21);
        assert_eq!(e.ranges, vec![(20, 22), (30, 31), (40, 41)], "21 < 22 라 (20,22) 는 아직 남는다");
        let mut e = RepaintEcho::default();
        e.record_lines(10, 16, 0);
        e.record_lines(20, 22, 0);
        for n in [9u64, 16, 19, 22] {
            assert!(!line_in_echo(&e.ranges, n), "{n}");
        }
        for n in [10u64, 15, 20, 21] {
            assert!(line_in_echo(&e.ranges, n), "{n}");
        }
        e.clear_window();
        assert_eq!(e.ranges.len(), 2, "창이 닫혀도 구간은 남는다");
        let mut e = RepaintEcho::default();
        for i in 0..(REPAINT_ECHO_RANGES_MAX as u64 + 5) {
            e.record_lines(i * 10, i * 10 + 1, 0);
        }
        assert_eq!(e.ranges.len(), REPAINT_ECHO_RANGES_MAX);
        assert_eq!(e.ranges[0], (50, 51), "가장 오래된 다섯 구간을 잊었다");
        assert!(!line_in_echo(&e.ranges, 0) && line_in_echo(&e.ranges, 50));
    }
}
