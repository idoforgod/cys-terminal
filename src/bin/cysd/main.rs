//! cysd — CYSJavis 터미널 헤드리스 코어 데몬.
//! UI와 완전 분리: UI가 hang이어도 이 데몬과 소켓 제어 채널은 항상 살아있다 (OOB 회생).
// Windows: 데몬은 콘솔이 없어야 한다. 콘솔 서브시스템으로 두면 GUI(windows_subsystem)가
// cysd.exe 를 띄울 때 Windows가 실제 콘솔을 할당(Win11=Windows Terminal 검은 빈 창)하고,
// 그 상속 콘솔이 ConPTY 유사콘솔 핸드오프를 오염시켜 셸 surface가 즉시 종료된다([surface exited]).
// GUI 앱과 동일하게 릴리스에서 windows subsystem 으로 빌드해 콘솔을 원천 제거한다(디버그는 콘솔 유지).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod accounts;
mod alerts;
mod analytics;
mod approval;
mod approval_risk;
mod authority_broker;
mod caps;
mod channels;
mod classifier;
mod cost;
mod deadman;
mod events;
mod governance;
mod handlers;
mod hwmon;
mod queue_policy;
mod recall;
mod schedule;
mod severity;
mod skillrun;
mod state;
mod undo;
mod usage;

use cys::Request;
use handlers::Reply;
use serde_json::json;
use state::Daemon;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

type Stream = Box<dyn AsyncReadWrite>;

trait AsyncReadWrite: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> AsyncReadWrite for T {}

/// Claude Code 세션 안에서 spawn된 데몬이 그 세션의 정체성 env를 PTY 자식들에게
/// 물려주면, pane의 claude가 **child-session 모드**(부모 세션 종속)로 동작해 트랜스크립트
/// .jsonl을 영속하지 않는다 — 복원(restore)·recall·사용량 관측(T5)이 전부 깨진다
/// (2026-06-13 실측: 데몬을 `cys ping`으로 claude Bash에서 재기동하자 신규 노드 4종
/// 전부 트랜스크립트 미생성, env에 CLAUDE_CODE_SESSION_ID=부모세션 확인).
/// 데몬은 어떤 환경에서 spawn되든 자식에게 세션 정체성을 누설하면 안 된다 — 기동 즉시 제거.
fn scrub_claude_session_env() {
    const LEAKY: [&str; 5] = [
        "CLAUDECODE",
        "CLAUDE_CODE_CHILD_SESSION",
        "CLAUDE_CODE_SESSION_ID",
        "CLAUDE_CODE_ENTRYPOINT",
        "CLAUDE_CODE_SSE_PORT",
    ];
    for k in LEAKY {
        if std::env::var_os(k).is_some() {
            std::env::remove_var(k);
            eprintln!("[cysd] scrubbed leaky claude session env: {k}");
        }
    }
    // ★데몬 root env에 PYTHONUTF8=1 주입(정밀진단 diagnose-utf8-fix (a) 확정).
    // ⓐ근거: Windows embeddable python은 기본 코드페이지(cp1252/cp949)로 파일을 열어 한글 UTF-8
    //   팩파일 open().read()가 UnicodeDecodeError로 크래시한다. 데몬 root에서 1회 설정하면 데몬이
    //   spawn하는 **전 파이썬 자식**(fire_command 스케줄 잡·office-bridge·phoenix·auto-restore 등)이
    //   상속으로 자동 커버된다 — spawn 지점 개별 주입 금지(누락 시 6곳 산발). state.rs:1705의 pane
    //   builder.env("PYTHONUTF8","1") 주입과 정합(pane 밖 데몬 자식까지 root가 커버). unix는 이미
    //   UTF-8이라 no-op(무영향).
    // ⓑ2024 edition 마이그레이션 시: std::env::set_var는 2024부터 unsafe fn이므로 `unsafe { … }`
    //   래핑이 필요하다(현 edition은 safe — 위 scrub 루프의 remove_var와 동일 계약).
    std::env::set_var("PYTHONUTF8", "1");
}

#[tokio::main]
async fn main() {
    scrub_claude_session_env();

    // ★W1(조기 단일 인스턴스 게이트): 소켓 경로 확정 직후·pack 설치보다 먼저 단일 인스턴스 게이트를
    // 통과시킨다. 목적 — 락/싱글턴 경쟁의 **패자**가 상태를 오염시키는 부트 부수효과 전에 죽게 하는 것.
    // (게이트 뒤 부수효과: Daemon::new 의 operator.token 디스크 덮어쓰기·feed.jsonl compaction, 워치독·
    //  스케줄러·오피스 브리지 spawn, pack install, daemon.started 발행 등.) ★리뷰어1 F2: 패자의 잔여
    // 부수효과는 상태디렉터리 mkdir/chmod 0o700(멱등·무해)뿐 — operator.token·feed.jsonl 등 상태 파일과
    // 프로세스 spawn 은 무접촉이다. 과거엔 락 획득을 accept_loop 진입까지 미뤄 패자가 부수효과 전량을
    // 실행한 뒤에야 죽었다 → launchd KeepAlive 재기동 폭주 시 패자가 매번 operator.token 을 덮어써 라이브
    // 데몬 메모리 토큰과 불일치 → GUI 승인 Feed 우회가 무력화되어 Allow 전멸.
    let socket_path = cys::socket_path();

    // ★WS-9 ①: boot-id 구간 마커는 **락 시도 이전**에 1줄 발행한다 — 락 경합의 패자도 exit(1) 전에
    // 자기 구간 헤더를 남겨야 lock-loss 줄이 어느 부팅에 속하는지 사후 판별된다(패자 귀속).
    // 승자용 라이브 구간 헤더는 회전 완료 직후 한 번 더 발행한다(아래 ②).
    let boot_id = new_boot_id();
    let daemon_state_dir = socket_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    eprintln!("{}", boot_marker_line(&boot_id, "pre-lock", &socket_path));

    // unix: flock 기반 startup 락 — 경합 시 hung 홀더는 데드맨이 회수·인수, 건강한 홀더/구 락파일은
    // fail-closed exit. 락 파일 핸들은 이 main 스코프에서 데몬 수명 동안 보유한다(핸들 drop = flock 해제 =
    // 게이트 소멸이므로 절대 조기 drop 금지 — accept_loop 는 반환하지 않아 main 종료까지 살아있다).
    #[cfg(unix)]
    let _lock_file = {
        use std::os::unix::fs::PermissionsExt;
        // 상태 디렉터리 선생성: 락 파일이 이 디렉터리에 놓이므로 락 획득 전에 반드시 존재해야 한다
        // (소유자 전용 0o700 — transcripts.db·feed.jsonl·소켓을 같은 UID로 봉인).
        if let Some(dir) = socket_path.parent() {
            let _ = std::fs::create_dir_all(dir);
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
        }
        // ★W3: 경합 시 단순 exit(1)이 아니라, 홀더가 hung(무응답 + heartbeat stale)이면 데드맨이
        // 회수·인수한다. 건강한 홀더/구 락파일(pid 미상)은 fail-closed로 exit(무손실·오살상 차단).
        let lock_path = socket_path.with_extension("lock");
        let state_dir = socket_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let lock = acquire_startup_lock(&lock_path, &socket_path, &state_dir);
        // ★W3 heartbeat: 승자만 주기적으로 mtime을 갱신한다 → 런타임이 wedge되면 interval 태스크가
        // 진행하지 못해 자연히 stale이 되고, 다음 경합자의 데드맨이 dead로 판정할 수 있다.
        // 기동 창(락 획득~첫 주기 touch)은 claim_lock의 동기 초기 touch가 방어한다.
        {
            let hb = deadman::heartbeat_path(&state_dir);
            tokio::spawn(async move {
                let mut tick = tokio::time::interval(deadman::HEARTBEAT_INTERVAL);
                loop {
                    tick.tick().await;
                    deadman::touch_heartbeat(&hb);
                }
            });
        }
        lock
    };

    // windows: named pipe first-instance 선점 = 데몬 싱글턴 가드. 조기에 first 인스턴스를 만들어
    // accept_loop 로 넘겨 재사용한다(probe-후-close-재open 레이스 없이 그대로 리스너 풀에 편입).
    // 선점 실패(이미 홀더 존재)는 기존 즉사 의미 유지 — eprintln 후 exit 1.
    #[cfg(windows)]
    let first_pipe = {
        let pipe_name = socket_path.to_string_lossy().into_owned();
        match create_pipe_instance(&pipe_name, true) {
            Ok(s) => s,
            Err(e) => {
                // ★리뷰어1 F3: 구 panic!(exit 101) → exit(1) 통일 — 즉사 의미는 동일하되 종료코드를
                // unix 패자(acquire_startup_lock 의 exit(1))와 일치화한다.
                eprintln!("error: another cysd already owns the pipe {pipe_name}: {e}");
                std::process::exit(1);
            }
        }
    };

    // ★WS-9 ②: 로그 회전은 싱글턴 게이트를 통과한 **승자만** 수행한다(패자가 회전하면 crashloop이
    // 자기 증거를 지운다). 크기 게이트(10MB)·O_APPEND 실측 게이트는 maybe_rotate_daemon_log 안에 있다.
    // 회전 완료 직후 라이브 구간 헤더를 1줄 더 발행 — 회전으로 비워진 cysd.log의 첫 줄이 이번 부팅
    // 마커여야 "지금 보고 있는 로그가 어느 부팅인지"가 승자에 대해서도 성립한다.
    maybe_rotate_daemon_log(&daemon_log_path(&daemon_state_dir));
    eprintln!("{}", boot_marker_line(&boot_id, "live", &socket_path));
    // 장수 데몬(수개월 무재부팅) 대비 주기 점검 — 부팅 시 1회 회전만으로는 10MB를 넘긴 채 무한 성장한다.
    // heartbeat(W3)와 무관한 독립 interval 태스크다(데드맨 판정 입력 무접촉).
    {
        let log = daemon_log_path(&daemon_state_dir);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(LOG_ROTATE_CHECK_INTERVAL);
            tick.tick().await; // 첫 tick 은 즉시 발화 — 위 부팅 회전과 중복이므로 소비한다.
            loop {
                tick.tick().await;
                maybe_rotate_daemon_log(&log);
            }
        });
    }

    // windows .prev sweep 은 위 싱글턴 게이트 **뒤**에서 수행 — 승자만 잔해를 정리한다(패자는 이미 즉사).
    // ★무중단 rename-swap 잔해 청소(nsis-hooks.nsh의 짝): 업데이트가 잠긴 파일을 죽이는 대신
    // <이름>.prev*(cysd/cys 고정 체인 + unlock-sweep의 <이름>.prev<rand> — msys-2.0.dll 등 세션이
    // 로드한 runtime 이미지)로 밀어두므로, 새 cysd 기동 시 설치 트리를 재귀 순회하며 이름에
    // ".prev"가 든 파일을 best-effort 삭제한다. lame-duck이 아직 점유 중이면 실패가 정상 —
    // 조용히 스킵하고 다음 기동이 마저 청소한다(fail-open · 세션 보존 우선). 깊이 상한 12.
    #[cfg(windows)]
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            fn sweep_prev(dir: &std::path::Path, depth: u8) {
                if depth == 0 {
                    return;
                }
                let Ok(entries) = std::fs::read_dir(dir) else {
                    return;
                };
                for e in entries.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        sweep_prev(&p, depth - 1);
                    } else if p
                        .file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.contains(".prev"))
                        && std::fs::remove_file(&p).is_ok()
                    {
                        eprintln!("[cysd] stale update leftover removed: {}", p.display());
                    }
                }
            }
            sweep_prev(dir, 12);
        }
    }
    // crash recovery(§7-⑤): 직전 pack-update가 apply 도중 죽어 남긴 orphan 저널을 install(false)
    // **이전에** 자가치유한다(미커밋=rollback / 커밋완료=정리). 순서가 중요 — install(false)가
    // 부분반영 트리 위에서 돌면 안 되므로 반드시 선행한다.
    // ★리뷰어1 F1: 조기 락 전진(W1)으로 "락 보유~소켓 bind" 창이 이 pack recover/install 동기 블로킹을
    // 통째로 포함하게 됐다 — 단일 워커(1코어)에서 이 블로킹이 tokio 워커를 45초(HEARTBEAT_STALE_THRESHOLD)+
    // 굶기면 heartbeat interval 태스크가 못 돌아 stale → 경합자 데드맨이 정당한 승자를 Dead 오판·SIGKILL 할
    // 수 있다. spawn_blocking 으로 별도 블로킹 풀에 태워 main 태스크가 yield 해도 interval 이 계속 돌게 한다.
    match tokio::task::spawn_blocking(cys::pack::recover_pack_journal).await {
        Ok(Ok(true)) => eprintln!("[cysd] pack-update orphan journal recovered (self-heal)"),
        Ok(Ok(false)) => {}
        Ok(Err(e)) => eprintln!("[cysd] pack journal recovery skipped: {e}"),
        Err(e) => eprintln!("[cysd] pack journal recovery task failed: {e}"),
    }
    // 온보딩②: 팩이 이 바이너리 버전으로 미커밋일 때만 자동 설치 — 신규 머신·바이너리 업그레이드·
    // 팩 소실(.pack-version/매니페스트 부재 = 게이트 개방)이 실행 조건. launch-agent·디렉티브·acl이
    // "init-pack을 아는 사람"에게만 동작하는 것을 없앤다는 원목적은 유지된다(보존 모드·사용자 파일 불가침).
    // ★게이트(pack_current_for): 평시 부트는 stat 2회로 조기 반환 — 부서 데몬 N개·RestartOnFailure
    // 재기동·로그온 자동기동마다 전량 스윕(320파일 read+해시)이 돌던 비용 제거(2026-07-12 Win11 이슈 실측).
    // 손상+마커 무결 상태의 치유는 cys init-pack/pack-update/doctor --fix 명시 경로가 담당한다
    // (매 부트 전량 치유는 seed-once 원복 사고(7-12)의 원인 기전 — 의도적 축소).
    if !cys::pack::pack_current_for(env!("CARGO_PKG_VERSION")) {
        // ★리뷰어1 F1: install(false)도 동기 블로킹(최대 320파일 read+해시+write)이라 위 heartbeat 굶김
        // 위험이 동일 — spawn_blocking 으로 분리한다. (pack_current_for 게이트는 stat 2회라 동기 유지.)
        // W0-d: cysd 부팅 자동설치는 라이브 팩 쓰기 프로덕션 진입점 — 인가 부여.
        match tokio::task::spawn_blocking(|| {
            cys::pack::install(false, Some(cys::pack::PackWriteAuth::production()))
        })
        .await
        {
            Ok(Ok((written, _))) if written > 0 => eprintln!(
                "[cysd] CYSJavis Pack: {written} file(s) installed at {}",
                cys::pack::pack_dir().display()
            ),
            Ok(Ok(_)) => {}
            Ok(Err(e)) => eprintln!("[cysd] pack auto-install skipped: {e}"),
            Err(e) => eprintln!("[cysd] pack auto-install task failed: {e}"),
        }
    }
    let daemon = Daemon::new(socket_path.clone());

    governance::spawn_watchdog(Arc::clone(&daemon));
    // ★B2-1(W3): built-in phoenix 잡을 부트 시 idempotent ensure — schedule.json 이 user-owned 로 전환돼
    //   팩 배달로는 built-in 잡을 갱신할 수 없으므로 코드가 upsert 한다(부재 생성·구버전 갱신·중복 0). 스케줄러 기동 전.
    schedule::ensure_builtin_jobs();
    schedule::spawn_scheduler(Arc::clone(&daemon));
    usage::spawn_usage_collector(Arc::clone(&daemon));
    usage::spawn_agy_collector(Arc::clone(&daemon));
    // CC v2 WS-A: 계정 발견(프로필 스캔)+스냅샷 예열 — 관측 전에도 전 계정이 CC에 보인다.
    // 전부 fail-open(파일 부재·파싱 실패=빈 뷰) — 부트체인 비치명.
    accounts::seed_known(&daemon);
    accounts::spawn_custom_adapters(Arc::clone(&daemon));
    // CC v2 WS-B: 스킬 run 생애주기 — 이전 데몬의 열린 run 정리 후 전이 워처 기동.
    skillrun::reconcile_boot(&daemon);
    skillrun::spawn_watcher(Arc::clone(&daemon));
    // CC "🏢 오피스" 탭의 상시 가용성 — 메타버스 오피스 브리지(127.0.0.1:8642) 자동기동.
    spawn_office_bridge(crate::state::state_dir(&socket_path));
    // C0: 채널 부팅 재조정(고아 선-kill→새 토큰 재스폰) — 이벤트버스·state 준비 후(§2.1-2).
    // 불사조 복원 프로토콜의 "채널 재조정" 단계. 그 다음 주기 sweep(재배달·타임아웃·재스폰) 등록.
    channels::reconcile(&daemon);
    channels::spawn_channel_sweep(Arc::clone(&daemon));
    // 셧다운 경로: 원장은 메모리 전용이라 데몬이 죽으면 scoped 프로세스를 아무도 회수하지
    // 못한다 — SIGTERM/SIGINT 때 scoped 그룹을 전부 정리한 뒤 종료한다.
    #[cfg(unix)]
    {
        let d = Arc::clone(&daemon);
        tokio::spawn(async move {
            use tokio::signal::unix::{signal, SignalKind};
            let (Ok(mut term), Ok(mut int)) = (
                signal(SignalKind::terminate()),
                signal(SignalKind::interrupt()),
            ) else {
                return;
            };
            tokio::select! { _ = term.recv() => {}, _ = int.recv() => {} }
            shutdown_cleanup(&d, "signal");
            std::process::exit(0);
        });
    }
    // Windows: SIGTERM/SIGINT가 없으므로 콘솔 제어 이벤트로 같은 회수를 건다.
    // Ctrl-C·콘솔 닫힘·로그오프/셧다운(=catchable) 시 scoped 그룹을 정리한다.
    // (taskkill /F는 TerminateProcess라 어떤 핸들러도 못 받음 — 그 경로는 호출측
    //  taskkill /T·원장 정리의 몫. 여기선 unix가 잡던 모든 catchable 종료를 대칭화.)
    #[cfg(windows)]
    {
        let d = Arc::clone(&daemon);
        tokio::spawn(async move {
            use tokio::signal::windows::{ctrl_c, ctrl_close, ctrl_shutdown};
            let (Ok(mut cc), Ok(mut close), Ok(mut shutdown)) =
                (ctrl_c(), ctrl_close(), ctrl_shutdown())
            else {
                return;
            };
            tokio::select! {
                _ = cc.recv() => {},
                _ = close.recv() => {},
                _ = shutdown.recv() => {},
            }
            shutdown_cleanup(&d, "console_event");
            std::process::exit(0);
        });
    }
    daemon.bus.publish(
        "daemon.started",
        "system",
        None,
        json!({"pid": std::process::id(), "socket": socket_path.to_string_lossy()}),
    );

    eprintln!(
        "cysd (CYSJavis terminal daemon) listening on {}",
        socket_path.display()
    );
    #[cfg(unix)]
    accept_loop(daemon, &socket_path).await;
    // windows: main()에서 조기 선점한 first 파이프 인스턴스를 넘겨 리스너 풀에 재사용시킨다.
    #[cfg(windows)]
    accept_loop(daemon, &socket_path, first_pipe).await;
}

/// 종료 직전 회수: 원장의 scoped 그룹을 전부 죽이고, stopping 이벤트 발행 후
/// 소켓 파일을 제거한다. unix·windows 양쪽 종료 핸들러가 공유한다 (크로스플랫폼 대칭).
/// (windows named pipe엔 제거할 파일이 없어 remove_file은 무해한 no-op이 된다.)
fn shutdown_cleanup(daemon: &Arc<Daemon>, reason: &str) {
    let scoped = governance::collect_scoped_for_shutdown(&daemon.ledger.lock().unwrap());
    for (pid, pgid) in scoped {
        governance::kill_group_or_pid(pid, pgid);
    }
    daemon
        .bus
        .publish("daemon.stopping", "system", None, json!({"reason": reason}));
    let _ = std::fs::remove_file(&daemon.socket_path);
}

#[cfg(unix)]
async fn accept_loop(daemon: Arc<Daemon>, socket_path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    // ★W1: startup 락 획득·heartbeat spawn·상태 디렉터리 선생성은 부트 부수효과보다 먼저 실행돼야
    // 하므로 main()으로 전진했다(경쟁 패자가 부수효과 실행 전 즉사). 락 파일 핸들은 main 스코프에서
    // 데몬 수명 동안 보유된다. 여기(accept_loop)에는 소켓 바인드·수신 준비만 남긴다.
    // Refuse to start if a live daemon already owns the socket (중복 기동 방지 — 거버넌스 철학).
    if socket_path.exists() {
        if std::os::unix::net::UnixStream::connect(socket_path).is_ok() {
            eprintln!(
                "error: another cysd is already listening on {}",
                socket_path.display()
            );
            std::process::exit(1);
        }
        let _ = std::fs::remove_file(socket_path);
    }
    let listener = tokio::net::UnixListener::bind(socket_path)
        .unwrap_or_else(|e| panic!("bind {} failed: {e}", socket_path.display()));
    // 소켓 파일은 소유자만 read/write — 인증 없는 제어 채널을 같은 UID로 한정한다.
    // (master·worker·gemini·codex 노드는 모두 오너 UID로 도는 단일 사용자 구조)
    let _ = std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o600));

    // ★W2 콜드부트 자동 복원: 소켓 바인드·수신 준비가 끝난 '이후'에만 1회 발화한다(자식
    // phoenix가 이 데몬 소켓으로 즉시 RPC할 수 있어야 하므로 바인드 성공이 선행 조건).
    // raw `cys restore`가 아니라 phoenix를 태워 desired_roster·묘비·회로차단기·저널을 경유한다.
    // ★P0-7(D1/W5): prune + auto-restore 를 공통 post_listen_boot 로 — Windows accept_loop 와 동일 함수 호출
    //   (한쪽만 배선되던 미배선 결함 봉인). state_dir 은 함수 내부에서 canonical 매핑으로 재계산.
    post_listen_boot(socket_path, &daemon);

    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                // T1-3 발신자 신원: 커널이 보증하는 peer pid (자기신고 from의 검증 토대)
                let caller_pid = peer_pid(&stream);
                let daemon = Arc::clone(&daemon);
                tokio::spawn(async move {
                    handle_connection(daemon, Box::new(stream) as Stream, caller_pid).await;
                });
            }
            Err(e) => eprintln!("accept error: {e}"),
        }
    }
}

/// ★W3 startup lock 획득 — 경합 시 데드맨 에스컬레이션(hung 홀더 회수·인수)까지 수행한다.
/// 성공 시 락파일에 자기 pid 기록 + heartbeat 초기 touch 후 락 핸들 반환(데몬 수명 동안 보유).
/// 락 파일 자체를 못 열면 None(기존 동작 — connect 점검만으로 진행).
/// 회수 실패·건강한 홀더·구 락파일(pid 미상)은 fail-closed로 exit(1)(dedupe 로그).
#[cfg(unix)]
fn acquire_startup_lock(
    lock_path: &std::path::Path,
    socket_path: &std::path::Path,
    state_dir: &std::path::Path,
) -> Option<std::fs::File> {
    use std::os::unix::io::AsRawFd;
    let mut file = match std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(lock_path)
    {
        Ok(f) => f,
        Err(_) => return None, // 락 파일 생성 실패 — 기존 connect 점검만으로 진행
    };
    let try_flock = |f: &std::fs::File| unsafe {
        libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) == 0
    };

    if try_flock(&file) {
        deadman::claim_lock(&mut file, state_dir);
        return Some(file);
    }

    // ★WS-7: try_flock 실패는 곧바로 데드맨 판정으로 넘기지 않고 **지수 백오프+지터로 재시도**한다.
    // 근거: `cys doctor`가 진단 스팬 동안 같은 락을 순간 보유한다(cys.rs diag_orphan_socket·
    // diag_stale_lock). 재시도가 없으면 그 순간 부팅한 데몬이 홀더를 dead로 오판→회수 실패→
    // `dead-holder-reclaim-failed` **오사유로 exit(1)** 하고, 30회당 1줄 로그 억제(deadman.rs:197)와
    // launchd 10s 재기동이 겹쳐 **무로그 crashloop**이 된다.
    // ★적용 범위 엄수: 재시도는 **try_flock에만** 붙인다. judge_holder의 입력(holder_pid·responded·
    // hb_stale)과 진리표는 무접촉이다 — 데드맨 계약(X5·X6)을 침범하지 않는다.
    for backoff in lock_retry_schedule() {
        std::thread::sleep(jittered(backoff));
        if try_flock(&file) {
            deadman::claim_lock(&mut file, state_dir);
            return Some(file);
        }
    }

    // 경합: 현재 홀더 상태 진단(pid·소켓 응답·heartbeat 신선도) → 판정.
    let holder_pid = deadman::read_holder_pid(lock_path);
    let responded = deadman::probe_holder(socket_path, deadman::PROBE_TIMEOUT);
    let hb_stale = deadman::heartbeat_stale(
        &deadman::heartbeat_path(state_dir),
        deadman::HEARTBEAT_STALE_THRESHOLD,
    );
    match deadman::judge_holder(holder_pid, responded, hb_stale) {
        deadman::HolderVerdict::Dead => {
            // 홀더 hung 확정 → 회수(SIGTERM→SIGKILL, cysd 검증 후) → 락 1회 재획득 시도.
            let pid = holder_pid.expect("Dead 판정은 pid 존재를 함의");
            if deadman::reclaim_from_dead_holder(pid, deadman::RECLAIM_GRACE, deadman::pid_is_cysd)
                && try_flock(&file)
            {
                deadman::claim_lock(&mut file, state_dir);
                eprintln!("[cysd] deadman: reclaimed startup lock from dead holder (pid {pid})");
                return Some(file);
            }
            log_lock_loss(state_dir, lock_path, "dead-holder-reclaim-failed");
            std::process::exit(1);
        }
        deadman::HolderVerdict::Healthy => {
            log_lock_loss(state_dir, lock_path, "healthy-holder");
            std::process::exit(1);
        }
        deadman::HolderVerdict::FailClosed => {
            // 구 락파일(pid 미상) — 오살상 방지 위해 개입하지 않고 exit.
            log_lock_loss(state_dir, lock_path, "unknown-holder-pid");
            std::process::exit(1);
        }
    }
}

/// startup flock 재시도 백오프 스케줄(순수 — 테스트 가능). 기본 50→100→200→400→800ms(총 1550ms).
/// 총 예산이 1초를 넘어야 doctor의 순간 보유(진단 2건 연속 + 테스트 노브)를 흡수한다.
/// `CYS_LOCK_RETRY_MS`로 총 예산을 주입할 수 있다(테스트 결정론용 — 0이면 재시도 없음).
fn lock_retry_schedule() -> Vec<std::time::Duration> {
    schedule_for_budget(
        std::env::var("CYS_LOCK_RETRY_MS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(1550),
    )
}

fn schedule_for_budget(budget_ms: u64) -> Vec<std::time::Duration> {
    let mut out = Vec::new();
    let (mut used, mut step) = (0u64, 50u64);
    while used < budget_ms {
        let d = step.min(budget_ms - used);
        out.push(std::time::Duration::from_millis(d));
        used += d;
        step = (step * 2).min(800);
    }
    out
}

/// 백오프에 ±20% 지터 — 여러 데몬이 동시에 재시도해 같은 순간에 몰리는 thundering herd를 흩는다.
/// 신규 크레이트 금지 계약을 지키려 시스템 시각 나노초를 엔트로피로 쓴다(공정성 요구 없음).
fn jittered(d: std::time::Duration) -> std::time::Duration {
    let ms = d.as_millis() as u64;
    if ms < 5 {
        return d;
    }
    let span = ms / 5;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|x| x.subsec_nanos() as u64)
        .unwrap_or(0);
    std::time::Duration::from_millis(ms + (nanos % (2 * span + 1)) - span)
}

/// ★W3 crashloop 로그 dedupe — 동일 사유 연속 패배는 N회당 1줄만(누적 카운트 병기).
/// 상태는 state_dir 파일 기반(프로세스가 매번 새로 뜨므로 in-memory 불가).
#[cfg(unix)]
fn log_lock_loss(state_dir: &std::path::Path, lock_path: &std::path::Path, reason: &str) {
    let state_path = state_dir.join("lockloss.state");
    let prev = std::fs::read_to_string(&state_path).ok();
    let (should_log, count, new_state) =
        deadman::dedupe_loss_log(prev.as_deref(), reason, deadman::LOCK_LOSS_LOG_EVERY_N);
    let _ = std::fs::write(&state_path, new_state);
    if should_log {
        eprintln!(
            "error: another cysd holds the startup lock ({}) — reason={reason}, occurrence #{count}",
            lock_path.display()
        );
    }
}

// ─────────────────────────── WS-9: boot-id 구간 마커 + 로그 회전 ───────────────────────────
//
// F12(로그 무한 성장·구간 미상)의 수리. 두 축이다:
//   ① **boot-id 구간 마커**: 부팅마다 16자리 hex 구간 ID를 로그에 심어 "이 줄이 어느 부팅의 것인가"를
//      결정론으로 판별한다. 락 시도 **이전**(패자 귀속) + 회전 직후(승자 라이브 헤더) 총 2회 발행.
//   ② **copy-truncate 회전**: 라이브 기록자(launchd fd·앱스폰·부서 셸 리다이렉트)가 쥔 inode를 보존해야
//      하므로 `cysd.log` 자체는 rename 하지 않고 **복사 후 truncate(0)** 한다. 세대 이동(.2→.3, .1→.2)은
//      라이브 기록자가 없으므로 rename(원자적·무복사)을 쓴다.
//
// ★치명 전제(실측 확증): copy-truncate는 기록자 fd가 **O_APPEND일 때만** 안전하다. non-append 기록자는
//   자기 파일 오프셋을 그대로 유지하므로 truncate 후 첫 write가 10MB 지점에 착지해 **거대한 NUL 홀**을
//   만든다(로그가 NUL 늪이 되어 F12보다 나쁜 상태). HQ 데몬 로그 fd는 O_APPEND(`AP`)지만 **부서 데몬
//   로그 fd는 O_APPEND가 아니다**(`W,0x10000;SH`). 따라서 회전 직전 fcntl(F_GETFL)로 실측하고 미충족이면
//   **회전을 스킵**한다(경고 1줄). 근본 수리(부팅 시 자기 로그 fd를 O_APPEND로 재개설해 dup2)는 후속 티켓.

/// 회전 임계 — cysd.log가 이 크기 이상일 때만 회전한다. **크기 게이트는 필수**: 조건 없이 매 부팅
/// 회전하면 crashloop 3회에 보관 세대(.1/.2/.3)가 전부 밀려나 crashloop의 증거를 crashloop이 지운다.
const LOG_ROTATE_THRESHOLD: u64 = 10 * 1024 * 1024;
/// 장수 데몬 대비 주기 점검 간격(24h).
const LOG_ROTATE_CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);
/// 보관 세대 수 — cysd.log.1 ~ .3.
const LOG_ROTATE_GENERATIONS: u32 = 3;

/// 데몬 로그 경로 — 소켓 스코프별(`state_dir/cysd.log`). 본부·부서는 소켓이 다르므로 **다른 파일**이며,
/// 공유는 동일 스코프 내 기록자들(launchd + 앱스폰 + 중첩 기동) 사이에서만 발생한다.
fn daemon_log_path(state_dir: &std::path::Path) -> std::path::PathBuf {
    state_dir.join("cysd.log")
}

/// 부팅 구간 ID(16 hex). **신규 크레이트 도입 금지** — 기존 CSPRNG(channels::random_token_hex)를
/// 앞 16자만 쓰고, 그마저 실패하면 pid+나노초 조합으로 폴백한다. boot-id는 인가 토큰이 아니라
/// 구간 라벨이므로 예측 가능해도 무해하다(폴백 허용 근거 — random_token_hex의 hard-fail 정책과 별개).
fn new_boot_id() -> String {
    if let Ok(t) = channels::random_token_hex() {
        if t.len() >= 16 {
            return t[..16].to_string();
        }
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    format!("{:08x}{:08x}", std::process::id(), nanos as u32)
}

/// 구간 마커 1줄을 만든다(순수 — 테스트 가능). **80자 이내** 계약: phoenix 하네스가 두 번째 cysd의
/// 결합 출력을 400자로 절단해 `holds the startup lock` 리터럴을 찾으므로(javis_phoenix_harness.py:788-798),
/// 마커가 길면 그 리터럴을 400자 밖으로 밀어내 락 경합 드릴이 거짓 NOT-REPRODUCED가 된다.
/// 소켓 전체 경로를 넣으면 80자를 넘길 수 있으므로 넘칠 때만 파일명으로 축약한다.
fn boot_marker_line(boot_id: &str, phase: &str, socket_path: &std::path::Path) -> String {
    let v = env!("CARGO_PKG_VERSION");
    let full = socket_path.display().to_string();
    let line = format!("[cysd] ==== boot={boot_id} v={v} {phase} sock={full} ====");
    if line.len() <= 80 {
        return line;
    }
    let short = socket_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "?".into());
    format!("[cysd] ==== boot={boot_id} v={v} {phase} sock={short} ====")
}

/// 회전 기록자 안전성 실측 — 우리 로그 fd(stdout·stderr)가 **둘 다** O_APPEND인가.
/// 하나라도 아니면 copy-truncate가 NUL 홀을 만드므로 회전을 포기한다(fail-closed).
/// tty·파이프(개발 실행·phoenix 하네스 캡처)도 O_APPEND가 아니므로 자연히 스킵된다 — 의도된 보수성이다.
#[cfg(unix)]
fn fd_is_append(fd: std::os::unix::io::RawFd) -> bool {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    flags >= 0 && (flags & libc::O_APPEND) != 0
}

#[cfg(unix)]
fn log_fds_are_append() -> bool {
    fd_is_append(1) && fd_is_append(2)
}

/// windows: fcntl이 없다 — best-effort로 회전을 시도하고 실패는 무시한다(다른 기록자의 공유 모드에
/// 따라 복사·truncate가 실패할 수 있다).
#[cfg(windows)]
fn log_fds_are_append() -> bool {
    true
}

/// `cysd.log` → `cysd.log.<n>` 세대 경로.
fn log_generation_path(log: &std::path::Path, n: u32) -> std::path::PathBuf {
    let mut name = log
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_else(|| std::ffi::OsString::from("cysd.log"));
    name.push(format!(".{n}"));
    log.with_file_name(name)
}

/// 0바이트가 아닌 파일만 대상으로 하는 존재 판정(부서 셸 `>` 리다이렉트가 만든 빈 파일이 무한히
/// `.N`으로 승격되는 것을 막는다).
fn nonempty(p: &std::path::Path) -> bool {
    std::fs::metadata(p).map(|m| m.len() > 0).unwrap_or(false)
}

/// 세대 이동 + copy-truncate 실행부(게이트 통과 후에만 호출 — 단위 테스트가 직접 부른다).
///
/// 순서 불변식: `.2→.3`, `.1→.2`는 **rename**(원자적·무복사 — 라이브 기록자가 없는 파일이다),
/// `cysd.log → .1`만 **복사 후 truncate(0)**(라이브 기록자가 쥔 inode 보존).
///
/// 알려진 고유 한계: 복사 시작~truncate 사이에 도착한 소량 라인은 유실될 수 있다(logrotate copytruncate와
/// 동일). 10MB 시점의 수 라인 손실은 수용한다.
fn rotate_log_generations(log: &std::path::Path) -> std::io::Result<()> {
    for n in (1..LOG_ROTATE_GENERATIONS).rev() {
        let from = log_generation_path(log, n);
        let to = log_generation_path(log, n + 1);
        if nonempty(&from) {
            let _ = std::fs::rename(&from, &to); // best-effort — 실패해도 아래 단계는 진행한다.
        }
    }
    std::fs::copy(log, log_generation_path(log, 1))?;
    std::fs::OpenOptions::new()
        .write(true)
        .open(log)?
        .set_len(0)?;
    Ok(())
}

/// 크기 게이트 + O_APPEND 게이트를 통과할 때만 회전한다. 어떤 실패도 데몬 부팅을 막지 않는다.
///
/// ★append 여부를 **주입**받는다(테스트 가능성). 종전에는 내부에서 `log_fds_are_append()`를 직접
/// 불러 판정이 **테스트 하니스의 stdout/stderr 종류에 종속**됐다: `cargo test`를 파이프로 받으면
/// 게이트가 닫히고 `cargo test >> out.log`로 돌리면 열려, 같은 테스트가 환경에 따라 다른 결론을
/// 냈다. 더 나쁜 것은 **성공 경로(크기 게이트+append 게이트+실제 회전의 합성)가 어떤 테스트에서도
/// 실행되지 않았다**는 점이다 — 회전 자체는 `rotate_log_generations` 직접 호출로만 덮여 있었다.
fn maybe_rotate_daemon_log_with(log: &std::path::Path, fds_are_append: bool) {
    let len = match std::fs::metadata(log) {
        Ok(m) => m.len(),
        Err(_) => return, // 로그 파일 없음(포그라운드 실행 등) — 무해 스킵.
    };
    if len < LOG_ROTATE_THRESHOLD {
        return;
    }
    if !fds_are_append {
        eprintln!(
            "[cysd] log rotation skipped: log fd is not O_APPEND — copy-truncate would punch a NUL hole ({} bytes)",
            len
        );
        return;
    }
    match rotate_log_generations(log) {
        Ok(()) => eprintln!("[cysd] log rotated: {len} bytes → cysd.log.1 (copy-truncate)"),
        Err(e) => eprintln!("[cysd] log rotation failed (skipped): {e}"),
    }
}

/// 프로덕션 진입점 — 자기 로그 fd를 **실측**해 위 판정에 주입한다.
fn maybe_rotate_daemon_log(log: &std::path::Path) {
    maybe_rotate_daemon_log_with(log, log_fds_are_append());
}

/// ★W2 콜드부트 자동 복원 판정(순수 함수 — 부수효과 없음, 단위 테스트 가능).
/// opt-out(CYS_NO_AUTORESTORE)이 아니면 항상 Ready — ★B1: phoenix 는 바이너리 임베드본이 권위이므로
/// 디스크 팩 phoenix 부재가 "미설치 skip"이 아니다(임베드 추출로 실행). args[0]=디스크 phoenix(폴백 후보).
#[derive(Debug, PartialEq)]
enum AutoRestore {
    /// CYS_NO_AUTORESTORE=1 — 사용자가 콜드부트 복원을 껐다.
    OptedOut,
    /// 스폰 대상: `python3 <phoenix> restore --auto`. args[0]=디스크 phoenix(B1 폴백 후보).
    /// ★W1/B3(§5-1): env 에 PHOENIX_CYS(exe 옆 cys 절대경로)·PATH(runtime 선두주입)를 주입한다 —
    /// GUI/데몬 최소 PATH(/usr/bin:/bin:…)에서 phoenix 가 `cys` 를 못 찾아 FileNotFoundError→exit 1
    /// 침묵사하던 라이브 결함(2026-07-06 실증)의 근원 수리. 순수 판정이라 단위 테스트로 env 를 검증한다.
    Ready {
        program: String,
        args: Vec<String>,
        env: Vec<(String, String)>,
    },
}

/// ★W1/B3: exe_dir·current_path 를 인자로 받는 순수 함수(부수효과 없음·env 주입까지 단위 테스트 가능).
/// ★W6/E1: socket_path 를 phoenix 에 `--socket` 으로 명시 전달 — phoenix 의 상태 디렉터리(topology/desired/저널)가
/// 데몬 자신의 소켓에서 파생되게 한다. 프로덕션 무변경(dirname(라이브 소켓)==phoenix LIVE_STATE 로 동일 해석)이면서,
/// 격리 상태 디렉터리 E2E(데몬 교체 시뮬레이션)에서 phoenix 가 올바른 격리 소켓/상태를 타게 하는 enabler다.
fn decide_auto_restore(
    pack_dir: &std::path::Path,
    opted_out: bool,
    exe_dir: &std::path::Path,
    current_path: &str,
    socket: &str,
) -> AutoRestore {
    if opted_out {
        return AutoRestore::OptedOut;
    }
    // ★B1: 디스크 존재 게이트 제거 — 임베드본이 권위(디스크 부재여도 추출 실행). 이 경로는 폴백 후보다.
    let phoenix = pack_dir.join("bin").join("javis_phoenix.py");
    let mut env: Vec<(String, String)> = Vec::new();
    // PHOENIX_CYS: 데몬 exe 옆 동봉 cys 절대경로. 실존할 때만 주입한다(없으면 phoenix 의 which→표준경로
    // 폴백에 맡긴다 — 존재하지 않는 경로를 강제 주입해 재차 FileNotFoundError 를 만들지 않는다).
    let cys_name = if cfg!(windows) { "cys.exe" } else { "cys" };
    let cys_path = exe_dir.join(cys_name);
    if cys_path.is_file() {
        env.push((
            "PHOENIX_CYS".to_string(),
            cys_path.to_string_lossy().into_owned(),
        ));
    }
    // PATH 재합성 — pane 자식(state.rs)과 동일 유틸 재사용(중복 구현 금지). 무변경이면 None(무주입).
    if let Some(newp) = cys::runtime_prefixed_path(exe_dir, current_path) {
        env.push(("PATH".to_string(), newp));
    }
    // ★B3(§2 축B): 인터프리터 절대경로 해석 — 동봉 runtime python3 우선(win runtime\python\python3.exe /
    // mac Resources/runtime/python/bin/python3), 없으면 "python3" 리터럴(PATH 폴백). 순정 Windows(python3 부재)·
    // mac CLT 미설치 소비자에서 첫 스폰 단절(P0-7·P1-9)을 절대경로로 끊는다. PATH 선두주입과 이중 방어.
    let python = bundled_python3(exe_dir).unwrap_or_else(|| "python3".to_string());
    // args[0]=디스크 phoenix(폴백 후보) · 이후 `--socket <s> restore --auto`. spawn 이 args[0]을 실 실행원으로 교체.
    AutoRestore::Ready {
        program: python,
        args: vec![
            phoenix.to_string_lossy().into_owned(),
            "--socket".to_string(),
            socket.to_string(),
            "restore".to_string(),
            "--auto".to_string(),
        ],
        env,
    }
}

/// 메타버스 오피스 브리지(팩 javis_hud_bridge.py · 127.0.0.1 한정) 자동기동 — CC "🏢 오피스" 탭이
/// 수동 python3 기동 없이 항상 열리게 한다. 단일 인스턴스 가드: HUD 포트가 이미 listen 중이면
/// (선행 cysd·수동 기동) 스폰하지 않는다 — 동일 서버 누적이 구조적으로 0(자원 거버넌스 '누적·미종료' 차단).
/// 사망·부재는 60s 주기 재확인이 이어받고(KeepAlive), cysd 정상 종료 시 kill_on_drop이 자식을 동반 정리한다.
/// CYS_NO_OFFICE_BRIDGE=1 opt-out · 팩에 브리지 부재(구팩)면 조용히 skip.
/// python 해석·PATH·cys 주입은 auto-restore(★B3)와 동일 SOT(bundled_python3·runtime_prefixed_path).
fn spawn_office_bridge(state_dir: std::path::PathBuf) {
    if cys::env_compat("CYS_NO_OFFICE_BRIDGE")
        .map(|v| v == "1")
        .unwrap_or(false)
    {
        eprintln!("[cysd] office-bridge skipped (CYS_NO_OFFICE_BRIDGE=1)");
        return;
    }
    let script = cys::pack::pack_dir()
        .join("bin")
        .join("javis_hud_bridge.py");
    if !script.is_file() {
        return; // 구팩(브리지 미배포) — 다음 팩 업데이트가 채운다.
    }
    let port: u16 = std::env::var("HUD_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8642);
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));
    tokio::spawn(async move {
        let exe_dir_ref = exe_dir
            .as_deref()
            .unwrap_or_else(|| std::path::Path::new("."));
        let python = bundled_python3(exe_dir_ref).unwrap_or_else(|| "python3".to_string());
        let log_path = state_dir.join("office-bridge.log");
        loop {
            // 단일 인스턴스 가드 — 이미 서비스 중(선행 데몬·수동 기동)이면 스폰하지 않고 재확인만.
            if tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .is_ok()
            {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                continue;
            }
            let mut cmd = tokio::process::Command::new(&python);
            cmd.arg(&script)
                // 브리지의 cys 호출이 라이벌 데몬을 autostart하는 재귀 차단(auto-restore와 동일 계약).
                .env("CYS_NO_AUTOSTART", "1")
                // 런타임 상태는 팩 트리 밖으로(팩 본체 오염 0 — 팩 편입 계약 HUD_STATE_DIR).
                .env("HUD_STATE_DIR", state_dir.join("office-bridge"))
                .stdin(std::process::Stdio::null())
                .kill_on_drop(true);
            {
                // Windows: 콘솔 없는 cysd가 콘솔 자식(python3.exe)을 그냥 스폰하면 새 콘솔 창이
                // 할당된다(Win11 기본터미널=WT → AppData 경로 제목의 검은 상주 탭). 브리지는
                // 장수 프로세스라 앱 기동마다 빈 터미널 창이 함께 뜨던 실사고(2026-07-10)의 주범.
                use crate::state::HideConsole;
                cmd.hide_console();
            }
            if let Some(newp) =
                cys::runtime_prefixed_path(exe_dir_ref, &std::env::var("PATH").unwrap_or_default())
            {
                cmd.env("PATH", newp);
            }
            let cys_name = if cfg!(windows) { "cys.exe" } else { "cys" };
            let cys_path = exe_dir_ref.join(cys_name);
            if cys_path.is_file() {
                cmd.env("HUD_CYS_BIN", &cys_path); // 사이드카 cys 절대경로(PHOENIX_CYS 주입과 동일 패턴)
            }
            match std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)
            {
                Ok(log) => {
                    if let Ok(err) = log.try_clone() {
                        cmd.stderr(err);
                    }
                    cmd.stdout(log);
                }
                Err(_) => {
                    cmd.stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null());
                }
            }
            match cmd.spawn() {
                Ok(mut child) => {
                    eprintln!("[cysd] office-bridge spawned (127.0.0.1:{port})");
                    let _ = child.wait().await; // 사망 감지 → 아래 백오프 후 루프가 재스폰 판단
                    eprintln!("[cysd] office-bridge exited — 60s 후 재확인");
                }
                Err(e) => eprintln!("[cysd] office-bridge spawn failed: {e}"),
            }
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        }
    });
}

/// ★B3: 동봉 runtime python3 절대경로(exe 옆 번들). runtime_bin_dirs(pane 자식과 동일 SOT)에서 python3 실행파일을
/// 찾는다. 없으면 None(호출측이 "python3" 리터럴로 폴백 — PATH 선두주입이 동봉본을 잡거나 시스템 python3).
fn bundled_python3(exe_dir: &std::path::Path) -> Option<String> {
    let names: &[&str] = if cfg!(windows) {
        &["python3.exe", "python.exe"]
    } else {
        &["python3"]
    };
    for d in cys::runtime_bin_dirs(exe_dir) {
        for n in names {
            let p = d.join(n);
            if p.is_file() {
                return Some(p.to_string_lossy().into_owned());
            }
        }
    }
    None
}

/// ★P0-7 최종 층위(D1/W5·CI 28780215417 계열): 소켓/파이프 listening 직후 공통 부트 — **양 플랫폼 accept_loop가
/// 반드시 호출한다**. ①이전 실행 잔재 phoenix-embed prune(temp 누수 0) ②콜드부트 auto-restore 1회 발화.
/// state_dir 은 canonical 매핑(crate::state::state_dir — Windows LOCALAPPDATA/cys/<slug>·unix 소켓 부모)으로
/// 계산해 phoenix·스모크와 로그 경로가 일치한다(unix 의 socket_path.parent()는 Windows 파이프엔 부적합).
/// ★과거 `#[cfg(windows)] accept_loop` 에 이 호출이 없어(unix 만 배선) Windows 는 auto-restore 가 발동조차
/// 안 하고(triggered/skipped 라인 전무) phoenix-restore.log 가 빈 파일이던 P0-7 마지막 결함(CI 주입 우회가
/// 가려온 미배선)을 봉인. cfg 무관 단일 함수라 한쪽 누락이 재발하지 않는다(회귀 테스트로 소스 잠금).
fn post_listen_boot(socket_path: &std::path::Path, daemon: &Arc<Daemon>) {
    let state_dir = crate::state::state_dir(socket_path);
    prune_stale_phoenix_embed(&state_dir);
    spawn_auto_restore(&state_dir, socket_path, daemon);
}

/// 콜드부트 auto-restore를 detached 스폰한다(env에 CYS_NO_AUTOSTART=1 — 자식 CLI가 라이벌
/// 데몬을 autostart하는 재귀를 차단). 대기 스레드가 자식을 reap해 좀비 잔존을 막는다.
/// ★W1: PHOENIX_CYS·PATH 주입(§5-1 침묵사 근원 수리) · stdout/stderr 를 null 대신 phoenix-restore.log 로
/// 캡처(P0-5 사후 진단 불가 수리) · exit 계약 처리(5·6=재시도 금지, 그 외 비0=60s 후 1회 재시도).
fn spawn_auto_restore(
    state_dir: &std::path::Path,
    socket_path: &std::path::Path,
    daemon: &std::sync::Arc<Daemon>,
) {
    let opted_out = cys::env_compat("CYS_NO_AUTORESTORE")
        .map(|v| v == "1")
        .unwrap_or(false);
    // exe_dir(데몬 바이너리 디렉터리) — PHOENIX_CYS·PATH 계산 기준.
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));
    let current_path = std::env::var("PATH").unwrap_or_default();
    let exe_dir_ref = exe_dir
        .as_deref()
        .unwrap_or_else(|| std::path::Path::new("."));
    let socket = socket_path.to_string_lossy();
    match decide_auto_restore(
        &cys::pack::pack_dir(),
        opted_out,
        exe_dir_ref,
        &current_path,
        &socket,
    ) {
        AutoRestore::OptedOut => {
            eprintln!("[cysd] auto-restore skipped (CYS_NO_AUTORESTORE=1)");
        }
        AutoRestore::Ready {
            program,
            mut args,
            env,
        } => {
            // args = [disk_phoenix, "restore", "--auto"]. disk_phoenix 는 B1 폴백 후보.
            let disk_phoenix = std::path::PathBuf::from(args.remove(0));
            let tail = args; // ["restore","--auto"]
            let log_path = state_dir.join("phoenix-restore.log");
            let state_dir = state_dir.to_path_buf();
            let daemon = daemon.clone();
            std::thread::spawn(move || {
                let log_for_panic = log_path.clone();
                // ★P0-5 침묵사 차단(D3/W5·CI 28780215417 실증: auto-restore 스레드가 std/time.rs panic 으로 즉사
                //   → phoenix-restore.log 빈 파일·원인 불가시). 스레드 본문을 guard_restore_panic(catch_unwind)로
                //   감싸 panic 을 삼키지 않고 stderr + phoenix-restore.log 에 **1회 기록**한다 — 무한 재스폰 금지
                //   (재기동은 다음 데몬 부트/schtasks 소관). 이 웨이브가 죽이려는 '스레드 침묵사' 클래스의 구조 수리.
                guard_restore_panic(&log_for_panic, || {
                    // ★B1: 임베드 추출 실행 우선(바이너리=스크립트 동일 커밋 하드보장) → 실패 시 manifest-검증 디스크 폴백.
                    match resolve_phoenix_source(&state_dir, &disk_phoenix, &program, &daemon) {
                        PhoenixResolve::Ready { script, cleanup } => {
                            let mut run_args = vec![script.to_string_lossy().into_owned()];
                            run_args.extend(tail);
                            loop_auto_restore(&daemon, &program, &run_args, &env, &log_path);
                            // temp 누수 0: 추출본은 실행 후 정리(디스크 폴백은 cleanup=None).
                            if let Some(dir) = cleanup {
                                let _ = std::fs::remove_dir_all(&dir);
                            }
                        }
                        PhoenixResolve::Failed(reason) => {
                            eprintln!(
                                "[cysd] auto-restore ABORTED — 안전한 phoenix 없음: {reason}"
                            );
                            daemon.push_feed_notification(
                                "error",
                                "auto-restore 중단",
                                &format!("안전한 phoenix 실행원 없음(임베드 추출·디스크 폴백 모두 실패): {reason}"),
                                None,
                            );
                        }
                    }
                });
            });
            eprintln!("[cysd] auto-restore triggered (phoenix restore --auto · 임베드 추출 우선)");
        }
    }
}

/// ★B1 phoenix 실행원 해석 결과.
enum PhoenixResolve {
    /// 실행 가능한 phoenix 스크립트. cleanup=Some(dir)면 실행 후 그 임시 디렉터리를 정리한다(추출본).
    Ready {
        script: std::path::PathBuf,
        cleanup: Option<std::path::PathBuf>,
    },
    /// 임베드 추출·디스크 폴백 모두 실패 — auto-restore 중단(사유 보고).
    Failed(String),
}

/// PACK_ALL 에서 phoenix 실행에 필요한 bin/ 트리(javis_phoenix.py + 형제 의존 javis_state_snapshot.py 등)를 추린다.
fn phoenix_embed_files() -> Vec<(&'static str, &'static str)> {
    cys::pack::PACK_ALL
        .iter()
        .copied()
        .filter(|(rel, _)| rel.starts_with("bin/"))
        .collect()
}

/// ★B1①: 임베드 phoenix 트리를 <state>/phoenix-embed/<version>-<uuid>/ 에 추출한다(버전+고유 ID 격리).
/// 추출 실패(공간·권한·noexec)는 Err — 호출측이 디스크 폴백으로 강등한다. 반환=(추출 루트, phoenix 스크립트 경로).
/// ★codex W4 major: 중간 실패(create_dir_all/write) 시 이미 만든 partial root 를 즉시 remove_dir_all(정리 후 Err)
///   — temp 누수 0(다음 부팅 prune 에 의존하지 않는다).
fn extract_phoenix_embed(
    state_dir: &std::path::Path,
) -> std::io::Result<(std::path::PathBuf, std::path::PathBuf)> {
    let version = env!("CARGO_PKG_VERSION");
    let uniq = {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        format!("{version}-{}-{nanos}", std::process::id())
    };
    let root = state_dir.join("phoenix-embed").join(uniq);
    let write_all = || -> std::io::Result<()> {
        let mut written = 0u32;
        for (rel, content) in phoenix_embed_files() {
            let path = root.join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, content)?;
            written += 1;
            // 테스트 seam: root+일부 파일 생성 후 강제 실패 주입(중간 실패 정리 결정론 검증).
            if written == 1 && std::env::var("CYS_PHOENIX_EXTRACT_FAIL").is_ok() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "injected mid-extraction failure",
                ));
            }
        }
        Ok(())
    };
    match write_all() {
        Ok(()) => {
            let script = root.join("bin").join("javis_phoenix.py");
            Ok((root, script))
        }
        Err(e) => {
            // partial root 즉시 정리(temp 누수 0). best-effort — 정리 실패해도 원 에러를 반환.
            let _ = std::fs::remove_dir_all(&root);
            Err(e)
        }
    }
}

/// ★B1③: 추출된 phoenix self-test — `<python> <script> --selftest` 가 exit 0 + "selftest ok" 응답이면 통과.
/// 실행성만 확인(데몬·상태 무접촉). 실패=false(호출측이 정리 후 디스크 폴백).
fn phoenix_self_test(python: &str, script: &std::path::Path) -> bool {
    use crate::state::HideConsole;
    let out = std::process::Command::new(python)
        .arg(script)
        .arg("--selftest")
        .env("CYS_NO_AUTOSTART", "1")
        .stdin(std::process::Stdio::null())
        .hide_console()
        .output();
    match out {
        Ok(o) => o.status.success() && String::from_utf8_lossy(&o.stdout).contains("selftest ok"),
        Err(_) => false,
    }
}

/// ★B1②④: phoenix 실행원 해석 — 임베드 추출+self-test 우선, 실패 시 manifest-해시 검증 디스크 폴백.
/// stale 디스크(임베드와 해시 불일치)는 거부+보고(구버전 phoenix 실행 금지). 전 폴백 실패=Failed.
fn resolve_phoenix_source(
    state_dir: &std::path::Path,
    disk_phoenix: &std::path::Path,
    python: &str,
    daemon: &std::sync::Arc<Daemon>,
) -> PhoenixResolve {
    // 1) 임베드 추출 우선.
    match extract_phoenix_embed(state_dir) {
        Ok((root, script)) => {
            if phoenix_self_test(python, &script) {
                return PhoenixResolve::Ready {
                    script,
                    cleanup: Some(root),
                };
            }
            let _ = std::fs::remove_dir_all(&root); // temp 누수 0(self-test 실패분 즉시 정리)
            eprintln!("[cysd] phoenix 임베드 self-test 실패 — 디스크 폴백 시도");
            daemon.push_feed_notification(
                "warn",
                "phoenix 임베드 self-test 실패",
                "임베드 추출본이 --selftest 를 통과하지 못함 — 디스크 폴백으로 강등(침묵 금지).",
                None,
            );
        }
        Err(e) => {
            eprintln!("[cysd] phoenix 임베드 추출 실패({e}) — 디스크 폴백 시도");
            daemon.push_feed_notification(
                "warn",
                "phoenix 임베드 추출 실패",
                &format!("추출 실패({e}) — manifest-검증 디스크 폴백으로 강등(침묵 금지)."),
                None,
            );
        }
    }
    // 2) 디스크 폴백 — ★codex W4 major: script-only 해시가 아니라 phoenix 실행 closure **전체**(phoenix_embed_files
    //    단일 소스 — 추출과 동일 목록)를 대조한다. javis_phoenix.py 만 일치하고 형제 의존(javis_state_snapshot.py)이
    //    부재/stale 인 디스크 팩이 통과하던 구멍을 막는다. 하나라도 불일치/부재=거부+어느 rel 인지 보고.
    match disk_fallback_verify(disk_phoenix) {
        Ok(()) => {
            eprintln!(
                "[cysd] phoenix 디스크 폴백 채택(전 closure 해시 일치·verified): {}",
                disk_phoenix.display()
            );
            PhoenixResolve::Ready {
                script: disk_phoenix.to_path_buf(),
                cleanup: None,
            }
        }
        Err(reason) => {
            daemon.push_feed_notification(
                "error",
                "phoenix 디스크 폴백 거부(stale/불완전)",
                &format!("디스크 팩 phoenix closure 검증 실패 — 실행 거부(구/불완전 phoenix 부활 금지): {reason}"),
                None,
            );
            PhoenixResolve::Failed(format!("디스크 폴백 closure 검증 실패 — {reason}"))
        }
    }
}

/// ★B1②(codex W4): 디스크 팩 phoenix closure 전체 검증 — phoenix_embed_files(추출과 동일 단일 소스)의
/// 각 rel 이 <pack>/<rel> 로 존재하고 임베드 내용과 해시 일치해야 Ok. 부재/불일치=Err(어느 rel 인지 명시).
/// disk_phoenix = <pack>/bin/javis_phoenix.py → pack_dir = 그 조부모(bin 의 부모).
fn disk_fallback_verify(disk_phoenix: &std::path::Path) -> Result<(), String> {
    let pack_dir = disk_phoenix
        .parent()
        .and_then(|bin| bin.parent())
        .ok_or_else(|| "디스크 phoenix 경로에서 pack_dir 파생 실패".to_string())?;
    let files = phoenix_embed_files();
    if files.is_empty() {
        return Err("임베드 phoenix closure 비었음(빌드 이상)".to_string());
    }
    for (rel, content) in files {
        let path = pack_dir.join(rel);
        match std::fs::read_to_string(&path) {
            Ok(disk) => {
                if cys::pack::content_hash_pub(&disk) != cys::pack::content_hash_pub(content) {
                    return Err(format!("stale(해시 불일치): {rel}"));
                }
            }
            Err(_) => return Err(format!("부재/읽기실패: {rel}")),
        }
    }
    Ok(())
}

/// ★B1: 이전 실행의 잔여 phoenix-embed 디렉터리를 정리한다(크래시로 cleanup 못한 잔재 — temp 누수 방지).
fn prune_stale_phoenix_embed(state_dir: &std::path::Path) {
    let root = state_dir.join("phoenix-embed");
    if let Ok(rd) = std::fs::read_dir(&root) {
        for ent in rd.flatten() {
            let _ = std::fs::remove_dir_all(ent.path());
        }
    }
}

/// ★W1 재시도 지연(codex major test seam): 기본 60000ms. CYS_AUTORESTORE_RETRY_DELAY_MS 로 override —
/// 테스트가 sleep 0 으로 결정론 검증(1차 비0→2차 NOOP·중복 스폰 0, 5/6 무재시도)을 돌리게 한다.
fn autorestore_retry_delay() -> std::time::Duration {
    let ms = std::env::var("CYS_AUTORESTORE_RETRY_DELAY_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(60_000);
    std::time::Duration::from_millis(ms)
}

/// auto-restore 자식을 실행하고 exit 계약에 따라 처리한다. 비0(단 5·6 제외)은 delay 후 정확히 1회 재시도한다
/// — 재시도의 멱등성은 phoenix 의 lease·liveness 재산정에 맡긴다(수동 복원이 이미 끝났으면 재시도는 NOOP·중복 스폰 0).
fn loop_auto_restore(
    daemon: &std::sync::Arc<Daemon>,
    program: &str,
    args: &[String],
    env: &[(String, String)],
    log_path: &std::path::Path,
) {
    let daemon = daemon.clone();
    let program = program.to_string();
    let args = args.to_vec();
    let env = env.to_vec();
    let log_path = log_path.to_path_buf();
    loop_auto_restore_with(
        |_attempt| run_auto_restore_once(&daemon, &program, &args, &env, &log_path),
        autorestore_retry_delay(),
    );
}

/// ★재시도 결정 루프(test seam · 순수 로직 — 러너·지연 주입). 반환 = 실행 횟수(테스트 단언용).
/// exit 계약: 0=성공 종료 · 5(BREAKER)/6(CORRUPT·identity)=재시도 금지 · 그 외 비0/None=delay 후 1회 재시도.
fn loop_auto_restore_with<F>(mut run: F, retry_delay: std::time::Duration) -> u32
where
    F: FnMut(u32) -> Option<i32>,
{
    let mut attempt = 0u32;
    loop {
        let code = run(attempt);
        attempt += 1;
        match code {
            Some(0) => {
                eprintln!("[cysd] auto-restore finished (exit=0)");
                return attempt;
            }
            Some(5) => {
                eprintln!("[cysd] auto-restore BREAKER_OPEN (exit=5) — 재시도 금지(크래시루프 정지·사람 승인 필요)");
                return attempt;
            }
            Some(6) => {
                eprintln!(
                    "[cysd] auto-restore CORRUPT/identity (exit=6) — 재시도 금지(사람 개입 필요)"
                );
                return attempt;
            }
            other => {
                if attempt >= 2 {
                    eprintln!(
                        "[cysd] auto-restore finished (exit={other:?}) — 재시도 소진(1회). phoenix-restore.log 참조"
                    );
                    return attempt;
                }
                eprintln!(
                    "[cysd] auto-restore non-zero (exit={other:?}) — {}ms 후 1회 재시도 (lease/liveness 재산정에 위임)",
                    retry_delay.as_millis()
                );
                std::thread::sleep(retry_delay);
            }
        }
    }
}

/// ★W1 로그 대상 결정(codex major): phoenix-restore.log(primary) → temp_dir 폴백 → 둘 다 실패면 inherit.
/// null 로 떨어뜨리지 않는다 — 파일시스템/경로 실패가 진단 대상인데 그 순간 증거를 소실시키는 게 정확히 W1 관측성
/// 위반이므로, 최악이라도 자식 stdio 를 데몬 stderr 로 inherit 해 증거를 보존한다.
fn open_restore_log(log_path: &std::path::Path) -> Option<std::fs::File> {
    use std::io::Write;
    let open = |p: &std::path::Path| {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(p)
            .ok()
    };
    let f = open(log_path).or_else(|| {
        let tmp = std::env::temp_dir().join("cys-phoenix-restore.log");
        let alt = open(&tmp);
        if alt.is_some() {
            eprintln!(
                "[cysd] auto-restore primary log 실패({}) — temp 폴백 {}",
                log_path.display(),
                tmp.display()
            );
        }
        alt
    });
    if let Some(mut f) = f {
        let epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(
            f,
            "\n===== phoenix auto-restore @ epoch={epoch} (pid cysd={}) =====",
            std::process::id()
        );
        Some(f)
    } else {
        eprintln!(
            "[cysd] auto-restore log(primary+temp) 모두 open 실패 — 자식 stdio 를 데몬 stderr 로 inherit(증거 소실 방지)"
        );
        None
    }
}

/// ★P0-5 침묵사 차단(D3/W5): auto-restore 스레드 본문을 catch_unwind 로 감싸 panic 을 삼키지 않는다. panic 시
/// stderr + phoenix-restore.log 에 1회 기록하고 반환(스레드는 자연 종료 — 무한 재스폰 없음). 순수·테스트 가능:
/// 반환 true=정상 완료·false=panic 포착(테스트 단언용).
fn guard_restore_panic<F: FnOnce()>(log_path: &std::path::Path, body: F) -> bool {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)) {
        Ok(()) => true,
        Err(panic) => {
            let msg = panic
                .downcast_ref::<&str>()
                .map(|s| (*s).to_string())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown panic payload".to_string());
            eprintln!(
                "[cysd] ★auto-restore 스레드 panic 포착(P0-5 침묵사 차단·재스폰 안 함): {msg}"
            );
            // phoenix-restore.log 에도 남겨 관측성 확보(빈 로그 → panic 기록으로 원인 직결).
            if let Some(mut f) = open_restore_log(log_path) {
                use std::io::Write;
                let _ = writeln!(
                    f,
                    "[cysd] AUTO-RESTORE THREAD PANIC (P0-5 차단·재스폰 안 함): {msg}"
                );
            }
            false
        }
    }
}

/// 자식 1회 실행 — stdout/stderr 를 phoenix-restore.log(폴백 포함)에 append. exit code 반환(None=스폰 실패/대기 실패).
/// ★T6: status()→spawn()+wait() 전환. spawn 직후 무블로킹 최우선으로 pid+start_time 을 확보해,
/// Some(start_time)일 때만 RestoreRootGuard 로 restore_roots 에 등록한다(자식이 살아있는 동안만
/// authoritative 면제 창을 연다·게이트=handlers.rs). 관측 실패(None)면 bounded retry 후에도 None 이면
/// **등록 없이** 진행하고(면제 없음 — phoenix 2회 재시도 경로가 커버) 자식은 반드시 wait/reap 한다(좀비 0).
/// guard 는 함수 종료(정상·early return·panic unwind)에서 Drop 되어 등록을 해제한다. exit 매핑은
/// 기존 status().code() 계약과 동형(0/5/6/비0/None).
fn run_auto_restore_once(
    daemon: &std::sync::Arc<Daemon>,
    program: &str,
    args: &[String],
    env: &[(String, String)],
    log_path: &std::path::Path,
) -> Option<i32> {
    let mut cmd = std::process::Command::new(program);
    cmd.args(args).env("CYS_NO_AUTOSTART", "1");
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.stdin(std::process::Stdio::null());
    {
        // Windows: 콜드부트 auto-restore(launch-agent 등)가 수십 초 돌며 콘솔 창을 띄우지 않게.
        use crate::state::HideConsole;
        cmd.hide_console();
    }
    match open_restore_log(log_path) {
        Some(f) => {
            // stderr 는 clone 으로 같은 파일에 합류. clone 실패 시 null 이 아니라 inherit(증거 보존).
            match f.try_clone() {
                Ok(errf) => {
                    cmd.stdout(std::process::Stdio::from(f))
                        .stderr(std::process::Stdio::from(errf));
                }
                Err(e) => {
                    eprintln!("[cysd] auto-restore log stderr clone 실패({e}) — stderr inherit 폴백(null 금지)");
                    cmd.stdout(std::process::Stdio::from(f))
                        .stderr(std::process::Stdio::inherit());
                }
            }
        }
        None => {
            cmd.stdout(std::process::Stdio::inherit())
                .stderr(std::process::Stdio::inherit());
        }
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[cysd] auto-restore spawn failed: {e}");
            return None;
        }
    };
    let pid = child.id();
    // spawn 직후 다른 blocking 없이 최우선으로 start_time 확보(publication race 최소화·C2).
    // bounded retry(3회) — 갓 스폰된 자식이 프로세스표에 반영될 짧은 창을 흡수한다.
    let start_time = {
        let mut st = None;
        for _ in 0..3 {
            if let Some(s) = crate::state::peer_start_time(pid) {
                st = Some(s);
                break;
            }
        }
        st
    };
    // Some(start_time)일 때만 등록 — None 은 restore_roots 에 저장 금지(면제 없음·fail-safe).
    let _guard = start_time.map(|s| crate::state::RestoreRootGuard::new(daemon.clone(), pid, s));
    match child.wait() {
        Ok(s) => Some(s.code().unwrap_or(-1)),
        Err(e) => {
            eprintln!("[cysd] auto-restore wait failed: {e}");
            None
        }
    }
}

/// T1-3: UDS peer pid 조회 — macOS LOCAL_PEERPID, Linux SO_PEERCRED.
#[cfg(unix)]
fn peer_pid(stream: &tokio::net::UnixStream) -> Option<u32> {
    use std::os::unix::io::AsRawFd;
    let fd = stream.as_raw_fd();
    #[cfg(target_os = "macos")]
    {
        const SOL_LOCAL: libc::c_int = 0;
        const LOCAL_PEERPID: libc::c_int = 0x002;
        let mut pid: libc::pid_t = 0;
        let mut len = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
        let r = unsafe {
            libc::getsockopt(
                fd,
                SOL_LOCAL,
                LOCAL_PEERPID,
                &mut pid as *mut _ as *mut libc::c_void,
                &mut len,
            )
        };
        if r == 0 && pid > 0 {
            return Some(pid as u32);
        }
        None
    }
    #[cfg(target_os = "linux")]
    {
        let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        let r = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                &mut cred as *mut _ as *mut libc::c_void,
                &mut len,
            )
        };
        if r == 0 && cred.pid > 0 {
            return Some(cred.pid as u32);
        }
        None
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = fd;
        None
    }
}

/// Windows accept_loop가 `connect()` 오류 후 같은 broken 인스턴스에 곧장 재시도하다
/// 100% CPU로 spin하지 않도록 두는 backoff. mio `ConnectNamedPipe`는 정상 대기는
/// WouldBlock(→tokio가 await)으로, 진짜 OS 오류는 즉시 Err로 반환하므로(connecting 플래그도
/// 즉시 해제 → self-throttle 없음), 오류 분기는 ①로그 ②인스턴스 재생성 ③이 짧은 sleep로
/// 회생해야 Unix arm(accept err→다음 await)·tokio 표준 루프(?로 전파)와 대칭이 된다.
/// (Windows arm은 이 호스트에서 컴파일/실행 불가하므로, 정책 값을 모듈 최상위로 빼
///  비-Windows 테스트가 'spin 방지=non-zero backoff' 불변을 박제하게 한다.)
#[cfg_attr(not(windows), allow(dead_code))]
const PIPE_ACCEPT_ERROR_BACKOFF: std::time::Duration = std::time::Duration::from_millis(100);

/// Windows named pipe 리스너 풀 크기 — UDS listen backlog 의 대응물. named pipe 엔 backlog 가
/// 없어 '여분 listening 인스턴스 수'가 곧 동시 접속 수용량이다. 1이면 accept→인스턴스 재생성
/// 사이 창(tokio 스케줄링 지연 포함)에 도착한 동시 접속이 전부 ERROR_PIPE_BUSY(os error 231,
/// "모든 파이프 인스턴스가 사용 중")로 튕긴다 — 멀티 노드(master·cso·worker·reviewer 동시 RPC)
/// + GUI 기동 fan-out(daemon_status·pane attach·event forwarder)에서 상시 재현
/// (2026-07-10 Windows 실사고: GUI "startup failed … os error 231"). 클라이언트 busy-retry 와
/// 이중 방어. (Windows arm 은 이 호스트에서 컴파일/실행 불가하므로, 정책 값을 모듈 최상위로 빼
///  비-Windows 테스트가 '풀 ≥ 2' 불변을 박제하게 한다 — PIPE_ACCEPT_ERROR_BACKOFF 와 같은 방식.)
#[cfg_attr(not(windows), allow(dead_code))]
const PIPE_LISTENER_POOL: usize = 8;

/// owner-only DACL의 SDDL: D:P=보호된(상속차단) DACL, FA=full access를
/// OW(OWNER_RIGHTS=creator)·SY(SYSTEM)·BA(BUILTIN\Administrators)에게만 부여.
/// WD(Everyone)·AU(Authenticated Users) 같은 광역 SID가 없어 같은 머신의 임의 사용자를 배제한다.
/// (cfg(windows) 밖에서도 회귀 테스트가 참조할 수 있게 모듈 최상위 const로 둔다.
///  비-Windows 비-test 빌드에서는 실사용처가 없으므로 dead_code를 명시 허용한다.)
#[cfg_attr(not(windows), allow(dead_code))]
const PIPE_SDDL_OWNER_ONLY: &str = "D:P(A;;FA;;;OW)(A;;FA;;;SY)(A;;FA;;;BA)";

/// Windows named pipe 보안 디스크립터: 소유자(creator)·SYSTEM·Administrators에게만
/// full access를 허용하는 owner-only DACL(PIPE_SDDL_OWNER_ONLY)을 SECURITY_ATTRIBUTES에 싣는다.
/// UDS 0o700 dir + 0o600 소켓의 단일-UID 봉인과 대칭 — 같은 머신의 임의 로컬 사용자가
/// 인증 없는 제어 채널(send_text·send_key·ledger.kill)에 접근하는 권한 우회를 차단한다.
/// 반환된 PSECURITY_DESCRIPTOR는 LocalFree로 해제해야 하므로, RAII 가드로 SA와 함께 수명을 묶는다.
#[cfg(windows)]
struct OwnerOnlySecurity {
    sa: windows_sys::Win32::Security::SECURITY_ATTRIBUTES,
    psd: windows_sys::Win32::Security::PSECURITY_DESCRIPTOR,
}

#[cfg(windows)]
impl OwnerOnlySecurity {
    fn new() -> Option<Self> {
        use windows_sys::Win32::Security::Authorization::{
            ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
        };
        use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
        // 와이드 널종단 SDDL 문자열
        let sddl: Vec<u16> = PIPE_SDDL_OWNER_ONLY
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let mut psd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut psd,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 || psd.is_null() {
            return None;
        }
        let sa = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: psd,
            bInheritHandle: 0,
        };
        Some(Self { sa, psd })
    }

    /// create_with_security_attributes_raw에 넘길 *mut SECURITY_ATTRIBUTES (가드 수명 동안 유효).
    fn as_ptr(&self) -> *mut std::ffi::c_void {
        &self.sa as *const _ as *mut std::ffi::c_void
    }
}

#[cfg(windows)]
impl Drop for OwnerOnlySecurity {
    fn drop(&mut self) {
        // ConvertString…가 LocalAlloc로 잡은 SD를 해제 (가드가 데몬 수명 동안 살아있으므로
        // 실무상 프로세스 종료 시점에만 호출되나, 누수 방지를 위해 명시 해제).
        unsafe {
            windows_sys::Win32::Foundation::LocalFree(self.psd as *mut _);
        }
    }
}

/// named pipe listening 인스턴스 1개 생성. owner-only DACL 은 인스턴스마다 새로 변환한다 —
/// 커널이 CreateNamedPipe 시점에 SD 를 파이프 객체로 복사하므로 SECURITY_ATTRIBUTES 는 이 호출
/// 동안만 살아있으면 되고(호출 후 drop 안전), 가드를 태스크 간 공유하지 않아 리스너 풀의
/// spawn(Send 경계)과 충돌하지 않는다. SDDL 변환 실패(이론상 거의 없음)면 null 폴백 + 경고
/// — 기존 accept_loop 폴백 정책 그대로.
#[cfg(windows)]
fn create_pipe_instance(
    pipe_name: &str,
    first: bool,
) -> std::io::Result<tokio::net::windows::named_pipe::NamedPipeServer> {
    use tokio::net::windows::named_pipe::ServerOptions;
    let security = OwnerOnlySecurity::new();
    if security.is_none() {
        eprintln!(
            "warning: failed to build owner-only pipe security descriptor; \
             falling back to default DACL (any local user may connect)"
        );
    }
    let sa_ptr = security
        .as_ref()
        .map(|s| s.as_ptr())
        .unwrap_or(std::ptr::null_mut());
    // Safety: sa_ptr는 null이거나 `security` 가드가 소유한 유효한 SECURITY_ATTRIBUTES를 가리키며,
    // 그 가드는 이 함수 끝까지 살아있어 파이프 생성 호출보다 오래 산다.
    unsafe {
        ServerOptions::new()
            .first_pipe_instance(first)
            .create_with_security_attributes_raw(pipe_name, sa_ptr)
    }
}

/// listening 인스턴스 재생성 — 실패 시 backoff 후 무한 재시도(리스너 태스크 침묵사 방지).
/// 과거 `.expect()` panic 은 메인 accept 태스크에선 데몬 전사(fail-fast·Task Scheduler 재기동)
/// 였지만, 풀의 spawn 태스크에선 tokio 가 panic 을 삼켜 리스너만 조용히 줄어드는 최악
/// (전 리스너 소진 = 무증상 접속 불능)이 된다 — 로그 + 재시도가 정직하다.
#[cfg(windows)]
async fn recreate_pipe_instance(
    pipe_name: &str,
) -> tokio::net::windows::named_pipe::NamedPipeServer {
    loop {
        match create_pipe_instance(pipe_name, false) {
            Ok(s) => return s,
            Err(e) => {
                eprintln!("recreate pipe {pipe_name} failed: {e} — retrying");
                tokio::time::sleep(PIPE_ACCEPT_ERROR_BACKOFF).await;
            }
        }
    }
}

/// 리스너 풀의 태스크 1개 — 자기 listening 인스턴스로 accept 루프를 돈다.
#[cfg(windows)]
async fn pipe_listener(
    daemon: Arc<Daemon>,
    pipe_name: String,
    mut server: tokio::net::windows::named_pipe::NamedPipeServer,
) {
    loop {
        match server.connect().await {
            Ok(()) => {
                // 접속 완료된 클라이언트를 먼저 서빙한다 — 재생성(recreate)을 앞에 두면 재생성이
                // 실패를 반복하는 비정상 상태에서 '이미 accept 된' 클라이언트까지 무기한 기아가
                // 된다(liveness 역전). 재생성 지연으로 listening 정원이 잠깐 N-1이 되는 것은
                // 나머지 리스너 + 클라이언트 busy-retry 가 흡수한다.
                let connected = server;
                // 발신자 신원: 커널이 보증하는 named pipe 클라이언트 pid (UDS peer_pid와 대칭).
                // 박는 이유: claim_role·surface.close·status.set 등은 발신 신원이 None이면 무조건
                // 거부하므로, 미구현(None)이면 Windows에서 자기 surface 자가-claim('cys claim-role
                // master' 등 launch-agent 밖 직접 기동 노드)이 영영 막힌다. boxing 전에 조회한다.
                let caller_pid = peer_pid(&connected);
                let handler_daemon = Arc::clone(&daemon);
                tokio::spawn(async move {
                    handle_connection(handler_daemon, Box::new(connected) as Stream, caller_pid)
                        .await;
                });
                server = recreate_pipe_instance(&pipe_name).await;
            }
            Err(e) => {
                // connect()가 즉시 Err를 반환하면(broken 핸들 등) 같은 인스턴스에 곧장
                // 재시도해도 같은 Err가 무한 반복돼 100% CPU spin이 된다(mio가 connecting
                // 플래그를 즉시 해제해 self-throttle도 없음). Unix arm(accept err→다음 await)·
                // tokio 표준 루프(?로 전파)와 대칭이 되도록: ①로그 ②인스턴스 재생성 ③짧은 backoff.
                eprintln!("accept error: {e}");
                server = recreate_pipe_instance(&pipe_name).await;
                tokio::time::sleep(PIPE_ACCEPT_ERROR_BACKOFF).await;
            }
        }
    }
}

#[cfg(windows)]
async fn accept_loop(
    daemon: Arc<Daemon>,
    socket_path: &std::path::Path,
    first: tokio::net::windows::named_pipe::NamedPipeServer,
) {
    let pipe_name = socket_path.to_string_lossy().into_owned();
    // ★W1-c: 첫 인스턴스(= 데몬 싱글턴 가드)는 main()에서 부트 부수효과보다 먼저 조기 선점해 넘겨받는다.
    // 여기서 다시 만들지 않고 그대로 리스너 풀에 편입한다(probe-후-close-재open 레이스 제거·경쟁 패자는
    // main()의 선점 실패 지점에서 이미 즉사).
    // ★P0-7 최종 층위(D1/W5): 파이프 listening 직후 공통 부트 — unix accept_loop 와 **동일 함수**(prune +
    //   콜드부트 auto-restore). 과거 이 호출이 Windows 에만 빠져 auto-restore 가 발동조차 안 하고 phoenix-restore.log
    //   가 빈 파일이던 결함(CI 실경로 스모크 ⑧)을 봉인. state_dir 은 함수 내부 canonical 매핑(Windows 슬러그).
    post_listen_boot(socket_path, &daemon);
    // ★리스너 풀(PIPE_LISTENER_POOL): listening 인스턴스 N개를 병렬 대기 — 단일 인스턴스의
    // accept→재생성 사이 창에서 동시 접속이 ERROR_PIPE_BUSY(231)로 튕기던 결함 봉인(상수 주석 참조).
    let mut first = Some(first);
    let mut tasks = Vec::new();
    for _ in 0..PIPE_LISTENER_POOL {
        let server = match first.take() {
            Some(s) => s,
            None => recreate_pipe_instance(&pipe_name).await,
        };
        tasks.push(tokio::spawn(pipe_listener(
            Arc::clone(&daemon),
            pipe_name.clone(),
            server,
        )));
    }
    // accept_loop 는 반환하지 않는 계약(unix arm 대칭) — 리스너 태스크들을 영구 대기한다.
    for t in tasks {
        let _ = t.await;
    }
}

/// Windows named pipe 클라이언트 pid 조회 — UDS peer_pid(macOS LOCAL_PEERPID/Linux SO_PEERCRED)와
/// 대칭. GetNamedPipeClientProcessId는 서버 측 핸들에서 연결된 클라이언트 프로세스 id를 돌려준다.
/// 실패(0 반환 또는 pid 0)면 None — 호출부는 UDS와 동일하게 익명 발신으로 처리한다.
#[cfg(windows)]
fn peer_pid(pipe: &tokio::net::windows::named_pipe::NamedPipeServer) -> Option<u32> {
    use std::os::windows::io::AsRawHandle;
    let mut pid: u32 = 0;
    let ok = unsafe {
        windows_sys::Win32::System::Pipes::GetNamedPipeClientProcessId(
            pipe.as_raw_handle() as windows_sys::Win32::Foundation::HANDLE,
            &mut pid,
        )
    };
    if ok != 0 && pid != 0 {
        Some(pid)
    } else {
        None
    }
}

/// 개행 없는 무한 스트림이 데몬 메모리를 잠식하지 못하게 줄 길이 상한을 둔 line reader.
async fn next_line_capped<R: tokio::io::AsyncBufRead + Unpin>(
    r: &mut R,
    cap: usize,
) -> std::io::Result<Option<String>> {
    let mut buf: Vec<u8> = Vec::new();
    loop {
        let available = r.fill_buf().await?;
        if available.is_empty() {
            return Ok(if buf.is_empty() {
                None
            } else {
                Some(String::from_utf8_lossy(&buf).into_owned())
            });
        }
        if let Some(pos) = available.iter().position(|&b| b == b'\n') {
            buf.extend_from_slice(&available[..pos]);
            r.consume(pos + 1);
            return Ok(Some(String::from_utf8_lossy(&buf).into_owned()));
        }
        let n = available.len();
        buf.extend_from_slice(available);
        r.consume(n);
        if buf.len() > cap {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "request line too long",
            ));
        }
    }
}

const MAX_REQUEST_LINE: usize = 10 * 1024 * 1024; // 지침 주입(수백 KB)에 충분한 10MB

/// `cancellation`은 **호출자(handle_connection)가 소유**한다 — 요청 주체의 연결이 죽으면 호출자가
/// 이 플래그를 set 해 진행 중인 ensure 워커를 취소한다(WS-0-③). 등록되는 요청은 이 동일한 Arc를
/// 레지스트리에 넣으므로 `browser.runtime.cancel`의 명시 취소와 **같은 플래그**를 공유한다
/// (취소 경로 이원화 금지 — 새 경로를 만들지 않고 기존 배선을 그대로 쓴다).
async fn dispatch_request_isolated(
    daemon: Arc<Daemon>,
    req: Request,
    caller_pid: Option<u32>,
    cancellation: Arc<std::sync::atomic::AtomicBool>,
) -> Reply {
    if !req.method.starts_with("browser.runtime.") {
        return handlers::dispatch(&daemon, req, caller_pid);
    }
    let id = req.id.clone();
    if req.method == "browser.runtime.cancel" {
        let caller_pid = match caller_pid.filter(|pid| *pid != 0) {
            Some(pid) => pid,
            None => {
                return Reply::Single(cys::err_response(
                    &id,
                    "AUTHORITY_REJECTED",
                    "Browser cancellation requires a kernel-authenticated peer",
                ))
            }
        };
        let pane_nonce = req
            .params
            .get("pane_nonce")
            .and_then(serde_json::Value::as_str);
        let request_id = req
            .params
            .get("request_id")
            .and_then(serde_json::Value::as_str);
        let (Some(pane_nonce), Some(request_id)) = (pane_nonce, request_id) else {
            return Reply::Single(cys::err_response(
                &id,
                "INVALID_PARAMS",
                "Browser cancellation requires pane_nonce and request_id",
            ));
        };
        if !valid_browser_request_credential(pane_nonce)
            || !valid_browser_request_credential(request_id)
        {
            return Reply::Single(cys::err_response(
                &id,
                "INVALID_PARAMS",
                "Browser cancellation identity must be 32-byte lowercase hex",
            ));
        }
        let cancelled = browser_request_registry().cancel(caller_pid, pane_nonce, request_id);
        return Reply::Single(cys::ok_response(
            &id,
            serde_json::json!({"cancelled": cancelled}),
        ));
    }
    let request_guard = if req.method == "browser.runtime.ensure"
        && req
            .params
            .get("authority_kind")
            .and_then(serde_json::Value::as_str)
            == Some("user_gesture")
    {
        let caller_pid = match caller_pid.filter(|pid| *pid != 0) {
            Some(pid) => pid,
            None => {
                return Reply::Single(cys::err_response(
                    &id,
                    "AUTHORITY_REJECTED",
                    "Browser launch requires a kernel-authenticated GUI peer",
                ))
            }
        };
        let pane_nonce = req
            .params
            .get("pane_nonce")
            .and_then(serde_json::Value::as_str);
        let request_id = req
            .params
            .get("request_id")
            .and_then(serde_json::Value::as_str);
        let (Some(pane_nonce), Some(request_id)) = (pane_nonce, request_id) else {
            return Reply::Single(cys::err_response(
                &id,
                "INVALID_PARAMS",
                "GUI Browser launch requires pane_nonce and request_id",
            ));
        };
        if !valid_browser_request_credential(pane_nonce)
            || !valid_browser_request_credential(request_id)
        {
            return Reply::Single(cys::err_response(
                &id,
                "INVALID_PARAMS",
                "Browser launch identity must be 32-byte lowercase hex",
            ));
        }
        match browser_request_registry().register(
            caller_pid,
            pane_nonce.to_string(),
            request_id.to_string(),
            cancellation.clone(),
        ) {
            Ok(guard) => Some(guard),
            Err(error) => {
                return Reply::Single(cys::err_response(&id, "BROWSER_REQUEST_CONFLICT", &error))
            }
        }
    } else {
        None
    };
    let timeout = if req.method == "browser.runtime.operation" {
        std::time::Duration::from_secs(70)
    } else {
        cys::browser_runtime::ENSURE_WORKER_DEADLINE
    };
    // 등록 요청이든 아니든 **호출자가 준 동일 플래그**를 쓴다 — 연결 소멸(WS-0-③)은 등록되지 않는
    // 브라우저 verb(operation 등)에도 똑같이 취소로 작용해야 한다.
    match run_browser_job_bounded_with_cancellation(
        browser_job_gate(),
        timeout,
        cancellation,
        move |cancelled| {
            let _request_guard = request_guard;
            authority_broker::with_browser_cancellation(cancelled, || {
                handlers::dispatch(&daemon, req, caller_pid)
            })
        },
    )
    .await
    {
        Ok(reply) => reply,
        Err(error) if error == "backpressure" => Reply::Single(cys::err_response(
            &id,
            "BROWSER_BACKPRESSURE",
            "another Browser RPC is already in flight; retry after it completes",
        )),
        Err(error) if error != "timeout" => Reply::Single(cys::err_response(
            &id,
            "BROWSER_WORKER_FAILED",
            &format!("Browser RPC worker failed: {error}"),
        )),
        Err(_) => Reply::Single(cys::err_response(
            &id,
            "BROWSER_TIMEOUT",
            "Browser RPC exceeded its bounded worker deadline",
        )),
    }
}

fn valid_browser_request_credential(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct BrowserRequestKey {
    caller_pid: u32,
    pane_nonce: String,
    request_id: String,
}

#[derive(Default)]
struct BrowserRequestRegistry {
    requests: std::sync::Mutex<
        std::collections::HashMap<BrowserRequestKey, Arc<std::sync::atomic::AtomicBool>>,
    >,
}

struct BrowserRequestGuard {
    registry: Arc<BrowserRequestRegistry>,
    key: BrowserRequestKey,
    cancellation: Arc<std::sync::atomic::AtomicBool>,
}

impl BrowserRequestRegistry {
    /// `cancellation`은 호출자 소유 플래그다(WS-0-③ — 연결 소멸 감지가 같은 Arc를 set 한다).
    /// 레지스트리에 이 Arc를 그대로 넣어 `cancel()`의 명시 취소와 단일 플래그를 공유한다.
    fn register(
        self: &Arc<Self>,
        caller_pid: u32,
        pane_nonce: String,
        request_id: String,
        cancellation: Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<BrowserRequestGuard, String> {
        if caller_pid == 0
            || !valid_browser_request_credential(&pane_nonce)
            || !valid_browser_request_credential(&request_id)
        {
            return Err("invalid Browser request identity".into());
        }
        let key = BrowserRequestKey {
            caller_pid,
            pane_nonce,
            request_id,
        };
        let mut requests = self.requests.lock().unwrap();
        if requests.contains_key(&key) {
            return Err("duplicate Browser request identity".into());
        }
        requests.insert(key.clone(), cancellation.clone());
        drop(requests);
        Ok(BrowserRequestGuard {
            registry: self.clone(),
            key,
            cancellation,
        })
    }

    fn cancel(&self, caller_pid: u32, pane_nonce: &str, request_id: &str) -> bool {
        let key = BrowserRequestKey {
            caller_pid,
            pane_nonce: pane_nonce.to_string(),
            request_id: request_id.to_string(),
        };
        let cancellation = self.requests.lock().unwrap().get(&key).cloned();
        if let Some(cancellation) = cancellation {
            cancellation.store(true, std::sync::atomic::Ordering::SeqCst);
            true
        } else {
            false
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

impl Drop for BrowserRequestGuard {
    fn drop(&mut self) {
        let mut requests = self.registry.requests.lock().unwrap();
        let remove = requests
            .get(&self.key)
            .is_some_and(|current| Arc::ptr_eq(current, &self.cancellation));
        if remove {
            requests.remove(&self.key);
        }
    }
}

fn browser_request_registry() -> &'static Arc<BrowserRequestRegistry> {
    static REGISTRY: std::sync::OnceLock<Arc<BrowserRequestRegistry>> = std::sync::OnceLock::new();
    REGISTRY.get_or_init(|| Arc::new(BrowserRequestRegistry::default()))
}

struct BrowserJobGate {
    permits: Arc<tokio::sync::Semaphore>,
}

impl BrowserJobGate {
    fn new(max_in_flight: usize) -> Self {
        Self {
            permits: Arc::new(tokio::sync::Semaphore::new(max_in_flight)),
        }
    }
}

fn browser_job_gate() -> &'static BrowserJobGate {
    static GATE: std::sync::OnceLock<BrowserJobGate> = std::sync::OnceLock::new();
    GATE.get_or_init(|| BrowserJobGate::new(1))
}

// ※ 취소 플래그 없는 편의 래퍼 `run_browser_job_bounded`는 삭제했다. WS-0-3이 프로덕션 호출부를
//   `run_browser_job_bounded_with_cancellation`(연결 소멸 감시 포함)로 옮기면서 래퍼는 **테스트만
//   부르는 죽은 코드**가 됐고, 그 상태로 두면 "프로덕션이 쓰지 않는 함수를 4종 테스트가 검증"하는
//   가짜 커버리지가 된다. 테스트는 이제 아래 프로덕션 함수를 직접 호출한다.

async fn run_browser_job_bounded_with_cancellation<T, F>(
    gate: &BrowserJobGate,
    timeout: std::time::Duration,
    cancelled: Arc<std::sync::atomic::AtomicBool>,
    job: F,
) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(Arc<std::sync::atomic::AtomicBool>) -> T + Send + 'static,
{
    let permit = gate
        .permits
        .clone()
        .try_acquire_owned()
        .map_err(|_| "backpressure".to_string())?;
    let worker_cancelled = cancelled.clone();
    let mut task = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        job(worker_cancelled)
    });
    match tokio::time::timeout(timeout, &mut task).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(error.to_string()),
        Err(_) => {
            cancelled.store(true, std::sync::atomic::Ordering::SeqCst);
            Err("timeout".into())
        }
    }
}

#[cfg(all(test, unix))]
mod startup_lock_retry_tests {
    use super::*;
    use std::os::unix::io::AsRawFd;
    use std::time::Duration;

    #[test]
    fn default_schedule_exceeds_one_second_and_backs_off() {
        let s = schedule_for_budget(1550);
        let ms: Vec<u64> = s.iter().map(|d| d.as_millis() as u64).collect();
        assert_eq!(ms, vec![50, 100, 200, 400, 800], "지수 백오프");
        let total: u64 = ms.iter().sum();
        assert!(
            total >= 1000,
            "총 예산 ≥1s여야 doctor의 순간 락 보유를 흡수한다(실제 {total}ms)"
        );
    }

    #[test]
    fn budget_knob_is_injectable_for_deterministic_tests() {
        assert!(
            schedule_for_budget(0).is_empty(),
            "예산 0 = 재시도 없음(구 동작 재현용)"
        );
        let s = schedule_for_budget(120);
        assert_eq!(
            s.iter().map(|d| d.as_millis() as u64).sum::<u64>(),
            120,
            "예산을 넘지 않는다"
        );
    }

    #[test]
    fn jitter_stays_within_twenty_percent() {
        for _ in 0..200 {
            let d = jittered(Duration::from_millis(100));
            assert!(
                (80..=120).contains(&(d.as_millis() as u64)),
                "±20% 밖: {d:?}"
            );
        }
        assert_eq!(jittered(Duration::from_millis(1)).as_millis(), 1, "미세값 무변");
    }

    // ───────────── WS-7 수용 기준: 데몬 부팅 ↔ doctor 동시 실행 통합 테스트 ─────────────
    //
    // 기획서 §4 WS-7 / 설계 §9-1이 명령한 상호작용 검증이다. 검증 대상은 **프로덕션
    // `acquire_startup_lock` 그 자체**이며, 테스트가 재시도 루프를 재구현하지 않는다
    // (종전 `retry_loop_wins_the_lock_a_doctor_briefly_held`는 `try_flock`+백오프를 테스트 안에서
    //  다시 짜, `acquire_startup_lock`에서 재시도를 통째로 지워도 초록이었다 = 커버리지 0).
    //
    // 구조와 그 근거:
    //   · 부팅 데몬 = **자식 프로세스**(이 테스트 바이너리 재실행 → `boot_lock_child_entrypoint`).
    //     실패 경로가 `std::process::exit(1)`이라 in-process로는 "오사유 exit이 나지 않았다"를
    //     단언할 수 없다(하니스째 죽어 단언 지점에 도달조차 못 한다). 자식이라야 exit code와
    //     stderr를 **증거로** 검사할 수 있다.
    //   · doctor = 가능하면 **진짜 `cys` 바이너리**(`cys doctor` + `CYS_DOCTOR_LOCK_HOLD_MS`).
    //     게이트 명령이 `cargo build --bin cysd --bin cys`를 선행하므로 평시 이 경로를 탄다.
    //     바이너리가 없으면 같은 보유 구간을 내는 스레드 픽스처로 낙하한다 — **검증의 핵심
    //     단언(프로덕션 락 획득 성공/실패)은 두 모드에서 동일하며, 어떤 경우에도 skip하지 않는다.**
    //   · 결정론화는 벽시계 경합이 아니라 **노브 주입**이다(설계 §10-1-A-5): 보유 1500ms ≪ 재시도
    //     예산 5000ms로 고정하면 성패가 스케줄러 운에 좌우되지 않는다.
    //   · 음성 대조(`CYS_LOCK_RETRY_MS=0`)를 함께 돌려 **재시도가 없으면 정확히 그 사고
    //     (`dead-holder-reclaim-failed` 오사유 exit(1))가 재현됨**을 증명한다. 이 대조가 없으면
    //     "재시도가 실제로 일하는가"를 아무도 확인하지 않은 채 초록만 본다.

    /// 부팅 데몬 역할의 자식 프로세스 진입점 — 부모가 재실행으로만 구동한다(단독 실행은 no-op).
    #[test]
    #[ignore = "부모 통합 테스트가 재실행으로 구동한다"]
    fn boot_lock_child_entrypoint() {
        let Ok(dir) = std::env::var("CYS_TEST_BOOT_LOCK_DIR") else {
            return; // 환경변수 없이 우연히 실행돼도 아무 일도 하지 않는다(재귀·오염 방지).
        };
        let dir = std::path::PathBuf::from(dir);
        let t0 = std::time::Instant::now();
        // ★프로덕션 함수를 그대로 호출한다. 여기서 실패하면 이 프로세스가 exit(1)로 죽고,
        //   부모가 그 exit code와 stderr를 증거로 읽는다.
        match acquire_startup_lock(&dir.join("cys.lock"), &dir.join("cys.sock"), &dir) {
            Some(_held) => println!("BOOT-LOCK-ACQUIRED elapsed_ms={}", t0.elapsed().as_millis()),
            None => println!("BOOT-LOCK-NONE"),
        }
    }

    /// 같은 target 디렉토리의 `cys` 바이너리(진짜 doctor). 없으면 None → 스레드 픽스처로 낙하.
    fn sibling_cys_binary() -> Option<std::path::PathBuf> {
        if let Ok(p) = std::env::var("CYS_TEST_CYS_BIN") {
            let p = std::path::PathBuf::from(p);
            if p.exists() {
                return Some(p);
            }
        }
        // 테스트 바이너리는 target/<profile>/deps/cysd-<hash> — 두 단계 위가 target/<profile>.
        let exe = std::env::current_exe().ok()?;
        let profile_dir = exe.parent()?.parent()?;
        let p = profile_dir.join(if cfg!(windows) { "cys.exe" } else { "cys" });
        p.exists().then_some(p)
    }

    /// doctor 역할로 startup 락을 `hold_ms` 동안 붙잡는다. 반환값을 drop할 때까지 유효.
    enum LockHolder {
        RealDoctor(std::process::Child),
        Fixture(Option<std::thread::JoinHandle<()>>),
    }

    impl LockHolder {
        fn start(dir: &std::path::Path, lock: &std::path::Path, hold_ms: u64) -> (Self, bool) {
            if let Some(cys) = sibling_cys_binary() {
                // 진짜 `cys doctor`: 소켓 파일이 없으므로 socket 진단은 즉시 통과하고,
                // startup-lock 진단이 flock을 쥔 채 CYS_DOCTOR_LOCK_HOLD_MS 만큼 머문다.
                let child = std::process::Command::new(cys)
                    .arg("doctor")
                    .env("CYS_SOCKET", dir.join("cys.sock"))
                    .env("CYS_PACK_DIR", dir.join("pack"))
                    .env("CYS_NO_AUTOSTART", "1")
                    .env("CYS_DOCTOR_LOCK_HOLD_MS", hold_ms.to_string())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn();
                if let Ok(child) = child {
                    return (LockHolder::RealDoctor(child), true);
                }
            }
            let lock = lock.to_path_buf();
            let handle = std::thread::spawn(move || {
                let f = std::fs::OpenOptions::new()
                    .create(true)
                    .truncate(false)
                    .write(true)
                    .open(&lock)
                    .expect("픽스처 홀더가 락 파일을 열지 못했다");
                assert_eq!(
                    unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
                    0,
                    "픽스처 홀더가 락을 잡지 못했다"
                );
                std::thread::sleep(Duration::from_millis(hold_ms));
                drop(f);
            });
            (LockHolder::Fixture(Some(handle)), false)
        }

        fn finish(self) {
            match self {
                LockHolder::RealDoctor(mut c) => {
                    let _ = c.wait();
                }
                LockHolder::Fixture(h) => {
                    if let Some(h) = h {
                        let _ = h.join();
                    }
                }
            }
        }
    }

    /// 락이 실제로 누군가에게 잡힐 때까지 유계 대기(최대 5s). 잡히면 true.
    fn wait_until_locked(lock: &std::path::Path) -> bool {
        for _ in 0..250 {
            if let Ok(f) = std::fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .open(lock)
            {
                let got = unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0;
                if got {
                    // 프로브가 잡아버렸다 — 즉시 놓아 본 시나리오를 방해하지 않는다.
                    unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_UN) };
                } else {
                    return true;
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    fn boot_lock_scenario_dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "cysd-bootlock-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        // 홀더 pid = 1(init/launchd): **살아있지만 cysd가 아니다**. 재시도가 없으면 데드맨이
        // Dead로 오판(무응답 소켓 + heartbeat 부재=stale) → 회수 시도 → pid_is_cysd(1)=false로
        // 거부 → `dead-holder-reclaim-failed` 오사유 exit(1). 즉 원사고를 정확히 재현하는 값이다.
        // (거부 경로라 pid 1에는 어떤 시그널도 전송되지 않는다 — reclaim_from_dead_holder 참조.)
        std::fs::write(d.join("cys.lock"), b"1").unwrap();
        d
    }

    fn run_boot_child(dir: &std::path::Path, retry_ms: u64) -> std::process::Output {
        std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "startup_lock_retry_tests::boot_lock_child_entrypoint",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("CYS_TEST_BOOT_LOCK_DIR", dir)
            .env("CYS_LOCK_RETRY_MS", retry_ms.to_string())
            .env_remove("CYS_TEST_CYS_BIN")
            .output()
            .expect("부팅 자식 프로세스 실행")
    }

    #[test]
    fn boot_daemon_wins_the_lock_while_doctor_holds_it() {
        const HOLD_MS: u64 = 1500; // doctor 보유 구간(노브 주입)
        const RETRY_MS: u64 = 5000; // 부팅 데몬 재시도 예산(노브 주입) — 보유 구간을 크게 상회

        // ── ① 본 시나리오: doctor가 락을 쥔 채로 데몬이 부팅한다 → 재시도로 획득해 성공해야 한다.
        let dir = boot_lock_scenario_dir("win");
        let lock = dir.join("cys.lock");
        let (holder, real_doctor) = LockHolder::start(&dir, &lock, HOLD_MS);
        assert!(
            wait_until_locked(&lock),
            "홀더가 락을 잡지 못했다 — 시나리오 전제 붕괴(real_doctor={real_doctor})"
        );
        let out = run_boot_child(&dir, RETRY_MS);
        holder.finish();

        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        assert!(
            out.status.success(),
            "doctor가 락을 쥔 순간 부팅한 데몬이 죽었다(real_doctor={real_doctor})\n\
             status={:?}\nstdout={stdout}\nstderr={stderr}",
            out.status.code()
        );
        assert!(
            stdout.contains("BOOT-LOCK-ACQUIRED"),
            "부팅 데몬이 락을 획득하지 못했다\nstdout={stdout}\nstderr={stderr}"
        );
        assert!(
            !stderr.contains("dead-holder-reclaim-failed"),
            "오사유(dead-holder-reclaim-failed)로 판정했다 — WS-7이 없애려던 바로 그 경로\nstderr={stderr}"
        );
        // 재시도가 실제로 돌았다는 증거: 즉시 획득이었다면 경과가 0에 수렴한다.
        let elapsed_ms: u64 = stdout
            .split("elapsed_ms=")
            .nth(1)
            .and_then(|s| s.split_whitespace().next())
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or_else(|| panic!("자식이 경과를 보고하지 않았다: {stdout}"));
        assert!(
            elapsed_ms >= 100,
            "백오프 재시도를 거치지 않고 즉시 획득했다({elapsed_ms}ms) — 경합이 재현되지 않았다"
        );
        // 획득 뒤에는 `claim_lock`이 자기 pid를 기록하고 heartbeat를 touch한다 — 데드맨이 다음
        // 경합에서 홀더를 식별할 수 있어야 인수 계약이 성립한다(획득만 하고 대장을 안 남기면
        // 다음 부팅이 FailClosed로 굳는다).
        let recorded = std::fs::read_to_string(&lock).unwrap();
        let recorded: u32 = recorded.trim().parse().expect("락파일에 홀더 pid가 기록돼야 한다");
        assert_ne!(recorded, 1, "홀더 pid가 자식 프로세스 pid로 갱신돼야 한다");
        assert!(
            deadman::heartbeat_path(&dir).exists(),
            "획득 직후 heartbeat가 touch돼야 한다"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn boot_daemon_without_retry_reproduces_the_false_reclaim_exit() {
        // ── ② 음성 대조: 재시도 예산 0(= WS-7 이전 동작)이면 **정확히 그 사고**가 재현된다.
        //     이 대조가 초록이어야 ①의 초록이 "재시도 덕분"임이 증명된다(mutation 저항).
        const HOLD_MS: u64 = 1500;
        let dir = boot_lock_scenario_dir("noretry");
        let lock = dir.join("cys.lock");
        let (holder, real_doctor) = LockHolder::start(&dir, &lock, HOLD_MS);
        assert!(wait_until_locked(&lock), "홀더가 락을 잡지 못했다");
        let out = run_boot_child(&dir, 0);
        holder.finish();

        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        assert_eq!(
            out.status.code(),
            Some(1),
            "재시도 없이는 부팅 데몬이 exit(1)해야 한다(real_doctor={real_doctor})\nstderr={stderr}"
        );
        assert!(
            stderr.contains("dead-holder-reclaim-failed"),
            "사고 사유가 재현되지 않았다 — 시나리오가 다른 경로를 탔다\nstderr={stderr}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}

#[cfg(test)]
mod log_rotation_tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "cysd-logrot-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn boot_marker_is_within_80_chars_even_for_long_socket_paths() {
        // 계약: phoenix 하네스가 두 번째 cysd 출력을 400자로 절단해 `holds the startup lock`을 찾는다
        // (javis_phoenix_harness.py:788-798). 마커가 길면 그 리터럴이 절단면 밖으로 밀린다.
        let short = std::path::Path::new("/tmp/c.sock");
        let line = boot_marker_line("0123456789abcdef", "pre-lock", short);
        assert!(line.len() <= 80, "짧은 경로 마커 {}자: {line}", line.len());
        assert!(line.contains("boot=0123456789abcdef"));
        assert!(line.contains("pre-lock"));

        // ★username은 `scripts/secret-scan.sh:33`의 더미 화이트리스트(user|x|youruser|USERNAME|
        // runner|home)에서만 고른다 — 그 밖의 이름은 개인경로로 판정돼 `secret-scan.sh --all`
        // (release.yml:151 fail-closed 게이트)에서 릴리스가 막힌다. 길이는 dept 세그먼트로 낸다.
        let long = std::path::Path::new(
            "/Users/user/Library/Application Support/cysjavis/dept-alpha-very-long-department/cysd.sock",
        );
        let line = boot_marker_line("0123456789abcdef", "live", long);
        assert!(line.len() <= 80, "긴 경로 마커 {}자: {line}", line.len());
        assert!(line.contains("cysd.sock"), "축약해도 스코프 식별자는 남는다: {line}");
    }

    #[test]
    fn boot_id_is_16_hex_chars_and_varies() {
        let a = new_boot_id();
        assert_eq!(a.len(), 16, "boot-id는 16자: {a}");
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()), "hex만: {a}");
        let b = new_boot_id();
        assert_ne!(a, b, "부팅마다 달라야 구간 판별이 성립한다");
    }

    #[test]
    fn size_gate_blocks_rotation_below_threshold() {
        // ★크기 게이트가 없으면 crashloop 3회에 전 이력이 소멸한다(회귀 핀).
        let d = tmp_dir("gate");
        let log = daemon_log_path(&d);
        std::fs::write(&log, b"small").unwrap();
        maybe_rotate_daemon_log(&log);
        assert!(
            !log_generation_path(&log, 1).exists(),
            "임계 미만은 회전 금지"
        );
        assert_eq!(std::fs::read(&log).unwrap(), b"small", "원본 무손상");
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn rotation_moves_generations_and_truncates_live_inode() {
        let d = tmp_dir("rot");
        let log = daemon_log_path(&d);
        std::fs::write(&log, b"current").unwrap();
        std::fs::write(log_generation_path(&log, 1), b"gen1").unwrap();
        std::fs::write(log_generation_path(&log, 2), b"gen2").unwrap();

        // 라이브 기록자를 모사 — 회전 후에도 **같은 fd**로 계속 쓸 수 있어야 한다(inode 보존).
        #[cfg(unix)]
        let live_ino = {
            use std::os::unix::fs::MetadataExt;
            std::fs::metadata(&log).unwrap().ino()
        };

        rotate_log_generations(&log).unwrap();

        assert_eq!(std::fs::read(log_generation_path(&log, 3)).unwrap(), b"gen2");
        assert_eq!(std::fs::read(log_generation_path(&log, 2)).unwrap(), b"gen1");
        assert_eq!(
            std::fs::read(log_generation_path(&log, 1)).unwrap(),
            b"current"
        );
        assert_eq!(std::fs::metadata(&log).unwrap().len(), 0, "원본은 truncate");
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(
                std::fs::metadata(&log).unwrap().ino(),
                live_ino,
                "cysd.log inode 보존(rename 금지) — 라이브 기록자 fd가 유령 inode로 새지 않는다"
            );
        }
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn empty_generation_files_are_not_promoted() {
        // 부서 셸 `>` 리다이렉트가 만든 0바이트 파일이 무한히 `.N`으로 승격되는 것을 막는다.
        let d = tmp_dir("empty");
        let log = daemon_log_path(&d);
        std::fs::write(&log, b"current").unwrap();
        std::fs::write(log_generation_path(&log, 1), b"").unwrap();
        rotate_log_generations(&log).unwrap();
        assert!(
            !log_generation_path(&log, 2).exists(),
            "0바이트 세대는 승격 금지"
        );
        assert_eq!(
            std::fs::read(log_generation_path(&log, 1)).unwrap(),
            b"current",
            "빈 .1은 그대로 덮어쓴다"
        );
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn missing_log_is_a_no_op() {
        let d = tmp_dir("absent");
        maybe_rotate_daemon_log(&daemon_log_path(&d));
        assert!(!daemon_log_path(&d).exists());
        std::fs::remove_dir_all(&d).ok();
    }

    #[cfg(unix)]
    #[test]
    fn append_gate_decides_rotation_on_both_branches() {
        // ★종전 결함 둘: ①`assert!(!log_fds_are_append())`가 **테스트 하니스 환경에 종속**됐다
        //   (`cargo test >> out.log`면 stdout이 O_APPEND라 그 자리에서 실패). ②그 게이트 때문에
        //   `maybe_rotate_daemon_log`의 **성공 경로가 어떤 테스트에서도 실행되지 않았다** —
        //   크기 게이트+append 게이트+실제 회전의 합성 커버리지가 0이었다.
        //   이제 append 여부를 주입해 **양쪽 분기를 모두** 덮는다.
        use std::os::unix::fs::MetadataExt;
        let big = vec![b'x'; (LOG_ROTATE_THRESHOLD + 1) as usize];

        // (a) non-append 기록자 → 회전 스킵(copy-truncate가 NUL 홀을 뚫는 것을 막는다).
        let d = tmp_dir("noappend");
        let log = daemon_log_path(&d);
        std::fs::write(&log, &big).unwrap();
        maybe_rotate_daemon_log_with(&log, false);
        assert!(
            !log_generation_path(&log, 1).exists(),
            "non-append 기록자면 회전 스킵(NUL 홀 방지)"
        );
        assert_eq!(
            std::fs::metadata(&log).unwrap().len(),
            LOG_ROTATE_THRESHOLD + 1,
            "스킵 시 원본 무손상"
        );
        std::fs::remove_dir_all(&d).ok();

        // (b) append 기록자 → 임계 초과이므로 **실제로 회전**한다: .1 생성 + 원본 truncate +
        //     inode 보존(라이브 기록자 fd가 유령 inode로 새지 않는다).
        let d = tmp_dir("append");
        let log = daemon_log_path(&d);
        std::fs::write(&log, &big).unwrap();
        let live_ino = std::fs::metadata(&log).unwrap().ino();
        maybe_rotate_daemon_log_with(&log, true);
        assert_eq!(
            std::fs::metadata(log_generation_path(&log, 1)).unwrap().len(),
            LOG_ROTATE_THRESHOLD + 1,
            "append 기록자면 실제 회전 — 전량이 .1로 보존된다"
        );
        assert_eq!(std::fs::metadata(&log).unwrap().len(), 0, "원본은 truncate");
        assert_eq!(
            std::fs::metadata(&log).unwrap().ino(),
            live_ino,
            "cysd.log inode 보존(rename 금지)"
        );
        std::fs::remove_dir_all(&d).ok();

        // (c) 크기 게이트가 append 게이트보다 우선한다 — append여도 임계 미만이면 회전 금지
        //     (crashloop 3회에 보관 세대가 전부 밀려나는 경로 차단).
        let d = tmp_dir("appendsmall");
        let log = daemon_log_path(&d);
        std::fs::write(&log, b"small").unwrap();
        maybe_rotate_daemon_log_with(&log, true);
        assert!(!log_generation_path(&log, 1).exists(), "임계 미만은 회전 금지");
        std::fs::remove_dir_all(&d).ok();
    }

    #[cfg(unix)]
    #[test]
    fn fd_append_probe_reads_the_real_open_flag() {
        // 주입 게이트의 **실측부**(프로덕션이 fd 1·2에 적용하는 그 술어)를 환경 무관하게 검증한다.
        // 이 술어가 뒤집히면 부서 데몬(비 O_APPEND)에서 copy-truncate가 NUL 홀을 뚫는다.
        use std::os::unix::io::AsRawFd;
        let d = tmp_dir("fdflag");
        let p = d.join("probe.log");
        let appending = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&p)
            .unwrap();
        assert!(fd_is_append(appending.as_raw_fd()), "O_APPEND fd=true");
        let writing = std::fs::OpenOptions::new()
            .write(true)
            .truncate(false)
            .open(&p)
            .unwrap();
        assert!(
            !fd_is_append(writing.as_raw_fd()),
            "W,0x10000 계열(부서 데몬 로그 fd)=false"
        );
        assert!(!fd_is_append(-1), "잘못된 fd는 fail-closed");
        drop(appending);
        drop(writing);
        std::fs::remove_dir_all(&d).ok();
    }
}

#[cfg(test)]
mod browser_rpc_isolation_tests {
    use super::{run_browser_job_bounded_with_cancellation, BrowserJobGate, BrowserRequestRegistry};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;

    /// 취소 플래그를 만들어 **프로덕션 진입점을 그대로** 호출한다(로직 없음 — 인자 조립만).
    /// 종전에는 테스트 전용 래퍼(`run_browser_job_bounded`)를 통했고, 그 래퍼는 WS-0-3 이후
    /// 프로덕션이 부르지 않는 죽은 코드였다 — 즉 4종 테스트가 프로덕션 경로를 덮지 못했다.
    fn fresh_cancellation() -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }

    #[tokio::test(flavor = "current_thread")]
    async fn slow_browser_start_does_not_starve_other_async_rpc_work() {
        let gate = BrowserJobGate::new(1);
        let slow = run_browser_job_bounded_with_cancellation(
            &gate,
            std::time::Duration::from_secs(1),
            fresh_cancellation(),
            |_| {
                std::thread::sleep(std::time::Duration::from_millis(120));
                "browser-ready"
            },
        );
        let other_rpc = async {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            "pty-ping"
        };
        let (slow_result, ping_result) = tokio::join!(slow, other_rpc);
        assert_eq!(ping_result, "pty-ping");
        assert_eq!(slow_result.unwrap(), "browser-ready");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn browser_worker_deadline_is_bounded() {
        let gate = BrowserJobGate::new(1);
        let launched = Arc::new(AtomicUsize::new(0));
        let launched_in_job = launched.clone();
        let result = run_browser_job_bounded_with_cancellation(
            &gate,
            std::time::Duration::from_millis(10),
            fresh_cancellation(),
            move |cancelled| {
                std::thread::sleep(std::time::Duration::from_millis(60));
                if !cancelled.load(Ordering::SeqCst) {
                    launched_in_job.fetch_add(1, Ordering::SeqCst);
                }
            },
        )
        .await;
        assert_eq!(result.unwrap_err(), "timeout");
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        assert_eq!(
            launched.load(Ordering::SeqCst),
            0,
            "timed-out work must observe cancellation before a late spawn"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn browser_worker_gate_rejects_backpressure_instead_of_queueing() {
        let gate = Arc::new(BrowserJobGate::new(1));
        let release = Arc::new(AtomicBool::new(false));
        let first_gate = gate.clone();
        let first_release = release.clone();
        let first = tokio::spawn(async move {
            run_browser_job_bounded_with_cancellation(
                &first_gate,
                std::time::Duration::from_secs(1),
                Arc::new(AtomicBool::new(false)),
                move |_| {
                    while !first_release.load(Ordering::SeqCst) {
                        std::thread::yield_now();
                    }
                },
            )
            .await
        });
        tokio::task::yield_now().await;
        let second = run_browser_job_bounded_with_cancellation(
            &gate,
            std::time::Duration::from_millis(20),
            Arc::new(AtomicBool::new(false)),
            |_| (),
        )
        .await;
        assert_eq!(second.unwrap_err(), "backpressure");
        release.store(true, Ordering::SeqCst);
        first.await.unwrap().unwrap();
    }

    /// ★WS-0-③: 요청 주체(GUI 프로세스·연결)가 ensure 진행 중 사라지면 커밋 전에 취소되어야 한다.
    /// 취소가 없으면 워커는 25초를 마저 돌아 supervisor+engine을 커밋하고, 그 런타임은 주인 없는
    /// **고아**가 된다(원사고의 또 다른 생성 경로). 커밋 0건이 수용 기준이다.
    #[tokio::test(flavor = "current_thread")]
    async fn peer_disconnect_cancels_in_flight_ensure_before_it_commits() {
        use super::await_watching_peer;
        use tokio::io::{AsyncWriteExt, BufReader};

        let (client, server) = tokio::io::duplex(64);
        let (server_read, _server_write) = tokio::io::split(server);
        let mut reader = BufReader::new(server_read);

        let cancellation = Arc::new(AtomicBool::new(false));
        let committed = Arc::new(AtomicUsize::new(0));

        // ensure 워커 모사: 취소 신호를 보면 커밋 없이 조기 종료, 아니면 커밋한다.
        let worker_flag = cancellation.clone();
        let worker_committed = committed.clone();
        let job = async move {
            for _ in 0..2_000 {
                if worker_flag.load(Ordering::SeqCst) {
                    return "cancelled";
                }
                tokio::task::yield_now().await;
            }
            worker_committed.fetch_add(1, Ordering::SeqCst);
            "committed"
        };

        // 요청 주체가 응답을 받기 전에 연결을 끊는다.
        drop(client);

        let outcome = await_watching_peer(job, &mut reader, &cancellation).await;
        assert!(
            cancellation.load(Ordering::SeqCst),
            "①연결 EOF가 취소 플래그를 set 해야 한다"
        );
        assert_eq!(outcome, "cancelled", "②워커가 조기 종료해야 한다");
        assert_eq!(
            committed.load(Ordering::SeqCst),
            0,
            "③커밋 0건 — 주인 없는 런타임을 만들지 않는다"
        );

        // 살아있는 연결에서는 오취소가 없어야 한다(정상 흐름 무해).
        let (mut client, server) = tokio::io::duplex(64);
        let (server_read, _sw) = tokio::io::split(server);
        let mut reader = BufReader::new(server_read);
        let alive_flag = Arc::new(AtomicBool::new(false));
        client.write_all(b"{\"id\":2}\n").await.unwrap(); // 파이프라인된 다음 요청
        let outcome = await_watching_peer(async { "done" }, &mut reader, &alive_flag).await;
        assert_eq!(outcome, "done");
        assert!(
            !alive_flag.load(Ordering::SeqCst),
            "살아있는 연결·파이프라인 바이트는 취소하지 않는다(오취소 금지)"
        );
    }

    #[test]
    fn pane_cancel_uses_peer_and_request_identity_and_leaves_no_late_commit() {
        let registry = Arc::new(BrowserRequestRegistry::default());
        let pane_nonce = "b".repeat(64);
        let request_id = "a".repeat(64);
        let guard = registry
            .register(
                77,
                pane_nonce.clone(),
                request_id.clone(),
                Arc::new(AtomicBool::new(false)),
            )
            .unwrap();
        let committed = Arc::new(AtomicUsize::new(0));
        let committed_by_job = committed.clone();
        let worker_cancelled = guard.cancellation.clone();
        let worker = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(30));
            if !worker_cancelled.load(Ordering::SeqCst) {
                committed_by_job.fetch_add(1, Ordering::SeqCst);
            }
            drop(guard);
        });
        assert!(!registry.cancel(78, &pane_nonce, &request_id));
        assert!(!registry.cancel(77, &"c".repeat(64), &request_id));
        assert!(registry.cancel(77, &pane_nonce, &request_id));
        worker.join().unwrap();
        assert_eq!(committed.load(Ordering::SeqCst), 0);
        assert_eq!(registry.len(), 0);
    }
}

/// ★WS-0-③: 응답을 기다리는 동안 **요청 주체의 연결 소멸**을 함께 감시한다.
///
/// `job`이 먼저 끝나면 그 결과를 그대로 돌려준다. 그 전에 read half가 EOF(또는 오류)를 내면
/// `cancellation`을 set 하고 — 새 취소 경로를 만들지 않고 `browser.runtime.cancel`이 쓰는 그
/// 플래그를 그대로 쓴다 — 이후에도 `job`이 스스로 정리를 마칠 때까지 기다린다(가드 drop·워커 회수).
///
/// 감시는 **한 번만** 한다: `fill_buf`는 EOF에서 즉시 반환하므로 계속 폴링하면 바쁜 루프가 된다.
/// 비어있지 않은 바이트(파이프라인된 다음 요청)는 연결이 살아있다는 뜻이므로 **취소하지 않는다**
/// (오취소가 정상 흐름을 깨는 것이 미탐지보다 나쁘다).
async fn await_watching_peer<T, F, R>(
    job: F,
    reader: &mut BufReader<R>,
    cancellation: &Arc<std::sync::atomic::AtomicBool>,
) -> T
where
    F: std::future::Future<Output = T>,
    R: AsyncRead + Unpin,
{
    tokio::pin!(job);
    let mut watching = true;
    loop {
        tokio::select! {
            done = &mut job => return done,
            read = reader.fill_buf(), if watching => {
                watching = false;
                match read {
                    Ok(buf) if buf.is_empty() => {
                        cancellation.store(true, std::sync::atomic::Ordering::SeqCst);
                        eprintln!("[cysd] browser rpc peer disconnected mid-flight — cancelling in-flight work (orphan guard)");
                    }
                    Err(_) => {
                        cancellation.store(true, std::sync::atomic::Ordering::SeqCst);
                        eprintln!("[cysd] browser rpc peer connection failed mid-flight — cancelling in-flight work (orphan guard)");
                    }
                    Ok(_) => {}
                }
            }
        }
    }
}

async fn handle_connection(daemon: Arc<Daemon>, stream: Stream, caller_pid: Option<u32>) {
    let (read_half, mut write_half) = tokio::io::split(stream);
    let mut reader = BufReader::new(read_half);

    while let Ok(Some(line)) = next_line_capped(&mut reader, MAX_REQUEST_LINE).await {
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }
        let req: Request = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let resp =
                    cys::err_response(&serde_json::Value::Null, "parse_error", &e.to_string());
                if write_line(&mut write_half, &resp).await.is_err() {
                    return;
                }
                continue;
            }
        };

        // ★WS-0-③ 요청 주체 소멸 감지: ensure가 도는 25초 동안 GUI 프로세스·연결이 죽어도 데몬은
        // 그걸 모른 채 supervisor+engine을 끝까지 커밋했다 — 그 런타임은 **주인 없는 고아**가 되고
        // (원사고의 또 다른 생성 경로), 죽은 GUI의 peer 엔트리도 대장에 남아 상한 소진을 가속한다.
        // 브라우저 verb에 한해 응답 대기 구간에서 read half의 EOF를 함께 감시하고, 끊기면
        // **기존 취소 플래그**(browser.runtime.cancel과 동일한 Arc)를 set 한다 — 새 취소 경로를
        // 만들지 않는다. 워커는 그 플래그를 보고 조기 종료하고, 레인 A의 terminate_supervisor
        // pre-readiness grace(100ms)가 이때 실효를 낸다.
        let cancellation = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let watch_peer = req.method.starts_with("browser.runtime.");
        let dispatched = dispatch_request_isolated(
            daemon.clone(),
            req,
            caller_pid,
            Arc::clone(&cancellation),
        );
        let reply = if watch_peer {
            await_watching_peer(dispatched, &mut reader, &cancellation).await
        } else {
            dispatched.await
        };
        match reply {
            Reply::Single(resp) => {
                if write_line(&mut write_half, &resp).await.is_err() {
                    return;
                }
            }
            Reply::EventStream {
                ack,
                after_seq,
                names,
                categories,
            } => {
                run_event_stream(&daemon, &mut write_half, ack, after_seq, names, categories).await;
                return;
            }
            Reply::Attach { ack, surface_id } => {
                run_attach(&daemon, &mut write_half, ack, surface_id).await;
                return;
            }
            Reply::FeedWait {
                id,
                request_id,
                rx,
                timeout_secs,
            } => {
                // T4-15: pause 중에는 카운트다운 동결 — kill-switch가 대기 중인 워커들을
                // timeout-deny로 우수수 떨어뜨리지 않는다 (resume 후 잔여 시간부터 재개).
                let mut rx = rx;
                let mut remaining = timeout_secs;
                let outcome: Option<String> = loop {
                    tokio::select! {
                        r = &mut rx => break r.ok(),
                        // 클라이언트 연결 끊김 감지: 대기 중에는 응답을 아직 쓰기 전이라
                        // events.stream·attach의 write 실패 안전망이 닿지 않는다. read half를
                        // 함께 감시해, 워커가 응답 전에 끊으면(EOF/에러) 즉시 정리하고 빠져나간다.
                        // 없으면 끊긴 워커의 waiter·연결 태스크가 timeout(최대 3600초)까지,
                        // pause 중에는 remaining이 동결돼 resume까지 무기한 잔존한다.
                        read = reader.fill_buf() => match read {
                            // EOF(빈 슬라이스) = 끊김. 비어있지 않은 바이트는 대기 중 추가 전송으로
                            // 프로토콜 위반이라 연결을 신뢰할 수 없다 — 셋 다 끊김으로 정리.
                            Ok([]) | Ok([_, ..]) | Err(_) => break None,
                        },
                        _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {
                            if !daemon.paused.load(std::sync::atomic::Ordering::Relaxed) {
                                if remaining <= 1 { break None; }
                                remaining -= 1;
                            }
                        }
                    }
                };
                let resp = match outcome {
                    Some(decision) => cys::ok_response(
                        &id,
                        json!({"request_id": request_id, "status": "resolved", "decision": decision}),
                    ),
                    None => {
                        // Timeout or dropped: mark the item and tell the caller.
                        daemon.feed_waiters.lock().unwrap().remove(&request_id);
                        let snapshot = {
                            let mut items = daemon.feed_items.lock().unwrap();
                            items
                                .iter_mut()
                                .find(|i| i.request_id == request_id)
                                .filter(|i| i.status == "pending")
                                .map(|item| {
                                    item.status = "timeout".into();
                                    item.resolved_at = Some(crate::state::now_epoch());
                                    item.clone()
                                })
                        };
                        if let Some(s) = &snapshot {
                            daemon.persist_feed_item(s);
                            daemon.bus.publish(
                                "feed.item.timeout",
                                "feed",
                                None,
                                json!({"request_id": request_id}),
                            );
                            cys::ok_response(
                                &id,
                                json!({"request_id": request_id, "status": "timeout", "decision": null}),
                            )
                        } else {
                            // 동시 feed.reply가 이미 종결 — 승인 결정을 삼키고 timeout으로
                            // 오보하는 대신 실제 결정을 돌려준다 (모순 이벤트도 미발행)
                            let decision = daemon
                                .feed_items
                                .lock()
                                .unwrap()
                                .iter()
                                .find(|i| i.request_id == request_id)
                                .and_then(|i| i.decision.clone());
                            match decision {
                                Some(d) => cys::ok_response(
                                    &id,
                                    json!({"request_id": request_id, "status": "resolved", "decision": d}),
                                ),
                                None => cys::ok_response(
                                    &id,
                                    json!({"request_id": request_id, "status": "timeout", "decision": null}),
                                ),
                            }
                        }
                    }
                };
                if write_line(&mut write_half, &resp).await.is_err() {
                    return;
                }
            }
            Reply::WaitFor {
                id,
                surface_id,
                pattern,
                timeout_secs,
                since_line,
            } => {
                // T3-14 완료 대기: 데몬 내부 폴링(토큰 비용 0) — plain-line 마커 규약 전제.
                let deadline =
                    std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
                let mut cursor = since_line;
                let resp = loop {
                    let Some(surface) = daemon.get_surface(surface_id) else {
                        break cys::err_response(
                            &id,
                            "not_found",
                            &format!("surface {surface_id} closed"),
                        );
                    };
                    let (lines, start) = {
                        // ★레이스 차단: scrollback 락을 먼저 잡고 그 안에서 line_count를 읽는다
                        // (writer가 push·fetch_add를 같은 락 아래 수행 — total/sb.len 일관 관측).
                        let sb = surface.scrollback.lock().unwrap_or_else(|e| e.into_inner());
                        let total = surface
                            .line_count
                            .load(std::sync::atomic::Ordering::Relaxed);
                        let oldest = total.saturating_sub(sb.len() as u64);
                        let start = cursor.max(oldest);
                        let skip = (start - oldest) as usize;
                        let lines: Vec<String> = sb.iter().skip(skip).cloned().collect();
                        (lines, start)
                    };
                    let mut matched = None;
                    for (i, line) in lines.iter().enumerate() {
                        if pattern.is_match(line) {
                            matched = Some((start + i as u64, line.clone()));
                            break;
                        }
                    }
                    cursor = start + lines.len() as u64;
                    if let Some((line_no, line)) = matched {
                        break cys::ok_response(
                            &id,
                            json!({"matched": true, "line": line, "line_no": line_no,
                                   "next_cursor": line_no + 1}),
                        );
                    }
                    if surface.exited.load(std::sync::atomic::Ordering::Relaxed) {
                        break cys::ok_response(
                            &id,
                            json!({"matched": false, "reason": "surface_exited",
                                   "next_cursor": cursor}),
                        );
                    }
                    if std::time::Instant::now() >= deadline {
                        break cys::ok_response(
                            &id,
                            json!({"matched": false, "reason": "timeout",
                                   "next_cursor": cursor}),
                        );
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                };
                if write_line(&mut write_half, &resp).await.is_err() {
                    return;
                }
            }
        }
    }
}

/// T1-6: cys↔cysd ABI producer 자기검증 경계. 응답 `Value`를 `cys::wire::frame_response`로
/// 통과시켜 round-trip 동일성(선언==실제 직렬화)을 검증하고 `_flen`/`_pv`를 additive하게
/// 부착한다(top-level `ok`/`result`는 보존 → 구 디코더 호환). 위반은 T1-3 `Severity`로
/// 사상해 fail-loud 기록한다(Drift/LenMismatch=Critical 격리, VersionSkew=Recoverable).
/// 검증 실패가 응답 자체를 삼켜 클라이언트를 무기한 대기시키지 않도록, 기록 후 legacy 직렬화로
/// 폴백해 한 줄은 항상 내보낸다(가용성 보존 — 격리 판정은 Severity 로그가 담당).
fn abi_severity(e: &cys::wire::AbiError) -> severity::Severity {
    match e {
        cys::wire::AbiError::Drift | cys::wire::AbiError::LenMismatch => {
            severity::Severity::Critical
        }
        cys::wire::AbiError::VersionSkew { .. } => severity::Severity::Recoverable,
    }
}

async fn write_line<W: AsyncWrite + Unpin>(
    w: &mut W,
    value: &serde_json::Value,
) -> std::io::Result<()> {
    // T4-5A(==T5-6 strand-3, ONE guard): 단일 RPC 응답 바이트 상한. cap 초과 시 fail-loud
    // 트렁케이트 sentinel로 치환(컨텍스트/메모리 폭주 차단). 직교 가드 — watchdog와 별개 책임.
    let capped = cys::wire::cap_response(value);
    let value: &serde_json::Value = capped.as_ref().unwrap_or(value);
    let line = match cys::wire::frame_response(value) {
        Ok(framed) => framed,
        Err(e) => {
            let sev = abi_severity(&e);
            eprintln!(
                "[cysd] ABI producer self-verify {} ({:?}) — falling back to legacy serialization",
                sev.as_str(),
                e
            );
            let mut body = serde_json::to_string(value).unwrap_or_default();
            body.push('\n');
            body
        }
    };
    w.write_all(line.as_bytes()).await?;
    w.flush().await
}

/// Push channel: replay missed events, then forward live events until the client disconnects.
async fn run_event_stream<W: AsyncWrite + Unpin>(
    daemon: &Arc<Daemon>,
    w: &mut W,
    ack: serde_json::Value,
    after_seq: Option<u64>,
    names: Vec<String>,
    categories: Vec<String>,
) {
    // Subscribe BEFORE replay so no events fall into the gap.
    let mut rx = daemon.bus.subscribe();
    // dispatch 시점이 아닌 구독 직후의 최신 seq로 갱신 — 클라이언트 커서 시드 정확화
    let mut ack = ack;
    let live_latest = daemon.bus.latest_seq();
    ack["latest_seq"] = json!(live_latest);
    // (1)-sync: resume 블록도 구독 직후 최신값으로 동기 — dispatch 시점 값과 어긋나지 않게
    if ack.get("resume").is_some() {
        ack["resume"]["latest_seq"] = json!(live_latest);
        ack["resume"]["next_seq"] = json!(live_latest + 1);
    }
    if write_line(w, &ack).await.is_err() {
        return;
    }
    let mut last_seq = after_seq.unwrap_or(0);
    if let Some(after) = after_seq {
        // 갭 신호: 커서 이후 일부 이벤트가 ring에서 밀려나 재생 불가하면 무음 유실 대신 알린다
        let (oldest, latest) = daemon.bus.replay_bounds();
        let gap_until = oldest.map(|o| o.saturating_sub(1)).unwrap_or(latest);
        if gap_until > after {
            let warn = json!({"type": "error", "ok": false,
                "error": {"code": "replay_gap",
                    "message": format!("events {}..={} no longer available (ring evicted or daemon restarted)", after + 1, gap_until)}});
            if write_line(w, &warn).await.is_err() {
                return;
            }
        }
        for event in daemon.bus.replay_after(after) {
            last_seq = event["seq"].as_u64().unwrap_or(last_seq);
            if events::event_matches(&event, &names, &categories)
                && write_line(w, &event).await.is_err()
            {
                return;
            }
        }
    }
    // (2b) live 루프: 15s heartbeat 타이머와 함께 select! — 이벤트 무발생 구간에서도
    // half-open 소켓을 조기 감지·재연결 유도. 패턴은 run_attach(아래)의 select! 동일.
    let mut hb = tokio::time::interval(std::time::Duration::from_secs(15));
    hb.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    hb.tick().await; // 첫 tick은 즉시 발화 — 소비해 15s 후부터 heartbeat
    loop {
        tokio::select! {
            r = rx.recv() => match r {
                Ok(event) => {
                    let seq = event["seq"].as_u64().unwrap_or(0);
                    if seq <= last_seq {
                        continue; // already replayed
                    }
                    last_seq = seq; // 중복 차단 커서 전진(원본 누락 — 의도 명확화, 동작 동일)
                    if events::event_matches(&event, &names, &categories)
                        && write_line(w, &event).await.is_err()
                    {
                        return;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    let warn = json!({"type": "error", "ok": false,
                        "error": {"code": "slow_consumer", "message": format!("dropped {n} events")}});
                    let _ = write_line(w, &warn).await;
                    return; // (2a) 종료해 클라이언트가 last_seq부터 재replay로 갭을 메우게 강제
                }
                Err(_) => return,
            },
            _ = hb.tick() => {
                let beat = json!({"type": "heartbeat", "latest_seq": daemon.bus.latest_seq()});
                if write_line(w, &beat).await.is_err() {
                    return;
                }
            }
        }
    }
}

/// Raw PTY output mirror: ack line (JSON), then raw bytes as they arrive.
async fn run_attach<W: AsyncWrite + Unpin>(
    daemon: &Arc<Daemon>,
    w: &mut W,
    ack: serde_json::Value,
    surface_id: u64,
) {
    let Some(surface) = daemon.get_surface(surface_id) else {
        // dispatch 검사와 재조회 사이에 surface가 닫힌 경우 — 무응답 종료 대신 에러를 알린다
        let err = json!({"type": "ack", "ok": false,
            "error": {"code": "not_found", "message": format!("surface {surface_id} closed")}});
        let _ = write_line(w, &err).await;
        return;
    };
    // parser 락 아래에서 구독+스냅샷 — 그 사이 도착한 청크가 스냅샷과 live 양쪽에
    // 중복 배달되는 창을 닫는다 (reader 스레드는 parser 락에서 직렬화됨)
    let (mut rx, snapshot) = {
        let parser = surface.parser.lock().unwrap_or_else(|e| e.into_inner());
        let rx = surface.out_tx.subscribe();
        (rx, parser.screen().contents_formatted())
    };
    if write_line(w, &ack).await.is_err() {
        return;
    }
    // Send a formatted (color/cursor-accurate) redraw of the current screen first.
    if !snapshot.is_empty() && w.write_all(&snapshot).await.is_err() {
        return;
    }
    loop {
        // out_tx Sender는 Surface 구조체가 소유라 자력 종료(셸 exit) 후에도 채널이 닫히지
        // 않는다 — exited 플래그를 주기 점검해 스트림을 끝내야 클라이언트가 EOF를 받는다.
        tokio::select! {
            r = rx.recv() => match r {
                Ok(chunk) => {
                    if w.write_all(&chunk).await.is_err() || w.flush().await.is_err() {
                        return;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => return,
            },
            _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {
                if surface.exited.load(std::sync::atomic::Ordering::Relaxed) {
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod env_scrub_tests {
    /// 회귀 박제: claude 세션 안에서 spawn된 데몬이 세션 정체성 env를 보존하면 PTY 자식
    /// claude가 child-session으로 강등돼 트랜스크립트 미영속(복원·recall·T5 전부 파괴).
    /// scrub은 누설 변수만 제거하고 무관 변수는 보존해야 한다.
    #[test]
    fn scrub_removes_leaky_session_vars_only() {
        std::env::set_var("CLAUDE_CODE_SESSION_ID", "parent-session");
        std::env::set_var("CLAUDE_CODE_CHILD_SESSION", "1");
        std::env::set_var("CLAUDECODE", "1");
        std::env::set_var("CYS_SCRUB_TEST_KEEP", "yes"); // 무관 변수 — 보존 확인용
        super::scrub_claude_session_env();
        assert!(std::env::var_os("CLAUDE_CODE_SESSION_ID").is_none());
        assert!(std::env::var_os("CLAUDE_CODE_CHILD_SESSION").is_none());
        assert!(std::env::var_os("CLAUDECODE").is_none());
        assert_eq!(
            std::env::var("CYS_SCRUB_TEST_KEEP").as_deref(),
            Ok("yes"),
            "무관 env까지 지우면 안 된다"
        );
        std::env::remove_var("CYS_SCRUB_TEST_KEEP");
    }

    /// 데몬 root env 정규화가 PYTHONUTF8=1을 주입하는지 박제(Windows cp1252/cp949 크래시 방지 —
    /// spawn 전 파이썬 자식 상속 커버). set_var는 "1"로만 설정되므로 병렬 테스트 간 경합 무해.
    #[test]
    fn sets_pythonutf8_for_spawned_python_children() {
        super::scrub_claude_session_env();
        assert_eq!(std::env::var("PYTHONUTF8").as_deref(), Ok("1"));
    }
}

#[cfg(test)]
mod abi_severity_tests {
    use crate::severity::Severity;

    /// T1-6: AbiError → T1-3 Severity 사상이 §4.2 계약과 일치하는지 박제.
    /// Drift/LenMismatch=Critical(격리), VersionSkew=Recoverable(graceful).
    #[test]
    fn abi_error_to_severity() {
        assert_eq!(
            super::abi_severity(&cys::wire::AbiError::Drift),
            Severity::Critical
        );
        assert_eq!(
            super::abi_severity(&cys::wire::AbiError::LenMismatch),
            Severity::Critical
        );
        assert_eq!(
            super::abi_severity(&cys::wire::AbiError::VersionSkew {
                peer_pv: 2,
                local_pv: cys::wire::PROTO_PV
            }),
            Severity::Recoverable
        );
        // 격리 술어와의 정합: Critical만 격리, Recoverable은 재시도.
        assert!(super::abi_severity(&cys::wire::AbiError::Drift).is_critical());
        assert!(!super::abi_severity(&cys::wire::AbiError::VersionSkew {
            peer_pv: 2,
            local_pv: cys::wire::PROTO_PV
        })
        .is_critical());
    }
}

#[cfg(test)]
mod attach_race_tests {
    use crate::state::Daemon;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;

    /// ★회귀 박제 (state.rs reader thread ↔ main.rs run_attach 불변식):
    /// run_attach는 parser 락 아래에서 `out_tx.subscribe()`+화면 스냅샷을 원자적으로 뜬다
    /// (main.rs:538-542). 그 불변식이 성립하려면 reader 스레드도 `parser.process(chunk)`와
    /// `out_tx.send(chunk)`를 같은 parser 락 임계영역에 묶어야 한다. 둘이 분리되면
    /// (과거 버그) 다음 인터리빙이 같은 청크를 스냅샷·live 양쪽에 중복 배달한다:
    ///   ① reader: process(C) 후 락 해제
    ///   ② attach: 락 획득→subscribe(rx)→스냅샷(C 반영됨)→락 해제
    ///   ③ reader: out_tx.send(C) → ②의 rx가 C를 live로 수신  ⇒ C가 스냅샷+live 중복
    ///
    /// 이 테스트는 run_attach가 하는 일(락 아래 subscribe+스냅샷)을 그대로 모사하는 관측자를
    /// 실제 Surface reader 스레드와 동시에 돌려, "스냅샷 시점에 파서에 이미 반영된 마지막
    /// 청크가 그 직후 새 rx로 live 도착하는" 중복 창이 닫혔는지 다회 검증한다. 버그(분리)면
    /// 충분한 반복에서 중복이 잡히고, 수정(결합)이면 불변식이 무조건 성립해 0건이다.
    ///
    /// 핵심 신호: parser 락을 쥔 채 화면에 반영된 출력 바이트 수(=process가 본 누적 바이트)와
    /// 같은 락 구간에서 subscribe한 rx로 이후 도착하는 바이트가 겹치면(겹친 청크 존재) 중복.
    /// 마커를 청크 단위로 유일하게 만들어 "스냅샷에 보였는데 live로도 온" 마커를 직접 센다.
    #[test]
    fn process_and_send_are_atomic_under_parser_lock_no_dup_delivery() {
        // 멀티스레드 런타임 불필요 — 동기 스레드만 사용. PTY reader는 create_surface가
        // 내부에서 std::thread로 띄운다.
        let tmp = std::env::temp_dir().join(format!(
            "cys-attach-race-{}-{}.sock",
            std::process::id(),
            now_nanos()
        ));
        let daemon = Daemon::new(tmp.clone());

        // 출력 스트림: 각 라인은 유일 토큰 "MK<seq>E". reader 스레드가 끊임없이 청크
        // 경계를 만들도록 긴 루프로 연속 출력하며, 32라인마다 짧은 양보(usleep 미사용 —
        // 셸 내장만)로 reader/observer가 process↔send 경계를 다수 통과하게 한다.
        const N: usize = 6000;
        let script = format!(
            "i=0; while [ $i -lt {N} ]; do printf 'MK%dE\\n' $i; i=$((i+1)); done; sleep 3"
        );
        let surface = daemon
            .create_surface(None, Some(script), None, None, 35, 120)
            .expect("create_surface");

        // 다수 관측자 스레드: run_attach의 '락-아래 subscribe+스냅샷'을 그대로 모사하며
        // process↔send 분리 시 열리는 중복 창(스냅샷에 이미 보인 마커가 새 rx로 live 도착)을
        // 동시 다발로 두드린다. 여러 스레드가 경합해야 좁은 창에 안정적으로 착지한다.
        const OBSERVERS: usize = 6;
        let mut handles = Vec::new();
        for _ in 0..OBSERVERS {
            let surf = Arc::clone(&surface);
            handles.push(std::thread::spawn(move || {
                let mut dup_incidents: Vec<usize> = Vec::new();
                loop {
                    if surf.exited.load(Ordering::Relaxed) {
                        break;
                    }
                    // ── run_attach와 동일: parser 락 아래 subscribe + 스냅샷 ──
                    let (mut rx, snapshot_markers) = {
                        let parser = surf.parser.lock().unwrap_or_else(|e| e.into_inner());
                        let rx = surf.out_tx.subscribe();
                        let snap = parser.screen().contents();
                        (rx, parse_markers(snap.as_bytes()))
                    };
                    // 스냅샷에 마지막으로 보인(=파서에 이미 반영된) 마커. 이 마커는
                    // 결합(수정) 시 '이미 send 완료'라 새 rx로는 절대 오면 안 된다.
                    let Some(&last_in_snapshot) = snapshot_markers.iter().max() else {
                        continue;
                    };
                    // 새 rx를 잠깐 비워 live 마커를 수집 (non-blocking try_recv 폴링).
                    let mut live: Vec<usize> = Vec::new();
                    let deadline =
                        std::time::Instant::now() + std::time::Duration::from_micros(500);
                    while std::time::Instant::now() < deadline {
                        match rx.try_recv() {
                            Ok(bytes) => live.extend(parse_markers(&bytes)),
                            Err(tokio::sync::broadcast::error::TryRecvError::Empty) => {
                                std::thread::yield_now()
                            }
                            Err(_) => break,
                        }
                    }
                    // 중복 판정: 스냅샷에 보였던(≤last_in_snapshot) 마커가 live로도 도착하면
                    // 그 청크가 스냅샷·live 양쪽에 배달된 것 — run_attach 주석이 막겠다던 케이스.
                    // (수정본은 process↔send가 원자적이라 새 rx에는 항상 >last_in_snapshot만 온다.)
                    for m in &live {
                        if *m <= last_in_snapshot {
                            dup_incidents.push(*m);
                        }
                    }
                }
                dup_incidents
            }));
        }

        let mut dup_incidents: Vec<usize> = Vec::new();
        for h in handles {
            dup_incidents.extend(h.join().expect("observer thread"));
        }

        // 정리: surface 종료 유도 (자력 종료 전에 kill — 좀비 방지)
        if let Ok(mut child) = surface.child.lock() {
            let _ = child.kill();
        }
        let _ = std::fs::remove_file(&tmp);

        assert!(
            dup_incidents.is_empty(),
            "process↔send가 parser 락에서 분리되어 청크 중복 배달 발생: {} 건 (예: {:?}). \
             reader 스레드는 process(chunk)와 out_tx.send(chunk)를 같은 parser 락 \
             임계영역에 묶어야 한다.",
            dup_incidents.len(),
            &dup_incidents[..dup_incidents.len().min(8)]
        );
    }

    /// "MK<n>E" 토큰을 바이트 스트림에서 추출 (청크/스냅샷 공통 파서).
    fn parse_markers(bytes: &[u8]) -> Vec<usize> {
        let s = String::from_utf8_lossy(bytes);
        let mut out = Vec::new();
        let mut rest = s.as_ref();
        while let Some(p) = rest.find("MK") {
            rest = &rest[p + 2..];
            if let Some(e) = rest.find('E') {
                if let Ok(n) = rest[..e].parse::<usize>() {
                    out.push(n);
                }
                rest = &rest[e + 1..];
            } else {
                break;
            }
        }
        out
    }

    fn now_nanos() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod feed_wait_disconnect_tests {
    use super::{handle_connection, Stream};
    use crate::state::Daemon;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::io::AsyncWriteExt;

    /// ★회귀 박제 (FeedWait 대기 중 클라이언트 끊김 + pause 동결):
    /// feed.push --wait의 대기 루프(main.rs)는 ① oneshot rx(=feed.reply) ② 1초 sleep ③ read
    /// half(끊김 감지) 세 가지를 select! 한다. ③이 없으면 워커가 응답 전에 연결을 끊어도
    /// 연결 태스크와 feed_waiters 엔트리가 timeout(최대 3600초)까지 살아남고, 데몬이 pause되면
    /// remaining이 영영 감소하지 않아(if !paused) timeout 분기에 절대 도달하지 못해 resume까지
    /// 무기한 잔존한다. 끊긴 워커가 pause 전후로 반복되면 연결 태스크·oneshot 채널이 단조 누적.
    ///
    /// 이 테스트는 ① feed.push --wait를 보내 waiter를 등록시키고 ② 데몬을 pause한 뒤
    /// ③ 클라이언트를 끊어, 연결 태스크가 (a) 유한 시간 내 종료하고 (b) feed_waiters 엔트리를
    /// 정리하는지 검증한다. 버그(③ 부재)면 pause 동결로 태스크가 영영 살아 timeout이 터지고
    /// waiter도 남는다. 수정(③ 존재)이면 끊김을 감지해 즉시 정리·종료한다.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feed_wait_releases_waiter_when_client_disconnects_during_pause() {
        // ★상태 격리: state_dir = socket의 부모 디렉터리이고 거기에 feed.jsonl이 영속된다.
        // 소켓을 고유 하위 디렉터리에 두지 않으면 temp_dir/feed.jsonl을 다른 실행과 공유해
        // 직전 실행이 남긴 같은 request_id가 replay되어 'duplicate request_id'로 오염된다.
        let dir = std::env::temp_dir().join(format!(
            "cys-feedwait-disc-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let tmp = dir.join("cysd.sock");
        let daemon = Daemon::new(tmp.clone());

        // 인메모리 양방향 스트림: server는 handle_connection이, client는 테스트가 보유.
        let (client, server) = tokio::io::duplex(64 * 1024);
        let server: Stream = Box::new(server);
        let conn = tokio::spawn(handle_connection(Arc::clone(&daemon), server, None));

        // feed.push --wait — timeout_secs는 길게 줘서 끊김이 아닌 timeout으로 빠지는 오판을 배제.
        let mut client = client;
        let req = serde_json::json!({
            "id": "1",
            "method": "feed.push",
            "params": {
                "request_id": "disc-test-1",
                "kind": "approval",
                "title": "t",
                "body": "b",
                "wait": true,
                "timeout_secs": 3600
            }
        });
        let mut line = serde_json::to_vec(&req).unwrap();
        line.push(b'\n');
        client.write_all(&line).await.unwrap();
        client.flush().await.unwrap();

        // waiter 등록 대기 (FeedWait 진입 확인).
        let registered = wait_until(Duration::from_secs(5), || {
            daemon
                .feed_waiters
                .lock()
                .unwrap()
                .contains_key("disc-test-1")
        })
        .await;
        assert!(registered, "feed.push --wait가 waiter를 등록하지 못함");

        // 데몬 pause — 이 상태에서 timeout 카운트다운은 동결된다.
        daemon.paused.store(true, Ordering::Relaxed);

        // 클라이언트 끊김 (워커 프로세스 kill 모사).
        drop(client);

        // 수정본: 끊김을 감지해 유한 시간 내 연결 태스크 종료 + waiter 정리.
        // 버그: pause 동결로 영영 살아 timeout이 터진다.
        let finished = tokio::time::timeout(Duration::from_secs(10), conn).await;
        assert!(
            finished.is_ok(),
            "FeedWait 대기 태스크가 클라이언트 끊김을 감지하지 못해 종료하지 않음 \
             (pause 중 remaining 동결 → timeout 분기 영구 미도달)"
        );

        let waiter_cleared = daemon
            .feed_waiters
            .lock()
            .unwrap()
            .get("disc-test-1")
            .is_none();
        assert!(
            waiter_cleared,
            "끊김 후 feed_waiters['disc-test-1'] 엔트리가 정리되지 않고 잔존"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    async fn wait_until<F: FnMut() -> bool>(limit: Duration, mut cond: F) -> bool {
        let deadline = std::time::Instant::now() + limit;
        while std::time::Instant::now() < deadline {
            if cond() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        cond()
    }
}

#[cfg(test)]
mod pipe_security_tests {
    use super::PIPE_SDDL_OWNER_ONLY;

    /// ★회귀 박제 (Windows named pipe = UDS 0o600 대칭 봉인):
    /// 기본 ServerOptions::create()는 lpSecurityAttributes=NULL로 파이프를 만들어
    /// 기본 DACL(같은 머신 임의 로컬 사용자에게 read/write 허용)을 받는다 — 인증 없는
    /// 제어 채널(send_text·send_key·ledger.kill)이 권한 우회로 노출되는 비대칭.
    /// 수정본은 owner-only SDDL을 SECURITY_ATTRIBUTES로 실어 creator·SYSTEM·Administrators만
    /// 접근하게 봉인한다. 이 테스트는 그 SDDL이 (a)광역 SID를 포함하지 않고 (b)보호된 DACL이며
    /// (c)owner를 명시 허용함을 단정해, 누군가 광역 권한을 다시 끼워넣거나 D:P를 떼어내면 깨진다.
    /// (Windows arm은 이 호스트에서 컴파일/실행 불가하므로, SDDL 문자열 정합성으로 의도를 박제한다.)
    #[test]
    fn pipe_sddl_excludes_world_and_is_protected_owner_only() {
        let sddl = PIPE_SDDL_OWNER_ONLY;
        // (b) 보호된 DACL — 부모 ACL 상속을 차단해 광역 ACE가 흘러들지 않게 한다.
        assert!(
            sddl.starts_with("D:P"),
            "DACL must be protected (D:P) to block inherited world ACEs: {sddl}"
        );
        // (c) owner(creator)·SYSTEM·Administrators full-access ACE 존재.
        assert!(
            sddl.contains("(A;;FA;;;OW)"),
            "owner (OW) must have full access: {sddl}"
        );
        assert!(
            sddl.contains("(A;;FA;;;SY)") && sddl.contains("(A;;FA;;;BA)"),
            "SYSTEM (SY) and Administrators (BA) must be present: {sddl}"
        );
        // (a) 광역 SID 금지: Everyone(WD)·Authenticated Users(AU)·Anonymous(AN)·
        //     Network(NU)가 ACE로 들어오면 같은 머신/네트워크의 타 사용자가 접근 가능 → 회귀.
        for world in [";;;WD)", ";;;AU)", ";;;AN)", ";;;NU)"] {
            assert!(
                !sddl.contains(world),
                "broad SID {world} would re-open the pipe to other users: {sddl}"
            );
        }
        // deny ACE("D;")가 아닌 allow ACE("A;")만으로 구성 — 의도된 화이트리스트.
        assert!(
            !sddl.contains("(D;"),
            "owner-only seal should be an allow-list, not contain deny ACEs: {sddl}"
        );
    }

    /// ★회귀 박제 (Windows accept_loop의 connect() 오류 후 100% CPU spin 방지):
    /// 과거 Windows arm은 `loop { if server.connect().await.is_ok() { ... } }` 형태로
    /// 오류 분기가 전무했다. mio `ConnectNamedPipe`는 진짜 OS 오류를 즉시 Err로 돌려주고
    /// (정상 대기만 WouldBlock→tokio await) connecting 플래그도 즉시 해제하므로, 같은 broken
    /// 인스턴스에 sleep 없이 곧장 재시도하면 같은 Err가 무한 반복돼 tokio 워커 스레드가 영구
    /// 100% CPU를 태운다(자원 거버넌스를 표방하는 24/365 데몬에 치명적). 수정본은 오류 분기에서
    /// ①로그 ②인스턴스 재생성 ③backoff sleep로 회생한다. 그 backoff가 0이면 spin이 되살아나므로,
    /// 정책 상수가 non-zero임을 단정해 누가 다시 0/제거하면 깨지게 박제한다.
    /// (Windows arm은 이 호스트에서 컴파일/실행 불가하므로 정책 상수 정합성으로 의도를 박제한다 —
    ///  PIPE_SDDL_OWNER_ONLY 박제와 같은 방식.)
    #[test]
    fn pipe_accept_error_backoff_is_nonzero_to_prevent_cpu_spin() {
        let backoff = super::PIPE_ACCEPT_ERROR_BACKOFF;
        assert!(
            !backoff.is_zero(),
            "accept-error backoff must be non-zero, else connect() Err re-tries on the same \
             broken pipe instance with no yield → 100% CPU spin: {backoff:?}"
        );
    }

    /// ★회귀 박제 (Windows named pipe 리스너 풀 — ERROR_PIPE_BUSY 231 봉인):
    /// named pipe 엔 UDS listen backlog 가 없어 '여분 listening 인스턴스 수'가 곧 동시 접속
    /// 수용량이다. 풀이 1로 돌아가면 accept→인스턴스 재생성 사이 창에 도착한 동시 접속
    /// (멀티 노드 RPC + GUI 기동 fan-out)이 전부 os error 231("모든 파이프 인스턴스가 사용 중")
    /// 로 튕긴다 — 2026-07-10 Windows GUI "startup failed" 실사고의 서버측 근원. 누가 풀을
    /// 다시 1로 줄이면 이 테스트가 깨진다. (Windows arm 은 이 호스트에서 컴파일/실행 불가하므로
    /// 정책 상수 정합성으로 의도를 박제한다 — PIPE_ACCEPT_ERROR_BACKOFF 박제와 같은 방식.)
    #[test]
    fn pipe_listener_pool_absorbs_concurrent_connects() {
        let pool = super::PIPE_LISTENER_POOL;
        assert!(
            pool >= 2,
            "listener pool must be ≥2, else concurrent connects hit ERROR_PIPE_BUSY(231) \
             in the accept→recreate window: {pool}"
        );
    }
}

#[cfg(test)]
mod auto_restore_tests {
    use super::{
        autorestore_retry_delay, decide_auto_restore, guard_restore_panic, loop_auto_restore_with,
        run_auto_restore_once, AutoRestore,
    };

    /// ★P0-7 회귀 잠금(D1/W5·CI 실경로 ⑧): 양 플랫폼 accept_loop 가 콜드부트 부트 공통 함수를 호출하는지 소스
    /// 수준으로 잠근다 — Windows accept_loop 에 배선이 빠져 auto-restore 가 발동조차 안 하던 P0-7 최종 결함 재발
    /// 차단. 호출 형태가 정확히 2회(unix+windows)여야 한다. (needle 은 concat! 으로 쪼개 이 테스트 자신을 세지
    /// 않게 한다 — 소스에 contiguous 리터럴이 없다.)
    #[test]
    fn post_listen_boot_wired_in_both_accept_loops() {
        let src = include_str!("main.rs");
        let needle = concat!("post_listen_boot", "(socket_path, &daemon)");
        let calls = src.matches(needle).count();
        assert_eq!(
            calls, 2,
            "콜드부트 부트 호출이 양 accept_loop(unix+windows)에 정확히 2회여야 한다(현재 {calls}회) — \
             한쪽 미배선/중복은 콜드부트 auto-restore 플랫폼 비대칭(P0-7) 재발"
        );
    }

    /// ★P0-5(D3/W5·CI 28780215417): auto-restore 스레드 panic 을 삼키지 않고 포착·기록하는지 — 재현 테스트.
    /// panic 하는 body → guard 는 false 반환(전파 안 함)·phoenix-restore.log 에 PANIC 기록. 정상 body → true.
    #[test]
    fn guard_restore_panic_catches_and_logs_no_propagation() {
        let dir = std::env::temp_dir().join(format!("cys-ar-panic-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("phoenix-restore.log");

        // ① panic body → 포착(프로세스 안 죽음)·false 반환·로그에 PANIC 기록.
        let ok = guard_restore_panic(&log, || panic!("boom time.rs"));
        assert!(!ok, "panic 은 false 로 포착돼야 한다(전파 금지)");
        let logged = std::fs::read_to_string(&log).unwrap_or_default();
        assert!(
            logged.contains("AUTO-RESTORE THREAD PANIC") && logged.contains("boom time.rs"),
            "panic 이 phoenix-restore.log 에 기록돼야 한다(침묵사 금지): {logged}"
        );

        // ② 정상 body → true·body 실행됨.
        let ran = std::sync::atomic::AtomicBool::new(false);
        let ok2 = guard_restore_panic(&log, || {
            ran.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        assert!(
            ok2 && ran.load(std::sync::atomic::Ordering::SeqCst),
            "정상 body 는 true·실행"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// opt-out(CYS_NO_AUTORESTORE=1)이면 phoenix가 있어도 스폰하지 않는다.
    #[test]
    fn opted_out_never_spawns() {
        let dir = std::env::temp_dir().join(format!("cys-ar-optout-{}", std::process::id()));
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("javis_phoenix.py"), "#!/usr/bin/env python3\n").unwrap();
        assert_eq!(
            decide_auto_restore(&dir, true, &bin, "/usr/bin:/bin", "sock:test"),
            AutoRestore::OptedOut
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★B1: 디스크 phoenix 부재여도 Ready(임베드 추출이 권위) — 과거 PhoenixMissing skip 은 폐기.
    /// args[0]=디스크 phoenix 경로(폴백 후보)로 유지된다.
    #[test]
    fn missing_disk_phoenix_still_ready_embed_authoritative() {
        let dir = std::env::temp_dir().join(format!("cys-ar-missing-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        match decide_auto_restore(&dir, false, &dir, "/usr/bin:/bin", "sock:test") {
            AutoRestore::Ready { args, .. } => {
                assert!(
                    args[0].ends_with("bin/javis_phoenix.py"),
                    "폴백 후보 경로: {}",
                    args[0]
                );
                // args = [phoenix, "--socket", <sock>, "restore", "--auto"] — W6/E1 소켓 명시 전달.
                assert_eq!(args[1], "--socket");
                assert_eq!(&args[3..], &["restore".to_string(), "--auto".to_string()]);
            }
            other => panic!("expected Ready, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// phoenix 설치 시 `python3 <phoenix> restore --auto` 스폰 스펙을 낸다(--auto 필수).
    #[test]
    fn present_phoenix_builds_auto_restore_command() {
        let dir = std::env::temp_dir().join(format!("cys-ar-ready-{}", std::process::id()));
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let ph = bin.join("javis_phoenix.py");
        std::fs::write(&ph, "#!/usr/bin/env python3\n").unwrap();
        match decide_auto_restore(&dir, false, &bin, "/usr/bin:/bin", "sock:test") {
            AutoRestore::Ready { program, args, .. } => {
                assert_eq!(program, "python3");
                assert_eq!(args[0], ph.to_string_lossy());
                // args = [phoenix, "--socket", <sock>, "restore", "--auto"] — W6/E1 소켓 명시 전달.
                assert_eq!(args[1], "--socket");
                assert_eq!(&args[3..], &["restore".to_string(), "--auto".to_string()]);
            }
            other => panic!("expected Ready, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★D4(W5): 인터프리터 해석 — 동봉 runtime python3 이 실존하면 program 은 **그 절대경로**여야 한다
    /// ("python3" 리터럴 폴백이 아니라). 순정 Windows(python3 부재)·mac CLT 미설치 소비자에서 첫 스폰 단절
    /// (P0-7·P1-9)을 절대경로로 끊는 핵심. 기존 present_phoenix_builds_auto_restore_command 는 **번들 부재
    /// 폴백**만 검증(program=="python3")했다 — 그 리터럴 단언만으로는 절대경로 해석 결함을 통과시킨다(설계 D4 지적).
    #[test]
    fn ready_prefers_bundled_python_absolute_path() {
        let dir = std::env::temp_dir().join(format!("cys-ar-bundlepy-{}", std::process::id()));
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("javis_phoenix.py"), "#!/usr/bin/env python3\n").unwrap();
        // exe_dir(bin) 기준 동봉 runtime python 디렉터리에 python3 실행파일을 둔다(runtime_bin_dirs SOT 와 일치).
        //   mac: <exe_dir>/runtime/python/bin/python3 · win: <exe_dir>/runtime/python/python3.exe
        let (py_dir, py_name) = if cfg!(windows) {
            (bin.join("runtime").join("python"), "python3.exe")
        } else {
            (bin.join("runtime").join("python").join("bin"), "python3")
        };
        std::fs::create_dir_all(&py_dir).unwrap();
        let py_path = py_dir.join(py_name);
        std::fs::write(&py_path, "#!/bin/sh\n").unwrap();
        match decide_auto_restore(&dir, false, &bin, "/usr/bin:/bin", "sock:test") {
            AutoRestore::Ready { program, .. } => {
                assert_eq!(
                    program,
                    py_path.to_string_lossy(),
                    "동봉 python3 실존 시 program 은 절대경로여야 한다(리터럴 'python3' 아님)"
                );
                assert_ne!(
                    program, "python3",
                    "리터럴 폴백이면 D4 결함(절대경로 미해석)"
                );
            }
            other => panic!("expected Ready, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★W1/B3: exe 옆에 cys 가 실존하면 PHOENIX_CYS 를 그 절대경로로 주입하고, exe_dir 가 PATH 에 없으면
    /// PATH 를 선두주입한다(GUI/데몬 최소 PATH 침묵사 근원 수리). "python3" 문자열 단언만으로는 불충분(D4).
    #[test]
    fn ready_injects_phoenix_cys_and_path_env() {
        let dir = std::env::temp_dir().join(format!("cys-ar-env-{}", std::process::id()));
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("javis_phoenix.py"), "#!/usr/bin/env python3\n").unwrap();
        // exe_dir 에 실행가능 cys 스텁을 둔다(PHOENIX_CYS 주입 조건 = 파일 실존).
        let cys_name = if cfg!(windows) { "cys.exe" } else { "cys" };
        let cys_path = bin.join(cys_name);
        std::fs::write(&cys_path, "#!/bin/sh\n").unwrap();
        // GUI/데몬 최소 PATH 모사 — exe_dir 미포함이라 선두주입이 일어나야 한다.
        match decide_auto_restore(
            &dir,
            false,
            &bin,
            "/usr/bin:/bin:/usr/sbin:/sbin",
            "sock:test",
        ) {
            AutoRestore::Ready { env, .. } => {
                let cys_env = env
                    .iter()
                    .find(|(k, _)| k == "PHOENIX_CYS")
                    .map(|(_, v)| v.clone());
                assert_eq!(
                    cys_env.as_deref(),
                    Some(cys_path.to_string_lossy().as_ref()),
                    "PHOENIX_CYS 는 exe 옆 cys 절대경로여야 한다"
                );
                let path_env = env
                    .iter()
                    .find(|(k, _)| k == "PATH")
                    .map(|(_, v)| v.clone())
                    .expect("PATH 선두주입이 있어야 한다(exe_dir 미포함 PATH)");
                assert!(
                    path_env.starts_with(bin.to_string_lossy().as_ref()),
                    "PATH 는 exe_dir 선두여야 한다: {path_env}"
                );
            }
            other => panic!("expected Ready, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★W1/B3: exe 옆에 cys 가 없으면 PHOENIX_CYS 를 주입하지 않는다(존재하지 않는 경로 강제 주입으로
    /// 재차 FileNotFoundError 를 만들지 않는다 — phoenix 의 which→표준경로 폴백에 위임).
    #[test]
    fn ready_omits_phoenix_cys_when_absent() {
        let dir = std::env::temp_dir().join(format!("cys-ar-nocys-{}", std::process::id()));
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("javis_phoenix.py"), "#!/usr/bin/env python3\n").unwrap();
        match decide_auto_restore(&dir, false, &bin, "/usr/bin:/bin", "sock:test") {
            AutoRestore::Ready { env, .. } => {
                assert!(
                    !env.iter().any(|(k, _)| k == "PHOENIX_CYS"),
                    "cys 부재 시 PHOENIX_CYS 무주입이어야 한다: {env:?}"
                );
            }
            other => panic!("expected Ready, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    use super::{
        bundled_python3, disk_fallback_verify, extract_phoenix_embed, phoenix_embed_files,
        phoenix_self_test,
    };
    use std::cell::RefCell;
    use std::time::Duration;

    /// ★codex W4 fix1: 추출 중간 실패(seam CYS_PHOENIX_EXTRACT_FAIL) 시 partial root 즉시 정리 — phoenix-embed 잔여 0.
    #[test]
    fn b1_extract_mid_failure_leaves_no_partial_root() {
        let sd = std::env::temp_dir().join(format!("cys-b1mf-{}", std::process::id()));
        std::fs::create_dir_all(&sd).unwrap();
        std::env::set_var("CYS_PHOENIX_EXTRACT_FAIL", "1");
        let res = extract_phoenix_embed(&sd);
        std::env::remove_var("CYS_PHOENIX_EXTRACT_FAIL");
        assert!(res.is_err(), "주입된 중간 실패가 Err 여야 한다");
        // phoenix-embed 하위 child dir 0(즉시 정리 — 다음 부팅 prune 의존 금지).
        let root = sd.join("phoenix-embed");
        let children = std::fs::read_dir(&root).map(|r| r.count()).unwrap_or(0);
        assert_eq!(children, 0, "중간 실패 후 partial root 잔존");
        let _ = std::fs::remove_dir_all(&sd);
    }

    /// ★codex W4 fix2: 디스크 폴백은 script-only 가 아니라 phoenix closure 전체 대조.
    /// phoenix.py 는 일치해도 형제(javis_state_snapshot.py) 부재/stale 이면 거부(어느 rel 인지 보고).
    #[test]
    fn b1_disk_fallback_full_tree_verify() {
        let pack = std::env::temp_dir().join(format!("cys-b1ft-{}", std::process::id()));
        let bin = pack.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        // 전 closure 를 임베드 내용 그대로 디스크에 배치 → verified(Ok).
        for (rel, content) in phoenix_embed_files() {
            let p = pack.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, content).unwrap();
        }
        let disk_phoenix = bin.join("javis_phoenix.py");
        assert!(
            disk_fallback_verify(&disk_phoenix).is_ok(),
            "전 closure 일치 → verified"
        );
        // 형제 stale: snapshot.py 를 변조 → 거부(rel 명시).
        std::fs::write(bin.join("javis_state_snapshot.py"), "STALE-SNAPSHOT-DRIFT").unwrap();
        let e = disk_fallback_verify(&disk_phoenix).unwrap_err();
        assert!(
            e.contains("javis_state_snapshot.py"),
            "stale 형제 rel 미보고: {e}"
        );
        // 형제 부재: snapshot.py 삭제 → 거부(부재 명시).
        std::fs::remove_file(bin.join("javis_state_snapshot.py")).unwrap();
        let e2 = disk_fallback_verify(&disk_phoenix).unwrap_err();
        assert!(
            e2.contains("javis_state_snapshot.py") && e2.contains("부재"),
            "부재 형제 미보고: {e2}"
        );
        let _ = std::fs::remove_dir_all(&pack);
    }

    /// ★B1①: 임베드 추출이 phoenix.py + 형제 의존(javis_state_snapshot.py)을 버전+uuid 격리 디렉터리에
    /// 임베드 내용 그대로 쓴다. temp 누수 0: 정리 후 디렉터리 소멸.
    #[test]
    fn b1_extract_writes_phoenix_and_deps() {
        let sd = std::env::temp_dir().join(format!("cys-b1x-{}", std::process::id()));
        std::fs::create_dir_all(&sd).unwrap();
        let (root, script) = extract_phoenix_embed(&sd).expect("추출 성공");
        assert!(script.ends_with("bin/javis_phoenix.py"));
        assert!(script.is_file(), "phoenix.py 추출 안됨");
        let snap = root.join("bin").join("javis_state_snapshot.py");
        assert!(snap.is_file(), "형제 의존 javis_state_snapshot.py 미추출");
        // 내용 == 임베드
        let embed_phoenix = phoenix_embed_files()
            .into_iter()
            .find(|(rel, _)| *rel == "bin/javis_phoenix.py")
            .map(|(_, c)| c)
            .unwrap();
        assert_eq!(std::fs::read_to_string(&script).unwrap(), embed_phoenix);
        // 버전+uuid 격리 경로
        assert!(root.parent().unwrap().ends_with("phoenix-embed"));
        // temp 누수 0
        std::fs::remove_dir_all(&root).unwrap();
        assert!(!root.exists());
        let _ = std::fs::remove_dir_all(&sd);
    }

    /// ★B1③: 추출된 실 phoenix 가 --selftest 를 통과한다(python3 가용 시). self-test 게이트 실증.
    #[test]
    fn b1_self_test_passes_on_real_embed() {
        let py = match std::process::Command::new("python3")
            .arg("--version")
            .output()
        {
            Ok(o) if o.status.success() => "python3".to_string(),
            _ => {
                eprintln!("python3 미가용 — self-test 게이트 skip");
                return;
            }
        };
        let sd = std::env::temp_dir().join(format!("cys-b1st-{}", std::process::id()));
        std::fs::create_dir_all(&sd).unwrap();
        let (root, script) = extract_phoenix_embed(&sd).expect("추출 성공");
        assert!(phoenix_self_test(&py, &script), "실 임베드 self-test 실패");
        // 존재하지 않는 스크립트는 self-test 실패(정직 강등 경로)
        assert!(!phoenix_self_test(&py, &root.join("bin").join("nope.py")));
        std::fs::remove_dir_all(&root).unwrap();
        let _ = std::fs::remove_dir_all(&sd);
    }

    /// ★B1 temp 누수 0: 크래시로 남은 이전 추출 디렉터리를 부트 시 prune 한다(정리 후 phoenix-embed 비움).
    #[test]
    fn b1_prune_stale_embed_dirs() {
        use super::prune_stale_phoenix_embed;
        let sd = std::env::temp_dir().join(format!("cys-b1p-{}", std::process::id()));
        let root = sd.join("phoenix-embed");
        // 이전 실행 잔재 2개 모사(크래시로 cleanup 못한 것).
        for u in ["0.12.20-111-222", "0.12.20-333-444"] {
            std::fs::create_dir_all(root.join(u).join("bin")).unwrap();
            std::fs::write(root.join(u).join("bin").join("x.py"), "stale").unwrap();
        }
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 2);
        prune_stale_phoenix_embed(&sd);
        // prune 후 잔재 0(디렉터리 자체는 남아도 하위 비움).
        let remaining = std::fs::read_dir(&root).map(|r| r.count()).unwrap_or(0);
        assert_eq!(remaining, 0, "prune 후 잔여 추출 디렉터리 존재");
        // phoenix-embed 부재(부트 첫 회)에서도 panic 없이 무해.
        let empty = std::env::temp_dir().join(format!("cys-b1p-empty-{}", std::process::id()));
        std::fs::create_dir_all(&empty).unwrap();
        prune_stale_phoenix_embed(&empty); // no-op·무패닉
        let _ = std::fs::remove_dir_all(&sd);
        let _ = std::fs::remove_dir_all(&empty);
    }

    /// ★B3: 동봉 runtime python3 가 있으면 program 은 그 절대경로(리터럴 "python3" 아님). mac 레이아웃
    /// (runtime/python/bin/python3)으로 검증 — 순정 Windows/mac CLT 미설치 첫 스폰 단절 수리의 핵심.
    #[cfg(target_os = "macos")]
    #[test]
    fn b3_bundled_python_absolute_path_preferred() {
        let dir = std::env::temp_dir().join(format!("cys-b3-{}", std::process::id()));
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("javis_phoenix.py"), "#!/usr/bin/env python3\n").unwrap();
        // exe_dir=bin. 동봉 python: bin/runtime/python/bin/python3.
        let pybin = bin.join("runtime").join("python").join("bin");
        std::fs::create_dir_all(&pybin).unwrap();
        let py = pybin.join("python3");
        std::fs::write(&py, "#!/bin/sh\n").unwrap();
        assert_eq!(
            bundled_python3(&bin).as_deref(),
            Some(py.to_string_lossy().as_ref())
        );
        match decide_auto_restore(&dir, false, &bin, "/usr/bin:/bin", "sock:test") {
            AutoRestore::Ready { program, .. } => {
                assert_eq!(
                    program,
                    py.to_string_lossy(),
                    "동봉 python3 절대경로여야 한다"
                );
            }
            other => panic!("expected Ready, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★B3: 동봉 runtime 이 없으면 program 은 "python3" 리터럴(PATH 폴백).
    #[test]
    fn b3_no_bundled_python_falls_back_to_literal() {
        let dir = std::env::temp_dir().join(format!("cys-b3-nolit-{}", std::process::id()));
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("javis_phoenix.py"), "#!/usr/bin/env python3\n").unwrap();
        assert_eq!(bundled_python3(&bin), None);
        match decide_auto_restore(&dir, false, &bin, "/usr/bin:/bin", "sock:test") {
            AutoRestore::Ready { program, .. } => assert_eq!(program, "python3"),
            other => panic!("expected Ready, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★W1(codex major): 1차 비0 → (수동 복원으로 대상 라이브) → 2차 NOOP(=0)·정확히 2회 실행·중복 스폰 0.
    /// run_auto_restore_once 대신 스크립트 러너를 주입해 sleep 0 으로 결정론 검증(60s 실 sleep 회귀 회피).
    #[test]
    fn retry_first_nonzero_then_noop_runs_twice() {
        let calls = RefCell::new(Vec::<u32>::new());
        // 1차=exit 1(비0) → 2차=exit 0(수동 복원 후 재산정 NOOP). 스폰은 각 attempt 1회씩만(중복 0).
        let scripted = |attempt: u32| -> Option<i32> {
            calls.borrow_mut().push(attempt);
            if attempt == 0 {
                Some(1)
            } else {
                Some(0)
            }
        };
        let runs = loop_auto_restore_with(scripted, Duration::from_millis(0));
        assert_eq!(runs, 2, "1차 비0→2차 실행이어야 한다(정확히 2회)");
        assert_eq!(
            *calls.borrow(),
            vec![0, 1],
            "attempt 0,1 각 1회 — 중복 스폰 0"
        );
    }

    /// ★재시도 소진: 2차도 비0이면 무한 재시도 금지(정확히 2회에서 종료).
    #[test]
    fn retry_exhausts_after_one_retry() {
        let n = RefCell::new(0u32);
        let runs = loop_auto_restore_with(
            |_a| {
                *n.borrow_mut() += 1;
                Some(1)
            },
            Duration::from_millis(0),
        );
        assert_eq!(runs, 2, "비0 후 1회만 재시도 — 무한 루프 금지");
        assert_eq!(*n.borrow(), 2);
    }

    /// ★exit 5(BREAKER)·6(CORRUPT/identity)=재시도 금지 — 정확히 1회 실행.
    #[test]
    fn breaker_and_corrupt_never_retry() {
        for code in [5, 6] {
            let n = RefCell::new(0u32);
            let runs = loop_auto_restore_with(
                |_a| {
                    *n.borrow_mut() += 1;
                    Some(code)
                },
                Duration::from_millis(0),
            );
            assert_eq!(runs, 1, "exit {code} 는 재시도 금지(1회 실행)");
        }
    }

    /// ★성공(0)은 재시도 없음 — 1회 실행.
    #[test]
    fn success_runs_once() {
        let runs = loop_auto_restore_with(|_a| Some(0), Duration::from_millis(0));
        assert_eq!(runs, 1);
    }

    /// ★스폰 실패(None)도 비0 클래스 — 1회 재시도 후 소진(2회).
    #[test]
    fn spawn_failure_retries_once() {
        let runs = loop_auto_restore_with(|_a| None, Duration::from_millis(0));
        assert_eq!(runs, 2, "None(스폰 실패)도 1회 재시도 후 종료");
    }

    /// ★CYS_AUTORESTORE_RETRY_DELAY_MS override 파싱(기본 60000·override 반영).
    #[test]
    fn retry_delay_env_override() {
        // 기본값
        std::env::remove_var("CYS_AUTORESTORE_RETRY_DELAY_MS");
        assert_eq!(autorestore_retry_delay(), Duration::from_millis(60_000));
        // override — 이 테스트만 단일 스레드 실행 계약(--test-threads=1)이라 env 격리 안전.
        std::env::set_var("CYS_AUTORESTORE_RETRY_DELAY_MS", "0");
        assert_eq!(autorestore_retry_delay(), Duration::from_millis(0));
        std::env::remove_var("CYS_AUTORESTORE_RETRY_DELAY_MS");
    }

    /// ★T6-L1: RestoreRootGuard 수명 계약 — 정상 스코프·panic unwind·loop 다중 attempt 모든 경로에서
    /// restore_roots 가 정확히 비워진다(등록 해제의 유일 경로가 Drop 임을 고정). guard drop 이 빠지면
    /// 복원 종료 후 잔존 자손이 authoritative 면제를 얻는 A7 취약이 재발한다.
    #[test]
    fn restore_roots_cleared_on_all_paths_l1() {
        use crate::state::{Daemon, RestoreRootGuard};
        let dir = std::env::temp_dir().join(format!(
            "cys-l1-{}-{}",
            std::process::id(),
            crate::state::now_epoch() as u64
        ));
        let _ = std::fs::create_dir_all(&dir);
        let daemon = Daemon::new(dir.join("cysd.sock"));

        // ① 정상 스코프: 등록 중 1개, 스코프 종료(drop) 후 빔.
        {
            let _g = RestoreRootGuard::new(daemon.clone(), 4242, 111);
            assert_eq!(
                daemon.restore_roots.lock().unwrap().len(),
                1,
                "등록 중 1개여야"
            );
        }
        assert!(
            daemon.restore_roots.lock().unwrap().is_empty(),
            "정상 drop 후 restore_roots 가 비지 않았다"
        );

        // ② panic unwind: catch_unwind 안에서 guard 살아있는 채 panic → Drop 이 unwind 중 해제.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _g = RestoreRootGuard::new(daemon.clone(), 4243, 222);
            assert_eq!(daemon.restore_roots.lock().unwrap().len(), 1);
            panic!("unwind through guard");
        }));
        assert!(
            daemon.restore_roots.lock().unwrap().is_empty(),
            "panic unwind 후 restore_roots 가 비지 않았다 (A7/L1)"
        );

        // ③ loop 다중 attempt: 각 attempt 가 자기 guard 를 등록·해제 → attempt 중 정확히 1개, 종료 후 빔.
        let d2 = daemon.clone();
        let runs = loop_auto_restore_with(
            move |attempt| {
                let _g = RestoreRootGuard::new(d2.clone(), 5000 + attempt, 333);
                assert_eq!(
                    d2.restore_roots.lock().unwrap().len(),
                    1,
                    "attempt 중 정확히 1개여야(누적 0)"
                );
                if attempt == 0 {
                    Some(1)
                } else {
                    Some(0)
                } // 1차 비0 → 2차 실행
            },
            Duration::from_millis(0),
        );
        assert_eq!(runs, 2, "1차 비0→2차 실행(정확히 2회)");
        assert!(
            daemon.restore_roots.lock().unwrap().is_empty(),
            "loop 종료 후 restore_roots 가 비지 않았다 (L1)"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★T6-L1(실 실행): run_auto_restore_once 가 자식을 spawn·reap(좀비 0)하고 종료 후 restore_roots 를
    /// 비우며 exit code 를 계약대로 매핑한다(status().code() 동형). sleep 자식으로 등록 창을 실재화한다.
    #[cfg(unix)]
    #[test]
    fn run_auto_restore_once_reaps_and_clears_l1() {
        use crate::state::Daemon;
        let dir = std::env::temp_dir().join(format!(
            "cys-l1run-{}-{}",
            std::process::id(),
            crate::state::now_epoch() as u64
        ));
        let _ = std::fs::create_dir_all(&dir);
        let daemon = Daemon::new(dir.join("cysd.sock"));
        let log = dir.join("phoenix-restore.log");

        // sleep 후 특정 코드 종료 — 관측 창 확보 + exit 매핑 검증. wait() 가 reap 한다.
        let code = run_auto_restore_once(
            &daemon,
            "sh",
            &["-c".to_string(), "sleep 0.2; exit 7".to_string()],
            &[],
            &log,
        );
        assert_eq!(
            code,
            Some(7),
            "exit code 계약 매핑이 깨졌다(status().code() 동형)"
        );
        assert!(
            daemon.restore_roots.lock().unwrap().is_empty(),
            "run_auto_restore_once 종료 후 guard drop 으로 restore_roots 가 비어야 한다 (L1)"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
