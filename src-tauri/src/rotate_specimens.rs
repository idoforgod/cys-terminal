//! ★W-8a(F2) 윈도우 실기 검체 D1–D10 — 설계 정본 D-1 §4.4(reviewer-codex · 2026-09-12)의 F2 표.
//!
//! 재는 것: 데몬 교대가 **성공이라고 말할 때** 그 엔드포인트에 정말 목표 버전의 **새 세대**가 붙어
//! 있는가 — 그리고 그렇지 않을 때(구 데몬 생존·신원 불명·다른 구버전·팩 실패) 성공이라고 **말하지
//! 않는가**. 실제 Win32 named pipe·프로세스로 돈다(`windows-build.yml` T7 스텝 전용 — `#[ignore]`).
//!
//! 오라클은 교대 코드와 **독립**이다(producer≠evaluator): 교대가 끝난 뒤 이 파일이 엔드포인트에 새로
//! 연결해 OS server PID·프로세스 생성 시각·보고 버전을 직접 재고, 프로세스 생존은 핸들로 본다. 교대
//! 코드가 내는 영수증은 오라클이 아니라 **대조 대상**이다(있으면 실측과 같아야 한다).
//!
//! ★음성 대조 계약: 파일 끝의 `rotate_entry`·`dept_before`·`dept_after` 세 함수만 수정 전/후 트리에서
//!   다르다. 검체 본문(준비·오라클)은 같다 — 1회차(수정 전 production 순서) 적색 → 2회차(수정본) 초록.
//!
//! 주입 지점은 둘뿐이다(교대 코드를 모사하지 않는다):
//!   ① 가짜 taskkill — Rust `Command` 는 System32 보다 **응용 프로그램 폴더**(테스트 exe 폴더)를 먼저
//!      찾으므로, 그 폴더에 놓인 동안만 production 의 `Command::new("taskkill")` 이 가짜를 부른다(D2·D3·D4).
//!      놓은 직후 같은 해소 규칙으로 불러 가짜가 실제로 잡히는지 확인한다(주입 불성립 = 검체 실패).
//!   ② '구 데몬 종료 직후' 훅 — 다른 서버가 바로 그 순간 끼어드는 경우(D5·D6·D7).
//!
//! env(T7 스텝이 넣는다 · 없으면 검체 **실패** — skip·미기동은 합격이 아니다):
//!   W8A_OLD_CYSD       실제 구버전 cysd.exe(공개 릴리스 설치기에서 추출 · T7 이 해시·버전 리소스 확인)
//!   W8A_TARGET_CYSD    이 SHA 로 빌드한 cysd 사이드카(목표 버전)
//!   W8A_TARGET_CYS     이 SHA 로 빌드한 cys 사이드카(D9 의 init-pack)
//!   W8A_FAKE_TASKKILL  가짜 taskkill.exe — 모드는 env W8A_FAKE_TASKKILL_MODE(fail·lie·killfail)
//!   W8A_PACK_ROOT      검체별 CYS_PACK_DIR 를 만들 임시 루트

use serde_json::{json, Value};
use std::io::BufRead as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _};
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions};

/// 목표 버전 = 이 빌드의 GUI 버전(교대가 요청 시작 시 고정하는 값과 같은 원천).
const TARGET: &str = env!("CARGO_PKG_VERSION");
/// 교대 1회 상한 — 넘으면 **실패**다(끝나지 않는 교대는 G-① 위반이지 '실패로 끝남'이 아니다).
const ROTATE_BUDGET: Duration = Duration::from_secs(60);
/// 데몬 준비 대기 상한(콜드 부트의 팩 설치 포함).
const READY_SECS: u64 = 40;

// ── env · 사이드카 배치 ─────────────────────────────────────────────────────

struct Env {
    old_cysd: PathBuf,
    target_cysd: PathBuf,
    target_cys: PathBuf,
    fake_taskkill: PathBuf,
    pack_root: PathBuf,
}

fn env() -> &'static Env {
    static E: std::sync::OnceLock<Env> = std::sync::OnceLock::new();
    E.get_or_init(|| {
        let need = |k: &str| -> PathBuf {
            let v = std::env::var(k).unwrap_or_else(|_| {
                panic!("{k} 미설정 — windows-build.yml T7 전용 검체다(skip 은 합격이 아니므로 실패로 끝낸다)")
            });
            let p = PathBuf::from(v);
            assert!(p.exists(), "{k} 경로 부재: {}", p.display());
            p
        };
        Env {
            old_cysd: need("W8A_OLD_CYSD"),
            target_cysd: need("W8A_TARGET_CYSD"),
            target_cys: need("W8A_TARGET_CYS"),
            fake_taskkill: need("W8A_FAKE_TASKKILL"),
            pack_root: need("W8A_PACK_ROOT"),
        }
    })
}

/// 테스트 exe 폴더 = production 이 사이드카(cysd.exe·cys.exe)를 찾는 '앱 옆' 자리.
fn app_dir() -> PathBuf {
    std::env::current_exe()
        .expect("current_exe")
        .parent()
        .expect("exe 폴더")
        .to_path_buf()
}

/// 목표 사이드카를 앱 옆에 둔다(프로세스당 1회) — ensure_daemon·sealed_sidecar_cys 가 production
/// 규칙(exe 옆 우선)으로 찾게 한다. 검체가 교대 코드에 경로를 따로 넘기지 않는다.
fn place_sidecars() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let e = env();
        for (src, name) in [(&e.target_cysd, "cysd.exe"), (&e.target_cys, "cys.exe")] {
            let dst = app_dir().join(name);
            std::fs::copy(src, &dst).unwrap_or_else(|err| {
                panic!("사이드카 배치 실패 {} → {}: {err}", src.display(), dst.display())
            });
        }
    });
}

fn pipe_name(tag: &str, role: &str) -> PathBuf {
    PathBuf::from(format!(r"\\.\pipe\cys-w8a-{tag}-{role}-{}", std::process::id()))
}

// ── 독립 오라클: OS 가 보는 신원 ────────────────────────────────────────────

/// 프로세스 생성 시각(FILETIME 100ns 단위) — PID 재사용을 가르는 세대 표지. 조회 불가 = None.
fn created_of(pid: u32) -> Option<u64> {
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME};
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if h.is_null() {
        return None;
    }
    let zero = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
    let (mut c, mut x, mut k, mut u) = (zero, zero, zero, zero);
    let ok = unsafe { GetProcessTimes(h, &mut c, &mut x, &mut k, &mut u) } != 0;
    unsafe { CloseHandle(h) };
    ok.then(|| ((c.dwHighDateTime as u64) << 32) | c.dwLowDateTime as u64)
}

/// (pid, 생성 시각) 세대가 아직 살아 있는가 — 생성 시각이 다르면 PID 재사용이라 그 프로세스는 없다.
fn alive(pid: u32, created: u64) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_TIMEOUT};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
    };
    if created_of(pid) != Some(created) {
        return false;
    }
    let h = unsafe { OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if h.is_null() {
        return false;
    }
    let r = unsafe { WaitForSingleObject(h, 0) };
    unsafe { CloseHandle(h) };
    r == WAIT_TIMEOUT
}

/// 정리 전용 — 같은 세대일 때만 끝낸다(자기 자신·재사용 PID 무접촉).
fn terminate(pid: u32, created: u64) {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, TerminateProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
    };
    if pid == std::process::id() || created_of(pid) != Some(created) {
        return;
    }
    let h = unsafe { OpenProcess(PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if h.is_null() {
        return;
    }
    unsafe {
        TerminateProcess(h, 1);
        CloseHandle(h);
    }
}

/// 연결 handle 의 **서버 쪽** PID — 서버가 identify 로 뭐라고 말하든 OS 가 아는 값.
fn server_pid(c: &NamedPipeClient) -> Option<u32> {
    use std::os::windows::io::AsRawHandle as _;
    use windows_sys::Win32::System::Pipes::GetNamedPipeServerProcessId;
    let mut pid = 0u32;
    let ok = unsafe { GetNamedPipeServerProcessId(c.as_raw_handle(), &mut pid) } != 0;
    (ok && pid != 0).then_some(pid)
}

#[derive(Clone, Debug, PartialEq)]
struct Seen {
    pid: u32,
    created: u64,
    rpc_pid: Option<u64>,
    version: Option<String>,
}

async fn open_pipe(pipe: &Path, budget: Duration) -> std::io::Result<NamedPipeClient> {
    let name = pipe.to_string_lossy().into_owned();
    let end = Instant::now() + budget;
    loop {
        match ClientOptions::new().open(&name) {
            Ok(c) => return Ok(c),
            Err(e) if e.raw_os_error() == Some(cys::PIPE_BUSY_ERROR) && Instant::now() < end => {
                tokio::time::sleep(Duration::from_millis(50)).await
            }
            Err(e) => return Err(e),
        }
    }
}

/// 새 연결 1개로 신원을 잰다 — 연결 handle 의 OS server PID·그 프로세스의 생성 시각·identify 응답.
async fn observe(pipe: &Path) -> Result<Seen, String> {
    let mut c = open_pipe(pipe, Duration::from_secs(3))
        .await
        .map_err(|e| format!("연결 불가: {e}"))?;
    let pid = server_pid(&c).ok_or_else(|| "server pid 조회 실패".to_string())?;
    let created = created_of(pid).ok_or_else(|| format!("pid {pid} 생성 시각 조회 실패"))?;
    c.write_all(b"{\"id\":1,\"method\":\"system.identify\",\"params\":{\"caller\":\"w8a-oracle\"}}\n")
        .await
        .map_err(|e| format!("identify 전송 실패: {e}"))?;
    let mut r = tokio::io::BufReader::new(c);
    let mut line = String::new();
    match tokio::time::timeout(Duration::from_secs(3), r.read_line(&mut line)).await {
        Ok(Ok(n)) if n > 0 => {}
        other => return Err(format!("identify 응답 없음: {other:?}")),
    }
    let v: Value = serde_json::from_str(line.trim()).map_err(|e| format!("identify 파싱 실패: {e}"))?;
    Ok(Seen {
        pid,
        created,
        rpc_pid: v["result"]["daemon_pid"].as_u64(),
        version: v["result"]["version"].as_str().map(String::from),
    })
}

/// 엔드포인트가 (주어진 pid 의) 버전 보고 서버로 응답할 때까지 기다린다 — 준비 실패는 검체 실패다.
async fn wait_ready(pipe: &Path, pid: Option<u32>, secs: u64) -> Result<Seen, String> {
    let end = Instant::now() + Duration::from_secs(secs);
    loop {
        let r = observe(pipe).await;
        match &r {
            Ok(s) if s.version.is_some() && pid.map_or(true, |p| p == s.pid) => return Ok(s.clone()),
            _ if Instant::now() >= end => {
                return Err(format!("{} 준비 실패({secs}s): {r:?}", pipe.display()))
            }
            _ => tokio::time::sleep(Duration::from_millis(200)).await,
        }
    }
}

/// 엔드포인트에 응답하는 서버가 없어질 때까지 기다린다.
async fn wait_gone(pipe: &Path, secs: u64) -> Result<(), String> {
    let end = Instant::now() + Duration::from_secs(secs);
    while observe(pipe).await.is_ok() {
        if Instant::now() >= end {
            return Err(format!("{} 가 {secs}s 안에 비지 않았다", pipe.display()));
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    Ok(())
}

// ── 격리 단위 ────────────────────────────────────────────────────────────────

#[derive(Default)]
struct Reg {
    children: Vec<std::process::Child>,
    tasks: Vec<tauri::async_runtime::JoinHandle<()>>,
    pipes: Vec<PathBuf>,
}
type Shared = Arc<Mutex<Reg>>;
type Slot<T> = Arc<Mutex<Option<T>>>;

fn slot<T>() -> Slot<T> {
    Arc::new(Mutex::new(None))
}
fn put<T>(s: &Slot<T>, v: T) {
    *s.lock().unwrap_or_else(|e| e.into_inner()) = Some(v);
}
fn take<T>(s: &Slot<T>) -> Option<T> {
    s.lock().unwrap_or_else(|e| e.into_inner()).take()
}

/// 검체 1개의 격리 단위 — 이 프로세스의 CYS_SOCKET·CYS_PACK_DIR 을 검체 전용으로 돌린다(교대 코드는
/// production 그대로 `default_socket()`·`pack_dir()` 를 읽는다). 끝나면 띄운 것을 전부 걷는다.
struct Scope {
    pipe: PathBuf,
    pack: PathBuf,
    reg: Shared,
}

impl Scope {
    fn new(tag: &str) -> Scope {
        place_sidecars();
        let pipe = pipe_name(tag, "main");
        let root = env().pack_root.join(tag);
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("검체 팩 루트 생성");
        let pack = root.join("pack");
        std::env::set_var(cys::ENV_SOCKET, &pipe);
        std::env::set_var(cys::pack::ENV_PACK_DIR, &pack);
        Scope { pipe, pack, reg: Arc::default() }
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        let pipes = {
            let mut g = self.reg.lock().unwrap_or_else(|e| e.into_inner());
            for t in g.tasks.drain(..) {
                t.abort();
            }
            for c in g.children.iter_mut() {
                let _ = c.kill();
                let _ = c.wait();
            }
            let mut p: Vec<PathBuf> = g.pipes.drain(..).collect();
            p.push(self.pipe.clone());
            p
        };
        // production ensure_daemon 이 띄운 데몬(우리 자식이 아니다)도 엔드포인트 신원으로 찾아 끝낸다.
        tauri::async_runtime::block_on(async move {
            for p in pipes {
                for _ in 0..10 {
                    match observe(&p).await {
                        Ok(s) if s.pid != std::process::id() => {
                            terminate(s.pid, s.created);
                            tokio::time::sleep(Duration::from_millis(300)).await;
                        }
                        _ => break,
                    }
                }
            }
        });
        std::env::remove_var(cys::ENV_SOCKET);
    }
}

fn spawn_daemon(reg: &Shared, exe: &Path, pipe: &Path) -> Result<u32, String> {
    let mut cmd = std::process::Command::new(exe);
    cmd.env(cys::ENV_SOCKET, pipe)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    crate::no_console(&mut cmd);
    let child = cmd.spawn().map_err(|e| format!("{} 기동 실패: {e}", exe.display()))?;
    let pid = child.id();
    let mut g = reg.lock().unwrap_or_else(|e| e.into_inner());
    g.children.push(child);
    g.pipes.push(pipe.to_path_buf());
    Ok(pid)
}

// ── 가짜 서버(이 프로세스 안) — D5·D6 의 '신원 불명' 조건 ───────────────────

#[derive(Clone, Copy, Debug)]
enum Fake {
    /// identify 에 daemon_pid 가 없다(버전은 목표로 사칭).
    NoPid,
    /// 응답이 JSON 이 아니다.
    Garbage,
    /// daemon_pid 가 연결 handle 의 server PID 와 다르다.
    WrongPid,
    /// 연결은 받되 응답하지 않는다(handshake 무응답).
    Silent,
    /// 유일한 인스턴스를 홀더가 점유 — 이후 모든 open 이 ERROR_PIPE_BUSY.
    Busy,
}

/// 가짜 pipe 서버를 띄운다 — 첫 인스턴스가 생긴 뒤에 돌아온다(준비 확인).
async fn start_fake(reg: &Shared, pipe: &Path, mode: Fake) -> Result<(), String> {
    let name = pipe.to_string_lossy().into_owned();
    let (tx, rx) = tokio::sync::oneshot::channel::<Result<(), String>>();
    let h = tauri::async_runtime::spawn(fake_loop(name, mode, tx));
    {
        let mut g = reg.lock().unwrap_or_else(|e| e.into_inner());
        g.tasks.push(h);
        g.pipes.push(pipe.to_path_buf());
    }
    rx.await.map_err(|_| "가짜 서버 태스크 소실".to_string())?
}

async fn fake_loop(name: String, mode: Fake, ready: tokio::sync::oneshot::Sender<Result<(), String>>) {
    let max = if matches!(mode, Fake::Busy) { 1 } else { 32 };
    let first = match ServerOptions::new()
        .first_pipe_instance(true)
        .max_instances(max)
        .create(&name)
    {
        Ok(s) => s,
        Err(e) => {
            let _ = ready.send(Err(format!("가짜 서버 생성 실패: {e}")));
            return;
        }
    };
    if let Fake::Busy = mode {
        let holder = match ClientOptions::new().open(&name) {
            Ok(c) => c,
            Err(e) => {
                let _ = ready.send(Err(format!("홀더 연결 실패: {e}")));
                return;
            }
        };
        let _ = first.connect().await;
        let _ = ready.send(Ok(()));
        let _keep = (first, holder);
        loop {
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
    }
    let _ = ready.send(Ok(()));
    let mut server = first;
    loop {
        if server.connect().await.is_err() {
            return;
        }
        let next = match ServerOptions::new().max_instances(max).create(&name) {
            Ok(s) => s,
            Err(_) => return,
        };
        let conn = std::mem::replace(&mut server, next);
        tauri::async_runtime::spawn(serve_fake(conn, mode));
    }
}

async fn serve_fake(conn: NamedPipeServer, mode: Fake) {
    let mut r = tokio::io::BufReader::new(conn);
    let mut line = String::new();
    if r.read_line(&mut line).await.unwrap_or(0) == 0 {
        return;
    }
    let me = std::process::id() as u64;
    let body = match mode {
        Fake::NoPid => {
            json!({"id": 1, "ok": true, "result": {"version": TARGET, "started_at": 1.0}}).to_string()
        }
        Fake::WrongPid => json!({"id": 1, "ok": true,
            "result": {"daemon_pid": me + 1, "version": TARGET, "started_at": 1.0}})
        .to_string(),
        Fake::Garbage => "not-json".to_string(),
        Fake::Silent | Fake::Busy => {
            tokio::time::sleep(Duration::from_secs(30)).await;
            return;
        }
    };
    let w = r.get_mut();
    let _ = w.write_all(format!("{body}\n").as_bytes()).await;
    let _ = w.flush().await;
    tokio::time::sleep(Duration::from_millis(500)).await;
}

// ── 주입 지점 ① 가짜 taskkill ────────────────────────────────────────────────

/// 앱 옆 taskkill.exe(가짜) — production 은 `Command::new("taskkill")` 그대로다. Drop 이 걷는다.
struct FakeTaskkill(PathBuf);

impl FakeTaskkill {
    fn install(mode: &str) -> Result<FakeTaskkill, String> {
        let dst = app_dir().join("taskkill.exe");
        std::fs::copy(&env().fake_taskkill, &dst).map_err(|e| format!("가짜 taskkill 배치 실패: {e}"))?;
        std::env::set_var("W8A_FAKE_TASKKILL_MODE", mode);
        let guard = FakeTaskkill(dst);
        // 주입 성립 확인 — production 과 같은 해소 규칙으로 불러 가짜가 잡히는가(아니면 검체 무효).
        let probe = std::process::Command::new("taskkill")
            .args(["/PID", "0", "/F"])
            .output()
            .map_err(|e| format!("taskkill 해소 실패: {e}"))?;
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&probe.stdout),
            String::from_utf8_lossy(&probe.stderr)
        );
        if !text.contains("(fake)") {
            return Err(format!("주입 불성립 — 앱 폴더의 가짜가 아니라 다른 taskkill 이 불렸다: {text}"));
        }
        Ok(guard)
    }
}

impl Drop for FakeTaskkill {
    fn drop(&mut self) {
        std::env::remove_var("W8A_FAKE_TASKKILL_MODE");
        let _ = std::fs::remove_file(&self.0);
    }
}

// ── 주입 없는 실제 잠금: CWD 홀더(D9) ────────────────────────────────────────

/// Win32 current directory 가 `dir` 인 cmd.exe. 준비 핸드셰이크 = 그 프로세스가 출력한 자기 CWD
/// (cmd 의 `cd` = GetCurrentDirectory). 해제 신호 = stdin EOF(`set /p` 가 돌아오면 종료한다).
struct CwdHolder {
    child: std::process::Child,
    pid: u32,
    created: u64,
}

impl CwdHolder {
    fn start(dir: &Path) -> Result<CwdHolder, String> {
        use std::os::windows::process::CommandExt as _;
        let mut child = std::process::Command::new("cmd.exe")
            .raw_arg("/d /q /c \"cd & set /p W8A_RELEASE=\"")
            .current_dir(dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("CWD 홀더 기동 실패: {e}"))?;
        let out = child.stdout.take().ok_or_else(|| "홀더 stdout 없음".to_string())?;
        let mut line = String::new();
        std::io::BufReader::new(out)
            .read_line(&mut line)
            .map_err(|e| format!("홀더 핸드셰이크 실패: {e}"))?;
        let norm = |p: &Path| {
            p.to_string_lossy()
                .trim_start_matches(r"\\?\")
                .trim_end_matches('\\')
                .to_lowercase()
        };
        let want = std::fs::canonicalize(dir).map(|p| norm(&p)).unwrap_or_else(|_| norm(dir));
        let got = norm(Path::new(line.trim()));
        let pid = child.id();
        match created_of(pid) {
            Some(created) if got == want => Ok(CwdHolder { child, pid, created }),
            created => {
                let _ = child.kill();
                let _ = child.wait();
                Err(format!("CWD 핸드셰이크 불일치: 홀더 CWD={got:?} · 기대={want:?} · created={created:?}"))
            }
        }
    }

    fn alive(&self) -> bool {
        alive(self.pid, self.created)
    }
}

impl Drop for CwdHolder {
    fn drop(&mut self) {
        drop(self.child.stdin.take()); // 해제 신호(EOF)
        std::thread::sleep(Duration::from_millis(300));
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn pack_state(pack: &Path) -> String {
    let n = |p: &Path| {
        std::fs::read_dir(p)
            .map(|d| d.count().to_string())
            .unwrap_or_else(|e| format!("읽기 불가({e})"))
    };
    let sibs: Vec<String> = pack
        .parent()
        .and_then(|p| std::fs::read_dir(p).ok())
        .map(|d| {
            d.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    format!("pack/bin 항목 {} · pack 항목 {} · 형제 {:?}", n(&pack.join("bin")), n(pack), sibs)
}

// ── 교대 호출과 판정 ─────────────────────────────────────────────────────────

#[derive(Debug)]
enum Intent {
    /// 버전 스큐 해소(자동·배지 수동) — 이미 목표 버전이면 아무것도 하지 않아야 한다.
    IfSkewed,
    /// 재시작(같은 버전이어도 새 세대 필요).
    Restart,
}

/// 교대 호출이 돌려준 것 — 오라클이 아니다(오라클은 observe·alive).
#[allow(dead_code)] // 수정 전 트리(음성 대조 1회차)는 Partial·AlreadyCurrent 를 만들지 않는다.
#[derive(Debug)]
enum Outcome {
    Success(Option<Value>),
    Partial(Value),
    AlreadyCurrent(Value),
    Failure(String),
}

#[derive(Debug)]
enum Ran {
    Returned(Outcome),
    TimedOut,
}

type BoxFut = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>;
type AfterStop = Box<dyn FnOnce() -> BoxFut + Send>;
type PackFn = Box<dyn FnOnce() -> bool + Send>;

async fn run_rotate(intent: Intent, after_stop: Option<AfterStop>, pack: PackFn) -> Ran {
    match tokio::time::timeout(ROTATE_BUDGET, rotate_entry(intent, after_stop, pack)).await {
        Ok(o) => Ran::Returned(o),
        Err(_) => Ran::TimedOut,
    }
}

fn pack_ok() -> PackFn {
    Box::new(|| true)
}

fn hook_fake(reg: &Shared, pipe: &Path, mode: Fake, fail: &Slot<String>) -> AfterStop {
    let (reg, pipe, fail) = (reg.clone(), pipe.to_path_buf(), fail.clone());
    Box::new(move || -> BoxFut {
        Box::pin(async move {
            if let Err(e) = start_fake(&reg, &pipe, mode).await {
                put(&fail, e);
            }
        })
    })
}

fn hook_daemon(reg: &Shared, exe: &Path, pipe: &Path, seen: &Slot<Seen>, fail: &Slot<String>) -> AfterStop {
    let (reg, exe, pipe) = (reg.clone(), exe.to_path_buf(), pipe.to_path_buf());
    let (seen, fail) = (seen.clone(), fail.clone());
    Box::new(move || -> BoxFut {
        Box::pin(async move {
            let r = match spawn_daemon(&reg, &exe, &pipe) {
                Ok(pid) => wait_ready(&pipe, Some(pid), READY_SECS).await,
                Err(e) => Err(e),
            };
            match r {
                Ok(s) => put(&seen, s),
                Err(e) => put(&fail, e),
            }
        })
    })
}

/// 목표 버전의 **새 세대**인가(엔드포인트 실측).
fn is_new_target(before: &Seen, after: &Seen) -> Result<(), String> {
    if after.version.as_deref() != Some(TARGET) {
        return Err(format!("교대 후 버전 {:?} ≠ 목표 {TARGET}", after.version));
    }
    if after.rpc_pid != Some(after.pid as u64) {
        return Err(format!("신원 불일치 — identify pid {:?} ≠ server pid {}", after.rpc_pid, after.pid));
    }
    if (after.pid, after.created) == (before.pid, before.created) {
        return Err(format!("같은 세대(pid {} · 생성 {}) — 교대 안 됨", after.pid, after.created));
    }
    Ok(())
}

fn same_generation(a: &Seen, b: &Seen) -> bool {
    (a.pid, a.created) == (b.pid, b.created)
}

/// 영수증이 있으면 실측과 같아야 한다(수정 전 트리는 영수증을 내지 않는다).
fn receipt_matches(rc: &Value, after: &Seen) -> Result<(), String> {
    let ok = rc["after"]["pid"].as_u64() == Some(after.pid as u64)
        && rc["after"]["created"].as_u64() == Some(after.created)
        && rc["expected_version"].as_str() == Some(TARGET);
    if ok {
        Ok(())
    } else {
        Err(format!("영수증 ≠ 실측: 영수증 {rc} · 실측 {after:?}"))
    }
}

fn expect_failure(ran: Ran, what: &str) -> Result<String, String> {
    match ran {
        Ran::Returned(Outcome::Failure(e)) => Ok(e),
        other => Err(format!("{what} — 실패로 끝나야 하는데 {other:?}")),
    }
}

fn verdict(id: &str, r: Result<(), String>) {
    match &r {
        Ok(()) => println!("W8A RESULT: {id} PASS"),
        Err(e) => println!("W8A RESULT: {id} FAIL {e}"),
    }
    if let Err(e) = r {
        panic!("{id}: {e}");
    }
}

// ── D1–D10 ──────────────────────────────────────────────────────────────────

/// D1 실제 구버전 교대: 연결된 pipe server 가 목표 버전·다른 세대여야 성공이다.
#[test]
#[ignore = "T7 전용(windows-build.yml) — 실제 구버전 cysd·가짜 taskkill env 필요"]
fn w8a_d01_real_old_to_target() {
    let s = Scope::new("d01");
    let r = tauri::async_runtime::block_on(async {
        let old = spawn_daemon(&s.reg, &env().old_cysd, &s.pipe)?;
        let before = wait_ready(&s.pipe, Some(old), READY_SECS).await?;
        if before.version.as_deref() == Some(TARGET) {
            return Err(format!("V_old 가 목표와 같은 버전({TARGET}) — D1 은 실제 구버전이 필요하다"));
        }
        let receipt = match run_rotate(Intent::IfSkewed, None, pack_ok()).await {
            Ran::Returned(Outcome::Success(rc)) => rc,
            other => return Err(format!("성공이어야 한다: {other:?}")),
        };
        let after = observe(&s.pipe).await?;
        is_new_target(&before, &after)?;
        if alive(before.pid, before.created) {
            return Err("구 데몬 세대가 살아 있다".into());
        }
        if let Some(rc) = receipt {
            receipt_matches(&rc, &after)?;
        }
        Ok::<(), String>(())
    });
    drop(s);
    verdict("D1", r);
}

/// D2 taskkill 실패·구 데몬 생존: 실패로 끝나야 하고 구 데몬에 다시 붙은 것을 성공이라 하면 안 된다.
#[test]
#[ignore = "T7 전용(windows-build.yml) — 실제 구버전 cysd·가짜 taskkill env 필요"]
fn w8a_d02_taskkill_fails_old_survives() {
    d_stop_fails("D2", "d02", "fail");
}

/// D3 taskkill 0 인데 구 데몬 생존: '명령 성공'은 '종료 완료'가 아니다.
#[test]
#[ignore = "T7 전용(windows-build.yml) — 실제 구버전 cysd·가짜 taskkill env 필요"]
fn w8a_d03_taskkill_zero_but_old_survives() {
    d_stop_fails("D3", "d03", "lie");
}

fn d_stop_fails(id: &str, tag: &str, mode: &str) {
    let s = Scope::new(tag);
    let r = tauri::async_runtime::block_on(async {
        let old = spawn_daemon(&s.reg, &env().old_cysd, &s.pipe)?;
        let before = wait_ready(&s.pipe, Some(old), READY_SECS).await?;
        let fake = FakeTaskkill::install(mode)?;
        let ran = run_rotate(Intent::IfSkewed, None, pack_ok()).await;
        drop(fake);
        expect_failure(ran, &format!("taskkill({mode})·구 데몬 생존"))?;
        if !alive(before.pid, before.created) {
            return Err("구 데몬이 죽었다 — 실패 주입이 성립하지 않았다(검체 무효)".into());
        }
        let now = observe(&s.pipe).await?;
        if !same_generation(&now, &before) {
            return Err(format!("엔드포인트가 구 데몬이 아니다: {now:?}"));
        }
        Ok::<(), String>(())
    });
    drop(s);
    verdict(id, r);
}

/// D4 taskkill 비0 이지만 구 데몬은 실제로 종료: 과잉 거부 없이 진행해 목표를 증명해야 한다.
#[test]
#[ignore = "T7 전용(windows-build.yml) — 실제 구버전 cysd·가짜 taskkill env 필요"]
fn w8a_d04_taskkill_nonzero_but_old_exited() {
    let s = Scope::new("d04");
    let r = tauri::async_runtime::block_on(async {
        let old = spawn_daemon(&s.reg, &env().old_cysd, &s.pipe)?;
        let before = wait_ready(&s.pipe, Some(old), READY_SECS).await?;
        let fake = FakeTaskkill::install("killfail")?;
        let ran = run_rotate(Intent::IfSkewed, None, pack_ok()).await;
        drop(fake);
        let receipt = match ran {
            Ran::Returned(Outcome::Success(rc)) => rc,
            other => return Err(format!("성공이어야 한다(구 데몬은 실제로 종료됐다): {other:?}")),
        };
        let after = observe(&s.pipe).await?;
        is_new_target(&before, &after)?;
        if alive(before.pid, before.created) {
            return Err("구 데몬 세대가 살아 있다".into());
        }
        if let Some(rc) = receipt {
            receipt_matches(&rc, &after)?;
            let req = rc["stop"]["request"].as_str().unwrap_or("");
            if !req.starts_with("exit:") || rc["stop"]["terminated"].as_bool() != Some(true) {
                return Err(format!("영수증이 '요청 실패·종료 확인'을 구분하지 않는다: {}", rc["stop"]));
            }
        }
        Ok::<(), String>(())
    });
    drop(s);
    verdict("D4", r);
}

/// D5 새 서버의 신원 불명(PID 부재·파싱 실패·server PID 불일치) — unknown 을 성공으로 바꾸면 안 된다.
fn d_new_server_unverifiable(id: &str, tag: &str, mode: Fake) {
    let s = Scope::new(tag);
    let r = tauri::async_runtime::block_on(async {
        let old = spawn_daemon(&s.reg, &env().old_cysd, &s.pipe)?;
        wait_ready(&s.pipe, Some(old), READY_SECS).await?;
        let fail = slot::<String>();
        let hook = hook_fake(&s.reg, &s.pipe, mode, &fail);
        let ran = run_rotate(Intent::IfSkewed, Some(hook), pack_ok()).await;
        if let Some(e) = take(&fail) {
            return Err(format!("주입 불성립(가짜 서버): {e}"));
        }
        expect_failure(ran, &format!("구 데몬 종료 뒤 신원 불명 서버({mode:?})"))?;
        Ok::<(), String>(())
    });
    drop(s);
    verdict(id, r);
}

#[test]
#[ignore = "T7 전용(windows-build.yml) — 실제 구버전 cysd·가짜 taskkill env 필요"]
fn w8a_d05a_new_server_without_pid() {
    d_new_server_unverifiable("D5a", "d05a", Fake::NoPid);
}

#[test]
#[ignore = "T7 전용(windows-build.yml) — 실제 구버전 cysd·가짜 taskkill env 필요"]
fn w8a_d05b_new_server_garbage_reply() {
    d_new_server_unverifiable("D5b", "d05b", Fake::Garbage);
}

#[test]
#[ignore = "T7 전용(windows-build.yml) — 실제 구버전 cysd·가짜 taskkill env 필요"]
fn w8a_d05c_new_server_pid_mismatch() {
    d_new_server_unverifiable("D5c", "d05c", Fake::WrongPid);
}

/// D5(구 쪽) 교대 전 신원 불명: 누구인지 모르는 서버를 죽이거나 그 위에 성공을 말하면 안 된다.
#[test]
#[ignore = "T7 전용(windows-build.yml) — 실제 구버전 cysd·가짜 taskkill env 필요"]
fn w8a_d05e_old_server_unverifiable() {
    let s = Scope::new("d05e");
    let r = tauri::async_runtime::block_on(async {
        start_fake(&s.reg, &s.pipe, Fake::NoPid).await?;
        let ran = run_rotate(Intent::IfSkewed, None, pack_ok()).await;
        expect_failure(ran, "교대 전 신원 불명 서버")?;
        Ok::<(), String>(())
    });
    drop(s);
    verdict("D5e", r);
}

/// D6 pipe busy(구 데몬 종료 직후 모든 인스턴스 점유): 버전을 모르면 성공 없음 · 끝이 있어야 한다.
#[test]
#[ignore = "T7 전용(windows-build.yml) — 실제 구버전 cysd·가짜 taskkill env 필요"]
fn w8a_d06b_pipe_busy_after_stop() {
    d_new_server_unverifiable("D6b", "d06b", Fake::Busy);
}

/// D6 handshake 무응답.
#[test]
#[ignore = "T7 전용(windows-build.yml) — 실제 구버전 cysd·가짜 taskkill env 필요"]
fn w8a_d06c_handshake_silent_after_stop() {
    d_new_server_unverifiable("D6c", "d06c", Fake::Silent);
}

/// D6 다른 PID 의 구버전이 pipe 를 차지: 새 PID 여도 구버전이면 실패 · 그 서버를 죽이지 않는다.
#[test]
#[ignore = "T7 전용(windows-build.yml) — 실제 구버전 cysd·가짜 taskkill env 필요"]
fn w8a_d06d_other_old_version_takes_pipe() {
    let s = Scope::new("d06d");
    let r = tauri::async_runtime::block_on(async {
        let old = spawn_daemon(&s.reg, &env().old_cysd, &s.pipe)?;
        wait_ready(&s.pipe, Some(old), READY_SECS).await?;
        let (seen, fail) = (slot::<Seen>(), slot::<String>());
        let hook = hook_daemon(&s.reg, &env().old_cysd, &s.pipe, &seen, &fail);
        let ran = run_rotate(Intent::IfSkewed, Some(hook), pack_ok()).await;
        if let Some(e) = take(&fail) {
            return Err(format!("주입 불성립(두 번째 구버전): {e}"));
        }
        let second = take(&seen).ok_or_else(|| "주입 불성립 — 두 번째 구버전이 뜨지 않았다".to_string())?;
        expect_failure(ran, "다른 PID 의 구버전")?;
        if !alive(second.pid, second.created) {
            return Err("끼어든 구버전 서버가 죽었다 — 남의 서버를 죽이면 안 된다".into());
        }
        Ok::<(), String>(())
    });
    drop(s);
    verdict("D6d", r);
}

/// D7 경쟁 수렴: 다른 실행자가 목표 버전 서버를 먼저 띄웠으면 신원 확인 뒤 성공 · 그 서버를 죽이지 않는다.
#[test]
#[ignore = "T7 전용(windows-build.yml) — 실제 구버전 cysd·가짜 taskkill env 필요"]
fn w8a_d07_race_other_actor_started_target() {
    let s = Scope::new("d07");
    let r = tauri::async_runtime::block_on(async {
        let old = spawn_daemon(&s.reg, &env().old_cysd, &s.pipe)?;
        let before = wait_ready(&s.pipe, Some(old), READY_SECS).await?;
        let (seen, fail) = (slot::<Seen>(), slot::<String>());
        let hook = hook_daemon(&s.reg, &env().target_cysd, &s.pipe, &seen, &fail);
        let ran = run_rotate(Intent::IfSkewed, Some(hook), pack_ok()).await;
        if let Some(e) = take(&fail) {
            return Err(format!("주입 불성립(다른 실행자의 목표 서버): {e}"));
        }
        let other = take(&seen).ok_or_else(|| "주입 불성립 — 다른 실행자 서버가 뜨지 않았다".to_string())?;
        let receipt = match ran {
            Ran::Returned(Outcome::Success(rc)) => rc,
            other_ran => return Err(format!("수렴 성공이어야 한다: {other_ran:?}")),
        };
        let after = observe(&s.pipe).await?;
        is_new_target(&before, &after)?;
        if !same_generation(&after, &other) || !alive(other.pid, other.created) {
            return Err(format!("다른 실행자의 서버를 대체·살해했다: 그 서버 {other:?} · 지금 {after:?}"));
        }
        if let Some(rc) = receipt {
            receipt_matches(&rc, &after)?;
        }
        Ok::<(), String>(())
    });
    drop(s);
    verdict("D7", r);
}

/// D8 이미 목표 버전: 스큐 해소 요청은 no-op 이어야 한다(같은 세대 생존 · 별도 결과).
#[test]
#[ignore = "T7 전용(windows-build.yml) — 실제 구버전 cysd·가짜 taskkill env 필요"]
fn w8a_d08a_already_current_is_noop() {
    let s = Scope::new("d08a");
    let r = tauri::async_runtime::block_on(async {
        let cur = spawn_daemon(&s.reg, &env().target_cysd, &s.pipe)?;
        let before = wait_ready(&s.pipe, Some(cur), READY_SECS).await?;
        let rc = match run_rotate(Intent::IfSkewed, None, pack_ok()).await {
            Ran::Returned(Outcome::AlreadyCurrent(rc)) => rc,
            other => return Err(format!("이미 목표 버전이면 no-op(AlreadyCurrent)이어야 한다: {other:?}")),
        };
        let now = observe(&s.pipe).await?;
        if !same_generation(&now, &before) || !alive(before.pid, before.created) {
            return Err(format!("no-op 이어야 하는데 세대가 바뀌었다: 전 {before:?} · 후 {now:?}"));
        }
        if rc["result"].as_str() != Some("already_current") {
            return Err(format!("영수증 결과가 already_current 가 아니다: {rc}"));
        }
        Ok::<(), String>(())
    });
    drop(s);
    verdict("D8a", r);
}

/// D8 같은 버전 강제 교대(재시작): 새 세대가 필요하다.
#[test]
#[ignore = "T7 전용(windows-build.yml) — 실제 구버전 cysd·가짜 taskkill env 필요"]
fn w8a_d08b_same_version_force_needs_new_generation() {
    let s = Scope::new("d08b");
    let r = tauri::async_runtime::block_on(async {
        let cur = spawn_daemon(&s.reg, &env().target_cysd, &s.pipe)?;
        let before = wait_ready(&s.pipe, Some(cur), READY_SECS).await?;
        let receipt = match run_rotate(Intent::Restart, None, pack_ok()).await {
            Ran::Returned(Outcome::Success(rc)) => rc,
            other => return Err(format!("재시작 성공이어야 한다: {other:?}")),
        };
        let after = observe(&s.pipe).await?;
        is_new_target(&before, &after)?;
        if alive(before.pid, before.created) {
            return Err("이전 세대가 살아 있다".into());
        }
        if let Some(rc) = receipt {
            receipt_matches(&rc, &after)?;
        }
        Ok::<(), String>(())
    });
    drop(s);
    verdict("D8b", r);
}

/// D9 목표 데몬 확인 뒤 **실제 CWD 잠금**으로 팩 적용 실패: 데몬 교대와 팩 실패를 분리해 보고해야 한다
/// (부분 실패 · 복원 보류 · 전체 성공 금지). 잠금 홀더를 죽여서 해결하면 안 된다.
#[test]
#[ignore = "T7 전용(windows-build.yml) — 실제 구버전 cysd·가짜 taskkill env 필요"]
fn w8a_d09_pack_fails_under_cwd_lock() {
    let s = Scope::new("d09");
    let mut holder: Option<CwdHolder> = None;
    let r = d09_body(&s, &mut holder);
    // W-8b 증거(판정 아님): 실패 뒤 팩 자리 상태.
    println!("W8A NOTE: D9 팩 자리 = {}", pack_state(&s.pack));
    drop(holder);
    drop(s);
    verdict("D9", r);
}

fn d09_body(s: &Scope, holder: &mut Option<CwdHolder>) -> Result<(), String> {
    let bin = s.pack.join("bin");
    // ① 실제 팩 준비(production 사이드카 init-pack) — pack/bin 실재가 잠금 검체의 전제다.
    let prep = crate::sealed_sidecar_cys(&["init-pack", "--no-install-hook"])
        .output()
        .map_err(|e| format!("팩 준비 실행 실패: {e}"))?;
    if !prep.status.success() || !bin.is_dir() {
        return Err(format!(
            "팩 준비 실패: exit={:?} · bin={} · stderr={}",
            prep.status.code(),
            bin.is_dir(),
            String::from_utf8_lossy(&prep.stderr)
        ));
    }
    let before = tauri::async_runtime::block_on(async {
        let old = spawn_daemon(&s.reg, &env().old_cysd, &s.pipe)?;
        wait_ready(&s.pipe, Some(old), READY_SECS).await
    })?;
    // ② 실제 잠금 — 구 데몬이 뜬 뒤에 건다(부트 팩 설치와 겹치지 않게).
    *holder = Some(CwdHolder::start(&bin)?);
    let rc = tauri::async_runtime::block_on(async {
        let pack: PackFn = Box::new(|| {
            crate::sealed_sidecar_cys(&["init-pack", "--no-install-hook"])
                .status()
                .map(|st| st.success())
                .unwrap_or(false)
        });
        match run_rotate(Intent::IfSkewed, None, pack).await {
            Ran::Returned(Outcome::Partial(rc)) => Ok(rc),
            other => Err(format!("데몬 교대됨 + 팩 실패 = 부분 실패여야 한다: {other:?}")),
        }
    })?;
    let after = tauri::async_runtime::block_on(observe(&s.pipe))?;
    is_new_target(&before, &after)?;
    receipt_matches(&rc, &after)?;
    if rc["pack"].as_str() != Some("failed") || rc["restore"].as_str() != Some("withheld") {
        return Err(format!("영수증이 팩 실패·복원 보류를 말하지 않는다: {rc}"));
    }
    match holder {
        Some(h) if h.alive() => Ok(()),
        _ => Err("잠금 홀더가 죽었다 — 교대가 잠금 주체를 죽여서 풀면 실패다".into()),
    }
}

/// D10 다른 scope 무접촉: 본부 교대가 다른 엔드포인트의 데몬을 건드리면 안 된다.
#[test]
#[ignore = "T7 전용(windows-build.yml) — 실제 구버전 cysd·가짜 taskkill env 필요"]
fn w8a_d10a_other_scope_untouched() {
    let s = Scope::new("d10a");
    let r = tauri::async_runtime::block_on(async {
        let pipe_b = pipe_name("d10a", "other");
        let b = spawn_daemon(&s.reg, &env().target_cysd, &pipe_b)?;
        let seen_b = wait_ready(&pipe_b, Some(b), READY_SECS).await?;
        let a = spawn_daemon(&s.reg, &env().old_cysd, &s.pipe)?;
        let before_a = wait_ready(&s.pipe, Some(a), READY_SECS).await?;
        match run_rotate(Intent::IfSkewed, None, pack_ok()).await {
            Ran::Returned(Outcome::Success(_)) => {}
            other => return Err(format!("본부 교대 성공이어야 한다: {other:?}")),
        }
        let after_a = observe(&s.pipe).await?;
        is_new_target(&before_a, &after_a)?;
        let now_b = observe(&pipe_b).await?;
        if !same_generation(&now_b, &seen_b) || !alive(seen_b.pid, seen_b.created) {
            return Err(format!("다른 scope 데몬이 바뀌었다: 전 {seen_b:?} · 후 {now_b:?}"));
        }
        Ok::<(), String>(())
    });
    drop(s);
    verdict("D10a", r);
}

/// D10 부서 경로도 같은 오라클: `cys-dept rotate` 가 성공을 말했어도 구 세대 그대로면 실패다.
#[test]
#[ignore = "T7 전용(windows-build.yml) — 실제 구버전 cysd·가짜 taskkill env 필요"]
fn w8a_d10b_dept_not_rotated_is_failure() {
    let s = Scope::new("d10b");
    let r = tauri::async_runtime::block_on(async {
        let pipe_b = pipe_name("d10b", "dept");
        let b = spawn_daemon(&s.reg, &env().old_cysd, &pipe_b)?;
        wait_ready(&pipe_b, Some(b), READY_SECS).await?;
        let t = dept_before(&pipe_b).await;
        // 교대가 실제로는 일어나지 않았다(외부 도구가 성공을 말한 뒤에도 구 데몬이 그대로인 경우).
        match dept_after(&pipe_b, t).await {
            Outcome::Failure(_) => Ok(()),
            other => Err(format!("구 세대 그대로인데 성공을 말했다: {other:?}")),
        }
    });
    drop(s);
    verdict("D10b", r);
}

/// D10 부서 경로 정상: 실제로 교대됐으면 새 세대·목표 버전을 확인하고 성공.
#[test]
#[ignore = "T7 전용(windows-build.yml) — 실제 구버전 cysd·가짜 taskkill env 필요"]
fn w8a_d10c_dept_rotated_is_verified() {
    let s = Scope::new("d10c");
    let r = tauri::async_runtime::block_on(async {
        let pipe_b = pipe_name("d10c", "dept");
        let b = spawn_daemon(&s.reg, &env().old_cysd, &pipe_b)?;
        let before_b = wait_ready(&pipe_b, Some(b), READY_SECS).await?;
        let t = dept_before(&pipe_b).await;
        // 외부 교대(= cys-dept rotate 가 하는 일): 구 세대 종료 → 목표 세대 기동.
        terminate(before_b.pid, before_b.created);
        wait_gone(&pipe_b, 15).await?;
        let nb = spawn_daemon(&s.reg, &env().target_cysd, &pipe_b)?;
        let after_b = wait_ready(&pipe_b, Some(nb), READY_SECS).await?;
        let receipt = match dept_after(&pipe_b, t).await {
            Outcome::Success(rc) => rc,
            other => return Err(format!("실제로 교대됐으면 성공이어야 한다: {other:?}")),
        };
        is_new_target(&before_b, &after_b)?;
        if let Some(rc) = receipt {
            receipt_matches(&rc, &after_b)?;
        }
        Ok::<(), String>(())
    });
    drop(s);
    verdict("D10c", r);
}

// ═══ 수정 전/후 트리에서 다른 곳은 이 아래 세 함수뿐이다(음성 대조 계약) ═══════════

/// 수정 전 트리의 교대 꼬리 — `rotate_daemon` 과 같은 순서(구 데몬 종료 → ensure_daemon → 팩 반영,
/// 결과는 무시). drain·세션 가드·복귀 마커는 F2(결과 확인)와 무관해 뺀다. 수정 전 코드에는 의도
/// 구분이 없어 항상 재기동한다.
async fn rotate_entry(intent: Intent, after_stop: Option<AfterStop>, pack: PackFn) -> Outcome {
    let _ = intent;
    crate::stop_running_daemon().await;
    if let Some(h) = after_stop {
        h().await;
    }
    match crate::ensure_daemon().await {
        Err(e) => Outcome::Failure(e),
        Ok(()) => {
            let _ = tokio::task::spawn_blocking(pack).await;
            Outcome::Success(None)
        }
    }
}

/// 수정 전 트리의 부서 교대는 교대 전 신원을 잡지 않는다.
struct DeptToken;

async fn dept_before(_pipe: &Path) -> DeptToken {
    DeptToken
}

/// 수정 전 트리의 부서 교대 꼬리 — `cys-dept rotate` 성공 뒤 identify 가 응답하면 그대로 성공
/// (`rotate_dept_daemon`).
async fn dept_after(pipe: &Path, _t: DeptToken) -> Outcome {
    match crate::rpc_on(pipe, "system.identify", json!({"caller": "ui"})).await {
        Ok(_) => Outcome::Success(None),
        Err(e) => Outcome::Failure(e),
    }
}
