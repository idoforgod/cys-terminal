use super::*;

struct Fixture {
    d: Arc<Daemon>,
    dir: std::path::PathBuf,
}
impl Fixture {
    fn new() -> Self {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!("s5-{}-{}", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&dir).unwrap();
        Self { d: Daemon::new(dir.join("d.sock")), dir }
    }
    fn seat(&self, role: Option<&str>) -> Arc<crate::state::Surface> {
        self.d.create_surface(None, Some("sleep 60".into()), None, role.map(str::to_string), 24, 80).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let seats: Vec<_> = self.d.surfaces.lock().unwrap_or_else(|e| e.into_inner()).values().cloned().collect();
        for s in seats {
            s.close_operation.clear_poison();
            let _ = close_surface(&self.d, s.id, CloseCause::Reap);
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn stamp(s: &Arc<crate::state::Surface>, age: u64) -> Option<std::time::Instant> {
    // Synthetic EOF state while the owned child sleeps. No reader EOF is racing
    // queue insertion. Automatic Reap intentionally does not require a dead child.
    s.exited.store(true, Ordering::Relaxed);
    let t = Some(std::time::Instant::now().checked_sub(Duration::from_secs(age)).unwrap());
    *s.exited_at.lock().unwrap() = t;
    t
}

fn lifecycle(d: &Arc<Daemon>) -> Vec<Value> {
    d.bus.tail(1000).into_iter().filter(|e| {
        matches!(e["name"].as_str(), Some("surface.reaped" | "surface.closed" | "queue.dropped" | "queue.parked" | "queue.expired_parked"))
    }).collect()
}

fn queue_probe(d: &Arc<Daemon>, s: &Arc<crate::state::Surface>) {
    s.pending_queue.lock().unwrap().push_back(d.next_queue_entry("active-s5".into(), None, "test"));
    let mut e = d.next_queue_entry("expired-s5".into(), None, "test");
    e.expired_at = Some(now_epoch() - 1.0);
    s.expired_queue.lock().unwrap().push_back(e);
}

fn assert_untouched(d: &Arc<Daemon>, s: &Arc<crate::state::Surface>, before: usize) {
    assert!(d.get_surface(s.id).is_some(), "S5 no membership effect");
    assert_eq!(s.pending_queue.lock().unwrap().len(), 1, "S5 no pending effect");
    assert_eq!(s.expired_queue.lock().unwrap().len(), 1, "S5 no expiry effect");
    assert!(d.restored_queue.lock().unwrap().is_empty());
    assert!(d.restored_expired.lock().unwrap().is_empty());
    assert!(d.tombstones.lock().unwrap().is_empty());
    assert_eq!(lifecycle(d).len(), before, "S5 no lifecycle event effect");
    assert!(!crate::delivery::ledger_path(&d.socket_path).exists(), "S5 no expiry ledger effect");
    assert!(!crate::state::state_dir(&d.socket_path).join("queue-expired.json").exists(), "S5 no persistence effect");
}

fn rpc(d: &Arc<Daemon>, params: Value) -> Value {
    let req = cys::Request {
        id: json!(1), method: "surface.reap_due".into(), params,
    };
    let crate::handlers::Reply::Single(r) = crate::handlers::dispatch(d, req, None) else {
        panic!("expected one reply");
    };
    r
}

#[test]
fn aid_s5_absent_target_is_gone_without_peer() {
    let f = Fixture::new();
    let r = rpc(&f.d, json!({"surface_id": u64::MAX}));
    assert_eq!(r["result"]["outcome"], "gone", "S5-GONE: {r}");
    let request = cys::Request { id:json!(2), method:"surface.close".into(),
        params:json!({"surface_id":u64::MAX, "owner_token":f.d.operator_token}) };
    let crate::handlers::Reply::Single(reply) = crate::handlers::dispatch(&f.d, request, Some(std::process::id())) else { panic!("single close reply") };
    assert_eq!(reply["error"]["code"], "not_found", "S5 actual absence remains not_found");
    assert!(lifecycle(&f.d).is_empty());
}

#[test]
fn aid_s5_deferred_matrix_has_zero_effects() {
    let _g = REAP_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _env = ReapEnvGuard::set(&[("CYS_REAP_EXITED", "1"), ("CYS_REAP_EXITED_GRACE_SECS", "60"), ("CYS_REAP_EXITED_NONROLE_GRACE_SECS", "10")]);
    for (case, reason) in [("disabled", "reaper_disabled"), ("active", "active_surface"), ("stamp", "missing_exit_stamp"), ("role_age", "grace_not_elapsed"), ("nonrole_age", "grace_not_elapsed")] {
        let f = Fixture::new();
        let s = f.seat(if case == "nonrole_age" { None } else { Some("worker-s5") });
        stamp(&s, if case == "role_age" { 59 } else if case == "nonrole_age" { 9 } else { 120 });
        if case == "active" { s.exited.store(false, Ordering::Relaxed); }
        if case == "stamp" { *s.exited_at.lock().unwrap() = None; }
        let _disabled = (case == "disabled").then(|| ReapEnvGuard::set(&[("CYS_REAP_EXITED", "0")]));
        queue_probe(&f.d, &s);
        let n = lifecycle(&f.d).len();
        let r = rpc(&f.d, json!({"surface_id": s.id}));
        assert_eq!(r["result"]["outcome"], "deferred", "S5-{case}: {r}");
        assert_eq!(r["result"]["reason"], reason, "S5-{case}-reason: {r}");
        assert_untouched(&f.d, &s, n);
    }
}

#[test]
fn aid_s5_client_facts_and_invalid_targets_cannot_override() {
    let f = Fixture::new();
    let s = f.seat(Some("worker-s5"));
    queue_probe(&f.d, &s);
    let n = lifecycle(&f.d).len();
    for extra in [json!({"cause":"reap"}), json!({"exited":true}), json!({"age":999999}), json!({"force":true}), json!({"internal":true})] {
        let mut p = extra;
        p["surface_id"] = json!(s.id);
        let r = rpc(&f.d, p);
        assert_eq!(r["error"]["code"], "invalid_params", "S5-CLIENT: {r}");
        assert_untouched(&f.d, &s, n);
    }
    for p in [json!({}), json!({"surface_id":-1}), json!({"surface_id":"bad"}), json!({"surface_id":true})] {
        assert_eq!(rpc(&f.d, p)["error"]["code"], "invalid_params");
    }
}

#[test]
fn aid_s5_gate_is_nonblocking_and_poison_is_failure() {
    let _g = REAP_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _env = ReapEnvGuard::set(&[("CYS_REAP_EXITED", "1")]);
    let f = Fixture::new();
    let s = f.seat(Some("worker-s5"));
    stamp(&s, 120);
    queue_probe(&f.d, &s);
    let n = lifecycle(&f.d).len();
    let guard = s.close_operation.lock().unwrap();
    let d = f.d.clone();
    let id = s.id;
    let (tx, rx) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || { tx.send(reap_surface_due(&d, id)).unwrap(); });
    let got = rx.recv_timeout(Duration::from_secs(2));
    drop(guard); // Even a blocking-lock mutant can terminate; no leaked thread.
    thread.join().unwrap();
    assert_eq!(got, Ok(ReapDue::Deferred { reason: "close_in_progress" }), "S5-GATE: due must return without waiting");
    assert_untouched(&f.d, &s, n);
    let copy = s.clone();
    let _ = std::thread::spawn(move || { let _g = copy.close_operation.lock().unwrap(); panic!("S5 deliberate poison"); }).join();
    assert!(matches!(reap_surface_due(&f.d, s.id), ReapDue::Failed { .. }), "S5-POISON");
    assert_untouched(&f.d, &s, n);
    let request = cys::Request { id:json!(2), method:"surface.close".into(), params:json!({"surface_id":s.id,"owner_token":f.d.operator_token}) };
    let crate::handlers::Reply::Single(reply) = crate::handlers::dispatch(&f.d, request, Some(std::process::id())) else { panic!("single close reply") };
    assert_eq!(reply["error"]["code"], "close_failed", "S5-POISON-CLOSE: retained target must not be reported gone");
    assert_untouched(&f.d, &s, n);
    s.close_operation.clear_poison();
}

#[test]
#[cfg(unix)]
fn aid_s5_manual_reap_retained_poison_is_not_not_found() {
    let _g = REAP_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _env = ReapEnvGuard::set(&[("CYS_REAP_EXITED_GRACE_SECS", "60")]);
    let f = Fixture::new();
    let caller = f.seat(Some("cso"));
    let target = f.seat(Some("worker-s5"));
    { let mut child = target.child.lock().unwrap(); child.kill().unwrap(); child.wait().unwrap(); }
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !f.d.bus.tail(1000).iter().any(|e| e["name"] == "surface.exited" && e["surface_id"] == target.id) {
        assert!(std::time::Instant::now() < deadline, "S5 manual fixture EOF must complete");
        std::thread::sleep(Duration::from_millis(5));
    }
    stamp(&target, 120);
    let copy = target.clone();
    let _ = std::thread::spawn(move || { let _g = copy.close_operation.lock().unwrap(); panic!("S5 manual poison"); }).join();
    let request = cys::Request { id:json!(3), method:"surface.reap".into(), params:json!({"surface_id":target.id}) };
    let crate::handlers::Reply::Single(reply) = crate::handlers::dispatch(&f.d, request, Some(caller.pid)) else { panic!("single reap reply") };
    assert_eq!(reply["error"]["code"], "close_failed", "S5-MANUAL-POISON: {reply}");
    assert!(f.d.get_surface(target.id).is_some());
    assert!(lifecycle(&f.d).iter().all(|e| e["name"] != "surface.reaped"));
    target.close_operation.clear_poison();
}

#[test]
fn aid_s5_candidate_rechecks_incarnation_and_stamp() {
    let _g = REAP_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _env = ReapEnvGuard::set(&[("CYS_REAP_EXITED", "1")]);
    let f = Fixture::new();
    let old = f.seat(Some("worker-s5"));
    let observed = stamp(&old, 120);
    queue_probe(&f.d, &old);
    let n = lifecycle(&f.d).len();
    stamp(&old, 121); // Still due, but a different exit generation.
    assert_eq!(reap_due_candidate(&f.d, &old, observed), ReapDue::Deferred { reason:"state_changed" }, "S5-STAMP");
    assert_untouched(&f.d, &old, n);
    *old.exited_at.lock().unwrap() = observed;
    let replacement = f.seat(Some("worker-replacement"));
    f.d.surfaces.lock().unwrap().insert(old.id, replacement.clone());
    let result = reap_due_candidate(&f.d, &old, observed);
    f.d.surfaces.lock().unwrap().insert(old.id, old.clone());
    assert_eq!(result, ReapDue::Deferred { reason:"state_changed" }, "S5-INCARNATION");
    assert_untouched(&f.d, &old, n);
    f.d.surfaces.lock().unwrap().remove(&old.id);
    let result = reap_due_candidate(&f.d, &old, observed);
    f.d.surfaces.lock().unwrap().insert(old.id, old.clone());
    assert_eq!(result, ReapDue::Gone, "S5-REMOVED");
    assert_untouched(&f.d, &old, n);
}

#[test]
fn aid_s5_watchdog_rpc_effects_match_role_and_nonrole() {
    let _g = REAP_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _env = ReapEnvGuard::set(&[("CYS_REAP_EXITED", "1"), ("CYS_REAP_EXITED_GRACE_SECS", "60"), ("CYS_REAP_EXITED_NONROLE_GRACE_SECS", "10")]);
    for role in [Some("worker-s5"), None] {
        let mut observations = Vec::new();
        for watchdog in [false, true] {
            let f = Fixture::new();
            let s = f.seat(role);
            stamp(&s, if role.is_some() { 60 } else { 10 });
            queue_probe(&f.d, &s);
            if watchdog { reap_exited_surfaces(&f.d); } else {
                assert_eq!(rpc(&f.d, json!({"surface_id":s.id}))["result"]["outcome"], "reaped", "S5-DUE");
            }
            assert!(f.d.get_surface(s.id).is_none());
            assert!(f.d.tombstones.lock().unwrap().is_empty(), "S5-REAP-NOT-OWNER");
            assert_eq!(lifecycle(&f.d).iter().filter(|e| e["name"] == "surface.reaped").count(), 1, "S5-ONE-EVENT");
            let mut ev: Vec<_> = lifecycle(&f.d).iter().map(|e| (e["name"].clone(), e["payload"]["reason"].clone(), e["payload"]["role"].clone(), e["payload"]["count"].clone())).collect();
            ev.sort_by_key(|e| e.0.to_string());
            observations.push(json!({"events":ev, "parked":f.d.restored_queue.lock().unwrap().len(), "roles":f.d.roles.lock().unwrap().len()}));
        }
        assert_eq!(observations[0], observations[1], "S5-WATCHDOG-EQUIVALENCE");
    }
}

#[test]
fn aid_s5_concurrent_requests_reap_once() {
    let _g = REAP_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _env = ReapEnvGuard::set(&[("CYS_REAP_EXITED", "1")]);
    let f = Fixture::new();
    let s = f.seat(Some("worker-s5"));
    stamp(&s, 120);
    let barrier = Arc::new(std::sync::Barrier::new(3));
    let jobs: Vec<_> = (0..2).map(|_| {
        let (d, b, id) = (f.d.clone(), barrier.clone(), s.id);
        std::thread::spawn(move || { b.wait(); reap_surface_due(&d, id) })
    }).collect();
    barrier.wait();
    let out: Vec<_> = jobs.into_iter().map(|j| j.join().unwrap()).collect();
    assert_eq!(out.iter().filter(|r| matches!(r, ReapDue::Reaped { .. })).count(), 1, "S5-CONCURRENT");
    assert!(out.iter().all(|r| matches!(r, ReapDue::Reaped {..} | ReapDue::Gone | ReapDue::Deferred { reason:"close_in_progress" })));
    assert_eq!(lifecycle(&f.d).iter().filter(|e| e["name"] == "surface.reaped").count(), 1);
}

#[test]
fn aid_s5_deferred_role_replacement_and_delivery_tick_rehome_once() {
    let _g = REAP_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _env = ReapEnvGuard::set(&[("CYS_REAP_EXITED", "1"), ("CYS_REAP_EXITED_GRACE_SECS", "60")]);
    let f = Fixture::new();
    let old = f.seat(Some("worker-s5"));
    stamp(&old, 0);
    let work = f.d.next_queue_entry("retained S5".into(), None, "test");
    let id = work.id.clone();
    old.pending_queue.lock().unwrap().push_back(work);
    assert_eq!(rpc(&f.d, json!({"surface_id":old.id}))["result"]["outcome"], "deferred");
    let new = f.seat(Some("worker-s5"));
    assert_eq!(new.role.lock().unwrap().as_deref(), Some("worker-s5"), "S5 no worker dedup on exited holder");
    stamp(&old, 60);
    assert_eq!(rpc(&f.d, json!({"surface_id":old.id}))["result"]["outcome"], "reaped");
    assert_eq!(f.d.roles.lock().unwrap().get("worker-s5"), Some(&new.id), "S5 retain new mapping");
    f.d.paused.store(true, Ordering::Relaxed);
    for _ in 0..2 { deliver_queued(&f.d, &mut HashMap::new(), &mut HashMap::new(), &mut HashMap::new()); }
    assert_eq!(new.pending_queue.lock().unwrap().iter().map(|e| e.id.clone()).collect::<Vec<_>>(), vec![id], "S5 rehome exactly once");
    assert!(f.d.restored_queue.lock().unwrap().is_empty());
    assert!(f.d.tombstones.lock().unwrap().is_empty());
}

#[test]
fn aid_s5_due_preserves_expired_work_on_ledger_failure() {
    let _g = REAP_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _env = ReapEnvGuard::set(&[("CYS_REAP_EXITED", "1")]);
    let f = Fixture::new();
    let s = f.seat(Some("worker-s5"));
    stamp(&s, 120);
    queue_probe(&f.d, &s);
    let id = s.expired_queue.lock().unwrap()[0].id.clone();
    let ledger = crate::delivery::ledger_path(&f.d.socket_path);
    std::fs::create_dir_all(&ledger).unwrap();
    assert_eq!(rpc(&f.d, json!({"surface_id":s.id}))["result"]["outcome"], "reaped");
    assert!(f.d.restored_expired.lock().unwrap().iter().any(|e| e["id"] == id), "S5 expiry retained");
    let rows: Value = serde_json::from_slice(&std::fs::read(crate::state::state_dir(&f.d.socket_path).join("queue-expired.json")).unwrap()).unwrap();
    assert!(rows.as_array().unwrap().iter().any(|e| e["id"] == id), "S5 expiry persisted");
}
