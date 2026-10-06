//! ★(0.14.44 · WP-B B1·B2·B3·B7) 오피스 브리지 감독의 판정과 도우미.
//!
//! 브리지 = 오피스 화면(3D 사무실)을 내주는 작은 로컬 웹 서버(파이썬 · 포트 8642). 데몬이 띄운다. 실제로 **띄우는 한 곳**은 `main.rs` 의 `spawn_office_bridge` 하나다
//! (파이썬 직스폰 지점 수를 고정한 census 가 있다 — 이 모듈에는 프로세스를 띄우는 코드가 없다). 여기에는 그 주변의 순수 판정 · 건강 확인 해석 · 옛 브리지 교체 절차 ·
//! 브리지 정지 절차가 있다.
//!
//! 이 모듈이 지키는 것(소스 고정 시험이 확인한다):
//! · **프로세스 그룹 신호 없음** — 브리지는 데몬과 같은 프로세스 그룹에 들어 있다[실측 R4]. 그룹 신호는 한 번 잘못 쓰면 데몬까지 죽인다(전 pane 사망).
//! · **Feed 발행 없음** — 재기동·상한 도달은 데몬 로그에만 남긴다(master 가 손쓸 수 없는 알림으로 턴을 쓰게 하지 않는다).
//! · **데몬의 잠금을 쥐지 않는다** · 모든 대기에 시간 상한 · `unwrap`/`expect`/색인 접근 없음.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use serde_json::Value;
use std::path::{Path, PathBuf};

// ── 상수(설계 B3 · B7) ─────────────────────────────────────────────────────────

/// 자기 자식이 살아 있는 동안 건강 확인을 하는 주기.
pub const PROBE_INTERVAL_SECS: u64 = 60;
/// 연속 실패가 이 횟수에 이르면(약 3분) 브리지를 끝내고 다시 띄운다. 한 번이라도 성공하면 0 으로 돌아간다.
pub const FAIL_LIMIT: u32 = 3;
/// 재기동 상한 — 이 시간 창 안에 스폰이 이 횟수를 넘으면 …
pub const RESTART_WINDOW_SECS: f64 = 1800.0;
pub const RESTART_MAX: usize = 3;
/// … 이 시간 쉬고(로그 한 줄) 다시 센다.
pub const REST_SECS: u64 = 1800;
/// 브리지를 끝낼 때 수명줄을 닫고 기다리는 시간. 그래도 살아 있으면 **그 프로세스 하나만** 강제 종료한다.
pub const LIFELINE_GRACE_SECS: u64 = 5;
/// 자식이 이만큼 살았으면 '뜨자마자 죽는다'가 아니다 — 죽은 뒤 대기 단계를 처음으로 되돌린다.
pub const HEALTHY_LIFE_SECS: f64 = 300.0;
/// B7: 후보가 하나로 좁혀지지 않을 때 기다리며 다시 보는 주기 수(주기 = 감독 루프의 60초).
pub const REPLACE_AMBIGUOUS_ROUNDS: u32 = 5;
/// B7: 토큰 파일이 쓰인 시각과 프로세스 시작 시각의 허용 차이(초). 구현 0단계에서 맥 설치본으로 재야 하는 값이다[미실측].
pub const REPLACE_TOKEN_START_SLACK_SECS: f64 = 10.0;
/// B7: 종료 신호 뒤 포트가 비는지 기다리는 상한.
pub const REPLACE_PORT_FREE_SECS: u64 = 3;

// ── B1: 주인 · 스폰 환경 ───────────────────────────────────────────────────────

/// 이 데몬이 브리지를 띄워도 되는가 — 본부 데몬만(`ANY_OWNER` 손잡이가 켜져 있으면 종전처럼 어느 데몬이든). 판별은 데몬 자신의 소켓 경로 하나(`cys::is_dept_socket`).
pub fn owner_may_spawn(socket: &Path, any_owner: bool) -> bool {
    any_owner || !cys::is_dept_socket(socket)
}

/// 브리지를 띄울 때 **더하는** 설정 — 0.14.43 이 이미 주던 것(`CYS_NO_AUTOSTART` · 봉인 환경 · `HUD_STATE_DIR` · `PATH` · `HUD_CYS_BIN`)은 `main.rs` 에 그대로 있다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnPlan {
    /// 표준입력을 파이프로 연다(수명줄). `false` 면 종전처럼 `null`.
    pub stdin_piped: bool,
    /// 더하는 환경변수.
    pub env: Vec<(&'static str, String)>,
}

/// 스폰 계획(순수). 본부 데몬이면 `CYS_SOCKET=<이 데몬의 소켓>` 을 명시한다(데몬이 어떤 환경에서 떴든 브리지가 같은 데몬을 "본부"로 보게 — B1).
/// `ANY_OWNER` 손잡이가 켜져 있으면 명시하지 않는다(종전처럼 환경을 그대로 물려준다). `Managed` 일 때만 수명줄(`HUD_LIFELINE=stdin` + 표준입력 파이프)과 `HUD_PACK_VERSION`(B2·B3).
/// `Legacy` 면 표준입력 `null` · 수명줄 환경 없음 · 팩 버전 없음 — **0.14.43 의 스폰 설정**이다(윈도우 기본).
pub fn spawn_plan(
    socket: &Path,
    any_owner: bool,
    mode: crate::knobs::BridgeMode,
    pack_version: Option<&str>,
) -> SpawnPlan {
    let mut env: Vec<(&'static str, String)> = Vec::new();
    if !any_owner {
        env.push(("CYS_SOCKET", socket.to_string_lossy().into_owned()));
    }
    let managed = mode == crate::knobs::BridgeMode::Managed;
    if managed {
        env.push(("HUD_LIFELINE", "stdin".to_string()));
        if let Some(v) = pack_version.filter(|v| !v.is_empty()) {
            env.push(("HUD_PACK_VERSION", v.to_string()));
        }
    }
    SpawnPlan { stdin_piped: managed, env }
}

/// 설치된 팩의 버전(`<팩>/.pack-version` — 앞뒤 공백을 뗀 첫 줄). 읽지 못했거나 비었으면 `None`(모름 — "다르다"로 치지 않는다).
pub fn installed_pack_version(pack_dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(pack_dir.join(".pack-version")).ok()?;
    let v = text.lines().next()?.trim().to_string();
    if v.is_empty() {
        None
    } else {
        Some(v)
    }
}

// ── B3: 재기동 상한 · 대기 · 건강 판정 ─────────────────────────────────────────────

/// 죽은 뒤 다시 띄우기 전의 대기(초): 첫 죽음 5 → 둘째 30 → 그 뒤 60(고정 60초 대신 — 첫 복구가 60초에서 5초대로 준다). `deaths` = 지금까지 이어서 죽은 횟수(이번 죽음 포함, 1부터).
pub fn restart_delay_secs(deaths: u32) -> u64 {
    match deaths {
        0 | 1 => 5,
        2 => 30,
        _ => 60,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnGate {
    Go,
    /// 상한에 닿았다 — 이 시간(초) 쉬고(로그 한 줄) 다시 센다.
    Rest(u64),
}

/// 재기동 상한(30분에 3회). 시각은 호출부가 단조 시계의 초(`f64`)로 넘긴다(시험이 가짜 시계를 쓴다).
#[derive(Debug, Default)]
pub struct SpawnLimiter {
    spawns: Vec<f64>,
}

impl SpawnLimiter {
    /// 지금 띄워도 되는가. 창 밖으로 밀린 기록은 버린다.
    pub fn gate(&mut self, now: f64) -> SpawnGate {
        self.spawns.retain(|t| now - *t < RESTART_WINDOW_SECS);
        if self.spawns.len() >= RESTART_MAX {
            SpawnGate::Rest(REST_SECS)
        } else {
            SpawnGate::Go
        }
    }

    pub fn record(&mut self, now: f64) {
        self.spawns.push(now);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeVerdict {
    /// 200 + 본문을 다 받음. `/health` 의 `pack_version`(있으면).
    Healthy { pack_version: Option<String> },
    /// `/health` 가 404 — 옛 스크립트다. 같은 꼴의 탐침을 `/world` 로 한다.
    NoHealthEndpoint,
    Failed,
}

/// 탐침 결과 → 판정(순수). `/health` 본문에서 팩 버전만 읽는다(`pack_version` 이 null 이면 `None`).
pub fn judge_probe(reply: &Result<cys::BridgeProbeReply, cys::BridgeProbeError>) -> ProbeVerdict {
    match reply {
        Ok(r) if r.status == 404 => ProbeVerdict::NoHealthEndpoint,
        Ok(r) if r.status == 200 && r.complete => {
            let pv = serde_json::from_slice::<Value>(&r.body)
                .ok()
                .and_then(|v| v.get("pack_version").and_then(|p| p.as_str()).map(|s| s.to_string()))
                .filter(|s| !s.is_empty());
            ProbeVerdict::Healthy { pack_version: pv }
        }
        _ => ProbeVerdict::Failed,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthAction {
    Nothing,
    /// 연속 실패가 한도에 이르렀다 — 끝내고 다시 띄운다.
    RestartUnhealthy,
    /// 브리지가 돌려준 팩 버전이 지금 설치된 팩과 다르다 — 한 번 다시 띄운다.
    RestartPackVersion,
}

/// 건강 확인의 연속 실패 · 팩 버전 불일치 추적(자식 하나의 생애 — 새로 띄우면 새로 만든다).
#[derive(Debug, Default)]
pub struct HealthTracker {
    pub fails: u32,
    /// 이 자식이 `/health` 가 없는 옛 스크립트라 `/world` 로 확인하는 중인가.
    pub use_world: bool,
    version_restarted: bool,
}

impl HealthTracker {
    /// 탐침 판정 하나를 먹이고 할 일을 돌려준다. `installed` = 지금 설치된 팩 버전(읽었으면). 두 값을 **모두 읽었고 서로 다를 때만** 다르다고 본다.
    pub fn observe(&mut self, v: ProbeVerdict, installed: Option<&str>) -> HealthAction {
        match v {
            ProbeVerdict::Healthy { pack_version } => {
                self.fails = 0;
                if let (Some(rep), Some(inst)) = (pack_version.as_deref(), installed) {
                    if rep != inst && !self.version_restarted {
                        self.version_restarted = true;
                        return HealthAction::RestartPackVersion;
                    }
                }
                HealthAction::Nothing
            }
            ProbeVerdict::NoHealthEndpoint => {
                // 호출부가 `use_world` 로 같은 주기에 `/world` 를 다시 본다 — 이 판정 자체는 실패가 아니다.
                self.use_world = true;
                HealthAction::Nothing
            }
            ProbeVerdict::Failed => {
                self.fails += 1;
                if self.fails >= FAIL_LIMIT {
                    HealthAction::RestartUnhealthy
                } else {
                    HealthAction::Nothing
                }
            }
        }
    }
}

// ── 프로세스 사실(B2 · B7) ──────────────────────────────────────────────────────

/// 프로세스 사실 하나 — 번호 · 부모 번호 · 시작 시각(epoch 초) · 명령줄(인자 배열).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcRec {
    pub pid: u32,
    pub ppid: Option<u32>,
    pub start: u64,
    pub cmd: Vec<String>,
}

/// 같은 사용자의 프로세스 목록(데몬이 이미 쓰는 프로세스 목록 라이브러리 — `sysinfo` — 로 명령줄을 읽는다). 열린 파일·소켓은 조회하지 않는다.
/// 블로킹이다 — 비동기 문맥에서는 `spawn_blocking` 으로 부른다.
pub fn list_user_processes() -> Vec<ProcRec> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_cmd(UpdateKind::Always)
            .with_user(UpdateKind::Always),
    );
    let me = sys
        .process(sysinfo::Pid::from_u32(std::process::id()))
        .and_then(|p| p.user_id().cloned());
    let mut out: Vec<ProcRec> = Vec::new();
    for (pid, p) in sys.processes() {
        // 같은 사용자의 프로세스만 — 내 사용자를 알 수 없으면 아무것도 후보로 삼지 않는다.
        match (&me, p.user_id()) {
            (Some(a), Some(b)) if a == b => {}
            _ => continue,
        }
        out.push(ProcRec {
            pid: pid.as_u32(),
            ppid: p.parent().map(|x| x.as_u32()),
            start: p.start_time(),
            cmd: p.cmd().iter().map(|s| s.to_string_lossy().into_owned()).collect(),
        });
    }
    out
}

/// 이벤트 구독 자식인가 — 브리지가 띄우는 `cys [--socket X] events --reconnect --cursor-file …`.
pub fn is_event_subscriber(cmd: &[String]) -> bool {
    cmd.iter().any(|t| t == "events") && cmd.iter().any(|t| t == "--reconnect")
}

/// 부모 번호가 `pid` 인 이벤트 구독 프로세스들(끝내기 전에 적어 둘 사실).
pub fn event_children_of(procs: &[ProcRec], pid: u32) -> Vec<ProcRec> {
    procs
        .iter()
        .filter(|p| p.ppid == Some(pid) && is_event_subscriber(&p.cmd))
        .cloned()
        .collect()
}

/// 적어 둔 프로세스와 **같은 프로세스**인가 — 번호 · 시작 시각 · 명령줄이 모두 같다(그 사이에 번호가 다른 프로세스로 넘어갔으면 아니다).
pub fn same_process(rec: &ProcRec, now: &[ProcRec]) -> bool {
    now.iter().any(|p| p.pid == rec.pid && p.start == rec.start && p.cmd == rec.cmd)
}

/// 프로세스 하나에 종료 신호(유닉스는 `SIGTERM` 한 프로세스 번호에만 — **그룹 신호가 아니다**). 윈도우는 `sysinfo` 의 종료.
pub fn terminate_pid(pid: u32) {
    #[cfg(unix)]
    {
        // pid 0 · 1 · 데몬 자신은 절대 보내지 않는다(방어 — 호출부가 이미 검증한 번호만 온다).
        // pid 가 i32 범위를 넘으면 `pid_t` 로 바꿀 때 음수(= 그룹 신호 · −1 은 전 프로세스)가 된다 — 절대 보내지 않는다.
        if pid > 1 && pid <= i32::MAX as u32 && pid != std::process::id() {
            unsafe {
                libc::kill(pid as libc::pid_t, libc::SIGTERM);
            }
        }
    }
    #[cfg(not(unix))]
    {
        use sysinfo::{ProcessesToUpdate, System};
        if pid > 1 && pid != std::process::id() {
            let mut sys = System::new();
            sys.refresh_processes(ProcessesToUpdate::All, true);
            if let Some(p) = sys.process(sysinfo::Pid::from_u32(pid)) {
                p.kill();
            }
        }
    }
}

// ── B2: 우리가 띄운 브리지를 끝내는 절차 ───────────────────────────────────────────

/// 우리가 띄운 브리지를 끝낸다: 자식(이벤트 구독 프로세스)을 **먼저 적어 두고** → 수명줄을 닫고 5초 기다린다 → 그래도 살아 있으면 **그 프로세스 하나만** 강제 종료한다
/// (`start_kill` — 프로세스 그룹 신호가 아니다) → 적어 둔 자식을 하나씩 끝낸다(끝내기 직전에 번호 · 시작 시각 · 명령줄이 적어 둔 것과 같은지 다시 본다).
/// 모든 대기에 시간 상한이 있다. 반환 = 끝낸 방법(로그용).
pub async fn stop_managed_child(
    child: &mut tokio::process::Child,
    lifeline: Option<tokio::process::ChildStdin>,
) -> &'static str {
    let pid = child.id();
    let recorded: Vec<ProcRec> = match pid {
        Some(p) => tokio::task::spawn_blocking(move || event_children_of(&list_user_processes(), p))
            .await
            .unwrap_or_default(),
        None => Vec::new(),
    };
    drop(lifeline); // 수명줄을 닫는다 — 브리지가 스스로 자식을 정리하고 끝난다.
    let how = match tokio::time::timeout(std::time::Duration::from_secs(LIFELINE_GRACE_SECS), child.wait()).await {
        Ok(_) => "수명줄 닫음 → 스스로 종료",
        Err(_) => {
            let _ = child.start_kill(); // 그 프로세스 하나만(그룹 아님)
            let _ = tokio::time::timeout(std::time::Duration::from_secs(2), child.wait()).await;
            "수명줄 닫음 5초 → 프로세스 하나만 강제 종료"
        }
    };
    if !recorded.is_empty() {
        let recorded2 = recorded.clone();
        let _ = tokio::task::spawn_blocking(move || {
            for rec in recorded2 {
                // 끝내기 직전에 한 번 더 — 그 사이에 번호가 다른 프로세스로 넘어갔으면 건드리지 않는다.
                if same_process(&rec, &list_user_processes()) {
                    terminate_pid(rec.pid);
                }
            }
        })
        .await;
    }
    how
}

// ── B7: 판을 올린 직후 남아 있는 옛 브리지를 한 번 교체한다 ───────────────────────────────

/// 포트의 주인일 수 있는 상태 폴더 하나 — 브리지 스크립트 경로와 브리지의 상태 폴더(`HUD_STATE_DIR` — 토큰 파일 `token` 이 있다).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerDir {
    pub label: String,
    pub script: PathBuf,
    pub bridge_state: PathBuf,
}

/// 본부와 등록된 부서의 주인 후보(본부 먼저). 부서의 팩은 `~/.cys/pack-dept-<이름>` 이다(`cys-dept` 규약 — 부서 팩 경로로 뜬 옛 브리지도 후보가 된다).
pub fn owner_dirs(hq_state_dir: &Path, hq_script: &Path, depts_json: &Path, home: &Path) -> Vec<OwnerDir> {
    let mut v = vec![OwnerDir {
        label: "본부".into(),
        script: hq_script.to_path_buf(),
        bridge_state: hq_state_dir.join("office-bridge"),
    }];
    let reg: Option<Value> = std::fs::read_to_string(depts_json)
        .ok()
        .and_then(|t| serde_json::from_str(cys::strip_utf8_bom(&t)).ok());
    if let Some(depts) = reg.as_ref().and_then(|r| r.get("depts")).and_then(|d| d.as_object()) {
        for (name, meta) in depts.iter().take(64) {
            let sock: PathBuf = meta
                .get("socket")
                .and_then(|s| s.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| cys::dept_socket_path(name));
            v.push(OwnerDir {
                label: format!("부서 {name}"),
                script: home.join(".cys").join(format!("pack-dept-{name}")).join("bin").join("javis_hud_bridge.py"),
                bridge_state: crate::state::state_dir(&sock).join("office-bridge"),
            });
        }
    }
    v
}

/// 옛 세대인가 — `/health` 404 · `/world` 200 (최상위에 `"v"`). 둘 다 **접속 직후의 요청**으로 본 응답이다(표식 상태의 옛 브리지도 접속 직후의 요청에는 답한다[실측 R1]).
pub fn is_old_generation(
    health: &Result<cys::BridgeProbeReply, cys::BridgeProbeError>,
    world: &Result<cys::BridgeProbeReply, cys::BridgeProbeError>,
) -> bool {
    let h404 = matches!(health, Ok(r) if r.status == 404);
    let w_ok = match world {
        Ok(r) if r.status == 200 => serde_json::from_slice::<Value>(&r.body)
            .ok()
            .map(|v| v.as_object().map(|o| o.contains_key("v")).unwrap_or(false))
            .unwrap_or(false),
        _ => false,
    };
    h404 && w_ok
}

/// 후보 프로세스인가 — ① 첫 인자(실행 파일)의 이름이 대소문자 구분 없이 `python` 으로 시작 ② **둘째 인자**가 주인의 브리지 스크립트 경로와 완전히 같다
/// ③ 시작 시각이 토큰 파일이 쓰인 시각과 맞는다(파일이 쓰인 시각이 프로세스 시작보다 뒤이고 그 차이가 10초 안 — 브리지는 토큰 파일을 뜰 때 한 번만 쓴다).
pub fn is_candidate(p: &ProcRec, script: &Path, token_mtime: f64) -> bool {
    let Some(exe) = p.cmd.first() else { return false };
    let name = exe.rsplit(['/', '\\']).next().unwrap_or(exe).to_ascii_lowercase();
    if !name.starts_with("python") {
        return false;
    }
    if p.cmd.get(1).map(|s| s.as_str()) != Some(script.to_string_lossy().as_ref()) {
        return false;
    }
    let diff = token_mtime - p.start as f64;
    (0.0..=REPLACE_TOKEN_START_SLACK_SECS).contains(&diff)
}

/// 교체 절차가 바깥 세계에 닿는 곳 — 시험이 가짜로 갈아 끼운다.
pub trait ReplaceEnv {
    /// 접속 직후의 요청(기다림 없음).
    fn probe(&self, path: &str, headers: &[(&str, &str)]) -> Result<cys::BridgeProbeReply, cys::BridgeProbeError>;
    /// 토큰 파일 — (내용, 쓰인 시각 epoch 초).
    fn token(&self, bridge_state: &Path) -> Option<(String, f64)>;
    fn procs(&self) -> Vec<ProcRec>;
    fn terminate(&self, pid: u32);
    /// 포트에 접속이 되는가.
    fn port_open(&self) -> bool;
    fn sleep_ms(&self, ms: u64);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplaceStep {
    /// 새 세대이거나 응답이 없다 — 건드리지 않는다.
    NotOld,
    /// 옛 세대인데 어느 토큰도 맞지 않았다(주인을 가리지 못함) — 아무것도 하지 않는다.
    NoOwner,
    /// 후보가 정확히 하나가 아니다(개수) — 이 주기에는 아무것도 하지 않고 다음 주기에 다시 본다.
    Ambiguous(usize),
    /// 후보의 번호·시작 시각·명령줄이 신호 직전에 달라졌다 — 중단.
    Changed,
    /// 종료 신호 뒤 3초 안에 포트가 비지 않았다 — 거기서 멈춘다(강제 종료로 올리지 않고 자식도 건드리지 않는다).
    PortNotFreed { pid: u32 },
    /// 교체했다(포트가 비었다) — 호출부가 새 브리지를 띄운다. `children` = 끝낸 자식 수.
    Replaced { pid: u32, owner: String, children: usize },
}

/// 옛 브리지 교체 절차 한 번(블로킹 · 시간 상한 안). 호출부는 **데몬 수명당 한 번**만 `Replaced`/`PortNotFreed`/`Changed` 에 이르게 하고,
/// `Ambiguous` 는 최대 [`REPLACE_AMBIGUOUS_ROUNDS`] 주기까지 다시 부른다.
pub fn replace_old_bridge(env: &dyn ReplaceEnv, owners: &[OwnerDir]) -> ReplaceStep {
    // ① 옛 세대일 때만 — 새 세대인데 내 자식이 아닌 브리지(사람이 직접 띄운 것) · 응답 없는 브리지는 건드리지 않는다.
    let health = env.probe("/health", &[]);
    let world = env.probe("/world", &[]);
    if !is_old_generation(&health, &world) {
        return ReplaceStep::NotOld;
    }
    // ② 포트의 주인 확인 — 토큰이 맞으면 `bad_key`, 틀리면 `bad_token`. 맞는 것이 나오면 거기서 멈춘다. (①을 먼저 통과시키는 이유: 토큰은 그 포트를 쥔 프로세스에 넘어간다.)
    let mut owner: Option<(&OwnerDir, f64)> = None;
    for o in owners {
        let Some((token, mtime)) = env.token(&o.bridge_state) else { continue };
        let token = token.trim().to_string();
        if token.is_empty() || token.chars().any(|c| c.is_control()) {
            continue;
        }
        let r = env.probe("/peek?key=none", &[("X-HUD-Token", token.as_str())]);
        if let Ok(rep) = r {
            if rep.status == 403 && String::from_utf8_lossy(&rep.body).contains("bad_key") {
                owner = Some((o, mtime));
                break;
            }
        }
    }
    let Some((owner, token_mtime)) = owner else { return ReplaceStep::NoOwner };
    // ③ 후보 찾기 — 네 조건(같은 사용자 · python 으로 시작 · 둘째 인자가 주인의 스크립트와 완전 일치 · 토큰 파일 시각과 시작 시각 일치).
    let procs = env.procs();
    let cands: Vec<&ProcRec> = procs.iter().filter(|p| is_candidate(p, &owner.script, token_mtime)).collect();
    // ④ 후보가 정확히 하나일 때만.
    let [cand] = cands.as_slice() else { return ReplaceStep::Ambiguous(cands.len()) };
    let cand: ProcRec = (*cand).clone();
    let children = event_children_of(&procs, cand.pid);
    // 신호 직전에 후보 자신의 번호 · 시작 시각 · 명령줄을 한 번 더 확인한다(달라졌으면 중단).
    if !same_process(&cand, &env.procs()) {
        return ReplaceStep::Changed;
    }
    env.terminate(cand.pid);
    // 포트가 비는지 3초까지 확인 — 안 비면 거기서 멈춘다(강제 종료 없음 · 자식도 건드리지 않음).
    let mut freed = false;
    let mut waited = 0u64;
    while waited <= REPLACE_PORT_FREE_SECS * 1000 {
        if !env.port_open() {
            freed = true;
            break;
        }
        env.sleep_ms(100);
        waited += 100;
    }
    if !freed {
        return ReplaceStep::PortNotFreed { pid: cand.pid };
    }
    // 적어 둔 자식 가운데 이벤트 구독 프로세스만 — 끝내기 직전에 번호 · 시작 시각 · 명령줄이 같은지 다시 보고 하나씩.
    let mut ended = 0usize;
    for rec in &children {
        if same_process(rec, &env.procs()) {
            env.terminate(rec.pid);
            ended += 1;
        }
    }
    ReplaceStep::Replaced { pid: cand.pid, owner: owner.label.clone(), children: ended }
}

/// 실제 세계에 닿는 구현.
pub struct RealReplaceEnv {
    pub port: u16,
}

impl ReplaceEnv for RealReplaceEnv {
    fn probe(&self, path: &str, headers: &[(&str, &str)]) -> Result<cys::BridgeProbeReply, cys::BridgeProbeError> {
        cys::bridge_probe_request(self.port, path, headers, 0, 3000)
    }
    fn token(&self, bridge_state: &Path) -> Option<(String, f64)> {
        let p = bridge_state.join("token");
        let text = std::fs::read_to_string(&p).ok()?;
        let mtime = std::fs::metadata(&p)
            .ok()?
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_secs_f64();
        Some((text, mtime))
    }
    fn procs(&self) -> Vec<ProcRec> {
        list_user_processes()
    }
    fn terminate(&self, pid: u32) {
        terminate_pid(pid)
    }
    fn port_open(&self) -> bool {
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], self.port));
        std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(500)).is_ok()
    }
    fn sleep_ms(&self, ms: u64) {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }
}

// ── B3: 우리가 띄운 브리지의 감독(자식이 살아 있는 동안) ───────────────────────────────────

/// 감독이 끝난 이유.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuperviseExit {
    /// 브리지가 스스로(또는 밖에서) 죽었다.
    Died,
    /// 건강 확인이 끝내기로 했다(끝내는 절차까지 마쳤다) — 이유와 끝낸 방법.
    Stopped { reason: &'static str, how: &'static str },
}

/// 우리가 띄운 브리지가 **살아 있는 동안** 60초마다 건강을 확인한다. 연속 3회 실패하면(약 3분) · 팩 버전이 다르면(한 번) B2 의 순서로 끝내고 돌아간다(다시 띄우는 것은 호출부).
/// `child.stdin` 에서 꺼낸 쓰기 쪽(`lifeline`)을 이 함수가 쥔다 — `Child::wait` 는 기다리기 전에 자식의 표준입력을 닫기 때문에(tokio 문서) 꺼내지 않으면 감독을 시작하는 순간
/// 수명줄이 닫혀 브리지가 뜨자마자 끝난다. 데몬이 어떤 이유로든 사라지면 운영체제가 파이프를 닫는다. 데몬의 잠금을 쥐지 않고, 모든 대기에 시간 상한이 있다.
pub async fn supervise_managed_child(
    child: &mut tokio::process::Child,
    lifeline: Option<tokio::process::ChildStdin>,
    port: u16,
    pack_dir: &Path,
) -> SuperviseExit {
    let mut lifeline = lifeline;
    let mut tracker = HealthTracker::default();
    let mut next = tokio::time::Instant::now() + std::time::Duration::from_secs(PROBE_INTERVAL_SECS);
    loop {
        tokio::select! {
            _ = child.wait() => {
                drop(lifeline.take());
                return SuperviseExit::Died;
            }
            _ = tokio::time::sleep_until(next) => {}
        }
        next += std::time::Duration::from_secs(PROBE_INTERVAL_SECS);
        let installed = installed_pack_version(pack_dir);
        let path: &'static str = if tracker.use_world { "/world" } else { "/health" };
        let reply = tokio::task::spawn_blocking(move || cys::bridge_probe_get(port, path))
            .await
            .unwrap_or(Err(cys::BridgeProbeError::NoResponse));
        let mut action = tracker.observe(judge_probe(&reply), installed.as_deref());
        if tracker.use_world && path == "/health" {
            // `/health` 가 없는 옛 스크립트 — 같은 주기에 같은 꼴의 탐침을 `/world` 로 한다(표식 상태면 3회 연속 실패로 잡혀 다시 뜬다).
            let reply = tokio::task::spawn_blocking(move || cys::bridge_probe_get(port, "/world"))
                .await
                .unwrap_or(Err(cys::BridgeProbeError::NoResponse));
            action = tracker.observe(judge_probe(&reply), installed.as_deref());
        }
        let reason = match action {
            HealthAction::Nothing => continue,
            HealthAction::RestartUnhealthy => "건강 확인 연속 실패",
            HealthAction::RestartPackVersion => "팩 버전 불일치",
        };
        let how = stop_managed_child(child, lifeline.take()).await;
        return SuperviseExit::Stopped { reason, how };
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::knobs::{bridge_mode_from, BridgeMode};
    use serde_json::json;
    use std::cell::RefCell;

    fn reply(status: u16, body: &str) -> Result<cys::BridgeProbeReply, cys::BridgeProbeError> {
        Ok(cys::BridgeProbeReply { status, body: body.as_bytes().to_vec(), complete: true })
    }

    // ── B1 ─────────────────────────────────────────────────────────────────

    #[test]
    fn b1_only_the_hq_daemon_spawns_unless_any_owner() {
        let hq_unix = Path::new("/Users/x/.local/state/cys/cys.sock");
        let dept_unix = Path::new("/Users/x/.local/state/cys-dept-dept-3/cys.sock");
        let hq_win = Path::new(r"\\.\pipe\cys");
        let dept_win = Path::new(r"\\.\pipe\cys-dept-dept-3");
        assert!(owner_may_spawn(hq_unix, false));
        assert!(!owner_may_spawn(dept_unix, false));
        assert!(owner_may_spawn(hq_win, false), "본부 파이프를 부서로 오판했다");
        assert!(!owner_may_spawn(dept_win, false), "부서 파이프를 본부로 오판했다");
        assert!(owner_may_spawn(dept_unix, true) && owner_may_spawn(dept_win, true), "ANY_OWNER 는 종전처럼 어느 데몬이든");
    }

    #[test]
    fn b1_spawn_plan_names_the_owner_socket_and_any_owner_restores_the_inherited_environment() {
        let sock = Path::new("/Users/x/.local/state/cys/cys.sock");
        let p = spawn_plan(sock, false, BridgeMode::Managed, Some("0.14.44"));
        assert!(p.env.contains(&("CYS_SOCKET", sock.to_string_lossy().into_owned())));
        assert!(p.env.contains(&("HUD_LIFELINE", "stdin".to_string())));
        assert!(p.env.contains(&("HUD_PACK_VERSION", "0.14.44".to_string())));
        assert!(p.stdin_piped);
        // ANY_OWNER: 환경을 그대로 물려준다 — CYS_SOCKET 을 명시하지 않는다.
        let p = spawn_plan(sock, true, BridgeMode::Managed, None);
        assert!(!p.env.iter().any(|(k, _)| *k == "CYS_SOCKET"));
        assert!(!p.env.iter().any(|(k, _)| *k == "HUD_PACK_VERSION"), "팩 버전을 못 읽었으면 싣지 않는다");
    }

    // ── B2·B3 윈도우 종전 ────────────────────────────────────────────────────

    /// 윈도우 빌드의 기본 방식은 `Legacy` 이고, 그 스폰 설정은 0.14.43 과 같다 — 표준입력 `null` · 수명줄·팩 버전 환경 없음(더하는 것은 B1 의 `CYS_SOCKET` 하나뿐).
    #[test]
    fn b2_windows_default_is_legacy_and_its_spawn_plan_is_the_0_14_43_configuration() {
        let d = bridge_mode_from(None, None, true);
        assert_eq!(d.mode, BridgeMode::Legacy);
        let sock = Path::new(r"\\.\pipe\cys");
        let p = spawn_plan(sock, false, d.mode, Some("0.14.44"));
        assert!(!p.stdin_piped, "윈도우 기본의 표준입력은 종전처럼 null 이어야 한다");
        let keys: Vec<&str> = p.env.iter().map(|(k, _)| *k).collect();
        assert_eq!(keys, vec!["CYS_SOCKET"], "윈도우 기본에서 수명줄·팩 버전 환경이 주어졌다: {keys:?}");
        assert!(!keys.contains(&"HUD_LIFELINE") && !keys.contains(&"HUD_PACK_VERSION"));
        // 맥 기본은 managed.
        assert_eq!(bridge_mode_from(None, None, false).mode, BridgeMode::Managed);
        // 윈도우의 켜는 길은 환경변수뿐이다.
        assert_eq!(bridge_mode_from(Some("managed"), None, true).mode, BridgeMode::Managed);
        assert_eq!(bridge_mode_from(Some(" Legacy "), None, false).mode, BridgeMode::Legacy);
    }

    /// 정책 파일에는 **되돌리는 값만** — 파일의 `managed` 는 무시하고(윈도우 빌드의 스폰 설정이 0.14.43 과 같다) 무시했음을 알린다.
    #[test]
    fn b3_policy_file_cannot_turn_the_managed_supervision_on_only_off() {
        let on = json!({"CYS_OFFICE_BRIDGE_MODE": "managed"});
        let d = bridge_mode_from(None, Some(&on), true);
        assert_eq!(d.mode, BridgeMode::Legacy);
        assert!(d.ignored_policy_managed, "무시했다는 사실이 표시돼야 로그 한 줄이 남는다");
        let p = spawn_plan(Path::new(r"\\.\pipe\cys"), false, d.mode, Some("0.14.44"));
        assert!(!p.stdin_piped && !p.env.iter().any(|(k, _)| *k == "HUD_LIFELINE"));
        // 맥에서는 이미 managed — 파일의 managed 는 무시되지만 결과는 같다.
        let d = bridge_mode_from(None, Some(&on), false);
        assert_eq!(d.mode, BridgeMode::Managed);
        assert!(d.ignored_policy_managed);
        // 파일의 legacy 는 받는다(환경이 managed 여도 되돌리는 쪽이 이긴다).
        let off = json!({"CYS_OFFICE_BRIDGE_MODE": "legacy"});
        assert_eq!(bridge_mode_from(None, Some(&off), false).mode, BridgeMode::Legacy);
        assert_eq!(bridge_mode_from(Some("managed"), Some(&off), false).mode, BridgeMode::Legacy);
        // 다른 두 손잡이도 되돌리는 값만.
        use crate::knobs::{bridge_any_owner_from as any, bridge_replace_old_enabled_from as rep};
        assert!(!any(None, None) && any(Some("1"), None) && any(None, Some(&json!({"CYS_OFFICE_BRIDGE_ANY_OWNER": 1}))));
        assert!(!any(None, Some(&json!({"CYS_OFFICE_BRIDGE_ANY_OWNER": 0}))), "0 은 되돌리는 값이 아니다");
        assert!(rep(None, None) && !rep(Some("0"), None) && !rep(None, Some(&json!({"CYS_OFFICE_BRIDGE_REPLACE_OLD": 0}))));
        assert!(rep(None, Some(&json!({"CYS_OFFICE_BRIDGE_REPLACE_OLD": 1}))), "켜는 값은 파일에서 받지 않는다(기본이 켬)");
    }

    // ── B3 ─────────────────────────────────────────────────────────────────

    #[test]
    fn b3_restart_delay_grows_5_30_60() {
        assert_eq!([0, 1, 2, 3, 4, 10].map(restart_delay_secs), [5, 5, 30, 60, 60, 60]);
    }

    /// 뜨자마자 죽는 브리지(모의): 30분 동안 스폰이 3회를 넘지 않는다. 상한에 닿으면 30분 쉬고 다시 센다.
    #[test]
    fn b3_a_bridge_that_dies_at_once_is_spawned_at_most_three_times_per_thirty_minutes() {
        let mut lim = SpawnLimiter::default();
        let mut now = 0.0f64;
        let mut deaths = 0u32;
        let mut spawn_times: Vec<f64> = Vec::new();
        while now < 7200.0 {
            match lim.gate(now) {
                SpawnGate::Rest(s) => now += s as f64,
                SpawnGate::Go => {
                    lim.record(now);
                    spawn_times.push(now);
                    deaths += 1; // 뜨자마자 죽는다
                    now += restart_delay_secs(deaths) as f64;
                }
            }
        }
        let first_window = spawn_times.iter().filter(|t| **t < RESTART_WINDOW_SECS).count();
        assert!(first_window <= RESTART_MAX, "30분 안에 {first_window}회 띄웠다: {spawn_times:?}");
        // 어느 30분 창을 잡아도 3회 이하.
        for (i, t0) in spawn_times.iter().enumerate() {
            let n = spawn_times.iter().skip(i).take_while(|t| **t - *t0 < RESTART_WINDOW_SECS).count();
            assert!(n <= RESTART_MAX, "창 {t0} 에서 {n}회: {spawn_times:?}");
        }
        assert!(spawn_times.len() >= 6, "쉰 뒤에는 다시 센다(2시간 동안 계속 3회씩): {spawn_times:?}");
    }

    #[test]
    fn b3_judge_probe_and_tracker_follow_the_design() {
        assert_eq!(judge_probe(&reply(200, r#"{"ok":true,"pid":1,"boot_id":"ab","pack_version":"0.14.44"}"#)), ProbeVerdict::Healthy { pack_version: Some("0.14.44".into()) });
        assert_eq!(judge_probe(&reply(200, r#"{"ok":true,"pack_version":null}"#)), ProbeVerdict::Healthy { pack_version: None });
        assert_eq!(judge_probe(&reply(404, "")), ProbeVerdict::NoHealthEndpoint);
        assert_eq!(judge_probe(&reply(500, "")), ProbeVerdict::Failed);
        assert_eq!(judge_probe(&Err(cys::BridgeProbeError::NoResponse)), ProbeVerdict::Failed);
        assert_eq!(judge_probe(&Err(cys::BridgeProbeError::NoConnect)), ProbeVerdict::Failed);
        let incomplete = Ok(cys::BridgeProbeReply { status: 200, body: vec![], complete: false });
        assert_eq!(judge_probe(&incomplete), ProbeVerdict::Failed);
        // 연속 3회 실패 → 재기동 · 한 번이라도 성공하면 0.
        let mut t = HealthTracker::default();
        let healthy = || ProbeVerdict::Healthy { pack_version: None };
        assert_eq!(t.observe(ProbeVerdict::Failed, None), HealthAction::Nothing);
        assert_eq!(t.observe(ProbeVerdict::Failed, None), HealthAction::Nothing);
        assert_eq!(t.observe(healthy(), None), HealthAction::Nothing);
        assert_eq!(t.fails, 0);
        for _ in 0..2 {
            assert_eq!(t.observe(ProbeVerdict::Failed, None), HealthAction::Nothing);
        }
        assert_eq!(t.observe(ProbeVerdict::Failed, None), HealthAction::RestartUnhealthy);
        // `/health` 404 = 옛 스크립트 — 실패가 아니고 `/world` 로 바꾼다.
        let mut t = HealthTracker::default();
        assert_eq!(t.observe(ProbeVerdict::NoHealthEndpoint, None), HealthAction::Nothing);
        assert!(t.use_world && t.fails == 0);
        // 팩 버전: 둘 다 읽었고 다를 때만 1회. 한쪽을 못 읽으면 다르다고 치지 않는다.
        let mut t = HealthTracker::default();
        let v = |s: &str| ProbeVerdict::Healthy { pack_version: Some(s.into()) };
        assert_eq!(t.observe(v("0.14.43"), None), HealthAction::Nothing, "설치 버전을 못 읽음");
        assert_eq!(t.observe(healthy(), Some("0.14.44")), HealthAction::Nothing, "브리지 버전을 모름");
        assert_eq!(t.observe(v("0.14.44"), Some("0.14.44")), HealthAction::Nothing);
        assert_eq!(t.observe(v("0.14.43"), Some("0.14.44")), HealthAction::RestartPackVersion);
        assert_eq!(t.observe(v("0.14.43"), Some("0.14.44")), HealthAction::Nothing, "한 번만");
    }

    #[test]
    fn b3_installed_pack_version_reads_the_marker_and_unreadable_is_none() {
        let dir = std::env::temp_dir().join(format!("cys-b3-pv-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        assert_eq!(installed_pack_version(&dir), None, "표식 파일이 없을 수 있다(팩을 바꿔 끼우는 순간)");
        std::fs::write(dir.join(".pack-version"), "0.14.44\n").expect("w");
        assert_eq!(installed_pack_version(&dir).as_deref(), Some("0.14.44"));
        std::fs::write(dir.join(".pack-version"), "  \n").expect("w");
        assert_eq!(installed_pack_version(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── 기계 강제(소스 고정) ──────────────────────────────────────────────────

    /// 브리지 감독 코드에 그룹 종료 호출 0건 · Feed 발행 호출 0건 · 프로세스 직스폰 0건 · 패닉 경로 0건(이 모듈 운영부 + `main.rs` 의 감독 함수).
    #[test]
    fn mechanical_pins_no_group_signal_no_feed_no_spawn_no_panic_in_the_supervision_code() {
        let me = include_str!("office_bridge.rs");
        let prod = &me[..me.find("\n#[cfg(test)]\n#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]\nmod tests {").expect("앵커")];
        let main = include_str!("main.rs");
        let a = main.find("fn spawn_office_bridge(").expect("감독 함수");
        let b = main.find("/// ★B3: 동봉 runtime python3 절대경로").expect("끝");
        let strip = |t: &str| -> String {
            t.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n")
        };
        let sup_owned = strip(&main[a..b]);
        let sup = sup_owned.as_str();
        let prod_owned = strip(prod);
        let prod = prod_owned.as_str();
        let spawn_needle = ["Command", "::new("].concat();
        for (name, code) in [("office_bridge.rs", prod), ("spawn_office_bridge", sup)] {
            for bad in ["killpg", "kill(-", "kill_process_group", "process_group(", "push_feed", "feed_items", "notify_master"] {
                assert!(!code.contains(bad), "{name}: 금지 호출 `{bad}`");
            }
            assert_eq!(code.matches(&spawn_needle).count(), usize::from(name == "spawn_office_bridge"), "{name}: 프로세스를 띄우는 곳은 감독 함수의 한 곳뿐이다(파이썬 직스폰 census)");
        }
        for bad in [".unwrap()", ".expect(", "panic!(", "unreachable!("] {
            assert!(!prod.contains(bad), "office_bridge.rs 운영부에 패닉 경로 `{bad}`");
        }
        // 수명줄: 스폰 직후 표준입력 쓰기 쪽을 꺼내고(`take()`) 그 뒤에 감독한다 — `wait` 가 먼저 오면 수명줄이 닫힌다.
        let take = sup.find("child.stdin.take()").expect("표준입력을 꺼내지 않는다 — 감독 시작 순간 수명줄이 닫힌다");
        let supervise = sup.find("supervise_managed_child(").expect("감독 호출");
        assert!(take < supervise);
        assert!(!sup[..take].contains("child.wait()"), "수명줄을 꺼내기 전에 wait 를 부른다");
        // 수명줄 환경은 managed 계획에서만 온다.
        assert!(!sup.contains("\"HUD_LIFELINE\""), "감독 함수가 수명줄 환경을 직접 준다 — 계획(spawn_plan)을 거쳐야 한다");
        // 감독 태스크의 대기에는 시간 상한이 있다: 탐침은 블로킹 함수(상한 내장)를 spawn_blocking 으로 · 포트 점검은 timeout.
        assert!(prod.contains("spawn_blocking") && sup.contains("timeout("));
    }

    // ── B7 ─────────────────────────────────────────────────────────────────

    struct Fake {
        health: Result<cys::BridgeProbeReply, cys::BridgeProbeError>,
        world: Result<cys::BridgeProbeReply, cys::BridgeProbeError>,
        /// 토큰 파일 경로 접미(상태 폴더 이름) → (토큰, mtime)
        tokens: Vec<(String, String, f64)>,
        /// 토큰이 맞는(= 이 포트를 쥔 브리지가 받아 주는) 토큰
        accepted_token: Option<String>,
        procs_calls: RefCell<Vec<Vec<ProcRec>>>,
        procs_default: Vec<ProcRec>,
        port_frees_after_terminate: bool,
        terminated: RefCell<Vec<u32>>,
        probes: RefCell<Vec<String>>,
    }

    impl Fake {
        fn old(accepted: &str) -> Fake {
            Fake {
                health: reply(404, ""),
                world: reply(200, r#"{"v":1,"depts":[]}"#),
                tokens: vec![],
                accepted_token: Some(accepted.into()),
                procs_calls: RefCell::new(vec![]),
                procs_default: vec![],
                port_frees_after_terminate: true,
                terminated: RefCell::new(vec![]),
                probes: RefCell::new(vec![]),
            }
        }
    }

    impl ReplaceEnv for Fake {
        fn probe(&self, path: &str, headers: &[(&str, &str)]) -> Result<cys::BridgeProbeReply, cys::BridgeProbeError> {
            self.probes.borrow_mut().push(format!("{path}|{}", headers.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(",")));
            match path {
                "/health" => self.health.clone(),
                "/world" => self.world.clone(),
                _ => {
                    let tok = headers.iter().find(|(k, _)| *k == "X-HUD-Token").map(|(_, v)| v.to_string());
                    if tok.is_some() && tok == self.accepted_token {
                        reply(403, r#"{"ok": false, "error": "bad_key"}"#)
                    } else {
                        reply(403, r#"{"ok": false, "error": "bad_token"}"#)
                    }
                }
            }
        }
        fn token(&self, bridge_state: &Path) -> Option<(String, f64)> {
            let name = bridge_state.to_string_lossy().into_owned();
            self.tokens.iter().find(|(k, _, _)| name.contains(k.as_str())).map(|(_, t, m)| (t.clone(), *m))
        }
        fn procs(&self) -> Vec<ProcRec> {
            let mut q = self.procs_calls.borrow_mut();
            if q.is_empty() {
                self.procs_default.clone()
            } else {
                q.remove(0)
            }
        }
        fn terminate(&self, pid: u32) {
            self.terminated.borrow_mut().push(pid);
        }
        fn port_open(&self) -> bool {
            !(self.port_frees_after_terminate && !self.terminated.borrow().is_empty())
        }
        fn sleep_ms(&self, _ms: u64) {}
    }

    fn py(pid: u32, ppid: Option<u32>, start: u64, args: &[&str]) -> ProcRec {
        ProcRec { pid, ppid, start, cmd: args.iter().map(|s| s.to_string()).collect() }
    }

    const HQ_SCRIPT: &str = "/Users/x/.cys/pack/bin/javis_hud_bridge.py";
    const DEPT_SCRIPT: &str = "/Users/x/.cys/pack-dept-dept-1/bin/javis_hud_bridge.py";

    fn owners() -> Vec<OwnerDir> {
        vec![
            OwnerDir { label: "본부".into(), script: PathBuf::from(HQ_SCRIPT), bridge_state: PathBuf::from("/s/cys/office-bridge") },
            OwnerDir { label: "부서 dept-1".into(), script: PathBuf::from(DEPT_SCRIPT), bridge_state: PathBuf::from("/s/cys-dept-dept-1/office-bridge") },
        ]
    }

    #[test]
    fn b7_replaces_exactly_one_old_bridge_and_its_event_children_only() {
        let mut f = Fake::old("TOKHQ");
        f.tokens = vec![("/s/cys/".into(), "TOKHQ".into(), 1000.4), ("/s/cys-dept-dept-1/".into(), "TOKD1".into(), 900.0)];
        let bridge = py(500, Some(1), 1000, &["/rt/python3", HQ_SCRIPT]);
        let kid = py(501, Some(500), 1001, &["/bin/cys", "events", "--reconnect", "--cursor-file", "/x"]);
        let other_kid = py(502, Some(500), 1001, &["/bin/cat"]); // 이벤트 구독이 아니다 — 건드리지 않는다
        let stranger = py(900, Some(1), 1000, &["/rt/python3", "/other/script.py"]);
        f.procs_default = vec![bridge.clone(), kid.clone(), other_kid, stranger];
        let step = replace_old_bridge(&f, &owners());
        assert_eq!(step, ReplaceStep::Replaced { pid: 500, owner: "본부".into(), children: 1 });
        assert_eq!(*f.terminated.borrow(), vec![500, 501], "브리지와 그 이벤트 구독 자식만 끝낸다(순서: 브리지 → 자식)");
    }

    #[test]
    fn b7_dept_owned_old_bridge_with_a_dept_pack_path_is_replaced() {
        let mut f = Fake::old("TOKD1");
        f.tokens = vec![("/s/cys/".into(), "TOKHQ".into(), 100.0), ("/s/cys-dept-dept-1/".into(), "TOKD1".into(), 2000.5)];
        f.procs_default = vec![py(700, Some(1), 2000, &["/rt/python3", DEPT_SCRIPT])];
        let step = replace_old_bridge(&f, &owners());
        assert_eq!(step, ReplaceStep::Replaced { pid: 700, owner: "부서 dept-1".into(), children: 0 });
        assert_eq!(*f.terminated.borrow(), vec![700]);
    }

    #[test]
    fn b7_wrong_targets_are_never_touched() {
        // ㉮/㉱ 새 세대(/health 200)이거나 포트를 다른 프로그램이 쥐었다 — 세대 확인에서 멈추고 **토큰도 보내지 않는다**.
        let mut f = Fake::old("T");
        f.health = reply(200, r#"{"ok":true}"#);
        f.tokens = vec![("/s/cys/".into(), "T".into(), 1000.0)];
        f.procs_default = vec![py(500, Some(1), 1000, &["/rt/python3", HQ_SCRIPT])];
        assert_eq!(replace_old_bridge(&f, &owners()), ReplaceStep::NotOld);
        assert!(f.terminated.borrow().is_empty());
        assert!(!f.probes.borrow().iter().any(|p| p.contains("X-HUD-Token")), "새 세대에 토큰을 보냈다: {:?}", f.probes.borrow());
        let mut f2 = Fake::old("T");
        f2.world = reply(200, "<html>other program</html>");
        assert_eq!(replace_old_bridge(&f2, &owners()), ReplaceStep::NotOld);
        let mut f3 = Fake::old("T");
        f3.health = Err(cys::BridgeProbeError::NoResponse);
        assert_eq!(replace_old_bridge(&f3, &owners()), ReplaceStep::NotOld, "응답 없는 브리지는 이 절차로 교체하지 않는다");
        // 어느 토큰도 맞지 않는다 — 아무것도 하지 않는다.
        let mut f4 = Fake::old("NOPE");
        f4.tokens = vec![("/s/cys/".into(), "T".into(), 1000.0)];
        f4.procs_default = vec![py(500, Some(1), 1000, &["/rt/python3", HQ_SCRIPT])];
        assert_eq!(replace_old_bridge(&f4, &owners()), ReplaceStep::NoOwner);
        assert!(f4.terminated.borrow().is_empty());
        // ㉯ 래퍼로 떠 있어 후보에 안 잡히고, 같은 스크립트의 다른 인스턴스(다른 때에 뜸)만 명령줄이 맞다 — 시작 시각 조건에서 걸러진다.
        let mut f5 = Fake::old("T");
        f5.tokens = vec![("/s/cys/".into(), "T".into(), 5000.0)];
        f5.procs_default = vec![
            py(600, Some(1), 100, &["/rt/python3", HQ_SCRIPT]),            // 오래전에 뜬 다른 인스턴스(시각 불일치)
            py(601, Some(1), 5000, &["/bin/sh", "-c", HQ_SCRIPT]),         // 래퍼
        ];
        assert_eq!(replace_old_bridge(&f5, &owners()), ReplaceStep::Ambiguous(0));
        assert!(f5.terminated.borrow().is_empty());
        // ㉰ 둘째 인자가 같은 경로인 다른 프로그램(그 파일을 열어 둔 편집기의 모의) — 후보가 아니다.
        let mut f6 = Fake::old("T");
        f6.tokens = vec![("/s/cys/".into(), "T".into(), 1000.2)];
        f6.procs_default = vec![py(610, Some(1), 1000, &["/usr/bin/vim", HQ_SCRIPT])];
        assert_eq!(replace_old_bridge(&f6, &owners()), ReplaceStep::Ambiguous(0));
        // 둘째 인자가 아니라 셋째에 있는 경우 · 경로가 다른 사본도 후보가 아니다.
        let mut f7 = Fake::old("T");
        f7.tokens = vec![("/s/cys/".into(), "T".into(), 1000.2)];
        f7.procs_default = vec![
            py(620, Some(1), 1000, &["/rt/python3", "-u", HQ_SCRIPT]),
            py(621, Some(1), 1000, &["/rt/python3", "/copy/javis_hud_bridge.py"]),
        ];
        assert_eq!(replace_old_bridge(&f7, &owners()), ReplaceStep::Ambiguous(0));
    }

    #[test]
    fn b7_two_candidates_means_do_nothing_this_round() {
        let mut f = Fake::old("T");
        f.tokens = vec![("/s/cys/".into(), "T".into(), 1000.4)];
        // 둘째 브리지가 잠깐 떴다 죽는 54~57밀리초 동안 후보가 2개로 보인다[실측 R10-N3].
        f.procs_default = vec![py(500, Some(1), 1000, &["/rt/python3", HQ_SCRIPT]), py(501, Some(1), 1000, &["/rt/python3", HQ_SCRIPT])];
        assert_eq!(replace_old_bridge(&f, &owners()), ReplaceStep::Ambiguous(2));
        assert!(f.terminated.borrow().is_empty());
    }

    #[test]
    fn b7_port_not_freed_stops_without_force_and_leaves_the_children_alone() {
        let mut f = Fake::old("T");
        f.port_frees_after_terminate = false;
        f.tokens = vec![("/s/cys/".into(), "T".into(), 1000.4)];
        f.procs_default = vec![
            py(500, Some(1), 1000, &["/rt/python3", HQ_SCRIPT]),
            py(501, Some(500), 1001, &["/bin/cys", "events", "--reconnect", "--cursor-file", "/x"]),
        ];
        assert_eq!(replace_old_bridge(&f, &owners()), ReplaceStep::PortNotFreed { pid: 500 });
        assert_eq!(*f.terminated.borrow(), vec![500], "신호는 후보에게 한 번뿐 — 강제 종료로 올리지 않고 자식도 건드리지 않는다");
    }

    #[test]
    fn b7_a_candidate_that_changed_before_the_signal_aborts_and_a_reused_child_pid_is_not_killed() {
        let mut f = Fake::old("T");
        f.tokens = vec![("/s/cys/".into(), "T".into(), 1000.4)];
        let bridge = py(500, Some(1), 1000, &["/rt/python3", HQ_SCRIPT]);
        let kid = py(501, Some(500), 1001, &["/bin/cys", "events", "--reconnect", "--cursor-file", "/x"]);
        f.procs_default = vec![bridge.clone(), kid.clone()];
        // 첫 목록은 정상, 두 번째 목록(신호 직전 재확인)에서 번호가 다른 프로세스로 바뀌었다.
        *f.procs_calls.borrow_mut() = vec![
            vec![bridge.clone(), kid.clone()],
            vec![py(500, Some(1), 4242, &["/rt/python3", "/else.py"])],
        ];
        assert_eq!(replace_old_bridge(&f, &owners()), ReplaceStep::Changed);
        assert!(f.terminated.borrow().is_empty());
        // 자식의 번호가 다른 프로세스로 넘어간 모의: 브리지는 교체되지만 그 번호의 프로세스는 건드리지 않는다.
        let g = {
            let mut g = Fake::old("T");
            g.tokens = vec![("/s/cys/".into(), "T".into(), 1000.4)];
            *g.procs_calls.borrow_mut() = vec![
                vec![bridge.clone(), kid.clone()],                 // ③ 후보 찾기
                vec![bridge.clone(), kid.clone()],                 // 신호 직전 재확인
                vec![py(501, Some(1), 9999, &["/bin/other"])],     // 자식 끝내기 직전 — 같은 번호의 다른 프로세스
            ];
            g
        };
        assert_eq!(replace_old_bridge(&g, &owners()), ReplaceStep::Replaced { pid: 500, owner: "본부".into(), children: 0 });
        assert_eq!(*g.terminated.borrow(), vec![500], "번호가 바뀐 프로세스를 끝냈다");
    }

    #[test]
    fn b7_old_generation_detection_and_candidate_rules() {
        assert!(is_old_generation(&reply(404, ""), &reply(200, r#"{"v":1}"#)));
        assert!(!is_old_generation(&reply(200, "{}"), &reply(200, r#"{"v":1}"#)), "새 세대는 /health 가 있다");
        assert!(!is_old_generation(&reply(404, ""), &reply(200, r#"{"x":1}"#)), "최상위에 v 가 없다");
        assert!(!is_old_generation(&reply(404, ""), &reply(200, r#"[{"v":1}]"#)));
        assert!(!is_old_generation(&reply(404, ""), &Err(cys::BridgeProbeError::NoResponse)));
        let s = Path::new(HQ_SCRIPT);
        let p = |cmd: &[&str], start: u64| py(1, None, start, cmd);
        assert!(is_candidate(&p(&["/rt/python3", HQ_SCRIPT], 1000), s, 1000.5));
        assert!(is_candidate(&p(&["C:\\rt\\Python.EXE", HQ_SCRIPT], 1000), s, 1000.5), "대소문자 구분 없이 python 으로 시작");
        assert!(!is_candidate(&p(&["/rt/node", HQ_SCRIPT], 1000), s, 1000.5));
        assert!(!is_candidate(&p(&["/rt/python3", HQ_SCRIPT], 1000), s, 999.0), "토큰 파일이 프로세스 시작보다 앞이면 아니다");
        assert!(!is_candidate(&p(&["/rt/python3", HQ_SCRIPT], 1000), s, 1011.0), "10초를 넘으면 아니다");
        assert!(!is_candidate(&p(&[], 1000), s, 1000.5));
    }

    #[test]
    fn b7_owner_dirs_lists_hq_first_then_registered_depts_with_their_pack_paths() {
        let dir = std::env::temp_dir().join(format!("cys-b7-owners-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let reg = dir.join("depts.json");
        std::fs::write(&reg, r#"{"depts": {"dept-1": {"display_name": "영업"}, "dept-2": {"socket": "/custom/cys-dept-dept-2/cys.sock"}}}"#).expect("w");
        let o = owner_dirs(Path::new("/s/cys"), Path::new(HQ_SCRIPT), &reg, Path::new("/Users/x"));
        assert_eq!(o.len(), 3);
        assert_eq!(o[0].script, Path::new(HQ_SCRIPT));
        assert_eq!(o[0].bridge_state, Path::new("/s/cys/office-bridge"));
        let d1 = o.iter().find(|x| x.label == "부서 dept-1").expect("dept-1");
        assert_eq!(d1.script, Path::new("/Users/x/.cys/pack-dept-dept-1/bin/javis_hud_bridge.py"));
        let d2 = o.iter().find(|x| x.label == "부서 dept-2").expect("dept-2");
        assert_eq!(d2.bridge_state, Path::new("/custom/cys-dept-dept-2/office-bridge"));
        // 등록부가 없거나 깨졌으면 본부만.
        assert_eq!(owner_dirs(Path::new("/s/cys"), Path::new(HQ_SCRIPT), &dir.join("none.json"), Path::new("/h")).len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn b2_event_children_and_same_process_checks() {
        let b = py(10, Some(1), 100, &["/rt/python3", "x.py"]);
        let k = py(11, Some(10), 101, &["cys", "--socket", "/s", "events", "--reconnect", "--cursor-file", "c"]);
        let z = py(12, Some(10), 101, &["cys", "ping"]);
        let all = vec![b.clone(), k.clone(), z];
        assert_eq!(event_children_of(&all, 10), vec![k.clone()]);
        assert!(same_process(&k, &all));
        assert!(!same_process(&py(11, Some(10), 999, &k.cmd.iter().map(|s| s.as_str()).collect::<Vec<_>>()), &all), "시작 시각이 다르면 다른 프로세스");
        // 실제 프로세스 목록 — 내 프로세스가 같은 사용자의 것으로 보여야 한다.
        let real = list_user_processes();
        assert!(real.iter().any(|p| p.pid == std::process::id()), "프로세스 목록 라이브러리가 자기 자신을 못 본다");
    }

    #[test]
    fn b7_terminate_pid_refuses_pid_0_1_and_self() {
        // 방어 — 이 호출들은 아무것도 보내지 않아야 한다(보냈다면 이 시험 프로세스가 죽는다).
        terminate_pid(0);
        terminate_pid(1);
        terminate_pid(std::process::id());
    }
}
