//! ★(0.14.45 · F2) 화면 사본 재동기 — **다시 그리기 요청**(PTY 크기를 한 칸 흔든다 · stdin 0 바이트).
//!
//! 【왜】 데몬의 vt100 화면 사본은 좌석 출력만으로 만들어진다. Claude Code 는 상대 이동으로 **고친 곳만** 다시 그리고,
//!   유휴 좌석은 프롬프트 줄을 다시 그리지 않는다. 그래서 사본이 한 번 어긋나면(파서 패닉 격리가 빈 파서로 갈았다 ·
//!   청크가 버려졌다) 커서 행에서 마커를 영영 못 찾고 큐가 `prompt_unknown`·`input_pending`(판독 불가)으로 굶는다
//!   (0.14.45 윈도우 11 제보 — master 16분 · worker 의 빈 프롬프트). 사람이 직접 send + Return 을 하면 풀렸던 것은
//!   그것이 다시 그리기를 일으켰기 때문이다. 여기서는 **키를 한 바이트도 쓰지 않고** 같은 효과를 낸다 — PTY 를
//!   (rows, cols-1) 로 줄였다가 ~150ms 뒤 (rows, cols) 로 되돌리면 SIGWINCH(유닉스)·ConPTY 크기 변경(윈도우)을 받은
//!   TUI 가 화면 전체를 다시 그린다. 화면 사본(vt100 파서)의 크기도 같이 맞춘다.
//!
//! 【치명위험 렌즈】
//!   ① 폭주 없음 — 좌석당 동시 1건(`repaint_in_flight`) · 좌석당 최소 간격 300초 · 복구되지 않은 연속 요청은 간격을
//!      두 배씩(최대 16배 = 80분) 늘린다. 판독 가능한 화면을 한 번이라도 보면 간격이 원래대로 돌아간다.
//!   ② clear 게이트 무관 — stdin 에 아무것도 쓰지 않는다(입력줄 계수·초안·clear 가드 경로를 건드리지 않는다).
//!   ③ 데몬 생존 — 크기 변경은 틱·reader 스레드 밖 전용 스레드에서 하고, 오류는 기록하고 건너뛴다(패닉·unwrap 0).
//!      그 스레드는 파서 락과 master 락을 **따로·잠깐씩** 쥔다(둘을 함께 쥐지 않는다 — `surface.resize` 핸들러와 같은 순서).
//!   ④ 사람 화면 — 마커를 아는 에이전트 좌석에서, 대체 화면(전체화면 앱)이 아닐 때만 부른다(호출부 조건).
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
pub(crate) const REPAINT_SETTLE_MS: u64 = 150;

/// 좌석별 재동기 상태(leaf 락 · 다른 락을 쥔 채 잡지 않는다).
#[derive(Debug, Default)]
pub(crate) struct RepaintState {
    /// 이 좌석이 판독 불가(막힘 ∧ 커서 행 마커 없음)로 처음 관측된 시각 — 판독 가능·다른 사유를 보면 지운다.
    pub(crate) unreadable_since: Option<Instant>,
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

/// 노브 — `CYS_SCREEN_REPAINT_NUDGE=0` 이면 끈다(기본 켬). 검체는 H 노브 덮개로 주입한다.
pub(crate) fn repaint_nudge_enabled() -> bool {
    crate::governance::h_knob("CYS_SCREEN_REPAINT_NUDGE").is_none_or(|v| v.trim() != "0")
}

/// 큐 틱이 마커 좌석을 관측할 때마다 부른다. `unreadable` = 막힘 사유가 `prompt_unknown`·`input_pending` 이고 커서 행에
/// 마커가 없으며(화면 판독 불가) 대체 화면·선택기 행·발행 중 프레임이 아니다(호출부가 가린다). 판독 불가가 아니면 시계를 지운다.
/// 요청이 도래하면 전용 스레드로 크기 흔들기를 띄우고 이벤트를 낸다. 반환 = 이번에 요청했는가.
/// 락: 좌석 `repaint`(leaf)만 잠깐 쥔다 — 이벤트 발행·스레드 생성은 그 락을 놓은 뒤다.
pub(crate) fn note_queue_screen(daemon: &Arc<Daemon>, s: &Arc<Surface>, unreadable: bool) -> bool {
    let now = Instant::now();
    let panics = s.parser_panics.load(Ordering::Relaxed);
    let (reason, unreadable_secs, attempt) = {
        let mut st = s.repaint.lock().unwrap_or_else(|e| e.into_inner());
        if !unreadable {
            st.unreadable_since = None;
            st.unrecovered = 0;
            return false;
        }
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
        // 진행 중 표식은 상태 락 안에서 세운다(같은 좌석 이중 요청 차단 · 아래 스레드가 끝에서 내린다).
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
            "note": "화면 사본이 어긋나 PTY 크기를 한 칸 줄였다 되돌려 다시 그리기를 요청한다(키 입력 없음)",
        }),
    );
    let seat = Arc::clone(s);
    let spawned = std::thread::Builder::new()
        .name(format!("cysd-repaint-{}", s.id))
        .spawn(move || {
            if let Err(e) = nudge_resize(&seat, Duration::from_millis(REPAINT_SETTLE_MS)) {
                eprintln!("[cysd] surface {} 다시 그리기 요청(크기 흔들기) 건너뜀: {e}", seat.id);
            }
            seat.repaint_in_flight.store(false, Ordering::Release);
        });
    if let Err(e) = spawned {
        s.repaint_in_flight.store(false, Ordering::Release);
        eprintln!("[cysd] surface {} 다시 그리기 스레드 생성 실패 — 건너뜀: {e}", s.id);
    }
    true
}

/// 크기 흔들기의 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NudgeOutcome {
    /// 줄였다가 되돌렸다.
    Restored,
    /// 줄인 사이에 다른 누가(GUI `surface.resize`) 크기를 바꿨다 — 그 크기를 존중하고 되돌리지 않았다.
    SupersededByOtherResize,
}

/// PTY 를 (rows, cols-1) 로 줄였다가 `settle` 뒤 (rows, cols) 로 되돌린다 — 파서 크기도 같이 맞춘다.
/// 파서 락과 master 락은 **따로·잠깐씩** 쥔다(`surface.resize` 핸들러와 같은 순서: master → parser). 오류는 `Err` 로 돌려준다(패닉 없음).
pub(crate) fn nudge_resize(s: &Surface, settle: Duration) -> Result<NudgeOutcome, String> {
    if s.exited.load(Ordering::Relaxed) {
        return Err("좌석이 종료됐다".into());
    }
    let (rows, cols) = s.parser.lock().unwrap_or_else(|e| e.into_inner()).screen().size();
    if rows == 0 || cols < 2 {
        return Err(format!("크기가 너무 작다({rows}x{cols})"));
    }
    let narrow = cols - 1;
    resize_pty(s, rows, narrow)?;
    s.parser.lock().unwrap_or_else(|e| e.into_inner()).set_size(rows, narrow);
    std::thread::sleep(settle);
    // 그 사이 다른 크기 변경이 들어왔으면(파서 크기가 우리가 둔 값이 아니다) 그것을 존중한다.
    let now_size = s.parser.lock().unwrap_or_else(|e| e.into_inner()).screen().size();
    if now_size != (rows, narrow) {
        return Ok(NudgeOutcome::SupersededByOtherResize);
    }
    if s.exited.load(Ordering::Relaxed) {
        return Err("되돌리기 전에 좌석이 종료됐다".into());
    }
    resize_pty(s, rows, cols)?;
    s.parser.lock().unwrap_or_else(|e| e.into_inner()).set_size(rows, cols);
    Ok(NudgeOutcome::Restored)
}

fn resize_pty(s: &Surface, rows: u16, cols: u16) -> Result<(), String> {
    s.master
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .resize(portable_pty::PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
        .map_err(|e| format!("PTY 크기 변경 실패({rows}x{cols}): {e}"))
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

    /// 진행 중 표식·간격 — 실제 좌석으로 `note_queue_screen` 을 두 번 연달아 불러도 요청은 한 번이다. 판독 가능 관측은 시계를 지운다.
    #[cfg(unix)]
    #[test]
    fn note_queue_screen_requests_once_and_restores_size() {
        let dir = std::env::temp_dir().join(format!("cys-repaint-{}-{}", std::process::id(), now_epoch() as u64));
        let _ = std::fs::create_dir_all(&dir);
        let daemon = Daemon::new(dir.join("cysd.sock"));
        let s = daemon.create_surface(None, Some("sleep 30".into()), None, None, 24, 80).expect("surface");
        // 판독 가능 → 요청 없음 · 시계 없음.
        assert!(!note_queue_screen(&daemon, &s, false));
        assert!(s.repaint.lock().unwrap().unreadable_since.is_none());
        // 첫 판독 불가 관측 = 시계만 선다.
        assert!(!note_queue_screen(&daemon, &s, true));
        // 61초 전부터 판독 불가였던 것으로 되돌린다.
        s.repaint.lock().unwrap().unreadable_since = Some(Instant::now() - Duration::from_secs(61));
        let mut rx = daemon.bus.subscribe();
        assert!(note_queue_screen(&daemon, &s, true), "60초 판독 불가 = 요청");
        assert!(!note_queue_screen(&daemon, &s, true), "곧바로 다시 불러도(진행 중·간격) 요청하지 않는다");
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
        let pty = s.master.lock().unwrap().get_size().expect("pty size");
        assert_eq!((pty.rows, pty.cols), (24, 80), "PTY 크기 원복");
        // 판독 가능 관측 → 시계·미복구 수 리셋.
        assert!(!note_queue_screen(&daemon, &s, false));
        let st = s.repaint.lock().unwrap();
        assert!(st.unreadable_since.is_none() && st.unrecovered == 0);
        drop(st);
        let _ = s.child.lock().unwrap().kill();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 줄인 사이에 다른 크기 변경이 들어오면 되돌리지 않는다(GUI 크기를 덮어쓰지 않는다).
    #[cfg(unix)]
    #[test]
    fn nudge_respects_concurrent_resize_and_rejects_tiny() {
        let dir = std::env::temp_dir().join(format!("cys-repaint2-{}-{}", std::process::id(), now_epoch() as u64));
        let _ = std::fs::create_dir_all(&dir);
        let daemon = Daemon::new(dir.join("cysd.sock"));
        let s = daemon.create_surface(None, Some("sleep 30".into()), None, None, 24, 80).expect("surface");
        let s2 = Arc::clone(&s);
        let h = std::thread::spawn(move || nudge_resize(&s2, Duration::from_millis(300)));
        std::thread::sleep(Duration::from_millis(100));
        // GUI 의 surface.resize 와 같은 두 단계.
        s.master
            .lock()
            .unwrap()
            .resize(portable_pty::PtySize { rows: 30, cols: 100, pixel_width: 0, pixel_height: 0 })
            .unwrap();
        s.parser.lock().unwrap().set_size(30, 100);
        assert_eq!(h.join().unwrap(), Ok(NudgeOutcome::SupersededByOtherResize));
        assert_eq!(s.parser.lock().unwrap().screen().size(), (30, 100), "GUI 크기 유지");
        // 너무 작은 화면은 건드리지 않는다.
        s.parser.lock().unwrap().set_size(24, 1);
        assert!(nudge_resize(&s, Duration::from_millis(1)).is_err());
        let _ = s.child.lock().unwrap().kill();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
