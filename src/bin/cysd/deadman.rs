//! W3 — 행(hung)-홀더 데드맨 에스컬레이션 + startup lock pid 기록 + crashloop 로그 dedupe.
//!
//! 배경: "살아있지만 멈춘(hung)" 데몬은 launchd KeepAlive(pid 생존 기준)로도 재기동되지 않고,
//! 신형 데몬은 startup flock 패배로 exit(1)만 반복해 영구 wedge된다. 이 모듈은 락 패자가
//! 홀더의 **생존(소켓 응답) + heartbeat 신선도**를 진단해, 무응답 && stale일 때만(그리고
//! 홀더 pid가 cysd로 확인될 때만) 홀더를 회수(SIGTERM→SIGKILL)하고 락을 인수한다.
//!
//! 안전 원칙:
//! - **fail-closed**: 홀더 pid 미상(구 락파일)·프로세스명 검증 실패·판정 애매 = 어떤 개입도 없이 exit.
//! - **무손실**: 건강한 홀더는 버전이 낮아도 절대 인수하지 않는다(인수=PTY 전멸). 오직 dead일 때만.
//! - **오살상 차단**: SIGTERM 전 pid의 프로세스명이 cysd인지 확인(pid 재사용 방어 — channels.rs MED-4 원칙).
#![cfg(unix)]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// 살아있는 데몬이 heartbeat mtime을 갱신하는 주기.
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);
/// heartbeat가 이 시간 이상 갱신되지 않으면 stale(=데몬 행)로 판정.
pub const HEARTBEAT_STALE_THRESHOLD: Duration = Duration::from_secs(45);
/// 홀더 소켓 status 질의 타임아웃(짧게 — hung 홀더는 응답 없음).
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
/// SIGTERM 후 SIGKILL까지의 유예.
pub const RECLAIM_GRACE: Duration = Duration::from_secs(5);
/// crashloop 로그 dedupe: 동일 사유 연속 패배는 이 횟수당 1줄만.
pub const LOCK_LOSS_LOG_EVERY_N: u64 = 30;

/// heartbeat 파일 경로 — state_dir/heartbeat.
pub fn heartbeat_path(state_dir: &Path) -> PathBuf {
    state_dir.join("heartbeat")
}

/// heartbeat touch(mtime=now). 내용은 현재 unix 타임스탬프(디버그·검증용, 판정은 mtime).
pub fn touch_heartbeat(path: &Path) {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let _ = std::fs::write(path, now.to_string());
}

/// heartbeat가 stale한가 — mtime이 now보다 threshold 이상 과거이면 true.
/// 파일 부재·mtime 조회 실패 = stale로 간주(살아있는 데몬은 락 획득 직후 반드시 touch하므로
/// 부재=비정상). 단, 데드맨은 [무응답 && stale] 교차조건이라, 방금 뜬 데몬은 probe 응답으로 걸러진다.
pub fn heartbeat_stale(path: &Path, threshold: Duration) -> bool {
    match std::fs::metadata(path).and_then(|m| m.modified()) {
        Ok(mtime) => mtime.elapsed().map(|e| e > threshold).unwrap_or(false),
        Err(_) => true,
    }
}

/// startup lock 파일에서 홀더 pid를 읽는다. 빈 파일·미기재·0·파싱 실패 = None
/// (구 락파일(빈 파일) 호환 → 데드맨 fail-closed: pid를 모르면 아무도 죽이지 않는다).
pub fn read_holder_pid(lock_path: &Path) -> Option<u32> {
    let s = std::fs::read_to_string(lock_path).ok()?;
    match s.trim().parse::<u32>().ok()? {
        0 => None,
        pid => Some(pid),
    }
}

/// 락 획득 직후: 락파일에 자기 pid를 기록(데드맨이 홀더를 식별)하고 heartbeat를 즉시 touch
/// (이후 주기 touch 전의 공백에서 다른 노드가 우릴 stale로 오판하지 않게 — 기동 창 방어).
pub fn claim_lock(file: &mut std::fs::File, state_dir: &Path) {
    use std::io::{Seek, SeekFrom};
    let pid = std::process::id();
    let _ = file.set_len(0);
    let _ = file.seek(SeekFrom::Start(0));
    let _ = write!(file, "{pid}");
    let _ = file.flush();
    touch_heartbeat(&heartbeat_path(state_dir));
}

/// 승자 소켓에 짧은 타임아웃으로 system.ping 질의 — 응답하면 true(홀더 살아있음).
/// 무응답·연결오류·타임아웃 = false. hung 홀더는 커널이 connect는 받아주지만(bound 소켓)
/// 핸들러가 wedge돼 응답이 없다 → false.
pub fn probe_holder(socket_path: &Path, timeout: Duration) -> bool {
    use std::os::unix::net::UnixStream;
    let Ok(mut stream) = UnixStream::connect(socket_path) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));
    let req = b"{\"id\":1,\"method\":\"system.ping\",\"params\":{}}\n";
    if stream.write_all(req).is_err() {
        return false;
    }
    let _ = stream.flush();
    let mut buf = [0u8; 1];
    matches!(stream.read(&mut buf), Ok(n) if n > 0)
}

/// pid 생존 프로브 — 단일 정의처 state::pid_alive 위임(생존 판정 소스 단일화 · 도구 출력과
/// 기억이 충돌하면 프로브가 이긴다). 이 모듈은 #![cfg(unix)]라 unix arm(kill(pid,0))을 탄다.
pub fn pid_alive(pid: u32) -> bool {
    crate::state::pid_alive(pid)
}

/// pid의 프로세스명이 정확히 cysd인가(kill 오살상 차단 — pid 재사용 방어).
/// basename이 정확히 "cysd"인지 확인(부분일치 금지 — `cysd-<hash>` 테스트 바이너리 등 유사명
/// 오살상 차단, fail-closed 강화). macOS·Linux 모두 배포 sibling·앱 번들 실행 바이너리의 basename이
/// 정확히 "cysd"다. 조회 실패·프로세스 부재 = false(개입 금지).
///
/// ★WS-7: 구현을 `ps` fork → **sysinfo 단일 pid 스냅샷**으로 교체했다. 고부하에서 fork+exec가
/// 50~150ms를 먹어 락 진단·재시도의 시간 예산을 깨뜨렸다(판정 의미론은 동일하게 유지).
pub fn pid_is_cysd(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    let target = sysinfo::Pid::from_u32(pid);
    let mut sys = sysinfo::System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[target]), true);
    sys.process(target)
        .map(|p| {
            p.name()
                .to_string_lossy()
                .rsplit('/')
                .next()
                .map(|b| b == "cysd")
                .unwrap_or(false)
        })
        .unwrap_or(false)
}

/// 락 경합 시 홀더 판정 — 순수 함수(부수효과 없음, 진리표 테스트 가능).
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum HolderVerdict {
    /// 홀더 pid 미상(구 락파일) → 개입 없이 패자 exit(fail-closed).
    FailClosed,
    /// 홀더 건강(소켓 응답 또는 heartbeat 신선) → 개입 없이 패자 exit.
    Healthy,
    /// 홀더 dead(무응답 && heartbeat stale) → 데드맨 에스컬레이션.
    Dead,
}

/// pid 유무·probe 응답·heartbeat stale 3입력으로 판정.
/// dead 판정은 오직 [pid 알려짐 && 무응답 && stale] 교차조건일 때만 — 하나라도 어긋나면 개입 없음.
pub fn judge_holder(holder_pid: Option<u32>, responded: bool, hb_stale: bool) -> HolderVerdict {
    match holder_pid {
        None => HolderVerdict::FailClosed,
        Some(_) if responded => HolderVerdict::Healthy,
        Some(_) if !hb_stale => HolderVerdict::Healthy,
        Some(_) => HolderVerdict::Dead,
    }
}

/// dead 판정된 홀더를 SIGTERM→유예→SIGKILL로 회수. `verify_cysd`가 false면 kill하지 않는다
/// (오살상 차단 — main은 pid_is_cysd를 주입, 테스트는 주입 가능). 반환: 실제로 회수를 시도했는가
/// (=락 재획득 재시도 가치가 있는가). false = 검증 실패로 개입 안 함.
pub fn reclaim_from_dead_holder(
    pid: u32,
    grace: Duration,
    verify_cysd: impl Fn(u32) -> bool,
) -> bool {
    if !verify_cysd(pid) {
        eprintln!("[cysd] deadman: pid {pid} is not cysd (stale pid reuse?) — refusing to kill");
        return false;
    }
    eprintln!(
        "[cysd] deadman: holder pid {pid} appears hung (no socket response + stale heartbeat) — SIGTERM"
    );
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGTERM);
    }
    let deadline = std::time::Instant::now() + grace;
    while std::time::Instant::now() < deadline {
        if !pid_alive(pid) {
            eprintln!("[cysd] deadman: holder pid {pid} exited after SIGTERM");
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    eprintln!("[cysd] deadman: holder pid {pid} survived grace — SIGKILL");
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGKILL);
    }
    std::thread::sleep(Duration::from_millis(200)); // 프로세스 정리·flock 해제 시간.
    true
}

/// 락 패배 로그 dedupe 판정(순수): 직전 상태 파일 내용(`reason\ncount`)과 현재 사유로
/// (로그할지, 누적 카운트, 새 상태 문자열)을 반환. 동일 사유 연속 패배는 every_n회당 1줄만
/// (첫 발생 + N배수), 사유가 바뀌면 카운트 리셋(전환은 로그 가치 있음).
pub fn dedupe_loss_log(prev: Option<&str>, reason: &str, every_n: u64) -> (bool, u64, String) {
    let (prev_reason, prev_count) = match prev {
        Some(s) => {
            let mut lines = s.lines();
            let r = lines.next().unwrap_or("");
            let c = lines
                .next()
                .and_then(|x| x.trim().parse::<u64>().ok())
                .unwrap_or(0);
            (r.to_string(), c)
        }
        None => (String::new(), 0),
    };
    let count = if prev_reason == reason {
        prev_count + 1
    } else {
        1
    };
    let should_log = count == 1 || (every_n > 0 && count % every_n == 0);
    (should_log, count, format!("{reason}\n{count}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 검체용 임시 디렉터리 — **호출마다 유일**해야 한다.
    ///
    /// ★2026-09-05 실측 수리: 종전 이름은 `pid + 나노초` 였는데 이 기계의 시계 해상도가
    /// **1µs** 다(2만 회 호출에 고유값 1093개). 즉 같은 마이크로초에 두 검체가 부르면 pid 도
    /// 나노초도 같아 **같은 디렉터리**를 만들고, 그러면 둘이 같은 `cys.lock` 을 공유한다.
    /// cargo 는 검체를 병렬 실행하므로 그때 한쪽의 홀더가 다른 쪽의 재획득을 막아
    /// `flock_reacquire_after_holder_release` 가 간헐 적색이 됐다(단독 실행에서도 3회 중 1회).
    /// 결함이 아니라 검체끼리의 충돌이었고, 그 적색은 **아무것도 재지 않는다**.
    /// 원자 카운터를 더해 커널이 아니라 프로그램이 유일성을 보증하게 한다.
    fn tmp_dir() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering as AOrd};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let d = std::env::temp_dir().join(format!(
            "cysd-deadman-test-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            SEQ.fetch_add(1, AOrd::Relaxed)
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn judge_truth_table() {
        // (e) 판정 진리표 — dead는 오직 [pid 알려짐 && 무응답 && stale].
        assert_eq!(judge_holder(None, false, true), HolderVerdict::FailClosed); // (c) 구 락파일=pid 미상
        assert_eq!(judge_holder(None, false, false), HolderVerdict::FailClosed);
        assert_eq!(judge_holder(Some(9), true, true), HolderVerdict::Healthy); // 응답=건강(stale 무관)
        assert_eq!(judge_holder(Some(9), false, false), HolderVerdict::Healthy); // (b) heartbeat 신선=건강
        assert_eq!(judge_holder(Some(9), true, false), HolderVerdict::Healthy);
        assert_eq!(judge_holder(Some(9), false, true), HolderVerdict::Dead); // (a) 무응답 && stale
    }

    #[test]
    fn read_holder_pid_fail_closed_on_legacy_lockfile() {
        // (c) 구 락파일(빈 파일)·미기재·0 = None → 데드맨 미발동(fail-closed).
        let d = tmp_dir();
        let lock = d.join("cys.lock");
        std::fs::write(&lock, "").unwrap();
        assert_eq!(read_holder_pid(&lock), None, "빈 파일=None");
        std::fs::write(&lock, "0").unwrap();
        assert_eq!(read_holder_pid(&lock), None, "pid 0=None");
        std::fs::write(&lock, "garbage").unwrap();
        assert_eq!(read_holder_pid(&lock), None, "파싱 실패=None");
        std::fs::write(&lock, "  4242 \n").unwrap();
        assert_eq!(read_holder_pid(&lock), Some(4242), "정상 pid=Some(트림)");
        assert_eq!(read_holder_pid(&d.join("absent")), None, "부재=None");
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn heartbeat_fresh_vs_stale_vs_absent() {
        // (b)/(a) heartbeat 신선도: touch 직후=신선, 임계 0이면 과거=stale, 부재=stale.
        let d = tmp_dir();
        let hb = heartbeat_path(&d);
        assert!(heartbeat_stale(&hb, Duration::from_secs(45)), "부재=stale");
        touch_heartbeat(&hb);
        assert!(
            !heartbeat_stale(&hb, Duration::from_secs(3600)),
            "방금 touch=신선"
        );
        std::thread::sleep(Duration::from_millis(20));
        assert!(
            heartbeat_stale(&hb, Duration::ZERO),
            "임계 0이면 과거 mtime=stale"
        );
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn claim_lock_writes_own_pid_and_touches_heartbeat() {
        let d = tmp_dir();
        let lock = d.join("cys.lock");
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock)
            .unwrap();
        claim_lock(&mut f, &d);
        assert_eq!(read_holder_pid(&lock), Some(std::process::id()));
        assert!(!heartbeat_stale(&heartbeat_path(&d), Duration::from_secs(3600)));
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn probe_responds_to_live_socket_and_times_out_on_hung() {
        use std::os::unix::net::UnixListener;
        let d = tmp_dir();
        // 응답 소켓: accept 후 한 줄 회신.
        let live = d.join("live.sock");
        let l = UnixListener::bind(&live).unwrap();
        let h = std::thread::spawn(move || {
            if let Ok((mut s, _)) = l.accept() {
                let mut buf = [0u8; 64];
                let _ = s.read(&mut buf);
                let _ = s.write_all(b"{\"ok\":true}\n");
            }
        });
        assert!(
            probe_holder(&live, Duration::from_secs(2)),
            "응답 소켓=true"
        );
        h.join().ok();

        // hung 소켓: bind만 하고 accept 안 함 → connect는 되지만 무응답 → 2s 타임아웃 → false.
        let hung = d.join("hung.sock");
        let _listener = UnixListener::bind(&hung).unwrap();
        let t0 = std::time::Instant::now();
        assert!(
            !probe_holder(&hung, Duration::from_millis(300)),
            "accept만 하고 무응답=false"
        );
        assert!(t0.elapsed() < Duration::from_secs(2), "타임아웃 준수");

        // 부재 소켓: 연결 자체 실패 → false.
        assert!(!probe_holder(&d.join("nope.sock"), PROBE_TIMEOUT));
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn pid_is_cysd_refuses_non_cysd() {
        // 안전 게이트: 테스트 바이너리·init(pid 1)은 cysd가 아니므로 false → kill 거부.
        assert!(!pid_is_cysd(std::process::id()), "테스트 프로세스≠cysd");
        assert!(!pid_is_cysd(1), "init/launchd≠cysd");
        assert!(!pid_is_cysd(0), "pid 0=false");
    }

    #[test]
    fn reclaim_refuses_when_not_cysd() {
        // (오살상 차단) verify_cysd=false면 kill하지 않고 false 반환 — 대상 프로세스 생존.
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let pid = child.id();
        let acted = reclaim_from_dead_holder(pid, Duration::from_secs(1), |_| false);
        assert!(!acted, "검증 실패 시 개입 안 함");
        assert!(pid_alive(pid), "대상 프로세스 생존(kill 안 됨)");
        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn reclaim_sigterm_kills_verified_holder() {
        // (a) 에스컬레이션: verify=true인 대상에 SIGTERM → 정상 종료 → true 반환·프로세스 소멸.
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let pid = child.id();
        assert!(pid_alive(pid));
        let acted = reclaim_from_dead_holder(pid, Duration::from_secs(5), |_| true);
        assert!(acted, "회수 시도함");
        let _ = child.wait();
        assert!(!pid_alive(pid), "SIGTERM으로 종료됨");
    }

    #[test]
    fn reclaim_escalates_to_sigkill_on_sigterm_ignorer() {
        // (a) 에스컬레이션: SIGTERM 무시 프로세스는 유예 후 SIGKILL로 회수.
        let mut child = std::process::Command::new("sh")
            .arg("-c")
            .arg("trap '' TERM; sleep 30")
            .spawn()
            .unwrap();
        let pid = child.id();
        assert!(pid_alive(pid));
        let acted = reclaim_from_dead_holder(pid, Duration::from_millis(600), |_| true);
        assert!(acted);
        let _ = child.wait();
        assert!(!pid_alive(pid), "SIGKILL로 종료됨");
    }

    #[test]
    fn flock_reacquire_after_holder_release() {
        // (a) 락 인수 의미론: 서로 다른 open 기술자 간 flock은 같은 프로세스에서도 배타 →
        // 홀더 fd 해제 후 재시도 성공(데드맨이 홀더 kill 후 락 인수하는 경로의 토대).
        use std::os::unix::io::AsRawFd;
        let d = tmp_dir();
        let lock = d.join("cys.lock");
        let holder = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock)
            .unwrap();
        assert_eq!(
            unsafe { libc::flock(holder.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
            0,
            "홀더 획득"
        );
        let contender = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock)
            .unwrap();
        assert_ne!(
            unsafe { libc::flock(contender.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
            0,
            "경합자 즉시 획득 실패"
        );
        drop(holder); // 홀더 사망 모사(fd 해제=flock 해제).
        // ★2026-09-05: 이 단언이 간헐로 -1 이었다(단독 실행에서도 3~6회 중 1회). 원인 규명 중이며
        //   확인된 사실만 적는다 — ①순수 flock 시퀀스는 2000회 동시 실행에서 실패 0(시맨틱 문제
        //   아님) ②같은 바이너리의 다른 검체가 자식을 spawn·SIGKILL 하므로 **SIGCHLD 가 오간다**.
        //   `flock` 은 신호로 `EINTR` 을 돌릴 수 있고 그것은 '락을 못 얻었다' 가 아니라 '다시
        //   불러라' 다 — 재시도가 표준 요구사항이므로 여기서 그것부터 지킨다(EINTR 만 재시도하고
        //   다른 errno 는 그대로 실패시킨다 · 마스킹 금지).
        //   그래도 실패하면 **errno 를 남긴다** — 종전엔 -1 만 보여 다음 사람이 같은 자리에서
        //   다시 추측해야 했다.
        let mut rc = unsafe { libc::flock(contender.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        let mut errs: Vec<i32> = Vec::new();
        for _ in 0..8 {
            if rc == 0 {
                break;
            }
            let e = std::io::Error::last_os_error();
            errs.push(e.raw_os_error().unwrap_or(0));
            if e.raw_os_error() != Some(libc::EINTR) {
                break;
            }
            rc = unsafe { libc::flock(contender.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        }
        assert_eq!(
            rc,
            0,
            "홀더 해제 후 재획득 성공 (errno 이력={errs:?} — EINTR(4)이면 신호 경합, 그 외는 실제 경합)"
        );
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn dedupe_loss_log_suppresses_repeats() {
        // (d) crashloop 로그 dedupe: 첫 발생 + N배수만 로그, 사이는 억제, 사유 전환은 리셋.
        let every = 30;
        let (log1, c1, s1) = dedupe_loss_log(None, "healthy-holder", every);
        assert!(log1 && c1 == 1, "첫 발생=로그");
        let (log2, c2, s2) = dedupe_loss_log(Some(&s1), "healthy-holder", every);
        assert!(!log2 && c2 == 2, "2회차=억제");
        // 3회차부터 29회차까지 억제, 30회차 로그.
        let mut state = s2;
        loop {
            let (lg, c, s) = dedupe_loss_log(Some(&state), "healthy-holder", every);
            state = s;
            if c == 30 {
                assert!(lg, "30회차=로그");
                break;
            }
            assert!(!lg, "{c}회차=억제");
        }
        // 사유 전환 → 카운트 리셋 + 로그.
        let (log_new, c_new, _) = dedupe_loss_log(Some(&state), "dead-holder-reclaim-failed", every);
        assert!(log_new && c_new == 1, "사유 전환=리셋+로그");
    }
}
