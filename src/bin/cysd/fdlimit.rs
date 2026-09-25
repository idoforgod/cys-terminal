//! ★E(0.14.42 WP-transport) cysd 파일 디스크립터 **soft 한도 자체 상향**(RLIMIT_NOFILE).
//!
//! 배경: cysd 는 rlimit 을 한 번도 만지지 않아(코드 0건) 기동한 쪽의 soft 값을 그대로 쓴다.
//! launchd·Finder(GUI)·수동 기동은 soft 256 이고, 장수 스트림이나 버스트가 256 을 다 쓰면 XNU 의
//! accept 는 EMFILE 을 내며 **대기 연결을 버린다** — 클라이언트는 EOF/EPIPE 를 받고(서버는 요청을
//! 읽지 않았으므로 부작용 0인 **거짓 실패**), wakeup·report_gate·heartbeat·send 같은 주기 신호
//! RPC 가 '계속 실패' 로 보인다. S05 L-prod C=256 오류율 21.4%(같은 바이너리를 65536 으로 띄운
//! L-high 는 0).
//!
//! 방침(herdr macos.rs 와 같은 값·규칙):
//! - unix 에서만 soft 를 `min(8192, hard, kern.maxfilesperproc)` 로 **올린다**. 내리지 않는다
//!   (never-lower — 에이전트 셸에서 autostart 된 약 1M 한도 데몬은 그대로). hard 는 건드리지 않는다.
//! - 실패하면 경고 한 줄을 남기고 **상속값 그대로 기동을 계속**한다(부트 중단 0 · 패닉 경로 0).
//! - 자식(pane 셸 등)은 올린 soft 를 상속한다 — zsh 는 1M 에서도 정상이고 node·bun 은 스스로
//!   올리며, portable-pty unix.rs 의 pre_exec 클로저는 /dev/fd 를 열거하므로 한도와 무관하다.
//!   (재검토 조건: 자식 쪽에서 fd 번호 ≥1024 를 select 로 다루는 도구가 확인되면 자식 원복을 다시 본다.)
//! - 롤백: `~/.cys/nofile-raise-off` 마커(지속 · 모든 기동 경로) 또는 `CYS_NOFILE_RAISE=0`
//!   (off·false·no 도 같다). 둘 다 **다음 기동부터** 적용된다.
//!
//! 호출 위치는 시작 락 획득 **뒤**(main.rs async_main)다 — 락 경쟁 패자는 이 줄에 닿기 전에
//! exit 하므로 패자의 부수효과는 종전대로 mkdir/chmod 뿐이고, crashloop 로그 dedupe 도 그대로다.
#![cfg(unix)]

use libc::rlim_t;

/// soft 목표값 — 필요량(약 1000 안팎)의 약 8배 여유, Darwin OPEN_MAX 10240 아래, herdr 와 같다.
pub const NOFILE_SOFT_TARGET: rlim_t = 8192;
/// 롤백 env(`cys::env_compat` 경유 — JAVIS_·AITERM_ 폴백 포함).
pub const OPT_OUT_ENV: &str = "CYS_NOFILE_RAISE";
/// 롤백 파일 마커(홈 상대) — 형제 선례 `.cys/win-wheel-guard-off`.
pub const OPT_OUT_FILE: &str = ".cys/nofile-raise-off";

/// 새 soft 값(순수). 올릴 필요가 없으면 None — cur 가 무한이거나 이미 목표 이상이면 **절대 내리지 않는다**.
pub fn soft_target(cur: rlim_t, hard: rlim_t, sys_cap: Option<rlim_t>, target: rlim_t) -> Option<rlim_t> {
    let mut want = target;
    if hard != libc::RLIM_INFINITY {
        want = want.min(hard);
    }
    if let Some(c) = sys_cap {
        want = want.min(c);
    }
    (cur < want).then_some(want)
}

/// sysctl 결과 해석(순수) — 성공 ∧ 길이가 c_int ∧ 값 > 0 일 때만 Some. 결측을 0 이나 cap 으로 접지 않는다.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn cap_from_sysctl(rc: libc::c_int, len: usize, v: libc::c_int) -> Option<rlim_t> {
    (rc == 0 && len == std::mem::size_of::<libc::c_int>() && v > 0).then_some(v as rlim_t)
}

/// 커널의 프로세스당 파일 상한(macOS `kern.maxfilesperproc`). 판독 실패 = None(clamp 없음).
#[cfg(target_os = "macos")]
pub fn sys_cap() -> Option<rlim_t> {
    let mut v: libc::c_int = 0;
    let mut len: libc::size_t = std::mem::size_of::<libc::c_int>();
    // SAFETY: 읽기 전용 sysctl — 이름은 NUL 종료 리터럴, oldp 는 c_int 지역 변수이고 oldlenp 에 그 크기를
    // 넘긴다(커널은 그 이하만 쓴다). newp=null·newlen=0 이라 아무것도 쓰지 않는다.
    let rc = unsafe {
        libc::sysctlbyname(
            b"kern.maxfilesperproc\0".as_ptr() as *const libc::c_char,
            &mut v as *mut libc::c_int as *mut libc::c_void,
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    cap_from_sysctl(rc, len, v)
}

/// macOS 밖의 unix 는 clamp 없음.
#[cfg(not(target_os = "macos"))]
pub fn sys_cap() -> Option<rlim_t> {
    None
}

/// 무엇이 상향을 껐는가.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisabledBy {
    Env,
    Marker,
}

/// 롤백 판정(순수) — 마커가 있으면(우선) 또는 env 가 trim·소문자 기준 {0, off, false, no} 면 끈다.
/// 실패 방향: env 오타·stat 오류(exists=false)는 **켬** 쪽이다 — 어느 경우든 로그에 결과가 남는다.
pub fn decide_gate(env: Option<&str>, marker: bool) -> Option<DisabledBy> {
    if marker {
        return Some(DisabledBy::Marker);
    }
    match env.map(|v| v.trim().to_ascii_lowercase()) {
        Some(v) if matches!(v.as_str(), "0" | "off" | "false" | "no") => Some(DisabledBy::Env),
        _ => None,
    }
}

/// `decide_gate` 의 불리언 형(켬 = true).
pub fn raise_enabled(env: Option<&str>, marker: bool) -> bool {
    decide_gate(env, marker).is_none()
}

/// 상향 시도의 결과 — 모든 칸이 로그 한 줄로 남는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Disabled { by: DisabledBy },
    ReadFailed { errno: Option<i32> },
    Unchanged { soft: rlim_t, hard: rlim_t },
    Raised { prev: rlim_t, now: rlim_t, hard: rlim_t, capped_by_sys: bool },
    WriteFailed { prev: rlim_t, want: rlim_t, hard: rlim_t, errno: Option<i32> },
}

/// 판정·집행(주입판). Disabled 면 read·cap·write 를 **부르지 않는다**(syscall 0). ReadFailed 면
/// cap·write 를 부르지 않는다. write 에는 rlim_cur=want 와 **읽어 온 rlim_max 그대로**를 넘긴다.
pub fn raise_with(
    disabled_by: Option<DisabledBy>,
    read: impl FnOnce() -> Result<libc::rlimit, Option<i32>>,
    cap: impl FnOnce() -> Option<rlim_t>,
    write: impl FnOnce(&libc::rlimit) -> Result<(), Option<i32>>,
    target: rlim_t,
) -> Outcome {
    if let Some(by) = disabled_by {
        return Outcome::Disabled { by };
    }
    let cur = match read() {
        Ok(r) => r,
        Err(errno) => return Outcome::ReadFailed { errno },
    };
    let sys = cap();
    let Some(want) = soft_target(cur.rlim_cur, cur.rlim_max, sys, target) else {
        return Outcome::Unchanged { soft: cur.rlim_cur, hard: cur.rlim_max };
    };
    let capped_by_sys = sys.is_some_and(|c| {
        c == want && c < target && (cur.rlim_max == libc::RLIM_INFINITY || c < cur.rlim_max)
    });
    let new = libc::rlimit { rlim_cur: want, rlim_max: cur.rlim_max };
    match write(&new) {
        Ok(()) => Outcome::Raised { prev: cur.rlim_cur, now: want, hard: cur.rlim_max, capped_by_sys },
        Err(errno) => Outcome::WriteFailed { prev: cur.rlim_cur, want, hard: cur.rlim_max, errno },
    }
}

fn read_nofile() -> Result<libc::rlimit, Option<i32>> {
    let mut r = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
    // SAFETY: getrlimit 은 넘긴 rlimit 지역 변수에만 쓴다.
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut r) } == 0 {
        Ok(r)
    } else {
        Err(std::io::Error::last_os_error().raw_os_error())
    }
}

fn write_nofile(r: &libc::rlimit) -> Result<(), Option<i32>> {
    // SAFETY: setrlimit 은 넘긴 rlimit 을 읽기만 한다.
    if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, r) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().raw_os_error())
    }
}

/// 실제 상향 — 롤백 입력(env·마커)을 읽고 getrlimit/setrlimit 을 부른다. 패닉 경로 없음.
pub fn raise() -> Outcome {
    let env = cys::env_compat(OPT_OUT_ENV);
    let marker = cys::home_dir().join(OPT_OUT_FILE).exists();
    raise_with(decide_gate(env.as_deref(), marker), read_nofile, sys_cap, write_nofile, NOFILE_SOFT_TARGET)
}

fn fmt_lim(v: rlim_t) -> String {
    if v == libc::RLIM_INFINITY {
        "unlimited".to_string()
    } else {
        v.to_string()
    }
}

fn fmt_errno(e: Option<i32>) -> String {
    e.map_or_else(|| "?".to_string(), |n| n.to_string())
}

/// 기동 로그 한 줄 — 모든 칸이 `[cysd] fd-limit:` 로 시작한다.
pub fn log_line(o: &Outcome) -> String {
    let body = match o {
        Outcome::Disabled { by: DisabledBy::Env } => {
            format!("disabled by {OPT_OUT_ENV} — keeping inherited limit")
        }
        Outcome::Disabled { by: DisabledBy::Marker } => {
            format!("disabled by ~/{OPT_OUT_FILE} — keeping inherited limit")
        }
        Outcome::ReadFailed { errno } => {
            format!("read FAILED errno={} — keeping inherited limit", fmt_errno(*errno))
        }
        Outcome::Unchanged { soft, hard } => {
            format!("unchanged soft {} (hard {})", fmt_lim(*soft), fmt_lim(*hard))
        }
        Outcome::Raised { prev, now, hard, capped_by_sys } => format!(
            "soft {} -> {} (hard {}){}",
            fmt_lim(*prev),
            fmt_lim(*now),
            fmt_lim(*hard),
            if *capped_by_sys { " (clamped by kern.maxfilesperproc)" } else { "" }
        ),
        Outcome::WriteFailed { prev, want, hard, errno } => format!(
            "raise FAILED errno={} — keeping soft {} (want {}, hard {})",
            fmt_errno(*errno),
            fmt_lim(*prev),
            fmt_lim(*want),
            fmt_lim(*hard)
        ),
    };
    format!("[cysd] fd-limit: {body}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    const INF: rlim_t = libc::RLIM_INFINITY;

    /// U1 — soft_target 진리표(sys_cap 축 포함). never-lower · hard 불변 · clamp.
    #[test]
    fn soft_target_truth_table() {
        let t = NOFILE_SOFT_TARGET;
        let table: [((rlim_t, rlim_t, Option<rlim_t>), Option<rlim_t>); 9] = [
            ((256, INF, None), Some(8192)),
            ((256, 4096, None), Some(4096)),
            ((1024, 2048, None), Some(2048)),
            ((256, INF, Some(4096)), Some(4096)),
            ((256, INF, Some(100)), None),
            ((256, 256, None), None),
            ((8192, INF, None), None),
            ((1_048_576, INF, None), None),
            ((INF, INF, None), None),
        ];
        for ((cur, hard, cap), want) in table {
            assert_eq!(soft_target(cur, hard, cap, t), want, "soft_target({cur}, {hard}, {cap:?})");
        }
    }

    /// U2 — sysctl 해석: 결측(rc≠0 · 길이 불일치 · 0 · 음수)을 값으로 접지 않는다.
    #[test]
    fn cap_from_sysctl_never_folds_missing_into_a_value() {
        let n = std::mem::size_of::<libc::c_int>();
        assert_eq!(cap_from_sysctl(0, n, 245_760), Some(245_760));
        assert_eq!(cap_from_sysctl(-1, n, 245_760), None);
        assert_eq!(cap_from_sysctl(0, 8, 245_760), None);
        assert_eq!(cap_from_sysctl(0, n, 0), None);
        assert_eq!(cap_from_sysctl(0, n, -1), None);
    }

    fn lim(cur: rlim_t, max: rlim_t) -> libc::rlimit {
        libc::rlimit { rlim_cur: cur, rlim_max: max }
    }

    /// U3 — 판독 실패면 cap·write 를 부르지 않는다(결측 음성 대조).
    #[test]
    fn read_failure_never_writes() {
        let (caps, writes) = (Cell::new(0), Cell::new(0));
        let o = raise_with(
            None,
            || Err(Some(libc::EPERM)),
            || {
                caps.set(caps.get() + 1);
                None
            },
            |_| {
                writes.set(writes.get() + 1);
                Ok(())
            },
            NOFILE_SOFT_TARGET,
        );
        assert_eq!(o, Outcome::ReadFailed { errno: Some(libc::EPERM) });
        assert_eq!((caps.get(), writes.get()), (0, 0), "ReadFailed 인데 cap/write 를 불렀다");
    }

    /// U4 — 쓰기 실패는 WriteFailed 로 끝나고 패닉하지 않는다.
    #[test]
    fn write_failure_keeps_inherited_limit() {
        let o = raise_with(None, || Ok(lim(256, INF)), || None, |_| Err(Some(libc::EINVAL)), NOFILE_SOFT_TARGET);
        assert_eq!(o, Outcome::WriteFailed { prev: 256, want: 8192, hard: INF, errno: Some(libc::EINVAL) });
    }

    /// U5 — 성공 경로에서 write 가 받은 rlim_max 는 읽은 rlim_max 그대로(hard 불변), rlim_cur 는 목표·hard·cap 의 최소.
    #[test]
    fn hard_limit_is_passed_through_unchanged() {
        for (hard, cap, want) in [(INF, None, 8192), (4096, None, 4096), (INF, Some(3000), 3000)] {
            let seen = Cell::new(None);
            let o = raise_with(
                None,
                || Ok(lim(256, hard)),
                || cap,
                |r| {
                    seen.set(Some((r.rlim_cur, r.rlim_max)));
                    Ok(())
                },
                NOFILE_SOFT_TARGET,
            );
            assert_eq!(seen.get(), Some((want, hard)), "hard={hard} cap={cap:?}");
            assert!(matches!(o, Outcome::Raised { prev: 256, now, hard: h, .. } if now == want && h == hard));
            if cap == Some(3000) {
                assert!(matches!(o, Outcome::Raised { capped_by_sys: true, .. }), "clamp 표시 누락");
            } else {
                assert!(matches!(o, Outcome::Raised { capped_by_sys: false, .. }));
            }
        }
    }

    /// U6 — 롤백 노브: 마커 우선, env 는 {0,off,false,no}(trim·소문자). 끄면 syscall 0.
    #[test]
    fn opt_out_knob_truth_table() {
        for v in ["0", "off", "false", "no", " OFF "] {
            assert!(!raise_enabled(Some(v), false), "env={v:?} 가 끄지 못했다");
        }
        assert!(!raise_enabled(None, true), "마커가 끄지 못했다");
        assert!(!raise_enabled(Some("1"), true), "마커가 env 보다 우선이어야 한다");
        assert_eq!(decide_gate(Some("1"), true), Some(DisabledBy::Marker));
        assert_eq!(decide_gate(Some("0"), false), Some(DisabledBy::Env));
        for v in [None, Some("1"), Some("yes"), Some("")] {
            assert!(raise_enabled(v, false), "env={v:?} 는 켬이어야 한다");
        }
        let calls = Cell::new(0);
        let o = raise_with(
            Some(DisabledBy::Env),
            || {
                calls.set(calls.get() + 1);
                Ok(lim(256, INF))
            },
            || {
                calls.set(calls.get() + 1);
                None
            },
            |_| {
                calls.set(calls.get() + 1);
                Ok(())
            },
            NOFILE_SOFT_TARGET,
        );
        assert_eq!(o, Outcome::Disabled { by: DisabledBy::Env });
        assert_eq!(calls.get(), 0, "Disabled 인데 syscall 클로저를 불렀다");
    }

    /// U7 — 로그 한 줄의 형태.
    #[test]
    fn log_line_shapes() {
        let all = [
            Outcome::Disabled { by: DisabledBy::Env },
            Outcome::Disabled { by: DisabledBy::Marker },
            Outcome::ReadFailed { errno: None },
            Outcome::Unchanged { soft: 1_048_576, hard: INF },
            Outcome::Raised { prev: 256, now: 8192, hard: INF, capped_by_sys: false },
            Outcome::Raised { prev: 256, now: 4096, hard: INF, capped_by_sys: true },
            Outcome::WriteFailed { prev: 256, want: 8192, hard: INF, errno: Some(22) },
        ];
        for o in &all {
            let l = log_line(o);
            assert!(l.starts_with("[cysd] fd-limit:"), "{l}");
            assert!(!l.contains('\n'), "한 줄이 아니다: {l}");
        }
        let raised = log_line(&all[4]);
        assert!(raised.contains("soft 256 -> 8192") && raised.contains("unlimited"), "{raised}");
        assert!(log_line(&all[5]).contains("kern.maxfilesperproc"));
        assert!(log_line(&all[0]).contains("CYS_NOFILE_RAISE"));
        assert!(log_line(&all[1]).contains("~/.cys/nofile-raise-off"));
        assert!(log_line(&all[3]).contains("unchanged soft 1048576"));
        assert!(log_line(&all[6]).contains("raise FAILED errno=22") && log_line(&all[6]).contains("keeping soft 256"));
    }

    // ── C1: 자식 프로세스 검체(CI 몫 · cargo test --bin cysd 에 자동 편입) ──────────────────────
    const CHILD_ENV: &str = "CYS_FDLIMIT_CHILD_PROBE";

    fn fmt_ulimit(v: rlim_t) -> String {
        fmt_lim(v)
    }

    /// 자식 쪽 — 부모가 soft 를 낮춘 셸로 이 하네스를 다시 띄워 이 검체 하나만 돌린다.
    /// env 가 없으면 즉시 반환(단독 실행·--ignored 전수 실행에서 무해).
    #[test]
    #[ignore]
    fn child_probe() {
        if std::env::var_os(CHILD_ENV).is_none() {
            return;
        }
        let pre = read_nofile().expect("PRE getrlimit");
        println!("\n@@PRE {} {}", fmt_ulimit(pre.rlim_cur), fmt_ulimit(pre.rlim_max));
        let o = raise();
        println!("@@LOG {}", log_line(&o));
        let post = read_nofile().expect("POST getrlimit");
        println!("@@POST {} {}", fmt_ulimit(post.rlim_cur), fmt_ulimit(post.rlim_max));
        let kid = std::process::Command::new("/bin/sh")
            .args(["-c", "ulimit -Sn; ulimit -Hn"])
            .output()
            .expect("KID sh");
        let words: Vec<String> =
            String::from_utf8_lossy(&kid.stdout).split_whitespace().map(str::to_string).collect();
        println!("@@KID {}", words.join(" "));
    }

    struct ChildRun {
        pre: Vec<String>,
        log: String,
        post: Vec<String>,
        kid: Vec<String>,
    }

    /// `/bin/sh -c 'ulimit -Sn <soft> && exec <harness> --exact fdlimit::tests::child_probe …'`.
    fn run_child(soft: &str, env_off: Option<&str>, marker: bool) -> ChildRun {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let home = std::env::temp_dir().join(format!("cys-fdlimit-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join(".cys")).expect("임시 HOME");
        if marker {
            std::fs::write(home.join(OPT_OUT_FILE), b"").expect("마커");
        }
        let exe = std::env::current_exe().expect("current_exe");
        let mut cmd = std::process::Command::new("/bin/sh");
        cmd.arg("-c")
            .arg(format!(
                "ulimit -Sn {soft} && exec \"$0\" --exact fdlimit::tests::child_probe --ignored --nocapture --test-threads=1"
            ))
            .arg(&exe)
            .env(CHILD_ENV, "1")
            .env("HOME", &home);
        for k in ["CYS_NOFILE_RAISE", "JAVIS_NOFILE_RAISE", "AITERM_NOFILE_RAISE"] {
            cmd.env_remove(k);
        }
        if let Some(v) = env_off {
            cmd.env(OPT_OUT_ENV, v);
        }
        let out = cmd.output().expect("자식 하네스 실행");
        let _ = std::fs::remove_dir_all(&home);
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        // 줄 표식은 `@@<TAG> ` — libtest 가 검체 이름 뒤에 같은 줄로 붙여 찍어도 찾도록 줄 안 위치로 찾는다.
        let pick = |tag: &str| -> Option<String> {
            stdout.lines().find_map(|l| l.find(tag).map(|i| l[i + tag.len()..].to_string()))
        };
        let (Some(pre), Some(log), Some(post), Some(kid)) =
            (pick("@@PRE "), pick("@@LOG "), pick("@@POST "), pick("@@KID "))
        else {
            panic!(
                "자식 검체 줄 누락(결측은 값이 아니다) — rc={:?}\nstdout:\n{stdout}\nstderr:\n{}",
                out.status.code(),
                String::from_utf8_lossy(&out.stderr)
            );
        };
        let words = |s: String| s.split_whitespace().map(str::to_string).collect::<Vec<_>>();
        ChildRun { pre: words(pre), log, post: words(post), kid: words(kid) }
    }

    fn expected_raised(hard: rlim_t, cap: Option<rlim_t>) -> rlim_t {
        let mut w = NOFILE_SOFT_TARGET;
        if hard != INF {
            w = w.min(hard);
        }
        if let Some(c) = cap {
            w = w.min(c);
        }
        w
    }

    /// C1 — soft 256 셸로 띄운 cysd 하네스가 min(8192, hard, cap) 로 올리고, hard 는 그대로이며,
    /// 자식은 올린 soft 와 같은 hard 를 상속한다.
    #[test]
    fn child_process_raises_soft_limit_and_children_inherit() {
        if std::env::var_os(CHILD_ENV).is_some() {
            return;
        }
        let parent = read_nofile().expect("부모 getrlimit");
        let hard = fmt_ulimit(parent.rlim_max);
        let want = expected_raised(parent.rlim_max, sys_cap());
        assert!(want > 256, "호스트 한도(hard {hard} · cap {:?})로는 256 초과 상향을 잴 수 없다", sys_cap());
        let r = run_child("256", None, false);
        assert_eq!(r.pre, vec!["256".to_string(), hard.clone()], "PRE — 시작 soft 256·부모 hard");
        assert_eq!(r.post, vec![want.to_string(), hard.clone()], "POST — soft=min(8192,hard,cap)·hard 불변");
        assert_eq!(r.kid, vec![want.to_string(), hard.clone()], "KID — 자식이 올린 soft·같은 hard 를 상속");
        assert!(r.log.contains(&format!("soft 256 -> {want}")), "LOG: {}", r.log);
    }

    /// C1-b — 롤백 두 채널(env·마커)은 256 을 유지하고, 이미 높은 soft(9000)는 내리지 않는다.
    #[test]
    fn child_process_opt_out_channels_and_never_lower() {
        if std::env::var_os(CHILD_ENV).is_some() {
            return;
        }
        let parent = read_nofile().expect("부모 getrlimit");
        let hard = fmt_ulimit(parent.rlim_max);

        let r = run_child("256", Some("0"), false);
        assert_eq!(r.post.first().map(String::as_str), Some("256"), "env 롤백인데 올렸다: {:?}", r.post);
        assert!(r.log.contains("disabled by CYS_NOFILE_RAISE"), "LOG: {}", r.log);

        let r = run_child("256", None, true);
        assert_eq!(r.post.first().map(String::as_str), Some("256"), "마커 롤백인데 올렸다: {:?}", r.post);
        assert!(r.log.contains("disabled by ~/.cys/nofile-raise-off"), "LOG: {}", r.log);

        // never-lower — 전제: 부모의 min(hard, cap) ≥ 9000(깨지면 조용한 skip 이 아니라 적색).
        let ceiling = {
            let mut c = parent.rlim_max;
            if let Some(s) = sys_cap() {
                c = c.min(s);
            }
            c
        };
        assert!(ceiling >= 9000, "never-lower 검체 전제 불성립 — 호스트 hard {hard} · cap {:?}", sys_cap());
        let r = run_child("9000", None, false);
        assert_eq!(r.post.first().map(String::as_str), Some("9000"), "9000 을 내렸다: {:?}", r.post);
        assert!(r.log.contains("unchanged soft 9000"), "LOG: {}", r.log);
        assert_eq!(r.kid.first().map(String::as_str), Some("9000"));
    }
}
