// Included only inside handlers::tests. Test-only constructed identities below exercise pure
// equality/overflow branches; production witnesses are minted only by checked_submit_sender.
mod checked_submit_sender_witness {
    use super::*;
    use crate::state::Surface;
    use crate::submit_assoc::CallerKey;
    use cys::process_observation::Identity;
    use std::sync::Weak;

    fn identity(pid: u32, parent: u32, seconds: u64, micros: u64) -> Identity {
        Identity {
            pid,
            parent,
            process_group: 300,
            birth_seconds: seconds,
            birth_microseconds: micros,
            pointer_width: std::mem::size_of::<usize>() * 8,
            executable: b"fixture-only".to_vec(),
        }
    }

    // This constructor is in the test module only. It is not an ingress authority factory.
    fn synthetic_sender(
        historical: Option<CallerKey>,
        authenticated: Option<SubmitAuthenticatedActor>,
    ) -> CheckedSubmitSender {
        CheckedSubmitSender {
            target: Weak::new(),
            target_id_at_check: 77,
            historical,
            authenticated,
        }
    }

    #[test]
    fn c8_checked_sender_process_incarnation_ignores_reparenting_not_birth() {
        let initial = identity(120, 40, 12, 34);
        let mut reparented = initial.clone();
        reparented.parent = 1;
        reparented.process_group = 400;
        reparented.executable = b"different-current-executable".to_vec();
        let before = submit_parent_incarnation(&initial).unwrap();
        let after = submit_parent_incarnation(&reparented).unwrap();
        assert_eq!(before, after, "parent/group/image changes do not change PID+birth identity");
        let a = synthetic_sender(None, Some(SubmitAuthenticatedActor::KernelParent(before)));
        let b = synthetic_sender(None, Some(SubmitAuthenticatedActor::KernelParent(after)));
        assert!(a.same_authenticated_actor(&b));
        let reused = submit_parent_incarnation(&identity(120, 40, 12, 35)).unwrap();
        let reused = synthetic_sender(None, Some(SubmitAuthenticatedActor::KernelParent(reused)));
        assert!(!a.same_authenticated_actor(&reused), "PID reuse must not inherit the actor");
        let another = submit_parent_incarnation(&identity(121, 40, 12, 34)).unwrap();
        let another = synthetic_sender(None, Some(SubmitAuthenticatedActor::KernelParent(another)));
        assert!(!a.same_authenticated_actor(&another));
    }

    #[test]
    fn c8_checked_sender_birth_projection_is_checked() {
        assert_eq!(submit_parent_incarnation(&identity(8, 1, 7, 123)).unwrap().birth_us, 7_000_123);
        assert!(submit_parent_incarnation(&identity(0, 1, 7, 123)).is_none());
        assert!(submit_parent_incarnation(&identity(1, 0, 7, 123)).is_none(), "init cannot identify unrelated orphan callers");
        assert!(submit_parent_incarnation(&identity(8, 1, 0, 0)).is_none());
        assert!(submit_parent_incarnation(&identity(8, 1, 7, 1_000_000)).is_none());
        assert!(submit_parent_incarnation(&identity(8, 1, u64::MAX, 0)).is_none());
        assert!(submit_parent_incarnation(&identity(8, 1, u64::MAX / 1_000_000, 999_999)).is_none());
        let seconds = u64::MAX / 1_000_000;
        let micros = u64::MAX % 1_000_000;
        assert_eq!(submit_parent_incarnation(&identity(8, 1, seconds, micros)).unwrap().birth_us, u64::MAX);
        assert!(observe_submit_parent(None).is_none());
        assert!(observe_submit_parent(Some(0)).is_none());
    }

    #[test]
    fn c8_checked_sender_parent_capture_rechecks_the_callers_birth_and_parent_edge() {
        let caller=identity(10,20,100,10);let parent=identity(20,1,90,5);
        let run=|again:Identity| {
            let mut facts=vec![caller.clone(),parent.clone(),again].into_iter();
            observe_submit_parent_with(Some(10),|pid| {
                let value=facts.next().expect("only3 observations");assert_eq!(pid,value.pid);Ok(value)
            })
        };
        assert_eq!(run(caller.clone()),submit_parent_incarnation(&parent));
        let mut reparented=caller.clone();reparented.parent=1;
        assert!(run(reparented).is_none(), "reparented caller cannot borrow the old parent witness");
        let mut reused=caller.clone();reused.birth_microseconds+=1;
        assert!(run(reused).is_none(), "a reused peer pid cannot borrow the old parent witness");
        for invalid in [identity(10,20,0,0),identity(10,20,100,1_000_000),identity(10,20,u64::MAX,0)] {
            let mut facts=vec![invalid.clone(),parent.clone(),invalid].into_iter();
            assert!(observe_submit_parent_with(Some(10),|_|Ok(facts.next().unwrap())).is_none(),
                "equal invalid peer birth cannot prove continuity");
        }
        let mut calls=0;
        assert!(observe_submit_parent_with(Some(10),|_| {calls+=1;Ok(identity(10,1,100,10))}).is_none());
        assert_eq!(calls,1,"an init-parent caller stops before treating init as its actor");
        assert!(observe_submit_parent_with(Some(10),|_|Err(cys::process_observation::Failure::Unknown {
            stage:"fixture",os_error:None})).is_none());
    }

    #[test]
    fn c8_checked_sender_claimed_and_unresolved_never_authenticate() {
        let claimed = synthetic_sender(Some(CallerKey::Claimed(42)), None);
        let claimed_again = synthetic_sender(Some(CallerKey::Claimed(42)), None);
        let unresolved = synthetic_sender(None, None);
        assert!(!claimed.same_authenticated_actor(&claimed_again));
        assert!(!unresolved.same_authenticated_actor(&unresolved));
        let independently_known = synthetic_sender(
            Some(CallerKey::Claimed(42)),
            Some(SubmitAuthenticatedActor::KernelParent(SubmitParentIncarnation { pid: 9, birth_us: 10 })),
        );
        assert!(!claimed.same_authenticated_actor(&independently_known));
        let audit = independently_known.audit();
        assert_eq!(audit["historical"]["grade"], "claimed");
        assert_eq!(audit["authenticated"]["grade"], "kernel_parent");
        assert_eq!(audit["authenticated"]["parent_pid"], 9);
        assert_eq!(audit["evidence_time"], "dispatch_acl_native_observation");
        assert_eq!(independently_known.historical_key(), Some(&CallerKey::Claimed(42)));
    }

    #[test]
    fn c8_checked_sender_two_dead_weak_references_do_not_match() {
        let a = synthetic_sender(None, Some(SubmitAuthenticatedActor::VerifiedSurface {
            surface: Weak::new(), surface_id_at_check: 17,
        }));
        let b = synthetic_sender(None, Some(SubmitAuthenticatedActor::VerifiedSurface {
            surface: Weak::new(), surface_id_at_check: 17,
        }));
        assert!(!a.same_authenticated_actor(&a));
        assert!(!a.same_authenticated_actor(&b), "even identical dead weak pointers are not actors");
        assert_eq!(a.audit()["authenticated"]["surface_id_at_check"], 17,
            "audit retains captured data without claiming present liveness");
    }

    // Exactly two real PTYs in one bounded fixture. No CLI, socket listener, scheduler,
    // workload dispatch or service daemon is started. The shell executable is a private
    // script that ignores -lc and execs cat: it cannot run a user's login profile or leave
    // a sleep child behind. This fixture uses the production Surface constructor/resolver.
    #[cfg(unix)]
    mod native {
        use super::*;
        use std::ffi::OsString;
        use std::os::unix::fs::PermissionsExt;
        use std::path::{Path, PathBuf};
        use std::time::{Duration, Instant};

        struct EnvRestore {
            key: &'static str,
            previous: Option<OsString>,
        }
        impl EnvRestore {
            fn set(key: &'static str, value: &Path) -> Self {
                let previous = std::env::var_os(key);
                std::env::set_var(key, value);
                Self { key, previous }
            }
        }
        impl Drop for EnvRestore {
            fn drop(&mut self) {
                match &self.previous {
                    Some(value) => std::env::set_var(self.key, value),
                    None => std::env::remove_var(self.key),
                }
            }
        }
        struct TempRoot(PathBuf);
        impl TempRoot {
            fn new() -> Self {
                static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                let seq = SEQ.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir().join(format!(
                    "cys-checked-sender-{}-{}-{}", std::process::id(),
                    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(), seq));
                std::fs::create_dir(&path).unwrap();
                Self(path)
            }
        }
        impl Drop for TempRoot {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        struct Rig {
            daemon: Arc<Daemon>,
            surfaces: Vec<Arc<Surface>>,
        }
        impl Rig {
            fn surface(&mut self, root: &Path) -> Arc<Surface> {
                let env = vec![
                    ("HOME".to_string(), root.to_string_lossy().into_owned()),
                    ("ZDOTDIR".to_string(), root.to_string_lossy().into_owned()),
                ];
                let surface = self.daemon.create_surface_with_env(
                    Some(root.to_string_lossy().into_owned()), Some("ignored".into()),
                    None, None, 24, 80, &env, None).expect("isolated Surface fixture");
                self.surfaces.push(surface.clone());
                // The fixture script execs cat once. Wait for that fixed image before
                // testing positive capture; a transient exec Changed is correctly not proof.
                let until=Instant::now()+Duration::from_secs(5);
                loop {
                    if cys::process_observation::observe(surface.pid).ok()
                        .is_some_and(|p|p.identity.executable.ends_with(b"/cat")) { break; }
                    assert!(Instant::now()<until,"owned fixture did not reach its fixed cat image");
                    std::thread::sleep(Duration::from_millis(10));
                }
                surface
            }
            fn close(&mut self) -> Result<(), String> {
                let mut errors = Vec::new();
                let owned = std::mem::take(&mut self.surfaces);
                for surface in &owned {
                    // Only the exact object created here is removed, never an unrelated map entry.
                    let mut map = self.daemon.surfaces.lock().unwrap_or_else(|e| e.into_inner());
                    if map.get(&surface.id).is_some_and(|s| Arc::ptr_eq(s, surface)) {
                        map.remove(&surface.id);
                    }
                }
                for surface in &owned {
                    let mut child = surface.child.lock().unwrap_or_else(|e| e.into_inner());
                    match child.try_wait() {
                        Ok(Some(_)) => (),
                        Ok(None) => {
                            if let Err(e) = child.kill() {
                                errors.push(format!("fixture kill {}: {e}", surface.id));
                            }
                        }
                        Err(e) => errors.push(format!("fixture query {}: {e}", surface.id)),
                    }
                }
                for surface in &owned {
                    let deadline = Instant::now() + Duration::from_secs(5);
                    loop {
                        let result = surface.child.lock().unwrap_or_else(|e| e.into_inner()).try_wait();
                        match result {
                            Ok(Some(_)) => break,
                            Err(e) => {
                                errors.push(format!("fixture reap {}: {e}", surface.id));
                                break;
                            }
                            Ok(None) if Instant::now() < deadline => {
                                std::thread::sleep(Duration::from_millis(10));
                            }
                            Ok(None) => {
                                errors.push(format!("fixture child {} did not exit", surface.id));
                                break;
                            }
                        }
                    }
                }
                if errors.is_empty() { Ok(()) } else { Err(errors.join("; ")) }
            }
        }
        impl Drop for Rig {
            fn drop(&mut self) {
                if let Err(e) = self.close() {
                    eprintln!("checked-sender fixture cleanup: {e}");
                }
            }
        }
        fn wait_surface_dropped(weak: &Weak<Surface>) {
            let deadline = Instant::now() + Duration::from_secs(5);
            while weak.upgrade().is_some() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(weak.upgrade().is_none(), "fixture Surface retained after child/reader shutdown");
        }

        #[test]
        fn c8_checked_sender_real_factory_acl_scope_and_weak_lifetimes() {
            let _env_lock = ACL_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let _state = crate::delivery::tests::isolate_state_dir("checked-sender");
            let root = TempRoot::new();
            let shell = root.0.join("fixture-shell");
            std::fs::write(&shell, b"#!/bin/sh\nexec /bin/cat\n").unwrap();
            std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o700)).unwrap();
            std::fs::write(root.0.join("acl.json"), r#"{"default":"allow","rules":[]}"#).unwrap();
            let _pack = EnvRestore::set(cys::pack::ENV_PACK_DIR, &root.0);
            let _shell = EnvRestore::set("CYS_SHELL", &shell);
            let mut rig = Rig { daemon: Daemon::new(root.0.join("cysd.sock")), surfaces: Vec::new() };
            let a = rig.surface(&root.0);
            let b = rig.surface(&root.0);
            assert_ne!(a.pid, 0);
            assert_ne!(b.pid, 0);
            assert_eq!(probe_caller_surface_uncached(&rig.daemon, a.pid), Some(a.id));
            assert_eq!(probe_caller_surface_uncached(&rig.daemon, b.pid), Some(b.id));
            let spoofed = json!({"from":b.id, "caller_pid":b.pid});
            let self_sender = checked_submit_sender(&rig.daemon, &a, Some(a.pid), &spoofed).unwrap();
            let a_to_b = checked_submit_sender(&rig.daemon, &b, Some(a.pid), &json!({})).unwrap();
            let b_to_a = checked_submit_sender(&rig.daemon, &a, Some(b.pid), &json!({})).unwrap();
            assert!(self_sender.same_authenticated_actor(&a_to_b));
            assert!(!self_sender.same_authenticated_actor(&b_to_a));
            assert_eq!(self_sender.historical_key(), Some(&CallerKey::Surface(a.id)));
            assert!(self_sender.matches_target(&a));
            assert!(!self_sender.matches_target(&b), "an ACL check cannot move to another target");

            // A live cached SID is only historical ACL data. Independently recapture the
            // actual root even when a positive cache entry (without a birth) says otherwise.
            rig.daemon.caller_cache.lock().unwrap().insert(a.pid,crate::state::CallerCacheEntry::new(
                Some(b.id),crate::state::now_epoch(),None,0));
            let stale = checked_submit_sender(&rig.daemon,&a,Some(a.pid),&json!({})).unwrap();
            assert_eq!(stale.historical_key(),Some(&CallerKey::Surface(b.id)));
            assert_eq!(stale.audit()["authenticated"]["surface_id_at_check"],a.id,
                "cached ACL sid became current caller evidence");
            assert!(stale.same_authenticated_actor(&self_sender));
            assert!(!stale.same_authenticated_actor(&b_to_a));
            rig.daemon.caller_cache.lock().unwrap().remove(&a.pid);

            // The same bounded production collector is driven by named native facts here.
            // Actual root handle is live; synthetic identities only test rejection branches.
            let root_fact=cys::process_observation::observe(a.pid).unwrap().identity;
            let peer_pid=900_001;assert_ne!(peer_pid,a.pid);assert_ne!(peer_pid,b.pid);
            let peer=identity(peer_pid,a.pid,root_fact.birth_seconds+1,10);
            let capture=|changed:Option<Identity>,missing_root:bool| {
                let mut reads=0;
                observe_submit_surface_with(&rig.daemon,Some(peer_pid),|pid| {
                    if pid==peer_pid {reads+=1;Ok(if reads==1 {peer.clone()} else {changed.clone().unwrap_or_else(||peer.clone())})}
                    else if pid==a.pid && !missing_root {Ok(root_fact.clone())}
                    else {Err(cys::process_observation::Failure::Unknown {stage:"fixture native missing",os_error:None})}
                })
            };
            assert!(capture(None,false).is_some_and(|s|Arc::ptr_eq(&s,&a)));
            let mut moved=peer.clone();moved.parent=b.pid;
            assert!(capture(Some(moved),false).is_none(),"changed descendant parent edge authenticated");
            let mut reused=peer.clone();reused.birth_microseconds+=1;
            assert!(capture(Some(reused),false).is_none(),"changed descendant birth authenticated");
            assert!(capture(None,true).is_none(),"direct map hit replaced missing native root evidence");
            assert!(observe_submit_surface_with(&rig.daemon,Some(a.pid),|_|Err(
                cys::process_observation::Failure::Unknown {stage:"fixture direct root missing",os_error:None})).is_none(),
                "direct root mapping authenticated an unavailable native observation");

            // Same declared number on a distinct actual object is not the same actor. This is
            // test-only object replacement simulation, not a production identity constructor.
            let same_number_other_object = synthetic_sender(None, Some(SubmitAuthenticatedActor::VerifiedSurface {
                surface: Arc::downgrade(&b), surface_id_at_check: a.id,
            }));
            assert!(!self_sender.same_authenticated_actor(&same_number_other_object));

            let unknown = checked_submit_sender(&rig.daemon, &a, None, &json!({})).unwrap();
            let claimed = checked_submit_sender(&rig.daemon, &a, None, &json!({"from":a.id})).unwrap();
            assert!(unknown.authenticated.is_none(), "ACL permission does not require an actor for token calls");
            assert!(claimed.authenticated.is_none(), "Claimed text cannot mint authentication");
            assert_eq!(claimed.historical_key(), Some(&CallerKey::Claimed(a.id)));
            assert!(unknown.matches_target(&a));
            assert!(!claimed.same_authenticated_actor(&self_sender));

            // Native parent capture is currently implemented by process_observation on these
            // platforms. A new platform gets no invented success; its factory remains usable
            // with authentication=None. This fixture does not claim Windows native coverage.
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            {
                let pid = std::process::id();
                assert_eq!(probe_caller_surface_uncached(&rig.daemon, pid), None);
                let first = checked_submit_sender(&rig.daemon, &a, Some(pid), &json!({"from":9001})).unwrap();
                let second = checked_submit_sender(&rig.daemon, &a, Some(pid), &json!({"from":9002})).unwrap();
                assert!(matches!(first.authenticated.as_ref(), Some(SubmitAuthenticatedActor::KernelParent(_))));
                assert!(first.same_authenticated_actor(&second), "real kernel evidence is independent of Claimed labels");
                assert_eq!(first.historical_key(), Some(&CallerKey::Claimed(9001)));
                assert_eq!(first.audit()["authenticated"]["grade"], "kernel_parent");
                rig.daemon.caller_cache.lock().unwrap().remove(&pid);
                rig.daemon.delegated_callers.lock().unwrap().insert(pid,a.id);
                let delegated=checked_submit_sender(&rig.daemon,&a,Some(pid),&json!({})).unwrap();
                assert_eq!(delegated.historical_key(),Some(&CallerKey::Surface(a.id)));
                assert_eq!(delegated.audit()["authenticated"]["grade"],"kernel_parent",
                    "delegated alias became actual surface ancestry");
                rig.daemon.delegated_callers.lock().unwrap().remove(&pid);
                rig.daemon.caller_cache.lock().unwrap().remove(&pid);
            }

            std::fs::write(root.0.join("acl.json"), r#"{"default":"deny","rules":[]}"#).unwrap();
            let denied = checked_submit_sender(&rig.daemon, &a, None, &json!({"from":a.id}));
            assert!(denied.is_err(), "ACL denial must precede witness construction");
            assert!(checked_submit_sender(&rig.daemon, &a, Some(b.pid), &json!({})).is_err());

            let weak_a = Arc::downgrade(&a);
            let weak_b = Arc::downgrade(&b);
            rig.close().expect("owned fixture child cleanup");
            let eof_deadline = Instant::now() + Duration::from_secs(5);
            while (!a.exited.load(Ordering::Acquire) || !b.exited.load(Ordering::Acquire))
                && Instant::now() < eof_deadline
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(a.exited.load(Ordering::Acquire) && b.exited.load(Ordering::Acquire),
                "fixture EOF must close both retained Surface objects");
            assert!(!self_sender.matches_target(&a));
            // Deterministic EOF-not-yet-visible window: even with the live flag delayed and
            // a synthetic native row, a reaped owned child cannot authenticate this root.
            a.exited.store(false,Ordering::Release);
            rig.daemon.surfaces.lock().unwrap().insert(a.id,a.clone());
            assert!(observe_submit_surface_with(&rig.daemon,Some(a.pid),|_|Ok(root_fact.clone())).is_none(),
                "dead owned child authenticated through a delayed exit flag");
            rig.daemon.surfaces.lock().unwrap().remove(&a.id);
            a.exited.store(true,Ordering::Release);
            assert!(!self_sender.same_authenticated_actor(&a_to_b),
                "a closed actor cannot remain live merely because a caller holds Arc<Surface>");
            drop(a);
            drop(b);
            // Keep all sender witnesses alive across removal. They include both self-send and
            // A->B / B->A edges. None may keep either Surface or its PTY resources alive.
            wait_surface_dropped(&weak_a);
            wait_surface_dropped(&weak_b);
            assert!(!self_sender.same_authenticated_actor(&a_to_b));
            assert!(!b_to_a.same_authenticated_actor(&b_to_a));
        }
    }
}
