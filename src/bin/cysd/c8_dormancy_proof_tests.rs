// C8 dormancy proof (HEAD-RULING-C8-SCOPE-0.14.52 point 2b): 0.14.52 ships C8 receiver binding INACTIVE.
// agent_proof() has one non-PassThrough source, Surface::receiver_proof; no production path may write it.
// Surfaces come only from the create_surface* constructors (the single Surface literal is in
// create_surface_with_env); claim-role and reclaim finish in finalize_agent_meta_after_role_change, the
// place C8 activation would wire policy and binding. The production entry-point census is separate evidence.
// Unix only: the finalize body is cfg(unix) and the fixture seat runs `sleep 30` (agent_queue_tests convention).
#[cfg(unix)]
struct DormancyFx {
    daemon: Arc<Daemon>,
    dir: std::path::PathBuf,
    _g: std::sync::MutexGuard<'static, ()>,
}

#[cfg(unix)]
impl Drop for DormancyFx {
    fn drop(&mut self) {
        for s in self.daemon.surfaces.lock().unwrap_or_else(|e| e.into_inner()).values() {
            let mut child = s.child.lock().unwrap_or_else(|e| e.into_inner());
            let _ = child.kill();
            let _ = child.wait();
        }
        std::env::remove_var(cys::pack::ENV_PACK_DIR);
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[cfg(unix)]
struct RefuseAllProof;
#[cfg(unix)]
impl crate::agent_write::Proof for RefuseAllProof {
    fn admit(&self) -> Result<(), crate::receiver::ProofLoss> {
        Err(crate::receiver::ProofLoss::Unknown("c8-dormancy-probe".into()))
    }
    fn recheck(&self, _: crate::agent_write::Phase) -> Result<(), crate::receiver::ProofLoss> {
        Err(crate::receiver::ProofLoss::Unknown("c8-dormancy-probe".into()))
    }
}

#[cfg(unix)]
fn assert_c8_pass_through(s: &crate::state::Surface, at: &str) {
    assert!(s.receiver_proof.lock().unwrap_or_else(|e| e.into_inner()).is_none(),
        "C8 dormancy: a production path wrote receiver_proof ({at}, surface {})", s.id);
    assert!(s.agent_proof().admit().is_ok(), "C8 dormancy: agent_proof is not PassThrough ({at})");
}

// One list, two instruments: the population is scripts/c8_dormancy_census.py output, never a hand list.
const C8_DORMANCY_POPULATION: &str = include_str!("c8_dormancy_population.txt");

#[cfg(unix)]
#[test]
fn c8_dormancy_production_surfaces_keep_pass_through_proof() {
    let rows: Vec<Vec<&str>> = C8_DORMANCY_POPULATION.lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty()).map(|l| l.split_whitespace().collect()).collect();
    assert!(!rows.iter().any(|r| r[0] == "field" && r[1] == "write"), "census found a production write of receiver_proof");
    for partial in ["decline_unstarted", "append_owned_submit"] {
        assert!(rows.iter().any(|r| r[0] == "partial" && r[1] == partial && r.get(2) == Some(&"none")),
            "partial landing {partial} has a production caller in the census");
    }
    let constructors: Vec<&str> = rows.iter().filter(|r| r[0] == "field" && r[1] == "init-none").map(|r| r[3]).collect();
    let finalizers: Vec<&str> = rows.iter().filter(|r| r[0] == "role_finalize" && r[1] == "ref").map(|r| r[3]).collect();
    assert!(!constructors.is_empty() && !finalizers.is_empty(), "census population lost its constructor or finalize rows");
    let g = crate::governance::PACK_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    crate::delivery::tests::isolate_state_dir_for_thread("c8-dormancy");
    let dir = std::env::temp_dir().join(format!("cys-c8-dormancy-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("acl.json"), r#"{"default":"allow","rules":[]}"#).unwrap();
    std::env::set_var(cys::pack::ENV_PACK_DIR, &dir);
    let fx = DormancyFx { daemon: Daemon::new(dir.join("cysd.sock")), dir, _g: g };
    let mut surfaces = Vec::new();
    for constructor in &constructors {
        let s = match *constructor {
            "create_surface_with_env" => fx.daemon.create_surface_with_env(None, Some("sleep 30".into()), None,
                Some("worker".into()), 24, 80, &[], None).expect("create_surface_with_env"),
            other => panic!("census population names a constructor this test cannot drive: {other}"),
        };
        fx.daemon.surfaces.lock().unwrap().insert(s.id, s.clone());
        assert_c8_pass_through(&s, constructor);
        surfaces.push(s);
    }
    for via in &finalizers {
        for s in &surfaces {
            finalize_agent_meta_after_role_change(&fx.daemon, s.id, "worker", via);
            assert_c8_pass_through(s, via);
        }
    }
    // agent_proof() reads only receiver_proof: a stored proof is used, clearing it restores PassThrough.
    let first = &surfaces[0];
    *first.receiver_proof.lock().unwrap() = Some(Arc::new(RefuseAllProof));
    assert!(first.agent_proof().admit().is_err(), "agent_proof ignored a stored receiver_proof");
    *first.receiver_proof.lock().unwrap() = None;
    assert_c8_pass_through(first, "cleared");
}
