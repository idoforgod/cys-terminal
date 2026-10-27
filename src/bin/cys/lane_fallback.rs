//! CLI-only lane discovery. Never called by cysd's socket_path().
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub(super) const BUDGET: Duration = Duration::from_secs(1);
#[derive(Debug, Clone)]
pub(super) struct Binding {
    pub socket: PathBuf,
    pub surface: Option<u64>,
}

fn remaining(deadline: Instant) -> Result<Duration, String> {
    deadline.checked_duration_since(Instant::now()).filter(|d| !d.is_zero())
        .ok_or_else(|| "lane discovery deadline exceeded".into())
}

/// One read-only worker per CLI invocation. recv_timeout bounds even an uninterruptible OS
/// read/ancestry/connect; no join on expiration. A blocked worker can remain until process exit,
/// but cannot publish env or send a mutating RPC. Each next operation checks the same deadline.
pub(super) fn bounded<T: Send + 'static>(budget: Duration,
    work: impl FnOnce(Instant) -> Result<T, String> + Send + 'static) -> Result<T, String> {
    let deadline = Instant::now() + budget;
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    std::thread::Builder::new().name("lane-discovery".into()).spawn(move || {
        let result = work(deadline);
        let _ = tx.send(result);
    }).map_err(|e| e.to_string())?;
    let result = rx.recv_timeout(remaining(deadline)?).map_err(|e| format!("lane discovery: {e}"))?;
    remaining(deadline)?; // A late success is never accepted, including scheduler delay.
    result
}

fn ancestry_with(mut pid: u32, deadline: Instant,
    mut parent: impl FnMut(u32) -> Option<u32>) -> Result<Vec<u32>, String> {
    let mut found = Vec::new();
    for _ in 0..32 {
        remaining(deadline)?;
        if pid <= 1 || found.contains(&pid) { break; }
        found.push(pid);
        let next = parent(pid);
        remaining(deadline)?;
        match next { Some(p) => pid = p, None => break }
    }
    Ok(found)
}

fn ancestry(pid: u32, deadline: Instant) -> Result<Vec<u32>, String> {
    let mut sys = sysinfo::System::new();
    ancestry_with(pid, deadline, |pid| {
        let id = sysinfo::Pid::from_u32(pid);
        sys.refresh_processes_specifics(sysinfo::ProcessesToUpdate::Some(&[id]), true,
            sysinfo::ProcessRefreshKind::nothing());
        sys.process(id).and_then(|p| p.parent()).map(|p| p.as_u32())
    })
}

/// Remaining time reaches the connect retry and IO watchdog. The outer bounded worker also
/// covers OS calls (including local file reads) that have no portable cancellable primitive.
fn identify_before(socket: &Path, deadline: Instant) -> Result<Value, String> {
    remaining(deadline)?;
    #[cfg(unix)]
    let mut stream = std::os::unix::net::UnixStream::connect(socket).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    let mut stream = loop {
        remaining(deadline)?;
        match std::fs::OpenOptions::new().read(true).write(true).open(socket) {
            Ok(file) => break file,
            Err(e) if e.raw_os_error() == Some(cys::PIPE_BUSY_ERROR) => {
                cys::wait_named_pipe(socket, remaining(deadline)?.min(cys::PIPE_BUSY_WAIT_SLICE));
            }
            Err(e) => return Err(e.to_string()),
        }
    };
    // In particular, a late connect must NOT send even the read-only identify request.
    let io_deadline = super::RpcDeadline::arm(&stream, Some(remaining(deadline)?))?;
    let reply = super::rpc_roundtrip(&mut stream, &io_deadline, "system.identify",
        serde_json::json!({"caller": null}));
    remaining(deadline)?;
    reply
}

fn live_cysd(pid: u32, deadline: Instant) -> Result<bool, String> {
    remaining(deadline)?;
    let mut sys = sysinfo::System::new();
    let id = sysinfo::Pid::from_u32(pid);
    sys.refresh_processes_specifics(sysinfo::ProcessesToUpdate::Some(&[id]), true,
        sysinfo::ProcessRefreshKind::nothing().with_exe(sysinfo::UpdateKind::Always));
    remaining(deadline)?;
    Ok(sys.process(id).is_some_and(|p| {
        p.status() != sysinfo::ProcessStatus::Zombie && p.status() != sysinfo::ProcessStatus::Dead
            && p.exe().and_then(|p| p.file_name()).and_then(|s| s.to_str())
                .is_some_and(|s| s == "cysd" || s == "cysd.exe")
    }))
}

/// Shared production selection/comparison core; fixtures replace only external observations.
fn resolve_with(
    candidates: Vec<PathBuf>, chain: &[u32], deadline: Instant,
    mut read: impl FnMut(&Path, Instant) -> Result<Value, String>,
    mut alive: impl FnMut(u32, Instant) -> Result<bool, String>,
    mut identify: impl FnMut(&Path, Instant) -> Result<Value, String>,
) -> Result<Binding, String> {
    let mut survivor = None;
    for socket in candidates {
        remaining(deadline)?;
        let epoch = read(&socket, deadline);
        remaining(deadline)?;
        let Ok(epoch) = epoch else { continue; };
        if epoch["socket"].as_str() != socket.to_str() { continue; }
        let Some(pid) = epoch["pid"].as_u64().and_then(|p| u32::try_from(p).ok()) else { continue; };
        if !chain.contains(&pid) { continue; }
        let live = alive(pid, deadline)?;
        remaining(deadline)?;
        if !live { continue; }
        if survivor.is_some() { return Err("ambiguous lane ancestry".into()); }
        survivor = Some((socket, epoch));
    }
    let (socket, epoch) = survivor.ok_or("no proven lane ancestor")?;
    let incarnation = epoch["incarnation"].as_str().filter(|s| !s.is_empty())
        .ok_or("epoch incarnation absent")?;
    remaining(deadline)?;
    let reply = identify(&socket, deadline)?;
    remaining(deadline)?;
    if reply["socket_path"] != epoch["socket"] || reply["daemon_pid"] != epoch["pid"]
        || reply["incarnation"].as_str() != Some(incarnation) {
        return Err("lane identity mismatch".into());
    }
    Ok(Binding { socket, surface: reply["caller"]["surface_id"].as_u64() })
}

pub(super) fn discover() -> Result<Binding, String> {
    bounded(BUDGET, |deadline| {
        remaining(deadline)?;
        let registry = cys::lane::registry_sockets(&cys::lane::registry_path());
        remaining(deadline)?;
        let mut candidates = vec![cys::default_socket_path()];
        candidates.extend(registry.unwrap_or_default().into_iter().map(PathBuf::from));
        candidates.sort(); candidates.dedup();
        let chain = ancestry(std::process::id(), deadline)?;
        let state = cys::lane::state_dir();
        resolve_with(candidates, &chain, deadline, |socket, d| {
            remaining(d)?;
            let text = std::fs::read_to_string(cys::lane::epoch_path_in(&state, socket))
                .map_err(|e| e.to_string())?;
            remaining(d)?;
            serde_json::from_str(&text).map_err(|e| e.to_string())
        }, live_cysd, identify_before)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn run(candidates: &[&str], epoch: Value, reply: Value) -> Result<Binding, String> {
        resolve_with(candidates.iter().map(PathBuf::from).collect(), &[42], Instant::now()+BUDGET,
            |_, _| Ok(epoch.clone()), |_, _| Ok(true), |_, _| Ok(reply.clone()))
    }
    fn epoch(socket: &str) -> Value {
        json!({"socket":socket,"pid":42,"daemon_epoch":100.000001,"incarnation":"64-2a"})
    }
    fn reply(socket: &str) -> Value {
        json!({"socket_path":socket,"daemon_pid":42,"started_at":100.000002,
            "incarnation":"64-2a","caller":{"surface_id":7,"surface_ref":"surface:7"}})
    }
    #[test]
    fn p2_ancestry_is_bounded_and_cycles_stop() {
        assert_eq!(ancestry_with(100, Instant::now()+BUDGET, |p| Some(p+1)).unwrap().len(),32);
        assert_eq!(ancestry_with(42, Instant::now()+BUDGET, |_| Some(42)).unwrap(),vec![42]);
        assert_eq!(ancestry_with(42, Instant::now()+BUDGET, |p| Some(if p==42 {43} else {42})).unwrap(),vec![42,43]);
    }
    #[test]
    fn p2_ambiguous_and_non_daemon_candidates_make_no_rpc() {
        for live in [false,true] {
            let r = resolve_with(vec!["/tmp/a.sock".into(),"/tmp/b.sock".into()], &[42], Instant::now()+BUDGET,
                |p,_| Ok(epoch(p.to_str().unwrap())), |_,_| Ok(live), |_,_| panic!("no unique candidate"));
            assert!(r.is_err());
        }
    }
    #[test]
    fn p2_distinct_clock_samples_resolve_socket_and_surface() {
        let b=run(&["/tmp/a.sock"],epoch("/tmp/a.sock"),reply("/tmp/a.sock")).unwrap();
        assert_eq!(b.surface,Some(7)); assert_eq!(b.socket,Path::new("/tmp/a.sock"));
    }
    #[test]
    fn p2_recycled_pid_and_missing_incarnation_refused() {
        let mut r=reply("/tmp/a.sock"); r["incarnation"]=json!("999-2a");
        assert!(run(&["/tmp/a.sock"],epoch("/tmp/a.sock"),r).is_err());
        let mut r=reply("/tmp/a.sock"); r.as_object_mut().unwrap().remove("incarnation");
        assert!(run(&["/tmp/a.sock"],epoch("/tmp/a.sock"),r).is_err());
        let mut e=epoch("/tmp/a.sock"); e.as_object_mut().unwrap().remove("incarnation");
        assert!(run(&["/tmp/a.sock"],e,reply("/tmp/a.sock")).is_err());
    }
    #[test]
    fn p2_collision_and_base_alias_use_recorded_socket() {
        for (a,b) in [("/tmp/a_b.sock","/tmp/a/b.sock"),("/tmp/cys/cys.sock","/other/cys/cys.sock")] {
            assert_eq!(cys::lane::lane_key(Path::new(a)),cys::lane::lane_key(Path::new(b)));
            assert!(run(&[a],epoch(b),reply(b)).is_err());
            assert_eq!(run(&[a,b],epoch(b),reply(b)).unwrap().socket,Path::new(b));
        }
    }
    #[test]
    fn p2_late_local_read_has_hard_return_bound() {
        let start=Instant::now();
        let result=bounded(Duration::from_millis(30), |d| {
            resolve_with(vec![PathBuf::from("/tmp/a.sock")],&[42],d,
                |_,_| { std::thread::sleep(Duration::from_millis(300)); Ok(epoch("/tmp/a.sock")) },
                |_,_| Ok(true), |_,_| panic!("expired reads must not reach RPC"))
        });
        assert!(result.is_err()); assert!(start.elapsed()<Duration::from_millis(200));
    }
    #[test]
    fn p2_late_rpc_has_hard_return_bound_and_slow_success_control() {
        for (delay, success) in [(5,true),(300,false)] {
            let start=Instant::now();
            let result=bounded(Duration::from_millis(80),move |d| {
                resolve_with(vec![PathBuf::from("/tmp/a.sock")],&[42],d,
                    |_,_| Ok(epoch("/tmp/a.sock")), |_,_| Ok(true), |_,_| {
                        std::thread::sleep(Duration::from_millis(delay)); Ok(reply("/tmp/a.sock"))
                    })
            });
            assert_eq!(result.is_ok(),success); assert!(start.elapsed()<Duration::from_millis(250));
        }
    }
    #[test]
    fn p2_post_operation_deadline_rejects_late_success() {
        let result=resolve_with(vec![PathBuf::from("/tmp/a.sock")],&[42],Instant::now()+Duration::from_millis(10),
            |_,_| Ok(epoch("/tmp/a.sock")),|_,_| Ok(true),|_,_| {
                std::thread::sleep(Duration::from_millis(20)); Ok(reply("/tmp/a.sock"))
            });
        assert!(result.is_err());
    }
}
