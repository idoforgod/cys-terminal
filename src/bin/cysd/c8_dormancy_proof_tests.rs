// C8 dormancy proof (HEAD-RULING-C8-SCOPE-0.14.52 point 2b): 0.14.52 ships C8 receiver binding INACTIVE.
// agent_proof() has one non-PassThrough source, Surface::receiver_proof; no production path may write it.
// Surfaces come only from the create_surface* constructors (the single Surface literal is in
// create_surface_with_env); claim-role and reclaim finish in finalize_agent_meta_after_role_change, the
// place C8 activation would wire policy and binding. The production entry-point census is separate evidence.
// The dynamic test drives each census constructor and calls the shared finalize function once per census route
// label (it does not run the claim-role handler or reclaim_commit themselves). Unix only: the finalize body is
// cfg(unix), and its fixture seat runs `sleep 30`, so the observed-agent branch (agent_meta set) is not entered;
// that branch, and every other production site, is covered by the lexical pin below (R-C8DORM-1 #1, #2).
// Limits (R-C8DORM-1 #7): the census compiles for the macOS host only (the lexical pin reads every cfg); the
// population is a static file pinned at the commit in its header and needs the script re-run after a change;
// CI runs these tests, not the census; macro-built names, build-script output and code outside src/bin/cysd
// are not seen; the census's positive control covered the classes it planted (constructor init, finalize
// assignment, a partial item, a C8 module).
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
    for via in &finalizers {
        assert!(matches!(*via, "dispatch_method" | "reclaim_commit"),
            "census population names a role-finalize route this test does not know: {via}");
    }
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

// ★R-C8DORM-1 #1/#2 lexical pin: outside test code, receiver_proof has exactly four known shapes in the cysd
// production modules - the state.rs field, its constructor init to None, the agent_proof read, and the governance.rs
// reconciliation read - each pinned as its whole statement. A production writer added later anywhere (a handler, a
// second constructor path, the observed-agent branch of finalize_agent_meta_after_role_change, a cfg(windows) block,
// or a mutating call on an existing read) changes a shape or adds one and turns this red without re-running the
// census. Tokens come from c8_sampler_tokens (comments and literals skipped); items attributed #[cfg(test)] or
// #[cfg(all(test, ..))] are cut by bracket balance; modules are followed from main.rs, and a production #[path] module
// not in the table, a nested file module or an include! fails closed. Not covered: a field name built by a macro from
// strings; code outside src/bin/cysd (the field is private to the daemon).
const C8_RECEIVER_PROOF_SHAPES: &[(&str, &str)] = &[
    ("governance.rs", ". lock ( ) . unwrap ( ) . is_some ( )"),
    ("state.rs", ". lock ( ) . unwrap_or_else ( | e | e . into_inner ( ) ) . clone ( ) . unwrap_or_else ( | | Arc : : new ( crate : : agent_write : : PassThrough ) )"),
    ("state.rs", ": Mutex < Option < Arc < dyn crate : : agent_write : : Proof > > >"),
    ("state.rs", ": Mutex : : new ( None )"),
];

/// Tokens with every item attributed #[cfg(test)] / #[cfg(all(test, ..))] removed. An item ends at ';' or ',' at its
/// own depth, before a '}' that closes the enclosing block, or after its own '{..}' block - never later, so a cut
/// cannot swallow production code that follows it.
fn c8_cut_test_items<'a>(tokens: &[&'a str]) -> Result<Vec<&'a str>, String> {
    let (mut out, mut i) = (Vec::new(), 0);
    while i < tokens.len() {
        let test_attr = tokens.get(i..i + 4) == Some(&["#", "[", "cfg", "("][..])
            && (tokens.get(i + 4..i + 6) == Some(&["test", ")"][..]) || tokens.get(i + 4..i + 7) == Some(&["all", "(", "test"][..]));
        if !test_attr { out.push(tokens[i]); i += 1; continue; }
        let mut j = i;
        while tokens.get(j) == Some(&"#") && tokens.get(j + 1) == Some(&"[") {
            let mut depth = 0usize;
            loop {
                match tokens.get(j) {
                    Some(&"[") => depth += 1,
                    Some(&"]") => { depth -= 1; if depth == 0 { j += 1; break; } }
                    None => return Err("unterminated attribute".into()),
                    _ => {}
                }
                j += 1;
            }
        }
        let mut depth = 0isize;
        loop {
            match tokens.get(j) {
                None => return Err("unterminated cfg(test) item".into()),
                Some(&"(") | Some(&"[") => depth += 1,
                Some(&")") | Some(&"]") => depth -= 1,
                Some(&";") | Some(&",") if depth == 0 => { j += 1; break; }
                Some(&"}") if depth == 0 => break,
                Some(&"{") if depth == 0 => {
                    let mut braces = 0usize;
                    loop {
                        match tokens.get(j) {
                            Some(&"{") => braces += 1,
                            Some(&"}") => { braces -= 1; if braces == 0 { break; } }
                            None => return Err("unterminated cfg(test) block".into()),
                            _ => {}
                        }
                        j += 1;
                    }
                    j += 1;
                    break;
                }
                _ => {}
            }
            j += 1;
        }
        i = j;
    }
    Ok(out)
}

/// (file, statement after the token) for every production receiver_proof token, following modules from main.rs.
fn c8_receiver_proof_shapes(read: &dyn Fn(&str) -> Result<String, String>) -> Result<Vec<(String, String)>, String> {
    // Production #[path] modules present today: (parent file, module, file). Anything else fails closed.
    const PATHED: &[(&str, &str, &str)] = &[("pending_requests.rs", "attempts", "pending_attempts.rs")];
    let (mut queue, mut seen, mut shapes) = (vec!["main.rs".to_string()], std::collections::BTreeSet::new(), Vec::new());
    while let Some(file) = queue.pop() {
        if !seen.insert(file.clone()) { continue; }
        let source = read(&file)?;
        let all = c8_sampler_tokens(&source).map_err(|e| format!("{file}: {e}"))?;
        let tokens = c8_cut_test_items(&all).map_err(|e| format!("{file}: {e}"))?;
        for (k, t) in tokens.iter().enumerate() {
            if *t == "include" && tokens.get(k + 1) == Some(&"!") {
                return Err(format!("{file}: a production include! is not scanned"));
            }
            if *t == "mod" && tokens.get(k + 2) == Some(&";") {
                let name = tokens[k + 1];
                // Walk back over the visibility (`pub`, or `pub(..)` with any path) and over EVERY outer attribute
                // of this item: #[path] may stand before #[cfg(..)] or other attributes (reflection R1-1).
                let mut b = k;
                if b > 0 && tokens[b - 1] == ")" {
                    let mut a = b - 1;
                    while a > 0 && tokens[a] != "(" { a -= 1; }
                    if a > 0 && tokens[a - 1] == "pub" { b = a - 1; }
                } else if b > 0 && tokens[b - 1] == "pub" { b -= 1; }
                let mut pathed = false;
                while b > 0 && tokens[b - 1] == "]" {
                    let (mut a, mut depth) = (b, 0usize);
                    while a > 0 {
                        a -= 1;
                        match tokens[a] { "]" => depth += 1, "[" => depth -= 1, _ => {} }
                        if depth == 0 { break; }
                    }
                    if depth != 0 || a == 0 || tokens[a - 1] != "#" { break; }
                    pathed |= tokens[a - 1..b] == ["#", "[", "path", "=", "]"];
                    b = a - 1;
                }
                if pathed {
                    match PATHED.iter().find(|(parent, m, _)| *parent == file && *m == name) {
                        Some((_, _, path)) => queue.push((*path).to_string()),
                        None => return Err(format!("{file}: production #[path] module {name} is not in the scan table")),
                    }
                } else if file == "main.rs" {
                    queue.push(format!("{name}.rs"));
                } else {
                    return Err(format!("{file}: nested file module {name} is not scanned"));
                }
            }
            if *t == "receiver_proof" {
                let (mut depth, mut window) = (0isize, Vec::new());
                for w in tokens[k + 1..].iter().take(60) {
                    match *w {
                        "(" | "[" => depth += 1,
                        ")" | "]" => depth -= 1,
                        ";" | "," | "{" | "}" if depth == 0 => break,
                        _ => {}
                    }
                    window.push(*w);
                }
                shapes.push((file.clone(), window.join(" ")));
            }
        }
    }
    shapes.sort();
    Ok(shapes)
}

#[test]
fn c8_dormancy_receiver_proof_has_only_the_known_production_shapes() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bin/cysd");
    let read = |f: &str| std::fs::read_to_string(root.join(f)).map_err(|e| format!("{f}: {e}"));
    let actual = c8_receiver_proof_shapes(&read).expect("receiver_proof lexical pin could not scan the production modules");
    let mut expected: Vec<(String, String)> =
        C8_RECEIVER_PROOF_SHAPES.iter().map(|(f, s)| (f.to_string(), s.to_string())).collect();
    expected.sort();
    assert_eq!(actual, expected, "C8 dormancy: receiver_proof production shapes changed - a writer or a new read was added \
        outside test code (re-run scripts/c8_dormancy_census.py and review before changing this list)");
}

#[test]
fn c8_dormancy_receiver_proof_pin_sees_added_writers_and_fails_closed() {
    fn run(main: &'static str, a: &'static str) -> Result<Vec<String>, String> {
        let read = move |f: &str| match f {
            "main.rs" => Ok(main.to_string()),
            "a.rs" => Ok(a.to_string()),
            other => Err(format!("missing {other}")),
        };
        c8_receiver_proof_shapes(&read).map(|v| v.into_iter().map(|(_, s)| s).collect())
    }
    // Test-only code is cut; an attribute on a struct-literal field does not swallow the field after it.
    assert_eq!(run("mod a;", "#[cfg(test)] mod t { fn g(s: &S) { *s.receiver_proof.lock().unwrap() = None; } }"),
        Ok(Vec::new()));
    assert_eq!(run("mod a;", "fn f() -> S { S { #[cfg(test)] x: 1, receiver_proof: Mutex::new(None), } }"),
        Ok(vec![": Mutex : : new ( None )".to_string()]));
    // Added writers are seen, including a cfg(windows) block and a mutating call on an existing read.
    for writer in ["fn f(s: &S) { *s.receiver_proof.lock().unwrap() = Some(p); }",
                   "fn f(s: &S) { #[cfg(windows)] { *s.receiver_proof.lock().unwrap() = Some(p); } }",
                   "fn f(s: &S) { s.receiver_proof.lock().unwrap().replace(p); }"] {
        assert_eq!(run("mod a;", writer).map(|v| v.len()), Ok(1), "writer not seen: {writer}");
    }
    // Routes the scan cannot follow fail closed.
    assert!(run("mod a;", "include!(\"x.rs\");").is_err(), "production include! accepted");
    assert!(run("mod a;", "mod q;").is_err(), "nested file module accepted");
    assert!(run("#[path = \"z.rs\"] mod a;", "").is_err(), "unknown #[path] module accepted");
    // #[path] is found among all of the item's attributes and behind any visibility (R-C8DORM-1 reflection R1-1).
    assert_eq!([run("#[path = \"z.rs\"] #[cfg(unix)] mod a;", "").is_err(), run("#[path = \"z.rs\"] pub(in crate::q) mod a;", "").is_err()],
        [true, true], "unknown #[path] module accepted when another attribute or pub(in ..) stands between it and mod");
}
