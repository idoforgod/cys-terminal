//! launchd 자동등록 — 앱 첫 실행 시 cysd를 RunAtLoad·KeepAlive로 상시 가동 등록.
//!
//! 수동 `cys daemon install`과 **동일 plist 포맷·경로를 단일 소스**로 공유해
//! 자동/수동 등록의 포맷 드리프트를 막는다. 전체가 macOS 한정.
#![cfg(target_os = "macos")]

use std::path::{Path, PathBuf};

pub const LAUNCHD_LABEL: &str = "com.cysjavis.cysd";

/// ~/Library/LaunchAgents/com.cysjavis.cysd.plist
pub fn plist_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Library/LaunchAgents")
        .join(format!("{LAUNCHD_LABEL}.plist"))
}

/// 데몬 로그 경로(소켓 디렉터리 옆 cysd.log).
pub fn log_path() -> PathBuf {
    crate::socket_path()
        .parent()
        .map(|d| d.join("cysd.log"))
        .unwrap_or_else(|| PathBuf::from("/tmp/cysd.log"))
}

/// XML `<string>` 콘텐츠 이스케이프 — 경로·사용자명에 `&`·`<`·`>`가 있어도
/// plist가 조기 종료·손상되지 않게 엔티티로 치환한다.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// ★B3(#6 · 0.14.30): launchd 잡의 PATH — plist `EnvironmentVariables` 로 정적 주입한다.
///
/// 【무엇이 틀렸었나 · PREP §D #6 실측】 plist 에 `EnvironmentVariables` 키가 없어서 launchd 가
/// cysd 를 **launchd 기본 최소 PATH**(`/usr/bin:/bin:/usr/sbin:/sbin`)로 띄웠다. 그 PATH 를
/// 물려받은 부트 체인은 `~/.local/bin/claude`(claude native 설치기의 기본 위치)를 못 찾고
/// `cys boot` 가 에이전트 미발견으로 exit 4 로 떨어진다 — 앱이 스폰한 데몬(사용자 셸 PATH 상속)
/// 에서는 재현되지 않아 launchd 경로에서만 나는 결손이다.
///
/// 【왜 정적 dict 인가 — 기각한 대안】
/// · `launchctl setenv PATH …`: 세션 휘발이라 재적재하면 사라지고, gui 도메인 **전체**(다른
///   사용자 에이전트까지) 를 오염시킨다.
/// · 로그인 셸 래핑(`/bin/zsh -lc cysd`): ProgramArguments 가 바뀌어 `extract_program_path` 의
///   stale 판정과 `pkill -f MacOS/cysd` 폴백이 동시에 깨지고, 사용자 rc 오류가 데몬 기동 실패로
///   번진다(부트체인 위험을 키우는 방향).
///
/// 후보는 **고정 목록**이다(현재 사용자 PATH 스냅샷이 아니다): 스냅샷은 등록 시점의 우연을
/// 영구히 박제해 기계마다 다른 plist 를 만든다. 여기 담긴 것은 claude·brew·시스템 표준 자리뿐이고,
/// HOME 등 다른 키는 넣지 않는다 — 실측된 결손은 PATH 하나이고 추측으로 키를 늘리지 않는다.
pub fn launchd_env_path(home: &Path) -> String {
    let local_bin = home.join(".local").join("bin");
    format!(
        "{}:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin",
        local_bin.display()
    )
}

/// launchd plist 본문 — RunAtLoad(로그인 자동 기동) + KeepAlive(사망 시 재기동).
pub fn render_plist(daemon: &Path, log: &Path) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{LAUNCHD_LABEL}</string>
  <key>ProgramArguments</key><array><string>{daemon}</string></array>
  <key>EnvironmentVariables</key><dict><key>PATH</key><string>{path}</string></dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>ThrottleInterval</key><integer>10</integer>
  <key>StandardOutPath</key><string>{log}</string>
  <key>StandardErrorPath</key><string>{log}</string>
</dict>
</plist>
"#,
        daemon = xml_escape(&daemon.display().to_string()),
        path = xml_escape(&launchd_env_path(&crate::home_dir())),
        log = xml_escape(&log.display().to_string()),
    )
}

/// launchd가 현재 LAUNCHD_LABEL을 적재 중인가(`launchctl list <label>` 성공 여부).
pub fn is_loaded() -> bool {
    std::process::Command::new("launchctl")
        .args(["list", LAUNCHD_LABEL])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// ★W3: CLI autostart가 launchd에 위임할지 판정(순수 함수). launchd가 cysd 서비스를 적재
/// 중이면 sibling spawn 대신 `launchctl kickstart`로 위임한다 — 구형 CLI가 자기 옆 구형 cysd를
/// 띄워 startup lock을 선점하고 launchd 신형(KeepAlive)과 crashloop 하는 경로를 원천 차단.
pub fn should_delegate_autostart(loaded: bool) -> bool {
    loaded
}

/// ★W3: `launchctl kickstart` 인자(순수 함수 — 테스트 가능). service target = gui/<uid>/<label>.
/// 미가동이면 즉시 기동하고, throttle(crashloop 억제)로 대기 중이어도 강제 기동한다.
pub fn kickstart_args(uid: u32) -> [String; 2] {
    ["kickstart".to_string(), format!("gui/{uid}/{LAUNCHD_LABEL}")]
}

/// launchd에 적재된 cysd 서비스를 kickstart(미가동이면 기동). 성공 여부 반환.
/// current uid로 gui 도메인 타깃을 구성한다(로그인 세션의 사용자 데몬).
pub fn kickstart() -> bool {
    let uid = unsafe { libc::getuid() };
    let args = kickstart_args(uid);
    std::process::Command::new("launchctl")
        .args(args.iter().map(String::as_str))
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// plist 본문에서 ProgramArguments의 첫 `<string>`(=cysd 경로, XML 이스케이프된 형태)을 추출.
/// stale drift(plist가 옛 cysd 경로를 가리킴) 감지에 쓴다. 순수 함수(테스트 가능).
fn extract_program_path(content: &str) -> Option<String> {
    let after = content.split("ProgramArguments").nth(1)?;
    let s = after.split("<string>").nth(1)?;
    let path = s.split("</string>").next()?;
    Some(path.to_string())
}

/// ★B3(#6): plist 본문에서 `EnvironmentVariables` 의 PATH 값을 추출한다(이스케이프된 형태 그대로).
/// 키가 없는 **구형 plist**(0.14.29 이하로 등록된 기계)를 stale 로 판정해 재기록을 유도하는 것이
/// 이 함수의 유일한 임무다 — 없으면 이미 등록된 기계는 영원히 옛 plist 로 남는다.
fn extract_env_path(content: &str) -> Option<String> {
    let after = content.split("EnvironmentVariables").nth(1)?;
    let after_key = after.split("<key>PATH</key>").nth(1)?;
    let s = after_key.split("<string>").nth(1)?;
    Some(s.split("</string>").next()?.to_string())
}

/// 현재 기록된 plist가 원하는 상태와 일치하는가 — **cysd 경로 ∧ PATH env** 둘 다.
/// plist가 없거나 파싱 실패 시 false(=불일치로 간주 → 재기록 유도).
fn plist_path_matches(daemon: &Path) -> bool {
    let Ok(content) = std::fs::read_to_string(plist_path()) else {
        return false;
    };
    let path_ok = extract_program_path(&content).as_deref()
        == Some(xml_escape(&daemon.display().to_string()).as_str());
    // ★B3(#6): PATH 결손·드리프트도 stale 이다. 이 항이 없으면 구형 plist 가 '신선'으로
    //   판정돼 재기록되지 않고, 결손이 그 기계에 영구 잔존한다.
    let env_ok = extract_env_path(&content).as_deref()
        == Some(xml_escape(&launchd_env_path(&crate::home_dir())).as_str());
    path_ok && env_ok
}

/// 이 결과에서 launchd가 cysd 기동을 책임지는가 — 앱 setup이 **수동 spawn을 건너뛰고**
/// launchd-owned cysd의 socket-ready를 폴링할지 결정한다(split-brain·이중 spawn 방지).
/// Registered/AlreadyRegistered = launchd가 띄움. DeferredDaemonRunning = 기존 데몬이
/// 이미 가동(connect로 충분, launchd 미적재).
pub fn launchd_will_serve(outcome: RegisterOutcome) -> bool {
    matches!(
        outcome,
        RegisterOutcome::Registered | RegisterOutcome::AlreadyRegistered
    )
}

/// plist를 ~/Library/LaunchAgents에 기록(부모 디렉터리 생성 포함). 기록한 경로 반환.
pub fn write_plist(daemon: &Path) -> std::io::Result<PathBuf> {
    let path = plist_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, render_plist(daemon, &log_path()))?;
    Ok(path)
}

/// 자동등록 결과 — 호출자가 로그·UI 분기에 사용.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegisterOutcome {
    /// plist가 이미 존재 — 이미 launchd 소유(무동작).
    AlreadyRegistered,
    /// plist 기록 + launchctl load 완료(launchd가 cysd 소유).
    Registered,
    /// 데몬이 이미 가동 중 — plist만 기록(load 보류, 다음 로그인 발효).
    /// 현 세션의 flock 단일소유를 깨지 않아 가동 세션 소멸을 피한다.
    DeferredDaemonRunning,
}

/// 자동등록 결정(순수 함수 — 부작용 없음, 진리표 테스트 가능).
/// 멱등성은 plist 존재만이 아니라 **'plist 존재 AND launchd 적재'** 교차검사로 판정한다 —
/// DeferredDaemonRunning(plist만 기록·load 보류) 후 데몬이 소멸하면 plist는 남아도
/// 적재는 안 된 상태이므로, 재호출 시 `loaded=false`를 보고 load를 시도해 KeepAlive 보호를 복원한다.
struct Plan {
    write: bool,
    load: bool,
    outcome: RegisterOutcome,
}

/// `fresh` = plist 존재 **그리고** 그 ProgramArguments 경로가 현재 원하는 cysd 경로와 일치.
/// stale drift(앱 이동·재설치로 plist가 옛 경로를 가리킴)면 fresh=false → 재기록 + reload 유도.
fn plan(fresh: bool, loaded: bool, daemon_running: bool) -> Plan {
    if fresh && loaded {
        // plist가 올바른 경로로 존재 AND launchd 적재 중 — 무동작.
        Plan { write: false, load: false, outcome: RegisterOutcome::AlreadyRegistered }
    } else if daemon_running {
        // 가동 데몬이 flock을 보유 — 지금 load하면 충돌. plist만 갱신하고 보류
        // (부재/stale면 재기록, 다음 로그인·데몬 소멸 후 재호출 시 load).
        Plan { write: !fresh, load: false, outcome: RegisterOutcome::DeferredDaemonRunning }
    } else {
        // 미가동 + (미적재 또는 stale) → 재기록 + load(신규 등록·보호 복원·stale 경로 교정).
        Plan { write: !fresh, load: true, outcome: RegisterOutcome::Registered }
    }
}

/// 앱 첫 실행/매 기동 시 자동등록(멱등). 부작용 없는 `plan()`이 결정한 대로 plist 기록·load.
/// 멱등성은 'plist 존재 AND 경로 일치(fresh) AND launchd 적재' 교차검사로 판정한다.
/// - 미등록 + 데몬 미가동(첫 실행): plist write + `launchctl load` → launchd가 즉시 cysd 소유
/// - plist 존재하나 미적재 + 데몬 미가동(deferred 후 데몬 소멸 등): load로 KeepAlive 보호 복원
/// - plist가 옛 경로(stale drift) + 데몬 미가동: 재기록 + reload로 경로 교정
/// - 데몬 가동 중: plist만 보장(load 보류) → 현 세션 flock 단일소유 비파괴
/// - fresh AND 적재 중: 무동작
pub fn register_if_absent(daemon: &Path, daemon_running: bool) -> std::io::Result<RegisterOutcome> {
    // fresh = plist 존재 AND 경로 일치(stale drift 아님).
    let fresh = plist_path().exists() && plist_path_matches(daemon);
    let p = plan(fresh, is_loaded(), daemon_running);
    if p.write {
        write_plist(daemon)?;
        if !p.load {
            // ★B3(#6): launchd 는 **적재 시점**의 plist 를 읽는다 — 파일만 갱신하면 가동 중인
            //   잡의 env 는 그대로다. 사용자가 "고쳤는데 그대로"로 오인하지 않게 1줄 고지한다
            //   (가동 중 데몬을 여기서 재적재하지 않는 것은 종전 계약 — 세션 단일 소유 비파괴).
            eprintln!(
                "[launchd] plist 갱신(PATH 등) — 가동 중 데몬에는 미적용 · 다음 로그인 또는 \
                 재적재(launchctl unload/load) 시 발효"
            );
        }
    }
    if p.load {
        let path = plist_path();
        // 재등록 대비 unload(실패 무시) 후 load.
        let _ = std::process::Command::new("launchctl")
            .args(["unload", "-w"])
            .arg(&path)
            .output();
        let out = std::process::Command::new("launchctl")
            .args(["load", "-w"])
            .arg(&path)
            .output()?;
        if !out.status.success() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!(
                    "launchctl load failed: {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                ),
            ));
        }
    }
    Ok(p.outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn render_plist_has_reboot_survival_keys_and_daemon_path() {
        let daemon = Path::new("/Applications/cys.app/Contents/MacOS/cysd");
        let log = Path::new("/tmp/cysd.log");
        let plist = render_plist(daemon, log);
        // 재부팅 생존 핵심: RunAtLoad(로그인 자동 기동) + KeepAlive(사망 시 재기동).
        assert!(plist.contains("<key>RunAtLoad</key><true/>"), "RunAtLoad 누락");
        assert!(plist.contains("<key>KeepAlive</key><true/>"), "KeepAlive 누락");
        // launchd가 띄울 대상은 정확한 cysd 절대경로.
        assert!(
            plist.contains("<string>/Applications/cys.app/Contents/MacOS/cysd</string>"),
            "ProgramArguments에 cysd 경로 누락"
        );
        assert!(plist.contains(&format!("<string>{LAUNCHD_LABEL}</string>")), "Label 누락");
        assert!(plist.contains("<string>/tmp/cysd.log</string>"), "로그 경로 누락");
        // 유효한 plist 골격.
        assert!(plist.starts_with("<?xml"), "plist 헤더 누락");
    }

    /// ★B3(#6): plist 가 PATH 를 실어야 launchd 로 뜬 cysd 가 `~/.local/bin/claude` 를 찾는다.
    /// 이 키가 없어서 `cys boot` 가 에이전트 미발견(exit 4)으로 떨어졌다(PREP §D #6).
    #[test]
    fn b3_plist_carries_env_path_with_local_bin_first() {
        let plist = render_plist(
            Path::new("/Applications/cys.app/Contents/MacOS/cysd"),
            Path::new("/tmp/cysd.log"),
        );
        assert!(
            plist.contains("<key>EnvironmentVariables</key>"),
            "EnvironmentVariables 키 부재 — launchd 최소 PATH 로 떨어진다"
        );
        let home = crate::home_dir();
        let want = xml_escape(&launchd_env_path(&home));
        assert!(plist.contains(&want), "plist PATH 값 불일치: {plist}");
        assert!(
            want.starts_with(&xml_escape(&home.join(".local").join("bin").display().to_string())),
            "claude native 설치 위치(~/.local/bin)가 선두여야 한다: {want}"
        );
        for p in ["/usr/bin", "/bin", "/opt/homebrew/bin", "/usr/local/bin"] {
            assert!(want.contains(p), "표준 경로 {p} 누락: {want}");
        }
    }

    /// PATH 후보는 **고정 목록**이다 — 등록 시점의 사용자 PATH 스냅샷을 박제하지 않는다
    /// (기계마다 다른 plist 가 생기고, 등록 당시의 우연이 영구화된다).
    #[test]
    fn b3_launchd_env_path_is_deterministic_not_a_snapshot() {
        let a = launchd_env_path(Path::new("/Users/x"));
        let b = launchd_env_path(Path::new("/Users/x"));
        assert_eq!(a, b);
        assert_eq!(
            a,
            "/Users/x/.local/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"
        );
        assert!(!a.contains("\n"), "PATH 에 개행이 들어가면 plist 가 깨진다");
    }

    /// 구형 plist(PATH 키 없음)는 **stale** 로 판정돼야 재기록된다 — 아니면 이미 등록된
    /// 기계에 결손이 영구 잔존한다. 추출기는 신형에서만 값을 낸다(음성 대조 포함).
    #[test]
    fn b3_extract_env_path_detects_legacy_plist_without_the_key() {
        let legacy = r#"<plist><dict>
  <key>ProgramArguments</key><array><string>/x/cysd</string></array>
  <key>RunAtLoad</key><true/>
</dict></plist>"#;
        assert_eq!(extract_env_path(legacy), None, "구형 plist 는 PATH 미보유 = stale");
        let current = render_plist(Path::new("/x/cysd"), Path::new("/tmp/l.log"));
        assert_eq!(
            extract_env_path(&current).as_deref(),
            Some(xml_escape(&launchd_env_path(&crate::home_dir())).as_str()),
            "신형 plist 에서 PATH 를 못 읽으면 매 등록마다 재기록돼 launchctl 이 흔들린다"
        );
    }

    #[test]
    fn label_matches_manual_install_contract() {
        // 수동 `cys daemon install`과 동일 라벨이어야 status/uninstall이 자동등록분을 인식한다.
        assert_eq!(LAUNCHD_LABEL, "com.cysjavis.cysd");
    }

    #[test]
    fn render_plist_escapes_xml_special_chars_in_path() {
        // 사용자명·경로에 &,<,> 가 있어도 plist가 손상되지 않아야 한다.
        let daemon = Path::new("/Users/x/a&b<c>/cys.app/Contents/MacOS/cysd");
        let plist = render_plist(daemon, Path::new("/tmp/l&g.log"));
        assert!(plist.contains("/Users/x/a&amp;b&lt;c&gt;/cys.app/Contents/MacOS/cysd"));
        assert!(plist.contains("/tmp/l&amp;g.log"));
        // 원시(미이스케이프) 앰퍼샌드가 <string> 안에 남으면 안 된다.
        assert!(!plist.contains("a&b<c>"));
    }

    #[test]
    fn plan_outcomes_with_fresh_loaded_running() {
        use RegisterOutcome::*;
        // (fresh, loaded, daemon_running) → (write, load, outcome). fresh = plist 존재 AND 경로 일치.
        // fresh + 적재 중 = 무동작.
        assert_eq!(tup(plan(true, true, false)), (false, false, AlreadyRegistered));
        assert_eq!(tup(plan(true, true, true)), (false, false, AlreadyRegistered));
        // 첫 실행(미존재=非fresh·미가동) → write + load.
        assert_eq!(tup(plan(false, false, false)), (true, true, Registered));
        // ★멱등성 회귀: fresh plist이나 미적재 + 데몬 소멸(미가동) → load로 보호 복원(write 불요).
        // fresh=true라 write=false지만 미적재라 load=true.
        assert_eq!(tup(plan(true, false, false)), (false, true, Registered));
        // ★stale drift: 非fresh(옛 경로) + 미적재 + 미가동 → 재기록 + reload로 경로 교정.
        assert_eq!(tup(plan(false, false, false)), (true, true, Registered));
        // 데몬 가동 중(非fresh) → plist write·load 보류.
        assert_eq!(tup(plan(false, false, true)), (true, false, DeferredDaemonRunning));
        // 데몬 가동 중(fresh·미적재) → load 보류(flock 충돌 회피·write 불요).
        assert_eq!(tup(plan(true, false, true)), (false, false, DeferredDaemonRunning));
    }

    fn tup(p: Plan) -> (bool, bool, RegisterOutcome) {
        (p.write, p.load, p.outcome)
    }

    #[test]
    fn extract_program_path_roundtrips_with_render_plist() {
        // render_plist가 쓴 경로를 그대로 추출해야 stale 비교가 정확하다(이스케이프 형태로).
        let daemon = Path::new("/Applications/cys.app/Contents/MacOS/cysd");
        let plist = render_plist(daemon, Path::new("/tmp/cysd.log"));
        assert_eq!(
            extract_program_path(&plist).as_deref(),
            Some("/Applications/cys.app/Contents/MacOS/cysd")
        );
        // 이스케이프 경로도 그대로(이스케이프된 형태로) 추출 — plist_path_matches는 동일 형태끼리 비교.
        let weird = Path::new("/Users/x/a&b/MacOS/cysd");
        let plist = render_plist(weird, Path::new("/tmp/cysd.log"));
        assert_eq!(extract_program_path(&plist).as_deref(), Some("/Users/x/a&amp;b/MacOS/cysd"));
        // 깨진 입력은 None.
        assert_eq!(extract_program_path("no program args here"), None);
    }

    #[test]
    fn should_delegate_autostart_iff_launchd_loaded() {
        // ★W3 (d): launchd 적재 시 위임(kickstart), 미적재 시 sibling spawn 경로.
        assert!(should_delegate_autostart(true), "적재=위임");
        assert!(!should_delegate_autostart(false), "미적재=sibling");
    }

    #[test]
    fn kickstart_args_targets_gui_domain_label() {
        // ★W3 (d): service target = gui/<uid>/<label> — 로그인 세션 사용자 데몬.
        assert_eq!(
            kickstart_args(501),
            ["kickstart".to_string(), "gui/501/com.cysjavis.cysd".to_string()]
        );
        // uid는 그대로 반영(다른 사용자 오타깃 방지).
        assert_eq!(kickstart_args(0)[1], "gui/0/com.cysjavis.cysd");
    }

    #[test]
    fn launchd_will_serve_only_for_registered_and_already() {
        use RegisterOutcome::*;
        // launchd가 띄울 책임 → 앱 setup이 수동 spawn 건너뛰고 socket-ready 폴링.
        assert!(launchd_will_serve(Registered));
        assert!(launchd_will_serve(AlreadyRegistered));
        // 기존 데몬 가동 중(deferred) → launchd 미적재, connect로 충분 → 수동 경로 허용.
        assert!(!launchd_will_serve(DeferredDaemonRunning));
    }
}
