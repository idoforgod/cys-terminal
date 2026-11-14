//! Track T4 — tests of the claim sequence, the cap, the pause and the decision file (C9 · RULINGS
//! B1, B4, F1, F7(1), G1–G3, G6, G7, K3, K5, K8, L2, M3, O5). Declared test-only from `grant.rs`.
//! Every test builds its own repairs root under the temporary directory; the clock and the local
//! date are injected; file ages are set with `File::set_modified`.

use super::*;
use crate::repair::ledger::{self, ChangeKind, FileChange, PreImage, RepairRecord};
use crate::repair::types::{Classification, Lease, TargetState, Trigger};
use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);
const D: &str = "2026-11-13";
const D2: &str = "2026-11-14";

fn root(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!("cys-t4-grant-{}-{tag}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn owner(lane: &str, seat: u64, inc: &str) -> LockOwner {
    LockOwner {
        lane: LaneKey(lane.into()),
        seat,
        incarnation: inc.into(),
    }
}

fn rid(n: u64) -> RepairId {
    RepairId::parse(&format!("R-{n}")).unwrap()
}

/// Wall time of the test run (file modification times are real, the clock is injected around it).
fn t0() -> f64 {
    ledger::now_epoch()
}

fn rec(id: &RepairId, state: RepairState, lock: Option<Lease>) -> RepairRecord {
    RepairRecord {
        schema: crate::repair::REPAIR_SCHEMA,
        id: id.clone(),
        created_at: 1.0,
        trigger: Trigger::Grant,
        symptom: String::new(),
        cause: String::new(),
        classification: Classification::General,
        files: vec![FileChange {
            rel: "bin/a.py".into(),
            kind: ChangeKind::Edit,
            pre: PreImage::Absent,
            post: TargetState::Absent,
            diff: None,
        }],
        worker_record: None,
        verify: None,
        audit: None,
        audit_due_at: None,
        approval: None,
        owner_token_sha256: None,
        attempt: 1,
        methods: vec![],
        resubmits: vec![],
        pending_request: None,
        lock,
        state,
        intake_signatures: vec![],
        owner_original: false,
        origin_lane: LaneKey("base".into()),
        resolves: None,
        committed: vec![],
        fixer_seats: vec![],
        checks: vec![],
    }
}

fn put(root: &Path, r: &RepairRecord) {
    std::fs::create_dir_all(ledger::repair_dir_in(root, &r.id)).unwrap();
    ledger::write_record_in(root, r).unwrap();
}

fn get(root: &Path, id: &RepairId) -> RepairRecord {
    ledger::read_record_in(root, id).unwrap().unwrap()
}

fn grant_on(root: &Path, cap: u32) {
    write_decision_in(
        root,
        &GrantDecision {
            schema: 1,
            state: GrantState::On,
            daily_cap: cap,
            decided_at: 1.0,
            decided_lane: LaneKey("base".into()),
            decided_seat: 1,
            prompt_digest: "d".into(),
            prompt_excerpt: "예".into(),
            paused_at: None,
            resumed_at: None,
            generation: 0,
        },
    )
    .unwrap();
}

fn continue_all(_: ClaimStep) -> HookAction {
    HookAction::Continue
}

/// One attempt with a fixed clock and date.
fn attempt(root: &Path, now: f64, day: &str, o: &LockOwner, id: &RepairId) -> Result<StartVerdict, RepairError> {
    let clock = move || now;
    let today = move || day.to_string();
    try_claim_in(
        &ClaimEnv {
            root,
            clock: &clock,
            today: &today,
            hook: &continue_all,
        },
        o,
        id,
    )
}

/// T3's new-repair path: the id is allocated, the claim is asked from inside the builder (no record
/// yet), and on `Allowed` the record is written already locked with that claim
/// (`ledger::create_locked_in`); otherwise the empty id directory is removed.
fn start_new(root: &Path, now: f64, day: &str, o: &LockOwner) -> (RepairId, Result<StartVerdict, RepairError>) {
    let id = ledger::allocate_id_in(root).unwrap();
    let v = attempt(root, now, day, o, &id);
    match v {
        Ok(StartVerdict::Allowed { claim }) => {
            put(root, &rec(&id, RepairState::InProgress, Some(lock::new_lease(o.clone(), now, Some(claim)))));
        }
        _ => {
            let _ = std::fs::remove_dir(ledger::repair_dir_in(root, &id));
        }
    }
    (id, v)
}

/// The repair is committed (or reverted): a closed state, no lock.
fn finish(root: &Path, id: &RepairId) {
    let mut r = get(root, id);
    r.state = RepairState::Probation;
    r.lock = None;
    ledger::write_record_in(root, &r).unwrap();
}

fn claims(root: &Path) -> Vec<(u32, GrantClaim)> {
    claim_numbers(root)
        .into_iter()
        .map(|n| {
            let b = std::fs::read(claim_path_in(root, u64::from(n))).unwrap();
            (n, serde_json::from_slice(&b).unwrap())
        })
        .collect()
}

fn closed(root: &Path, n: u32) -> bool {
    closed_path_in(root, u64::from(n)).exists()
}

/// Sets a file's modification time to `epoch` seconds.
fn age_file(p: &Path, epoch: f64) {
    let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs_f64(epoch.max(0.0));
    std::fs::File::options()
        .write(true)
        .open(p)
        .unwrap()
        .set_modified(t)
        .unwrap();
}

fn allowed(v: &Result<StartVerdict, RepairError>) -> bool {
    matches!(v, Ok(StartVerdict::Allowed { .. }))
}

// ───────────────────────────── layout, decision file ─────────────────────────────

/// The `_in` layouts are the T0 path helpers relative to the repairs root.
#[test]
fn layout_matches_paths_rs() {
    let _lock = crate::pack::PACK_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let local = root("layout");
    let _env = crate::pack::EnvGuard::set("CYS_LOCAL_DIR", &local);
    let r = crate::repair::paths::repairs_root();
    assert_eq!(grant_path_in(&r), crate::repair::paths::grant_path());
    assert_eq!(seq_dir_in(&r), crate::repair::paths::grant_seq_dir());
    assert_eq!(claim_path_in(&r, 7), crate::repair::paths::grant_claim_path(7));
    assert_eq!(closed_path_in(&r, 7), crate::repair::paths::grant_closed_path(7));
    // the `.closed` layout T3's holder side checks (`lock::claim_closed`) is the same file
    std::fs::create_dir_all(seq_dir_in(&r)).unwrap();
    std::fs::write(closed_path_in(&r, 7), b"").unwrap();
    assert!(lock::claim_closed(&r, Some(7)));
}

/// B1: the decision file round-trips; absent = no decision; garbage or a higher schema = unreadable.
#[test]
fn decision_file_read_write() {
    let r = root("decision");
    assert_eq!(read_decision_in(&r).unwrap(), None);
    grant_on(&r, 4);
    let d = read_decision_in(&r).unwrap().unwrap();
    assert_eq!((d.state, d.daily_cap), (GrantState::On, 4));
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(grant_path_in(&r)).unwrap()).unwrap();
    let mut keys: Vec<&String> = v.as_object().unwrap().keys().collect();
    keys.sort();
    assert_eq!(
        keys,
        // RULINGS CV1 (T4b): + `generation` (OLD expectation: the same list without it)
        [
            "daily_cap", "decided_at", "decided_lane", "decided_seat", "generation", "paused_at",
            "prompt_digest", "prompt_excerpt", "resumed_at", "schema", "state"
        ]
    );
    std::fs::write(grant_path_in(&r), b"{not json").unwrap();
    assert!(matches!(read_decision_in(&r), Err(RepairError::Corrupt(_))));
    let mut hi = v.clone();
    hi["schema"] = serde_json::json!(99);
    std::fs::write(grant_path_in(&r), serde_json::to_vec(&hi).unwrap()).unwrap();
    assert!(matches!(read_decision_in(&r), Err(RepairError::Corrupt(_))));
}

// ───────────────────────────── D:1105 (7) ─────────────────────────────

/// Off → 0 starts: no decision, `off`, or an unreadable decision → refused with the off sentence,
/// and nothing is placed (no claim, no marker, no record) — also with open defects in the ledger
/// (a boot without a mission).
#[test]
fn off_means_zero_starts() {
    let r = root("off");
    // ledger defects present (an intake row) — not a reason to start (9.5)
    std::fs::create_dir_all(r.join("intake").join("base")).unwrap();
    std::fs::write(r.join("intake/base/abc.json"), b"{}").unwrap();
    let o = owner("base", 3, "inc-1");
    let check = |r: &Path| {
        let (_, v) = start_new(r, t0(), D, &o);
        match v {
            Err(RepairError::Precondition(m)) => assert_eq!(m, GRANT_OFF_KO),
            other => panic!("expected the off refusal, got {other:?}"),
        }
        assert!(!seq_dir_in(r).exists(), "a claim was placed with the grant off");
        assert!(ledger::list_ids_in(r).is_empty());
    };
    check(&r);
    grant_on(&r, 5);
    let mut d = read_decision_in(&r).unwrap().unwrap();
    d.state = GrantState::Off;
    write_decision_in(&r, &d).unwrap();
    check(&r);
    std::fs::write(grant_path_in(&r), b"garbage").unwrap();
    check(&r);
}

/// On → one concurrent: the first start is allowed with claim 1; a second (another seat, another
/// lane) is `Busy` and places nothing; after the first is committed the next start closes claim 1
/// and takes claim 2.
#[test]
fn on_one_concurrent() {
    let r = root("one");
    grant_on(&r, 5);
    let (a, v) = start_new(&r, t0(), D, &owner("base", 3, "inc-1"));
    assert_eq!(v.unwrap(), StartVerdict::Allowed { claim: 1 });
    assert_eq!(get(&r, &a).lock.unwrap().claim, Some(1));
    let (_, v) = start_new(&r, t0(), D, &owner("dept-2", 4, "inc-9"));
    assert_eq!(v.unwrap(), StartVerdict::Busy);
    assert_eq!(claims(&r).len(), 1);
    assert!(!closed(&r, 1));
    finish(&r, &a);
    let (b, v) = start_new(&r, t0(), D, &owner("dept-2", 4, "inc-9"));
    assert_eq!(v.unwrap(), StartVerdict::Allowed { claim: 2 });
    assert!(closed(&r, 1), "the evaluation closes the finished repair's claim");
    let c = claims(&r);
    assert_eq!(c[1].1.repair, b);
    assert_eq!(c[1].1.lane_key, LaneKey("dept-2".into()));
}

/// Stops at 5 a day: five distinct repairs start (each finished before the next), the sixth is
/// `CapReached`; on the next local day the count starts again; a smaller user cap is honoured;
/// never more than five claims carry the same `day`.
#[test]
fn daily_cap_of_distinct_repairs() {
    let r = root("cap");
    grant_on(&r, 5);
    let o = owner("base", 3, "inc-1");
    for _ in 0..5 {
        let (id, v) = start_new(&r, t0(), D, &o);
        assert!(allowed(&v), "{v:?}");
        finish(&r, &id);
    }
    for _ in 0..3 {
        let (_, v) = start_new(&r, t0(), D, &o);
        assert_eq!(v.unwrap(), StartVerdict::CapReached);
    }
    assert_eq!(claims(&r).iter().filter(|(_, c)| c.day == D).count(), 5);
    let (id, v) = start_new(&r, t0(), D2, &o);
    assert!(allowed(&v), "a new day resets the count: {v:?}");
    finish(&r, &id);
    let r2 = root("cap2");
    grant_on(&r2, 2);
    for _ in 0..2 {
        let (id, v) = start_new(&r2, t0(), D, &o);
        assert!(allowed(&v));
        finish(&r2, &id);
    }
    assert_eq!(start_new(&r2, t0(), D, &o).1.unwrap(), StartVerdict::CapReached);
}

/// K3: the cap counts DISTINCT repairs — the pick-up of the same repair after its claim was closed
/// takes one more claim the same day and is counted once (also when the cap is already reached).
#[test]
fn pick_up_after_closure_counted_once() {
    let r = root("k3");
    grant_on(&r, 1);
    let old = owner("base", 3, "inc-1");
    let (id, v) = start_new(&r, t0(), D, &old);
    assert_eq!(v.unwrap(), StartVerdict::Allowed { claim: 1 });
    // the daemon restarted: the lock of incarnation inc-1 is released (C1 1.12); a new worker
    // picks the being-worked repair up
    let new = owner("base", 5, "inc-2");
    let v = attempt(&r, t0(), D, &new, &id).unwrap();
    assert_eq!(v, StartVerdict::Allowed { claim: 2 });
    assert!(closed(&r, 1));
    assert_eq!(get(&r, &id).lock.unwrap().claim, Some(2), "lock first: the record carries claim 2");
    assert_eq!(claims(&r).len(), 2);
    // another repair is over the cap of 1 distinct repair
    finish(&r, &id);
    assert_eq!(start_new(&r, t0(), D, &new).1.unwrap(), StartVerdict::CapReached);
}

/// B4: any owner input pauses at once (`Paused`, no new round); the pause ends by itself after 30
/// minutes without owner input; nothing is written while the grant is off; a clock set back keeps
/// the pause; `grant on` with the answer's time pauses too (the answer is an owner input).
#[test]
fn owner_input_pauses_and_resumes() {
    let r = root("pause");
    let lane = LaneKey("dept-4".into());
    let t = t0();
    assert!(!note_owner_input_in(&r, &lane, t).unwrap(), "off: nothing to pause");
    assert!(!pause_path_in(&r, &lane).exists());
    grant_on(&r, 5);
    assert!(may_open_new_round_in(&r, t).unwrap());
    assert!(note_owner_input_in(&r, &lane, t).unwrap());
    let o = owner("base", 3, "inc-1");
    assert_eq!(start_new(&r, t + 10.0, D, &o).1.unwrap(), StartVerdict::Paused);
    assert!(!may_open_new_round_in(&r, t + 10.0).unwrap());
    assert!(!seq_dir_in(&r).exists() || claims(&r).is_empty());
    assert!(is_paused_in(&r, t - 3_600.0), "a clock set back keeps the pause");
    assert!(is_paused_in(&r, t + 1_799.0));
    // RULINGS CV3 (T4b rewrite): a written mark covers the coalescing window — its pause ends at
    // input + 30 min + PAUSE_WRITE_COALESCE_SECS (never earlier than 30 minutes after an input it
    // may have coalesced). OLD expectations: `assert!(!is_paused_in(&r, t + 1_800.5));`
    // `assert!(may_open_new_round_in(&r, t + 1_801.0).unwrap());` `… start_new(&r, t + 1_801.0 …)`.
    assert!(is_paused_in(&r, t + 1_800.0 + PAUSE_WRITE_COALESCE_SECS - 0.5));
    assert!(!is_paused_in(&r, t + 1_800.0 + PAUSE_WRITE_COALESCE_SECS + 0.5));
    assert!(may_open_new_round_in(&r, t + 1_806.0).unwrap());
    assert!(allowed(&start_new(&r, t + 1_806.0, D, &o).1));
    // an unreadable pause file holds the pause by its modification time
    std::fs::write(pause_path_in(&r, &LaneKey("dept-9".into())), b"x").unwrap();
    assert!(is_paused_in(&r, t0() + 5.0));
    // the decision's own pause (the answer that turned it on)
    let r2 = root("pause2");
    grant_on(&r2, 5);
    let mut d = read_decision_in(&r2).unwrap().unwrap();
    d.paused_at = Some(t);
    d.resumed_at = Some(t + 1_800.0);
    write_decision_in(&r2, &d).unwrap();
    assert!(is_paused_in(&r2, t + 60.0));
    assert_eq!(new_round_refusal_in(&r2), NEW_ROUND_PAUSED_KO);
    // off: no new round either, and the refusal says the grant is off (CH7)
    let mut d = read_decision_in(&r2).unwrap().unwrap();
    d.state = GrantState::Off;
    d.paused_at = None;
    write_decision_in(&r2, &d).unwrap();
    assert!(!may_open_new_round_in(&r2, t + 60.0).unwrap());
    assert_eq!(new_round_refusal_in(&r2), NEW_ROUND_OFF_KO);
    assert!(NEW_ROUND_OFF_KO.contains("자동 수리 허락이 꺼져 있습니다") && !NEW_ROUND_OFF_KO.contains("일시정지"));
}

// ───────────────────────────── claim files (F1 / F7(1)) ─────────────────────────────

/// A claim is `{schema, repair, lane_key, day, claimed_at}`, placed complete; a placement on a
/// taken number fails and leaves the existing file byte-identical; temporary names are never
/// counted; no production code of this module calls `std::fs::rename` or deletes a claim / marker.
#[test]
fn claim_files_complete_immutable_never_deleted() {
    let r = root("files");
    grant_on(&r, 5);
    let (id, v) = start_new(&r, t0(), D, &owner("base", 3, "inc-1"));
    assert_eq!(v.unwrap(), StartVerdict::Allowed { claim: 1 });
    let raw = std::fs::read(claim_path_in(&r, 1)).unwrap();
    let v: serde_json::Value = serde_json::from_slice(&raw).unwrap();
    let mut keys: Vec<&String> = v.as_object().unwrap().keys().collect();
    keys.sort();
    assert_eq!(keys, ["claimed_at", "day", "lane_key", "repair", "schema"]);
    assert_eq!(v["repair"], id.as_str());
    assert_eq!(v["day"], D);
    let other = GrantClaim {
        schema: 1,
        repair: rid(99),
        lane_key: LaneKey("x".into()),
        day: D.into(),
        claimed_at: 0.0,
    };
    assert!(!place_claim(&r, 1, &other).unwrap(), "a taken number is a collision");
    assert_eq!(std::fs::read(claim_path_in(&r, 1)).unwrap(), raw, "never replaced");
    let names: Vec<String> = std::fs::read_dir(seq_dir_in(&r))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(names, ["1.json"], "the attempt's temporary file was left behind");
    // temporary / foreign names never count
    for n in [".tmp-claim-9-1-2", "9.json.tmp", "x.json", "-3.json", ".9.json"] {
        std::fs::write(seq_dir_in(&r).join(n), b"{}").unwrap();
    }
    assert_eq!(claim_numbers(&r), [1]);
    let src = include_str!("grant.rs");
    let prod = &src[..src.find("#[cfg(test)]").unwrap()];
    for needle in ["fs::rename(", "remove_dir_all", "fs::remove_dir("] {
        assert!(!prod.contains(needle), "grant.rs production code uses {needle}");
    }
    // the only removal in production code is of the attempt's own temporary file (`tmp`, or `src`
    // inside the placement helper — the same file)
    assert_eq!(
        prod.matches("remove_file(").count(),
        prod.matches("remove_file(&tmp)").count() + prod.matches("remove_file(src)").count()
    );
}

// ───────────────────────────── G7 — closer vs holder ─────────────────────────────

/// G7: every interleaving of the holder's {renew-write H1, marker check H2} with the closer's {fresh
/// read, marker create, second read} (H1 ≤ H2 over four slots — before the fresh read, before the
/// marker, before the second read, after the attempt): the holder (suspended past its `now <
/// expires_at` test, so its write lands late) and the closer never BOTH proceed.
#[test]
fn g7_every_interleaving_at_most_one_proceeds() {
    let mut both_back_off = 0;
    for h1 in 0..4usize {
        for h2 in h1..4usize {
            let r = root(&format!("g7-{h1}-{h2}"));
            grant_on(&r, 5);
            let t = t0();
            let holder = owner("base", 1, "inc-1");
            let held = rid(1);
            put(
                &r,
                &rec(
                    &held,
                    RepairState::InProgress,
                    Some(Lease {
                        owner: holder.clone(),
                        expires_at: t - 1_000.0,
                        claim: Some(1),
                    }),
                ),
            );
            place_claim(
                &r,
                1,
                &GrantClaim {
                    schema: 1,
                    repair: held.clone(),
                    lane_key: holder.lane.clone(),
                    day: D.into(),
                    claimed_at: t - 8_000.0,
                },
            )
            .unwrap();
            let done = RefCell::new(Vec::<u8>::new());
            let proceeds = Cell::new(None::<bool>);
            let run_holder = |slot: usize| {
                for (op, at) in [(1u8, h1), (2u8, h2)] {
                    if at == slot && !done.borrow().contains(&op) {
                        done.borrow_mut().push(op);
                        if op == 1 {
                            let mut x = get(&r, &held);
                            x.lock = Some(Lease {
                                owner: holder.clone(),
                                expires_at: t + 7_200.0,
                                claim: Some(1),
                            });
                            ledger::write_record_in(&r, &x).unwrap();
                        } else {
                            proceeds.set(Some(!lock::claim_closed(&r, Some(1))));
                        }
                    }
                }
            };
            let hook = |s: ClaimStep| {
                match s {
                    ClaimStep::BeforeFreshRead(1) => run_holder(0),
                    ClaimStep::BeforeMarker(1) => run_holder(1),
                    ClaimStep::BeforeSecondRead(1) => run_holder(2),
                    _ => {}
                }
                HookAction::Continue
            };
            let clock = move || t;
            let today = || D.to_string();
            let closer = owner("base", 2, "inc-1");
            let sid = ledger::allocate_id_in(&r).unwrap();
            let v = try_claim_in(
                &ClaimEnv {
                    root: &r,
                    clock: &clock,
                    today: &today,
                    hook: &hook,
                },
                &closer,
                &sid,
            )
            .unwrap();
            // whatever did not run inside the attempt runs after it, in order
            for slot in 0..4 {
                run_holder(slot);
            }
            let closer_started = matches!(v, StartVerdict::Allowed { .. });
            let holder_proceeds = proceeds.get().unwrap();
            assert!(
                !(closer_started && holder_proceeds),
                "H1 at {h1}, H2 at {h2}: the closer started ({v:?}) AND the holder proceeded"
            );
            if !closer_started && !holder_proceeds {
                both_back_off += 1;
            }
            if closer_started {
                assert!(closed(&r, 1));
                assert_eq!(claims(&r).len(), 2);
            }
        }
    }
    // the documented "both back off" outcome exists (holder renewed after the marker, before the
    // second read): the repair stays open for a new claim
    assert!(both_back_off >= 1);
}

/// G7: a closer that dies between its marker and its second read never starts; the holder's next
/// verb meets the marker (refused); the next attempt starts and no two starts are live.
#[test]
fn closer_dies_between_marker_and_second_read() {
    let r = root("g7-crash");
    grant_on(&r, 5);
    let t = t0();
    let holder = owner("base", 1, "inc-1");
    let held = rid(1);
    put(&r, &rec(&held, RepairState::InProgress, Some(Lease { owner: holder.clone(), expires_at: t - 1_000.0, claim: Some(1) })));
    place_claim(&r, 1, &GrantClaim { schema: 1, repair: held.clone(), lane_key: holder.lane.clone(), day: D.into(), claimed_at: t }).unwrap();
    let clock = move || t;
    let today = || D.to_string();
    let die = |s: ClaimStep| if s == ClaimStep::BeforeSecondRead(1) { HookAction::Crash } else { HookAction::Continue };
    let sid = ledger::allocate_id_in(&r).unwrap();
    let e = try_claim_in(&ClaimEnv { root: &r, clock: &clock, today: &today, hook: &die }, &owner("base", 2, "inc-1"), &sid);
    assert!(matches!(e, Err(RepairError::Io(_))), "{e:?}");
    assert!(closed(&r, 1));
    assert_eq!(claims(&r).len(), 1, "the dead closer placed nothing");
    assert!(lock::claim_closed(&r, Some(1)), "the holder's next verb is refused (G7(c))");
    let (_, v) = start_new(&r, t, D, &owner("dept-2", 4, "inc-7"));
    assert_eq!(v.unwrap(), StartVerdict::Allowed { claim: 2 });
}

/// C1 1.12 / G7: the holder's daemon restarts between its renewal and its check — the renewed
/// lock belongs to a dead incarnation, so the restarted lane's closer closes the claim; the old
/// holder's check meets the marker. Another lane judges the same lock by its lease (still live).
#[test]
fn holder_daemon_restart_between_renewal_and_check() {
    let r = root("restart");
    grant_on(&r, 5);
    let t = t0();
    let holder = owner("base", 1, "inc-1");
    let held = rid(1);
    put(&r, &rec(&held, RepairState::InProgress, Some(lock::new_lease(holder.clone(), t, Some(1)))));
    place_claim(&r, 1, &GrantClaim { schema: 1, repair: held.clone(), lane_key: holder.lane.clone(), day: D.into(), claimed_at: t }).unwrap();
    // another lane: the lease is live → Busy
    assert_eq!(start_new(&r, t, D, &owner("dept-2", 4, "inc-7")).1.unwrap(), StartVerdict::Busy);
    assert!(!closed(&r, 1));
    // the holder's own lane after a restart: released → closable
    let (_, v) = start_new(&r, t, D, &owner("base", 2, "inc-2"));
    assert_eq!(v.unwrap(), StartVerdict::Allowed { claim: 2 });
    assert!(lock::claim_closed(&r, Some(1)), "the old holder's check refuses");
}

/// G7(a): machine sleep across the expiry — a lock counts as expired for others only on a read
/// later than `expires_at` + 60 s; forward and backward clock steps move only that judgement.
#[test]
fn expiry_needs_the_grace_and_a_fresh_read() {
    let r = root("sleep");
    grant_on(&r, 5);
    let t = t0();
    let holder = owner("dept-1", 1, "inc-1");
    let held = rid(1);
    let exp = t + 100.0;
    put(&r, &rec(&held, RepairState::InProgress, Some(Lease { owner: holder.clone(), expires_at: exp, claim: Some(1) })));
    place_claim(&r, 1, &GrantClaim { schema: 1, repair: held.clone(), lane_key: holder.lane.clone(), day: D.into(), claimed_at: t }).unwrap();
    let o = owner("base", 2, "inc-1");
    for now in [t, exp - 1.0, exp + 30.0, exp + 60.0] {
        assert_eq!(start_new(&r, now, D, &o).1.unwrap(), StartVerdict::Busy, "at {}", now - exp);
        assert!(!closed(&r, 1));
    }
    // a backward step after a forward one changes nothing already decided: still held at exp + 59
    assert_eq!(start_new(&r, exp + 59.0, D, &o).1.unwrap(), StartVerdict::Busy);
    // the machine wakes long after the expiry
    assert!(allowed(&start_new(&r, exp + 61.0, D, &o).1));
    assert!(closed(&r, 1));
}

/// Clock steps and the cap: claims dated a LATER day (the clock was set back) do not count today;
/// the counted day is the claim's immutable `day`, never the current one.
#[test]
fn cap_reads_the_immutable_day() {
    let r = root("clockcap");
    grant_on(&r, 1);
    let o = owner("base", 3, "inc-1");
    let (id, v) = start_new(&r, t0(), D2, &o);
    assert!(allowed(&v));
    finish(&r, &id);
    // clock set back one day: D2's claim is not today's
    let (id, v) = start_new(&r, t0(), D, &o);
    assert!(allowed(&v), "{v:?}");
    finish(&r, &id);
    assert_eq!(start_new(&r, t0(), D, &o).1.unwrap(), StartVerdict::CapReached);
    assert_eq!(start_new(&r, t0(), D2, &o).1.unwrap(), StartVerdict::CapReached);
}

/// A crash at each step of a pick-up attempt (the daemon dies; it comes back with a new
/// incarnation): nothing double-starts, at most one claim is placed by the crashed attempt, and the
/// next attempt is either allowed or `Busy` only until the held files are older than lease + 60 s.
#[test]
fn crash_at_each_step() {
    for at in [
        ClaimStep::Snapshot,
        ClaimStep::AfterEvaluate,
        ClaimStep::AfterLock,
        ClaimStep::BeforePlace,
        ClaimStep::AfterPlace,
    ] {
        let r = root("crash");
        grant_on(&r, 5);
        let t = t0();
        let id = rid(1);
        put(&r, &rec(&id, RepairState::InProgress, None)); // a lock-less repair to pick up
        let clock = move || t;
        let today = || D.to_string();
        let die = move |s: ClaimStep| if s == at { HookAction::Crash } else { HookAction::Continue };
        let crashed_owner = owner("base", 1, "inc-1");
        let e = try_claim_in(&ClaimEnv { root: &r, clock: &clock, today: &today, hook: &die }, &crashed_owner, &id);
        assert!(matches!(e, Err(RepairError::Io(_))), "{at:?}: {e:?}");
        let placed = claims(&r).len();
        assert!(placed <= 1, "{at:?}");
        // the daemon restarted (inc-2): its old lock is released; another seat picks up
        let v = attempt(&r, t, D, &owner("base", 2, "inc-2"), &id).unwrap();
        assert!(matches!(v, StartVerdict::Allowed { .. }), "{at:?}: {v:?}");
        let live: Vec<_> = claims(&r)
            .into_iter()
            .filter(|(n, _)| !closed(&r, *n))
            .collect();
        assert_eq!(live.len(), 1, "{at:?}: exactly one unclosed claim");
        // a NEW repair whose claim was placed but whose record never got written (crash inside
        // the builder) holds the sequence until the claim is older than lease + 60 s, then frees it
        let r2 = root("crash-new");
        grant_on(&r2, 5);
        let ghost = ledger::allocate_id_in(&r2).unwrap();
        let v = attempt(&r2, t, D, &owner("base", 1, "inc-1"), &ghost).unwrap();
        assert_eq!(v, StartVerdict::Allowed { claim: 1 });
        // (no record written — the process died)
        let o2 = owner("dept-3", 9, "inc-5");
        assert_eq!(start_new(&r2, t, D, &o2).1.unwrap(), StartVerdict::Busy);
        age_file(&claim_path_in(&r2, 1), t - 7_400.0);
        assert!(allowed(&start_new(&r2, t, D, &o2).1), "no permanent Busy");
    }
}

/// An expiring lease re-claimed by its OLD holder: the expired lock bound to claim 1 is closed by
/// the holder's own new attempt, which takes claim 2 for the same repair (counted once).
#[test]
fn expiring_lease_reclaimed_by_old_holder() {
    let r = root("reclaim");
    grant_on(&r, 1);
    let t = t0();
    let holder = owner("base", 1, "inc-1");
    let id = rid(1);
    put(&r, &rec(&id, RepairState::InProgress, Some(Lease { owner: holder.clone(), expires_at: t - 100.0, claim: Some(1) })));
    place_claim(&r, 1, &GrantClaim { schema: 1, repair: id.clone(), lane_key: holder.lane.clone(), day: D.into(), claimed_at: t - 7_400.0 }).unwrap();
    let v = attempt(&r, t, D, &holder, &id).unwrap();
    assert_eq!(v, StartVerdict::Allowed { claim: 2 });
    assert!(closed(&r, 1));
    let l = get(&r, &id).lock.unwrap();
    assert_eq!((l.claim, l.owner), (Some(2), holder));
}

// ───────────────────────────── K5 / L2 / K8 / O5 — unreadable parts ─────────────────────────────

/// K5 + L2: a corrupt record bound to a claim keeps the start `Busy` until BOTH the claim file and
/// the record file are older than lease + 60 s (age on the newer — a 3-hour repair that keeps
/// renewing stays `Busy` through a transient unreadable record); a higher `schema` is unreadable.
#[test]
fn corrupt_record_held_until_both_files_old() {
    for corruption in [&b"{broken"[..], br#"{"schema": 99, "id": "R-1"}"#] {
        let r = root("k5rec");
        grant_on(&r, 5);
        let t = t0();
        let id = rid(1);
        put(&r, &rec(&id, RepairState::InProgress, None));
        place_claim(&r, 1, &GrantClaim { schema: 1, repair: id.clone(), lane_key: LaneKey("base".into()), day: D.into(), claimed_at: t }).unwrap();
        let recf = ledger::record_path_in(&r, &id);
        std::fs::write(&recf, corruption).unwrap();
        let o = owner("dept-2", 4, "inc-1");
        assert_eq!(start_new(&r, t, D, &o).1.unwrap(), StartVerdict::Busy);
        // a 3-hour repair: the claim is old, the record young (renewed)
        age_file(&claim_path_in(&r, 1), t - 3.0 * 3_600.0);
        assert_eq!(start_new(&r, t, D, &o).1.unwrap(), StartVerdict::Busy);
        // the record old, the claim young
        age_file(&claim_path_in(&r, 1), t);
        age_file(&recf, t - 7_400.0);
        assert_eq!(start_new(&r, t, D, &o).1.unwrap(), StartVerdict::Busy);
        // both old → closable
        age_file(&claim_path_in(&r, 1), t - 7_400.0);
        assert!(allowed(&start_new(&r, t, D, &o).1));
        assert!(closed(&r, 1));
    }
}

/// K5 + K8: a corrupt claim keeps the start `Busy` while it is young (bound to a live lock); once
/// closable it still counts toward today's cap while younger than 24 h; then it is dropped.
#[test]
fn corrupt_claim_held_then_counted_then_dropped() {
    let r = root("k8");
    grant_on(&r, 1);
    let t = t0();
    std::fs::create_dir_all(seq_dir_in(&r)).unwrap();
    std::fs::write(claim_path_in(&r, 1), b"{half a cla").unwrap();
    let o = owner("base", 3, "inc-1");
    assert_eq!(start_new(&r, t, D, &o).1.unwrap(), StartVerdict::Busy);
    age_file(&claim_path_in(&r, 1), t - 7_400.0);
    assert_eq!(start_new(&r, t, D, &o).1.unwrap(), StartVerdict::CapReached, "counted for 24 h");
    assert!(closed(&r, 1));
    age_file(&claim_path_in(&r, 1), t - 86_401.0);
    assert!(allowed(&start_new(&r, t, D, &o).1), "dropped after 24 h");
}

/// O5: an unreadable claim's record set is every record whose lock names its number — held while
/// any of them is young; closable once the claim file and all of them are old.
#[test]
fn unreadable_claim_record_set_scan() {
    let r = root("o5");
    grant_on(&r, 5);
    let t = t0();
    let other = owner("dept-9", 9, "inc-9");
    for n in [1u64, 2] {
        put(&r, &rec(&rid(n), RepairState::InProgress, Some(Lease { owner: other.clone(), expires_at: t - 9_000.0, claim: Some(1) })));
    }
    std::fs::create_dir_all(seq_dir_in(&r)).unwrap();
    std::fs::write(claim_path_in(&r, 1), b"??").unwrap();
    age_file(&claim_path_in(&r, 1), t - 7_400.0);
    age_file(&ledger::record_path_in(&r, &rid(1)), t - 7_400.0);
    let o = owner("base", 3, "inc-1");
    assert_eq!(start_new(&r, t, D, &o).1.unwrap(), StartVerdict::Busy, "R-2 is young");
    age_file(&ledger::record_path_in(&r, &rid(2)), t - 7_400.0);
    assert!(allowed(&start_new(&r, t, D, &o).1), "both old → closable");
}

// ───────────────────────────── M3 / races ─────────────────────────────

/// M3: midnight passes between the evaluation and the placement → `LostRace`, nothing placed, the
/// lock just written cleared (the record stays open, unlocked); the retry is evaluated, counted and
/// dated on the new day.
#[test]
fn midnight_between_evaluation_and_placement() {
    let r = root("m3");
    grant_on(&r, 5);
    let t = t0();
    let id = rid(1);
    put(&r, &rec(&id, RepairState::InProgress, None));
    let calls = Cell::new(0);
    let today = || {
        calls.set(calls.get() + 1);
        if calls.get() == 1 { D.to_string() } else { D2.to_string() }
    };
    let clock = move || t;
    let o = owner("base", 3, "inc-1");
    let seen_lock = Cell::new(false);
    let hook = |s: ClaimStep| {
        if s == ClaimStep::AfterLock {
            seen_lock.set(get(&r, &id).lock.as_ref().is_some_and(|l| l.claim == Some(1)));
        }
        HookAction::Continue
    };
    let v = try_claim_in(&ClaimEnv { root: &r, clock: &clock, today: &today, hook: &hook }, &o, &id).unwrap();
    assert_eq!(v, StartVerdict::LostRace);
    assert!(seen_lock.get(), "lock first: the record carried the attempt's lock before placing");
    assert!(claims(&r).is_empty(), "nothing placed");
    let after = get(&r, &id);
    assert_eq!(after.lock, None, "the attempt's lock is cleared");
    assert_eq!(after.state, RepairState::InProgress, "the record stays open");
    let v = attempt(&r, t, D2, &o, &id).unwrap();
    assert_eq!(v, StartVerdict::Allowed { claim: 1 });
    assert_eq!(claims(&r)[0].1.day, D2);
}

/// G6: lock first never overwrites another owner's LIVE lock (`repair_locked`), and a `LostRace`
/// clears only the attempt's own lock.
#[test]
fn lock_first_respects_live_locks_and_clears_only_its_own() {
    let r = root("g6");
    grant_on(&r, 5);
    let t = t0();
    let id = rid(1);
    let other = owner("base", 7, "inc-1");
    put(&r, &rec(&id, RepairState::InProgress, Some(lock::new_lease(other.clone(), t, None))));
    let e = attempt(&r, t, D, &owner("base", 3, "inc-1"), &id);
    assert!(matches!(e, Err(RepairError::Locked(_))), "{e:?}");
    assert_eq!(get(&r, &id).lock.unwrap().owner, other);
    assert!(claims(&r).is_empty());
    // a collision: number 1 taken between the snapshot and the placement → LostRace, own lock cleared
    let r = root("g6b");
    grant_on(&r, 5);
    put(&r, &rec(&id, RepairState::InProgress, None));
    let me = owner("base", 3, "inc-1");
    let hook = |s: ClaimStep| {
        if s == ClaimStep::BeforePlace {
            place_claim(&r, 1, &GrantClaim { schema: 1, repair: rid(50), lane_key: LaneKey("dept-5".into()), day: D.into(), claimed_at: t }).unwrap();
        }
        HookAction::Continue
    };
    let clock = move || t;
    let today = || D.to_string();
    let v = try_claim_in(&ClaimEnv { root: &r, clock: &clock, today: &today, hook: &hook }, &me, &id).unwrap();
    assert_eq!(v, StartVerdict::LostRace);
    assert_eq!(get(&r, &id).lock, None);
    assert_eq!(claims(&r)[0].1.repair, rid(50), "the winner's claim is untouched");
}

/// Two- and three-attempt races on one root (one per lane daemon, all holding the same snapshot):
/// exactly one start per number, the others `LostRace`; with midnight in the middle the attempt
/// whose date changed loses as well; the cap and one-at-a-time are never exceeded.
#[test]
fn concurrent_attempts_race() {
    for (n, midnight) in [(2usize, false), (3, false), (3, true)] {
        for round in 0..5 {
            let r = root(&format!("race-{n}-{round}"));
            grant_on(&r, 5);
            let t = t0();
            let barrier = std::sync::Barrier::new(n);
            let results: Vec<Result<StartVerdict, RepairError>> = std::thread::scope(|s| {
                let hs: Vec<_> = (0..n)
                    .map(|i| {
                        let (r, barrier) = (&r, &barrier);
                        s.spawn(move || {
                            let id = ledger::allocate_id_in(r).unwrap();
                            let calls = Cell::new(0);
                            let today = || {
                                calls.set(calls.get() + 1);
                                if midnight && i == 0 && calls.get() > 1 { D2.to_string() } else { D.to_string() }
                            };
                            let clock = move || t;
                            let hook = |st: ClaimStep| {
                                if st == ClaimStep::Snapshot {
                                    barrier.wait();
                                }
                                HookAction::Continue
                            };
                            let o = owner(&format!("lane-{i}"), i as u64 + 1, "inc-1");
                            try_claim_in(&ClaimEnv { root: r, clock: &clock, today: &today, hook: &hook }, &o, &id)
                        })
                    })
                    .collect();
                hs.into_iter().map(|h| h.join().unwrap()).collect()
            });
            let wins = results.iter().filter(|v| allowed(v)).count();
            assert!(wins <= 1, "{results:?}");
            if !midnight {
                assert_eq!(wins, 1, "{results:?}");
            }
            for v in &results {
                assert!(matches!(v, Ok(StartVerdict::Allowed { claim: 1 }) | Ok(StartVerdict::LostRace)), "{v:?}");
            }
            assert_eq!(claims(&r).len(), wins);
        }
    }
}

/// `observe_and_close_claims_in` closes exactly the closable claims ≤ the high water mark and
/// reports how many remain open; `may_open_new_round` follows the pause (B4).
#[test]
fn observe_and_close_and_new_rounds() {
    let r = root("observe");
    grant_on(&r, 5);
    let t = t0();
    let o = owner("base", 3, "inc-1");
    put(&r, &rec(&rid(1), RepairState::Probation, None)); // closable (closed state)
    put(&r, &rec(&rid(2), RepairState::InProgress, Some(lock::new_lease(o.clone(), t, Some(2))))); // held
    put(&r, &rec(&rid(3), RepairState::InProgress, None)); // closable (no lock)
    for n in 1..=3u32 {
        place_claim(&r, n, &GrantClaim { schema: 1, repair: rid(u64::from(n)), lane_key: LaneKey("base".into()), day: D.into(), claimed_at: t }).unwrap();
    }
    let clock = move || t;
    let today = || D.to_string();
    let env = ClaimEnv { root: &r, clock: &clock, today: &today, hook: &continue_all };
    assert_eq!(observe_and_close_claims_in(&env, &owner("dept-1", 1, "x"), 2).unwrap(), 1);
    assert!(closed(&r, 1) && !closed(&r, 2) && !closed(&r, 3));
    assert_eq!(observe_and_close_claims_in(&env, &owner("dept-1", 1, "x"), 3).unwrap(), 1);
    assert!(closed(&r, 3));
}

/// F1 / S2: the grant code sends nothing to a queue, an alert route or the event bus (a refusal is an
/// answer only; nothing is pushed when the grant pauses or resumes — B4).
#[test]
fn grant_code_sends_nothing_to_queues() {
    let src = include_str!("grant.rs");
    let prod = &src[..src.find("#[cfg(test)]").unwrap()];
    for needle in ["cys send", "send_text", "alert_route", "events::", "feed.push", "wakeup", "enqueue", "Command::new"] {
        assert!(!prod.contains(needle), "grant.rs production code mentions {needle}");
    }
}

// ───────────────────────────── RULINGS CV1 (T4b) ─────────────────────────────

/// CV1: a decision is ONE step under the grant lock — while one decision holds the lock between its
/// validation and its publication (B suspended right before its write), another decision cannot
/// interleave (bounded wait → refused, nothing written); B then publishes generation 1; a later
/// `off` publishes generation 2. A writer that does not take the lock (an older build) and writes
/// between validation and publication makes the decision STALE: refused, its file left as it is.
#[test]
fn cv1_decisions_are_one_step_under_the_shared_lock() {
    let r = root("cv1-lock");
    let d = |state: GrantState| GrantDecision {
        schema: 1,
        state,
        daily_cap: 5,
        decided_at: 1.0,
        decided_lane: LaneKey("base".into()),
        decided_seat: 1,
        prompt_digest: "d".into(),
        prompt_excerpt: "예".into(),
        paused_at: None,
        resumed_at: None,
        generation: 0,
    };
    let short = std::time::Duration::from_millis(150);
    let (to_main, at_main) = std::sync::mpsc::channel::<()>();
    let (to_b, at_b) = std::sync::mpsc::channel::<()>();
    let rb = r.clone();
    let b = std::thread::spawn(move || {
        let hook = move |s: DecideStep| {
            if s == DecideStep::BeforePublish {
                to_main.send(()).unwrap();
                at_b.recv_timeout(std::time::Duration::from_secs(20)).unwrap();
            }
        };
        decide_in(&rb, std::time::Duration::from_secs(5), &hook, |cur| {
            assert_eq!((cur.generation, cur.unreadable), (0, false));
            Ok(GrantDecision { state: GrantState::On, ..d(GrantState::On) })
        })
    });
    at_main.recv_timeout(std::time::Duration::from_secs(20)).unwrap();
    // B is validated and suspended before its write: nothing can interleave
    let e = lock_grant_in(&r, short).err().expect("the lock is held by B");
    assert!(matches!(&e, RepairError::Precondition(m) if m == GRANT_LOCK_BUSY_KO), "{e:?}");
    let a = decide_in(&r, short, &|_| {}, |_| Ok(d(GrantState::Off)));
    assert!(a.is_err(), "A waits at most `wait` and is refused");
    assert_eq!(read_decision_in(&r).unwrap(), None, "nothing written while B holds the lock");
    to_b.send(()).unwrap();
    let pb = b.join().unwrap().unwrap();
    assert_eq!((pb.state, pb.generation), (GrantState::On, 1));
    let off = decide_in(&r, short, &|_| {}, |cur| {
        assert_eq!(cur.generation, 1);
        Ok(d(GrantState::Off))
    })
    .unwrap();
    assert_eq!((off.state, off.generation), (GrantState::Off, 2));
    // a lock-less writer between validation and publication → stale, refused, nothing overwritten
    let sneak = |s: DecideStep| {
        if s == DecideStep::BeforePublish {
            write_decision_in(&r, &GrantDecision { generation: 7, ..d(GrantState::Off) }).unwrap();
        }
    };
    let e = decide_in(&r, short, &sneak, |_| Ok(d(GrantState::On))).unwrap_err();
    assert!(matches!(&e, RepairError::Precondition(m) if m == GRANT_STALE_KO), "{e:?}");
    let now = read_decision_in(&r).unwrap().unwrap();
    assert_eq!((now.state, now.generation), (GrantState::Off, 7), "the stale decision wrote nothing");
    // a validation error publishes nothing and releases the lock
    let e = decide_in(&r, short, &|_| {}, |_| Err(RepairError::Precondition("no".into())));
    assert!(e.is_err());
    assert!(lock_grant_in(&r, short).is_ok(), "released after a refused decision");
    // an old file (no generation) reads as generation 0
    std::fs::write(grant_path_in(&r), br#"{"schema":1,"state":"off","daily_cap":5,"decided_at":1.0,"decided_lane":"base","decided_seat":1,"prompt_digest":"d","prompt_excerpt":"","paused_at":null,"resumed_at":null}"#).unwrap();
    assert_eq!(current_generation_in(&r), 0);
}

/// CV1 / fatal four: the grant lock dies with its holder — a process that holds it and is killed
/// leaves it free (the kernel releases an advisory lock with the last descriptor); the wait is
/// bounded; a link or a non-regular file at the lock's name is refused, never followed.
#[cfg(unix)]
#[test]
fn cv1_the_grant_lock_dies_with_its_holder() {
    use std::io::BufRead;
    let r = root("cv1-dead");
    let p = grant_lock_path_in(&r);
    std::fs::write(&p, b"").unwrap();
    // the holder is another PROCESS (perl's flock — not a python spawn site, see lib.rs's census)
    let mut child = std::process::Command::new("perl")
        .arg("-MFcntl=:flock")
        .arg("-e")
        .arg("open(my $f, '>>', $ARGV[0]) or die; flock($f, LOCK_EX) or die; $| = 1; print \"held\\n\"; sleep 60;")
        .arg(&p)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("perl is on PATH in the test environment");
    let mut line = String::new();
    std::io::BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    assert_eq!(line.trim(), "held");
    let started = std::time::Instant::now();
    assert!(lock_grant_in(&r, std::time::Duration::from_millis(300)).is_err(), "held by the child");
    assert!(started.elapsed() < std::time::Duration::from_secs(5), "the wait is bounded");
    child.kill().unwrap(); // our own child, by its handle (exact pid)
    child.wait().unwrap();
    assert!(lock_grant_in(&r, std::time::Duration::from_secs(2)).is_ok(), "free after the holder died");
    // never follows a link at the lock's name
    let r2 = root("cv1-link");
    std::os::unix::fs::symlink(r2.join("elsewhere"), grant_lock_path_in(&r2)).unwrap();
    assert!(lock_grant_in(&r2, std::time::Duration::from_millis(50)).is_err());
    assert!(!r2.join("elsewhere").exists(), "the link's target was not created");
}

/// CV1: a start is admitted at the END — an owner input or a `grant off` published while the
/// attempt is suspended after its pause check (at AfterLock and at BeforePlace) refuses it: the
/// attempt closes its own claim (if placed) and releases its own lock; nothing else changes.
#[test]
fn cv1_a_start_is_admitted_at_the_end() {
    for (at, what) in [
        (ClaimStep::AfterLock, "pause"),
        (ClaimStep::BeforePlace, "pause"),
        (ClaimStep::AfterLock, "off"),
        (ClaimStep::BeforePlace, "off"),
        (ClaimStep::AfterPlace, "pause"),
        (ClaimStep::AfterPlace, "off"),
    ] {
        let r = root("cv1-admit");
        grant_on(&r, 5);
        let t = t0();
        let id = rid(1);
        put(&r, &rec(&id, RepairState::InProgress, None));
        let o = owner("base", 3, "inc-1");
        let lane = LaneKey(format!("cv1-{}", SEQ.fetch_add(1, Ordering::Relaxed)));
        let hook = |s: ClaimStep| {
            if s == at {
                if what == "pause" {
                    assert!(note_owner_input_in(&r, &lane, t).unwrap());
                } else {
                    decide_in(&r, std::time::Duration::from_secs(2), &|_| {}, |cur| {
                        let mut d = cur.decision.cloned().unwrap();
                        d.state = GrantState::Off;
                        Ok(d)
                    })
                    .unwrap();
                }
            }
            HookAction::Continue
        };
        let clock = move || t + 1.0;
        let today = || D.to_string();
        let v = try_claim_in(&ClaimEnv { root: &r, clock: &clock, today: &today, hook: &hook }, &o, &id);
        if what == "pause" {
            assert_eq!(v.as_ref().ok(), Some(&StartVerdict::Paused), "{at:?}: {v:?}");
        } else {
            assert!(matches!(&v, Err(RepairError::Precondition(m)) if m == GRANT_OFF_KO), "{at:?}: {v:?}");
        }
        assert_eq!(get(&r, &id).lock, None, "{at:?} {what}: the attempt's own lock is released");
        for (n, _) in claims(&r) {
            assert!(closed(&r, n), "{at:?} {what}: the attempt's own claim {n} is closed");
        }
        assert_eq!(get(&r, &id).state, RepairState::InProgress, "the record stays open");
    }
    // an unchanged decision and no pause → admitted (the final check reads the same generation)
    let r = root("cv1-admit-ok");
    grant_on(&r, 5);
    let id = rid(1);
    put(&r, &rec(&id, RepairState::InProgress, None));
    let v = attempt(&r, t0(), D, &owner("base", 3, "inc-1"), &id).unwrap();
    assert_eq!(v, StartVerdict::Allowed { claim: 1 });
    assert!(!closed(&r, 1));
}

// ───────────────────────────── RULINGS CV3 (T4b) ─────────────────────────────

/// CV3: every owner input moves the in-memory pause at once — two inputs 4 s apart (the second
/// coalesced, not written) → admission resumes 30 minutes after the SECOND input, never before, and
/// at most one coalescing window later; the written mark covers the window.
#[test]
fn cv3_two_inputs_four_seconds_apart() {
    let r = root("cv3-two");
    grant_on(&r, 5);
    let lane = LaneKey("cv3-two".into());
    let t = t0();
    assert!(note_owner_input_coalesced_in(&r, &lane, t).unwrap(), "the first input is written");
    assert!(!note_owner_input_coalesced_in(&r, &lane, t + 4.0).unwrap(), "the second is coalesced");
    let m: PauseMark = serde_json::from_slice(&std::fs::read(pause_path_in(&r, &lane)).unwrap()).unwrap();
    assert_eq!(m.paused_at, t);
    assert!(m.resumed_at >= t + 4.0 + 1_800.0, "the mark covers the coalesced input");
    assert!(is_paused_in(&r, t + 4.0 + 1_799.9), "not before 30 minutes after the second input");
    assert!(pause_deadline_in(&r, t).unwrap() >= t + 4.0 + 1_800.0);
    assert!(!is_paused_in(&r, t + 1_800.0 + PAUSE_WRITE_COALESCE_SECS + 0.1));
    let o = owner("base", 3, "inc-1");
    assert_eq!(start_new(&r, t + 1_803.0, D, &o).1.unwrap(), StartVerdict::Paused);
    assert!(allowed(&start_new(&r, t + 1_806.0, D, &o).1));
    // a stored resumed_at far in the future is honoured only up to PAUSE_EXTENSION_MAX_SECS
    let r2 = root("cv3-ext");
    let l2 = LaneKey("x".into());
    crate::atomic_write_json(&pause_path_in(&r2, &l2), &PauseMark { schema: 1, lane_key: l2.clone(), paused_at: t, resumed_at: t + 1e9 }).unwrap();
    assert_eq!(pause_deadline_in(&r2, t), Some(t + 1_800.0 + PAUSE_EXTENSION_MAX_SECS));
}

/// CV3: a pause write that fails (the pause directory cannot be created — a plain file stands at
/// its name) keeps autonomous starts paused — also after 30 minutes — until a write succeeds; the
/// retry happens at the next admission (or input); then the stored pause rules again.
#[test]
fn cv3_a_failed_pause_write_fails_closed() {
    let r = root("cv3-fail");
    grant_on(&r, 5);
    let lane = LaneKey("cv3-fail".into());
    let t = t0();
    std::fs::write(r.join("grant-pause"), b"not a directory").unwrap();
    assert!(note_owner_input_coalesced_in(&r, &lane, t).is_err(), "the write fails");
    assert!(pause_uncertain_in(&r));
    assert!(is_paused_in(&r, t + 3_600.0), "uncertain → paused, whatever the clock");
    let o = owner("base", 3, "inc-1");
    assert_eq!(start_new(&r, t + 3_600.0, D, &o).1.unwrap(), StartVerdict::Paused, "the retry fails too");
    assert!(!may_open_new_round_in(&r, t + 3_600.0).unwrap());
    // the disk is writable again: the next admission writes the remembered input and decides by it
    std::fs::remove_file(r.join("grant-pause")).unwrap();
    assert!(allowed(&start_new(&r, t + 3_600.0, D, &o).1), "written, then 30 minutes are over");
    assert!(!pause_uncertain_in(&r));
    let m: PauseMark = serde_json::from_slice(&std::fs::read(pause_path_in(&r, &lane)).unwrap()).unwrap();
    assert_eq!(m.paused_at, t, "the remembered input was written");
    // a failure within 30 minutes of the input, then recovery: the pause still ends 30 min after it
    let r3 = root("cv3-fail2");
    grant_on(&r3, 5);
    std::fs::write(r3.join("grant-pause"), b"x").unwrap();
    assert!(note_owner_input_coalesced_in(&r3, &lane, t).is_err());
    std::fs::remove_file(r3.join("grant-pause")).unwrap();
    assert_eq!(start_new(&r3, t + 60.0, D, &o).1.unwrap(), StartVerdict::Paused, "recovered, still within the pause");
    assert!(!pause_uncertain_in(&r3));
}

// ───────────────────────────── RULINGS DG (T4b round 2) ─────────────────────────────

/// DG2 (the verdict's sequence): the attempt passes the first pause check; an owner input arrives
/// before placement (a pause that ends soon); the claim is placed; the attempt is suspended between
/// its two admission reads while `grant off` is published and the pause runs out. Every instant
/// after placement was paused or off → not Allowed (off), its claim closed, its lock released.
/// Without the Off: the pause that was in force when admission began refuses it (Paused); a pause
/// that starts between the two reads refuses it too (the reads differ).
#[test]
fn dg2_final_admission_is_one_coherent_read() {
    for case in ["off-and-expiry", "expiry-only", "input-between"] {
        let r = root("dg2");
        grant_on(&r, 5);
        let t = t0();
        let id = rid(1);
        put(&r, &rec(&id, RepairState::InProgress, None));
        let o = owner("base", 3, "inc-1");
        let lane = LaneKey(format!("dg2-{}", SEQ.fetch_add(1, Ordering::Relaxed)));
        let clock_now = Cell::new(t);
        let hook = |s: ClaimStep| {
            match (s, case) {
                (ClaimStep::BeforePlace, "off-and-expiry" | "expiry-only") => {
                    // an owner input whose pause ends 2 s from now
                    let at = clock_now.get() - (PAUSE_QUIET_SECS as f64 + PAUSE_WRITE_COALESCE_SECS) + 2.0;
                    assert!(note_owner_input_in(&r, &lane, at).unwrap());
                }
                (ClaimStep::BetweenAdmissionReads, "off-and-expiry") => {
                    decide_in(&r, std::time::Duration::from_secs(2), &|_| {}, |cur| {
                        let mut d = cur.decision.cloned().unwrap();
                        d.state = GrantState::Off;
                        Ok(d)
                    })
                    .unwrap();
                    clock_now.set(clock_now.get() + 60.0); // the pause has run out
                }
                (ClaimStep::BetweenAdmissionReads, "expiry-only") => {
                    clock_now.set(clock_now.get() + 60.0);
                }
                (ClaimStep::BetweenAdmissionReads, "input-between") => {
                    assert!(note_owner_input_in(&r, &lane, clock_now.get()).unwrap());
                }
                _ => {}
            }
            HookAction::Continue
        };
        let clock = || clock_now.get();
        let today = || D.to_string();
        let v = try_claim_in(&ClaimEnv { root: &r, clock: &clock, today: &today, hook: &hook }, &o, &id);
        match case {
            "off-and-expiry" => assert!(matches!(&v, Err(RepairError::Precondition(m)) if m == GRANT_OFF_KO), "{case}: {v:?}"),
            _ => assert_eq!(v.as_ref().ok(), Some(&StartVerdict::Paused), "{case}: {v:?}"),
        }
        assert_eq!(get(&r, &id).lock, None, "{case}: own lock released");
        let placed = claims(&r);
        assert_eq!(placed.len(), 1, "{case}");
        assert!(closed(&r, placed[0].0), "{case}: own claim closed");
    }
}
