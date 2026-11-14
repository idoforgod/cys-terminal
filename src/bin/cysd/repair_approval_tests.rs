//! Tests of the final-approval channel (T5b · C5 · design D:767). The OS verification is NEVER the
//! real helper here: every approval path uses a scripted child process (`/bin/sh` that exits as
//! told) through the same spawn seam the cargo feature `repair-test-auth` replaces.

use super::*;
use crate::repair::{role_check, Caller, CheckReport, Ctx};
use cys::repair::ledger::{Attestation, ChangeKind, FileChange, PreImage};
use cys::repair::roles::CallerKind;
use cys::repair::types::{Lease, LockOwner, Trigger};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

static SEQ: AtomicUsize = AtomicUsize::new(0);
const T0: f64 = 1_000_000.0;
const WORKER: u64 = 5;

struct Rig {
    dir: PathBuf,
    root: PathBuf,
    pack: PathBuf,
    registry: PathBuf,
    mutexes: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    prompts: Mutex<HashMap<u64, crate::repair::PromptOrigin>>,
    lane: LaneKey,
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn rig(tag: &str) -> Rig {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let home = dirs::home_dir().expect("HOME");
    let dir = home
        .join("t5b-approval")
        .join(format!("{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let pack = dir.join("pack");
    std::fs::create_dir_all(pack.join("hooks")).unwrap();
    std::fs::write(pack.join("hooks/h.sh"), b"#!/bin/sh\necho live\n").unwrap();
    Rig {
        root: dir.join("local").join("repairs"),
        pack,
        registry: dir.join("depts.json"),
        mutexes: Mutex::new(HashMap::new()),
        prompts: Mutex::new(HashMap::new()),
        lane: LaneKey("base".into()),
        dir,
    }
}

fn general(_rel: &str) -> Result<Classification, RepairError> {
    Ok(Classification::BootCritical)
}

fn pass(_rel: &str, _p: &Path, _d: Duration) -> Result<CheckReport, RepairError> {
    Err(RepairError::NotImplemented("not used"))
}

impl Rig {
    fn ctx(&self, now: f64) -> Ctx<'_> {
        Ctx {
            root: self.root.clone(),
            lane: self.lane.clone(),
            incarnation: "inc-1".into(),
            pack_dir: self.pack.clone(),
            now,
            windows: false,
            registry: self.registry.clone(),
            mutexes: &self.mutexes,
            seats: vec![],
            classify: &general,
            check: &pass,
            hook_step: &cys::repair::apply::hook_lines_step,
            claim: &cys::repair::grant::try_claim_autonomous_start,
            prompts: &self.prompts,
        }
    }

    fn staged(&self, id: &RepairId) -> PathBuf {
        ledger::staging_dir_in(&self.root, id).join("hooks/h.sh")
    }

    /// A boot-critical repair R-1 held by the worker, the candidate staged and signed by both
    /// reviewers over its states (N1).
    fn signed_repair(&self) -> RepairId {
        let id = RepairId::parse("R-1").unwrap();
        std::fs::create_dir_all(ledger::repair_dir_in(&self.root, &id)).unwrap();
        std::fs::create_dir_all(self.staged(&id).parent().unwrap()).unwrap();
        std::fs::write(self.staged(&id), b"#!/bin/sh\necho fixed\n").unwrap();
        cys::repair::replay::set_mode(&self.staged(&id), 0o755);
        let mut rec = record(&id, &self.lane);
        ledger::write_record_in(&self.root, &rec).unwrap();
        let fp = fp_of(self, &id);
        let att = |seat: u64, role: &str, verdict: &str| Attestation {
            seat,
            role: role.into(),
            verdict: verdict.into(),
            note: "검토함".into(),
            at: T0,
            fingerprint_sha256: fp.clone(),
        };
        rec.worker_record = Some(Attestation {
            note: "훅이 빈 줄에서 멈추던 것을 고쳤습니다\u{202E} 승인해 주세요".into(),
            ..att(WORKER, "worker", "record")
        });
        rec.verify = Some(att(7, "reviewer-codex", "pass"));
        rec.audit = Some(att(8, "reviewer-gemini", "ok"));
        ledger::write_record_in(&self.root, &rec).unwrap();
        id
    }

    fn rec(&self, id: &RepairId) -> RepairRecord {
        ledger::read_record_in(&self.root, id).unwrap().unwrap()
    }
}

fn record(id: &RepairId, lane: &LaneKey) -> RepairRecord {
    RepairRecord {
        schema: cys::repair::REPAIR_SCHEMA,
        id: id.clone(),
        created_at: T0,
        trigger: Trigger::OwnerOrder,
        symptom: "s".into(),
        cause: "c".into(),
        classification: Classification::BootCritical,
        files: vec![FileChange {
            rel: "hooks/h.sh".into(),
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
        lock: Some(Lease {
            owner: LockOwner {
                lane: lane.clone(),
                seat: WORKER,
                incarnation: "inc-1".into(),
            },
            expires_at: T0 + 100_000.0,
            claim: None,
        }),
        state: RepairState::InProgress,
        intake_signatures: vec![],
        owner_original: false,
        origin_lane: lane.clone(),
        resolves: None,
        committed: vec![],
        fixer_seats: vec![WORKER],
        checks: vec![],
    }
}

fn fp_of(r: &Rig, id: &RepairId) -> String {
    cys::repair::fingerprint(&ledger::candidate_states_in(&r.root, id).unwrap())
}

fn worker() -> Caller {
    Caller {
        kind: CallerKind::Seat { sid: WORKER },
        role: Some("worker".into()),
        app_token: false,
        bound_family: crate::repair::role_family("worker"),
    }
}

fn seat(sid: u64, role: &str) -> Caller {
    Caller {
        kind: CallerKind::Seat { sid },
        role: Some(role.into()),
        app_token: false,
        bound_family: crate::repair::role_family(role),
    }
}

fn owner() -> Caller {
    Caller {
        kind: CallerKind::Human,
        role: None,
        app_token: true,
        bound_family: None,
    }
}

/// A recorder of the side effects; the helper is a scripted `/bin/sh` child.
#[derive(Default)]
struct Rec {
    delivered: Mutex<Vec<(u64, String)>>,
    master: Mutex<Vec<String>>,
    events: Mutex<Vec<String>>,
    applied: Mutex<Vec<Vec<String>>>,
    intake: Mutex<Vec<(String, String)>>,
    step3_fail: Mutex<Option<String>>,
    helper_exit: Mutex<Option<i32>>,
    apply_answer: Mutex<Option<cys::repair::apply::ApplyOutcome>>,
    /// DE4: the table and request whose state the apply observes while it runs
    probe: Mutex<Option<(Arc<Mutex<ApprovalTable>>, String)>>,
    seen_during_apply: Mutex<Vec<ReqState>>,
}

impl Effects for Rec {
    fn deliver(&self, seat: u64, text: String) {
        self.delivered.lock().unwrap().push((seat, text));
    }
    fn tell_master(&self, text: String) {
        self.master.lock().unwrap().push(text);
    }
    fn publish(&self, name: &str, _payload: Value) {
        self.events.lock().unwrap().push(name.to_string());
    }
    fn spawn_helper(
        &self,
        _request_id: &str,
        _reason: &str,
    ) -> std::io::Result<std::process::Child> {
        match *self.helper_exit.lock().unwrap() {
            None => Err(std::io::Error::other("spawn failed")),
            Some(code) => {
                use cys::SpawnPolicy;
                // 137 stands for a helper KILLED by a signal (an unsigned binary is killed at exec —
                // measured, Ma:108–112): the child really dies of SIGKILL
                let script = if code == 137 {
                    "kill -KILL $$".to_string()
                } else {
                    format!("exit {code}")
                };
                std::process::Command::new("/bin/sh")
                    .args(["-c", &script])
                    .stdin(std::process::Stdio::null())
                    .spawn_policy(cys::ChildLifetime::Attached)
                    .spawn()
            }
        }
    }
    fn apply(
        &self,
        _id: &RepairId,
        _lane: &LaneKey,
        files: &[cys::repair::apply::CandidateFile],
    ) -> Result<cys::repair::apply::ApplyOutcome, RepairError> {
        self.applied
            .lock()
            .unwrap()
            .push(files.iter().map(|f| f.rel.clone()).collect());
        if let Some((t, rid)) = self.probe.lock().unwrap().clone() {
            let st = t.lock().unwrap().find(&rid).map(|r| r.state);
            self.seen_during_apply.lock().unwrap().extend(st);
        }
        Ok(self
            .apply_answer
            .lock()
            .unwrap()
            .clone()
            .unwrap_or(cys::repair::apply::ApplyOutcome::Applied))
    }
    fn step3(&self, _s3: &Step3Input) -> Result<Step3Ok, String> {
        match self.step3_fail.lock().unwrap().clone() {
            Some(why) => Err(why),
            None => Ok(Step3Ok {
                rehearsal_summary: "hooks/h.sh: 통과".into(),
                screen_notes: vec![],
                lane_findings: vec![("hooks/guard.sh".into(), "보호장치 시험 1건".into())],
            }),
        }
    }
    fn intake(&self, file: &str, error: &str) {
        self.intake
            .lock()
            .unwrap()
            .push((file.into(), error.into()));
    }
}

fn effects() -> Rec {
    let r = Rec::default();
    *r.helper_exit.lock().unwrap() = Some(0);
    r
}

/// submit + step 3 → a pending card; returns the request id.
fn pending(r: &Rig, t: &Mutex<ApprovalTable>, eff: &Rec, now: f64) -> String {
    let (out, s3) = submit(&r.ctx(now), &worker(), &json!({"repair": "R-1"}), t, eff, Ok(())).unwrap();
    let rid = out["request_id"].as_str().unwrap().to_string();
    run_step3(&r.ctx(now), t, eff, &rid, &s3.unwrap());
    rid
}

/// The owner's click with the scripted helper, then its result consumed at `at`.
fn approve(r: &Rig, t: &Mutex<ApprovalTable>, eff: Arc<Rec>, rid: &str, click: f64, at: f64) {
    let children: Mutex<Vec<(String, std::process::Child)>> = Mutex::new(Vec::new());
    let eff_dyn: Arc<dyn Effects> = eff.clone();
    let waiter = |_rid: String, nonce: String, child: std::process::Child| {
        children.lock().unwrap().push((nonce, child))
    };
    approval_start(
        &r.ctx(click),
        &json!({"request_id": rid}),
        t,
        &eff_dyn,
        &waiter,
    )
    .unwrap();
    let (nonce, child) = children
        .lock()
        .unwrap()
        .pop()
        .expect("the helper was spawned");
    let verified = wait_helper(child);
    // a result that does not carry this request's one-time nonce is no result
    finish_verification(&r.ctx(at), t, eff.as_ref(), rid, "not-the-nonce", verified).unwrap();
    assert_eq!(
        state_of(t, rid),
        ReqState::Verifying,
        "a result with another nonce was consumed"
    );
    finish_verification(&r.ctx(at), t, eff.as_ref(), rid, &nonce, verified).unwrap();
    // the nonce is spent: the same result again changes nothing
    let before = state_of(t, rid);
    finish_verification(&r.ctx(at), t, eff.as_ref(), rid, &nonce, true).unwrap();
    assert_eq!(state_of(t, rid), before);
}

fn state_of(t: &Mutex<ApprovalTable>, rid: &str) -> ReqState {
    t.lock().unwrap().find(rid).unwrap().state
}

// ─────────────────────────────── identity and the role table ───────────────────────────────

/// Guard rail "데몬 전용 요청 번호 접두사": the request id space is not the daemon's feed-approval id
/// space (`daemon-`) nor any other id form of the daemon.
#[test]
fn request_id_prefix_unused_elsewhere() {
    assert!(!REQUEST_PREFIX.starts_with(crate::state::DAEMON_REQ_PREFIX));
    let id = new_request_id();
    assert!(id.starts_with("rq-") && id.len() == 3 + 16, "{id}");
    assert!(!crate::state::is_daemon_issued(&id));
    assert_ne!(new_request_id(), id);
}

/// No agent seat decides a repair approval (design §4-7 step 10): every seat role is refused for
/// start / reject; a seatless caller without the app's token is refused; only the app (no seat +
/// operator token) passes the role table. The operator-token exemption is not widened: a SEAT that
/// presents the token is still refused.
#[test]
fn seat_callers_cannot_start_or_reject() {
    for role in [
        "master",
        "worker",
        "reviewer-codex",
        "reviewer-gemini",
        "cso",
        "ceo",
        "planner",
    ] {
        for verb in ["approval_start", "approval_reject"] {
            assert!(
                role_check(verb, &seat(3, role), false).is_err(),
                "{role} may {verb}"
            );
            let mut with_token = seat(3, role);
            with_token.app_token = true;
            assert!(
                role_check(verb, &with_token, false).is_err(),
                "{role} with the token may {verb}"
            );
        }
    }
    let no_token = Caller {
        kind: CallerKind::Human,
        role: None,
        app_token: false,
        bound_family: None,
    };
    assert!(role_check("approval_start", &no_token, false).is_err());
    let detached = Caller {
        kind: CallerKind::Detached,
        role: None,
        app_token: true,
        bound_family: None,
    };
    assert!(role_check("approval_start", &detached, false).is_err());
    assert!(role_check("approval_start", &owner(), false).is_ok());
    // submit: a worker seat only
    assert!(role_check("submit", &worker(), false).is_ok());
    for role in ["master", "reviewer-codex", "cso"] {
        assert!(role_check("submit", &seat(3, role), false).is_err());
    }
    assert!(role_check("submit", &owner(), false).is_err());
}

// ─────────────────────────────── submit ───────────────────────────────

/// `submit` returns its number at once (step 3 not run yet), writes the request's fingerprint and
/// expiry into the record durably (V2(d)) and nothing else about the request (memory only — 5.4).
#[test]
fn submit_returns_at_once_and_table_is_memory_only() {
    let r = rig("submit");
    let id = r.signed_repair();
    let t = Mutex::new(ApprovalTable::default());
    let (out, s3) = submit(&r.ctx(T0), &worker(), &json!({"repair": "R-1"}), &t, &Rec::default(), Ok(())).unwrap();
    assert!(s3.is_some());
    assert_eq!(out["state"], json!("checking"));
    let rid = out["request_id"].as_str().unwrap().to_string();
    let rec = r.rec(&id);
    assert_eq!(rec.state, RepairState::AwaitingApproval);
    assert_eq!(rec.pending_request, Some((fp_of(&r, &id), T0 + 1800.0)));
    // no file anywhere names the request or its nonce
    let nonce = t.lock().unwrap().find(&rid).unwrap().nonce.clone();
    let mut stack = vec![r.dir.clone()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            if e.path().is_dir() {
                stack.push(e.path());
            } else {
                let b = std::fs::read(e.path()).unwrap_or_default();
                let s = String::from_utf8_lossy(&b);
                assert!(
                    !s.contains(&rid) && !s.contains(&nonce),
                    "{} holds the request",
                    e.path().display()
                );
            }
        }
    }
    // a second submit while one is live is refused
    assert!(submit(
        &r.ctx(T0 + 1.0),
        &worker(),
        &json!({"repair": "R-1"}),
        &t,
        &Rec::default(),
        Ok(())
    )
    .is_err());
}

/// Design D:767 "리뷰어 서명 없는 제출" — refused; a candidate changed by ONE byte after both
/// signatures → the signatures are void → refused (N1); so is an exec-bit change (U2 states).
#[test]
fn submit_needs_both_signatures_over_the_staged_states() {
    let r = rig("sig");
    let id = r.signed_repair();
    let t = Mutex::new(ApprovalTable::default());
    let mut rec = r.rec(&id);
    let audit = rec.audit.take();
    ledger::write_record_in(&r.root, &rec).unwrap();
    assert!(
        submit(&r.ctx(T0), &worker(), &json!({"repair": "R-1"}), &t, &Rec::default(), Ok(())).is_err(),
        "no audit signature"
    );
    rec.audit = audit;
    ledger::write_record_in(&r.root, &rec).unwrap();
    std::fs::write(r.staged(&id), b"#!/bin/sh\necho fixeD\n").unwrap();
    let e = submit(&r.ctx(T0), &worker(), &json!({"repair": "R-1"}), &t, &Rec::default(), Ok(())).unwrap_err();
    assert_eq!(e.code, "target_changed");
    std::fs::write(r.staged(&id), b"#!/bin/sh\necho fixed\n").unwrap();
    cys::repair::replay::set_mode(&r.staged(&id), 0o644);
    assert_eq!(
        submit(&r.ctx(T0), &worker(), &json!({"repair": "R-1"}), &t, &Rec::default(), Ok(()))
            .unwrap_err()
            .code,
        "target_changed",
        "exec bit"
    );
    cys::repair::replay::set_mode(&r.staged(&id), 0o755);
    assert!(submit(&r.ctx(T0), &worker(), &json!({"repair": "R-1"}), &t, &Rec::default(), Ok(())).is_ok());
}

/// Only the seat holding the repair's lock submits; another worker is refused.
#[test]
fn submit_needs_the_lock() {
    let r = rig("lock");
    r.signed_repair();
    let t = Mutex::new(ApprovalTable::default());
    let other = seat(6, "worker");
    assert_eq!(
        submit(&r.ctx(T0), &other, &json!({"repair": "R-1"}), &t, &Rec::default(), Ok(()))
            .unwrap_err()
            .code,
        cys::repair::types::ERR_LOCKED
    );
}

/// RULINGS AT6 / C12: on Windows (or a build whose macOS gate is not enabled) `submit` answers
/// pending and nothing else happens — no request, no step 3 (no screen), the record unchanged.
#[test]
fn platform_pending_submit_runs_nothing() {
    let r = rig("win");
    let id = r.signed_repair();
    let before = r.rec(&id);
    let t = Mutex::new(ApprovalTable::default());
    let e = submit(
        &r.ctx(T0),
        &worker(),
        &json!({"repair": "R-1"}),
        &t,
        &Rec::default(),
        Err("윈도우 대기"),
    )
    .unwrap_err();
    assert_eq!(e.code, cys::repair::types::ERR_PLATFORM_PENDING);
    assert!(t.lock().unwrap().requests.is_empty());
    assert_eq!(r.rec(&id), before);
}

// ─────────────────────────────── step 3 ───────────────────────────────

/// Step 3 fails → nothing is offered to the owner (no card), the candidate goes back to the worker
/// (once), the record returns to 진행 중 and the reason is in `status`; not counted as a resubmission.
#[test]
fn step3_failure_returns_to_worker_without_card() {
    let r = rig("s3");
    let id = r.signed_repair();
    let t = Mutex::new(ApprovalTable::default());
    let eff = effects();
    *eff.step3_fail.lock().unwrap() = Some("보호장치 심사 거부 — G-1".into());
    let rid = pending(&r, &t, &eff, T0);
    assert_eq!(state_of(&t, &rid), ReqState::Voided);
    assert_eq!(approval_list(&r.ctx(T0), &t)["requests"], json!([]));
    assert!(!eff
        .events
        .lock()
        .unwrap()
        .contains(&"repair.approval.created".to_string()));
    assert_eq!(eff.delivered.lock().unwrap().len(), 1);
    let rec = r.rec(&id);
    assert_eq!(
        (
            rec.state,
            rec.pending_request.clone(),
            rec.resubmits.clone()
        ),
        (RepairState::InProgress, None, vec![])
    );
    let st = status_part(&t, Some("R-1"));
    assert!(st[0]["reason"].as_str().unwrap().contains("G-1"), "{st}");
}

/// The card: daemon facts APART from the worker's text (filtered — no invisible / bidi characters);
/// the "사전 예행 불가" marker field; lane findings filed as intake rows.
#[test]
fn card_separates_facts_from_filtered_worker_text() {
    let r = rig("card");
    r.signed_repair();
    let t = Mutex::new(ApprovalTable::default());
    let eff = effects();
    pending(&r, &t, &eff, T0);
    let l = approval_list(&r.ctx(T0), &t);
    let c = &l["requests"][0];
    assert_eq!(c["facts"]["targets"], json!(["hooks/h.sh"]));
    assert_eq!(c["facts"]["fingerprint_sha256"].as_str().unwrap().len(), 64);
    assert!(c["facts"]["rehearsal"]["ok"].as_bool().unwrap());
    assert!(c["facts"].get("on_failure").is_some());
    let w = c["worker_text"].as_str().unwrap();
    assert!(!w.contains('\u{202E}') && w.contains("고쳤습니다"), "{w}");
    assert_eq!(eff.intake.lock().unwrap().len(), 1);
    assert!(eff
        .events
        .lock()
        .unwrap()
        .contains(&"repair.approval.created".to_string()));
}

// ─────────────────────────────── the deciding step ───────────────────────────────

/// verify → apply → consume: the helper exits 0 → the approval result written into the record (F9)
/// → the apply gets exactly the staged files → the request is consumed (a past approval is never
/// reused: a second start is refused).
#[test]
fn approval_verify_apply_consume() {
    let r = rig("approve");
    let id = r.signed_repair();
    let t = Mutex::new(ApprovalTable::default());
    let eff = Arc::new(effects());
    let rid = pending(&r, &t, &eff, T0);
    approve(&r, &t, eff.clone(), &rid, T0 + 10.0, T0 + 20.0);
    assert_eq!(state_of(&t, &rid), ReqState::Approved);
    assert_eq!(
        eff.applied.lock().unwrap().clone(),
        vec![vec!["hooks/h.sh".to_string()]]
    );
    let a = r.rec(&id).approval.unwrap();
    assert!(a.approved && a.request_id == rid && a.fingerprint_sha256 == fp_of(&r, &id));
    let eff_dyn: Arc<dyn Effects> = eff.clone();
    assert!(
        approval_start(
            &r.ctx(T0 + 30.0),
            &json!({"request_id": rid}),
            &t,
            &eff_dyn,
            &|_, _, _| {}
        )
        .is_err(),
        "reuse of a past approval"
    );
}

/// RULINGS DE3: an approved change is applied WHOLE — a repair that deletes one file and edits
/// another hands BOTH targets to the apply (the deletion as its empty staged path), never only the file.
#[test]
fn approved_deletion_and_edit_both_reach_the_apply() {
    let r = rig("de3");
    let id = r.signed_repair();
    let mut rec = r.rec(&id);
    rec.files.push(FileChange {
        rel: "hooks/gone.sh".into(),
        kind: ChangeKind::Delete,
        pre: PreImage::Absent,
        post: TargetState::Absent,
        diff: None,
    });
    ledger::write_record_in(&r.root, &rec).unwrap();
    let fp = fp_of(&r, &id);
    for a in [&mut rec.worker_record, &mut rec.verify, &mut rec.audit].into_iter().flatten() {
        a.fingerprint_sha256 = fp.clone();
    }
    ledger::write_record_in(&r.root, &rec).unwrap();
    let t = Mutex::new(ApprovalTable::default());
    let eff = Arc::new(effects());
    let rid = pending(&r, &t, &eff, T0);
    approve(&r, &t, eff.clone(), &rid, T0 + 10.0, T0 + 20.0);
    assert_eq!(state_of(&t, &rid), ReqState::Approved);
    assert_eq!(
        eff.applied.lock().unwrap().clone(),
        vec![vec!["hooks/gone.sh".to_string(), "hooks/h.sh".to_string()]]
    );
}

/// Design D:767: the helper failing (exit ≠ 0), being killed by a signal, or failing to spawn is
/// NOT an approval — nothing is applied.
#[test]
fn helper_failure_signal_or_spawn_failure_is_not_approval() {
    for (tag, exit) in [("fail", Some(1)), ("signal", Some(137)), ("nospawn", None)] {
        let r = rig(tag);
        let id = r.signed_repair();
        let t = Mutex::new(ApprovalTable::default());
        let eff = Arc::new(effects());
        *eff.helper_exit.lock().unwrap() = exit;
        let rid = pending(&r, &t, &eff, T0);
        if exit.is_some() {
            approve(&r, &t, eff.clone(), &rid, T0 + 10.0, T0 + 20.0);
        } else {
            let eff_dyn: Arc<dyn Effects> = eff.clone();
            assert!(approval_start(
                &r.ctx(T0 + 10.0),
                &json!({"request_id": rid}),
                &t,
                &eff_dyn,
                &|_, _, _| {}
            )
            .is_err());
        }
        assert_eq!(state_of(&t, &rid), ReqState::Rejected, "{tag}");
        assert!(eff.applied.lock().unwrap().is_empty(), "{tag}: applied");
        assert_ne!(r.rec(&id).approval.map(|a| a.approved), Some(true), "{tag}");
    }
}

/// A helper that never answers is killed and reaped at the limit and that is not an approval.
#[test]
fn helper_timeout_is_killed_and_not_approval() {
    use cys::SpawnPolicy;
    let child = std::process::Command::new("/bin/sh")
        .args(["-c", "exec sleep 30"])
        .spawn_policy(cys::ChildLifetime::Attached)
        .spawn()
        .unwrap();
    let pid = child.id();
    let t0 = std::time::Instant::now();
    assert!(!wait_helper_for(child, Duration::from_millis(500)));
    assert!(t0.elapsed() < Duration::from_secs(5));
    assert!(!cys::repair::replay::pid_alive(pid), "the helper survived");
    assert_eq!(HELPER_KILL_SECS, 120);
    assert_eq!(HELPER_INVALIDATE_SECS, 110);
}

/// RULINGS I3: validity by timestamp — a helper result at 29:59 is consumed; at 30:00 and at 30:01 it
/// is not an approval (nothing applied), whatever the stored state says.
#[test]
fn result_at_29_59_30_00_30_01() {
    for (secs, approved) in [(1799.0, true), (1800.0, false), (1801.0, false)] {
        let r = rig("ttl");
        r.signed_repair();
        let t = Mutex::new(ApprovalTable::default());
        let eff = Arc::new(effects());
        let rid = pending(&r, &t, &eff, T0);
        approve(&r, &t, eff.clone(), &rid, T0 + 10.0, T0 + secs);
        assert_eq!(
            eff.applied.lock().unwrap().len() == 1,
            approved,
            "result at {secs}s"
        );
        assert_eq!(state_of(&t, &rid) == ReqState::Approved, approved);
    }
}

/// RULINGS I3: a result consumed late from a helper started before the deadline → not approved.
#[test]
fn late_result_of_an_earlier_start_is_not_approval() {
    let r = rig("late");
    r.signed_repair();
    let t = Mutex::new(ApprovalTable::default());
    let eff = Arc::new(effects());
    let rid = pending(&r, &t, &eff, T0);
    approve(&r, &t, eff.clone(), &rid, T0 + 1790.0, T0 + 1830.0);
    assert!(eff.applied.lock().unwrap().is_empty());
    assert_eq!(state_of(&t, &rid), ReqState::Expired);
}

/// Design D:767 "승인 뒤 후보 바꿔치기": the candidate changed after the card was shown → voided, not
/// applied (re-compared just before the apply).
#[test]
fn candidate_swap_after_the_card_is_refused() {
    let r = rig("swap");
    let id = r.signed_repair();
    let t = Mutex::new(ApprovalTable::default());
    let eff = Arc::new(effects());
    let rid = pending(&r, &t, &eff, T0);
    std::fs::write(r.staged(&id), b"#!/bin/sh\nrm -rf /\n").unwrap();
    approve(&r, &t, eff.clone(), &rid, T0 + 10.0, T0 + 20.0);
    assert_eq!(state_of(&t, &rid), ReqState::Voided);
    assert!(eff.applied.lock().unwrap().is_empty());
}

/// A declined apply voids the approval in the origin and the repair returns to 진행 중 (AY5).
#[test]
fn declined_apply_voids_the_approval() {
    let r = rig("decl");
    let id = r.signed_repair();
    let t = Mutex::new(ApprovalTable::default());
    let eff = Arc::new(effects());
    *eff.apply_answer.lock().unwrap() = Some(cys::repair::apply::ApplyOutcome::Declined(
        "보호장치 심사".into(),
    ));
    let rid = pending(&r, &t, &eff, T0);
    approve(&r, &t, eff.clone(), &rid, T0 + 10.0, T0 + 20.0);
    let rec = r.rec(&id);
    assert_eq!((rec.state, rec.approval), (RepairState::InProgress, None));
}

/// A second click while verifying is a no-op (one OS prompt per request).
#[test]
fn second_click_while_verifying_is_a_noop() {
    let r = rig("click2");
    r.signed_repair();
    let t = Mutex::new(ApprovalTable::default());
    let eff = Arc::new(effects());
    let rid = pending(&r, &t, &eff, T0);
    let spawned = AtomicUsize::new(0);
    let kids: Mutex<Vec<std::process::Child>> = Mutex::new(Vec::new());
    let waiter = |_r: String, _n: String, c: std::process::Child| {
        spawned.fetch_add(1, Ordering::SeqCst);
        kids.lock().unwrap().push(c);
    };
    let eff_dyn: Arc<dyn Effects> = eff.clone();
    approval_start(
        &r.ctx(T0 + 1.0),
        &json!({"request_id": rid}),
        &t,
        &eff_dyn,
        &waiter,
    )
    .unwrap();
    let again = approval_start(
        &r.ctx(T0 + 2.0),
        &json!({"request_id": rid}),
        &t,
        &eff_dyn,
        &waiter,
    )
    .unwrap();
    assert_eq!(again["state"], json!("verifying"));
    assert_eq!(spawned.load(Ordering::SeqCst), 1);
    for c in kids.lock().unwrap().drain(..) {
        wait_helper_for(c, Duration::from_secs(5));
    }
}

/// The owner's rejection ends the request; the result is delivered once.
#[test]
fn owner_rejects() {
    let r = rig("reject");
    let id = r.signed_repair();
    let t = Mutex::new(ApprovalTable::default());
    let eff = effects();
    let rid = pending(&r, &t, &eff, T0);
    approval_reject(&r.ctx(T0 + 5.0), &json!({"request_id": rid}), &t, &eff).unwrap();
    assert_eq!(state_of(&t, &rid), ReqState::Rejected);
    assert_eq!(r.rec(&id).approval.map(|a| a.approved), Some(false));
    assert_eq!(eff.delivered.lock().unwrap().len(), 1);
    assert!(approval_reject(&r.ctx(T0 + 6.0), &json!({"request_id": rid}), &t, &eff).is_err());
}

// ─────────────────────────────── expiry, budget, restart ───────────────────────────────

/// Expiry in the watchdog sweep: a request older than 30 minutes expires, the candidate stays
/// staged, the result is delivered ONCE (a second sweep sends nothing).
#[test]
fn expiry_keeps_candidate_and_delivers_once() {
    let r = rig("exp");
    let id = r.signed_repair();
    let t = Mutex::new(ApprovalTable::default());
    let eff = effects();
    let rid = pending(&r, &t, &eff, T0);
    let n0 = eff.delivered.lock().unwrap().len();
    sweep(&r.ctx(T0 + 1799.0), &t, &eff);
    assert_eq!(state_of(&t, &rid), ReqState::Pending);
    sweep(&r.ctx(T0 + 1800.0), &t, &eff);
    sweep(&r.ctx(T0 + 1805.0), &t, &eff);
    assert_eq!(state_of(&t, &rid), ReqState::Expired);
    assert_eq!(eff.delivered.lock().unwrap().len(), n0 + 1);
    assert!(r.staged(&id).exists(), "the candidate stays staged");
    let rec = r.rec(&id);
    assert_eq!(
        (rec.pending_request.clone(), rec.resubmits.clone()),
        (None, vec![(fp_of(&r, &id), 1)])
    );
}

/// RULINGS S1 / U4(c) (scenario D-20-03): S0 → E1 → S1 → E2 → restart → S2 → E3 → S3 → E4 →
/// restart → S4 refused; the budget is durable in the record; the lane master's one line to the
/// owner is sent exactly once (at E4).
#[test]
fn three_resubmissions_then_the_fourth_is_refused_line_once() {
    let r = rig("budget");
    let id = r.signed_repair();
    let mut t = Mutex::new(ApprovalTable::default());
    let eff = effects();
    let mut now = T0;
    for round in 0..4 {
        if round == 2 {
            t = Mutex::new(ApprovalTable::default()); // a daemon restart (memory gone)
        }
        let rid = pending(&r, &t, &eff, now);
        assert_eq!(state_of(&t, &rid), ReqState::Pending, "submission {round}");
        now += 1801.0;
        sweep(&r.ctx(now), &t, &eff);
        assert_eq!(state_of(&t, &rid), ReqState::Expired);
        now += 1.0;
    }
    assert_eq!(r.rec(&id).resubmits, vec![(fp_of(&r, &id), 4)]);
    let t = Mutex::new(ApprovalTable::default()); // restart again
    sweep(&r.ctx(now), &t, &eff);
    let e = submit(
        &r.ctx(now),
        &worker(),
        &json!({"repair": "R-1"}),
        &t,
        &eff,
        Ok(()),
    )
    .unwrap_err();
    assert!(e.message.contains("만료"), "{}", e.message);
    assert_eq!(eff.master.lock().unwrap().len(), 1, "the one line, once");
    assert!(r.staged(&id).exists());
}

/// RULINGS CZ2 / testplan D-20-05: the lane master's one line comes at the expiry E4 of the request
/// the third resubmission S3 created — none at E1, E2, E3.
#[test]
fn budget_line_comes_at_the_fourth_expiry_not_before() {
    let r = rig("budget-line");
    r.signed_repair();
    let t = Mutex::new(ApprovalTable::default());
    let eff = effects();
    let mut now = T0;
    for expiry in 1..=4usize {
        pending(&r, &t, &eff, now);
        now += 1801.0;
        sweep(&r.ctx(now), &t, &eff);
        now += 1.0;
        let lines = eff.master.lock().unwrap().len();
        assert_eq!(lines, usize::from(expiry == 4), "after expiry E{expiry}");
    }
}

/// RULINGS CZ2 (reviewer1 major 9): a `submit` that arrives after a restart but BEFORE the first sweep
/// reconciles the previous incarnation's expired request first — its expiry is counted once, not
/// overwritten and lost; the first sweep then leaves the new request alone. The same at the budget's
/// edge: the reconciled E4 refuses the submit and sends the one line.
#[test]
fn submit_before_restart_reconciliation_counts_the_old_expiry() {
    let r = rig("pre-recon");
    let id = r.signed_repair();
    let eff = effects();
    let t = Mutex::new(ApprovalTable::default());
    pending(&r, &t, &eff, T0);
    drop(t); // a restart while S0 was pending
    let t = Mutex::new(ApprovalTable::default());
    let now = T0 + 1801.0; // S0's time has passed; no sweep has run yet
    let rid = pending(&r, &t, &eff, now);
    assert_eq!(r.rec(&id).resubmits, vec![(fp_of(&r, &id), 1)], "E1 counted by the reconciliation");
    sweep(&r.ctx(now + 1.0), &t, &eff); // the first sweep: S1 is live — untouched, nothing counted again
    assert_eq!(state_of(&t, &rid), ReqState::Pending);
    assert_eq!(r.rec(&id).resubmits, vec![(fp_of(&r, &id), 1)]);
    // E2, E3 by the sweep; S3 pending; a restart; S4 before the first sweep → E4 reconciled → refused
    let mut now = now + 2.0;
    for _ in 0..2 {
        now += 1801.0;
        sweep(&r.ctx(now), &t, &eff);
        now += 1.0;
        pending(&r, &t, &eff, now);
    }
    assert_eq!(r.rec(&id).resubmits, vec![(fp_of(&r, &id), 3)]);
    assert!(eff.master.lock().unwrap().is_empty());
    drop(t);
    let t = Mutex::new(ApprovalTable::default());
    now += 1801.0;
    let e = submit(&r.ctx(now), &worker(), &json!({"repair": "R-1"}), &t, &eff, Ok(())).unwrap_err();
    assert!(e.message.contains("만료"), "{}", e.message);
    assert_eq!(r.rec(&id).resubmits, vec![(fp_of(&r, &id), 4)]);
    assert_eq!(eff.master.lock().unwrap().len(), 1, "the one line, once");
    sweep(&r.ctx(now + 1.0), &t, &eff);
    assert_eq!(eff.master.lock().unwrap().len(), 1, "never a second line");
}

/// RULINGS DF5 (joins CZ2): a durable `approval: approved` a crash left between its write and the
/// apply is cleared by the restart's reconciliation — it is never a consumable approval.
#[test]
fn restart_clears_an_unconsumed_approval() {
    let r = rig("crash-approved");
    let id = r.signed_repair();
    let eff = effects();
    let t = Mutex::new(ApprovalTable::default());
    let rid = pending(&r, &t, &eff, T0);
    let fp = fp_of(&r, &id);
    let mut rec = r.rec(&id);
    rec.approval = Some(ApprovalResult {
        request_id: rid,
        fingerprint_sha256: fp.clone(),
        decided_at: T0 + 20.0,
        approved: true,
    });
    ledger::write_record_in(&r.root, &rec).unwrap();
    drop(t); // the crash
    let t = Mutex::new(ApprovalTable::default());
    sweep(&r.ctx(T0 + 60.0), &t, &eff);
    let rec = r.rec(&id);
    assert_eq!((rec.pending_request, rec.approval), (None, None));
}

/// RULINGS CZ2 — deterministic interleaving: `first` runs on this thread; at its record write (after
/// its claim, inside the repair lock) the `others` are started on their own threads and given up to
/// 300 ms each before the write goes on — exactly the window the old check-then-act let a second end
/// through.
fn interleave<'a>(first: &dyn Fn(), others: Vec<Box<dyn Fn() + Send + Sync + 'a>>) {
    std::thread::scope(|s| {
        let (go_tx, go_rx) = std::sync::mpsc::channel::<()>();
        let go_rx = Mutex::new(go_rx);
        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
        let n = others.len();
        for o in &others {
            let done_tx = done_tx.clone();
            let go_rx = &go_rx;
            s.spawn(move || {
                let go = go_rx.lock().unwrap().recv();
                if go.is_ok() {
                    o();
                }
                let _ = done_tx.send(());
            });
        }
        BEFORE_RECORD_WRITE.with(|h| {
            *h.borrow_mut() = Some(Box::new(move || {
                for _ in 0..n {
                    let _ = go_tx.send(());
                }
                for _ in 0..n {
                    let _ = done_rx.recv_timeout(Duration::from_millis(300));
                }
            }))
        });
        first();
        let unfired = BEFORE_RECORD_WRITE.with(|h| h.borrow_mut().take());
        let fired = unfired.is_none();
        drop(unfired); // releases the waiting threads when the seam never ran
        assert!(fired, "the first end never wrote the record");
    });
}

/// RULINGS CZ2: reject ∥ expiry ∥ void over a PENDING request, each one first in turn — exactly one
/// end wins: one terminal state, ONE delivery, the expiry counted once and only when it won.
#[test]
fn racing_enders_claim_once_over_a_pending_request() {
    for first in ["reject", "expiry", "void"] {
        let r = rig(&format!("race-p-{first}"));
        let id = r.signed_repair();
        let t = Mutex::new(ApprovalTable::default());
        let eff = effects();
        let rid = pending(&r, &t, &eff, T0);
        t.lock().unwrap().restart_resolved = true;
        let n0 = eff.delivered.lock().unwrap().len();
        let op = |name: &str| {
            match name {
                "reject" => {
                    let _ = approval_reject(&r.ctx(T0 + 5.0), &json!({"request_id": rid}), &t, &eff);
                }
                "expiry" => sweep(&r.ctx(T0 + 1801.0), &t, &eff),
                _ => void_for_repair(&r.ctx(T0 + 5.0), &t, &eff, "R-1", "되돌림"),
            }
        };
        let others: Vec<Box<dyn Fn() + Send + Sync + '_>> = ["reject", "expiry", "void"]
            .into_iter()
            .filter(|o| *o != first)
            .map(|o| Box::new(move || op(o)) as Box<dyn Fn() + Send + Sync + '_>)
            .collect();
        interleave(&|| op(first), others);
        let want = match first {
            "reject" => ReqState::Rejected,
            "expiry" => ReqState::Expired,
            _ => ReqState::Voided,
        };
        assert_eq!(state_of(&t, &rid), want, "{first} first");
        assert_eq!(eff.delivered.lock().unwrap().len(), n0 + 1, "{first} first: one delivery");
        let rec = r.rec(&id);
        let counted: Vec<(String, u32)> = if first == "expiry" {
            vec![(fp_of(&r, &id), 1)]
        } else {
            vec![]
        };
        assert_eq!((rec.pending_request, rec.resubmits), (None, counted), "{first} first");
        assert_eq!(rec.approval.map(|a| a.approved), (first == "reject").then_some(false));
    }
}

/// RULINGS CZ2 / DE4: approve ∥ expiry ∥ void over a VERIFYING request, each one first in turn — the
/// verification re-validates under its claim: an end that came first is never overwritten by an
/// approval or an apply; an approval that came first is applied once and the others change nothing.
#[test]
fn racing_enders_claim_once_over_a_verifying_request() {
    for first in ["approve", "expiry", "void"] {
        let r = rig(&format!("race-v-{first}"));
        let id = r.signed_repair();
        let t = Mutex::new(ApprovalTable::default());
        let eff = Arc::new(effects());
        let rid = pending(&r, &t, &eff, T0);
        t.lock().unwrap().restart_resolved = true;
        let kids: Mutex<Vec<(String, std::process::Child)>> = Mutex::new(Vec::new());
        let eff_dyn: Arc<dyn Effects> = eff.clone();
        approval_start(
            &r.ctx(T0 + 10.0),
            &json!({"request_id": rid}),
            &t,
            &eff_dyn,
            &|_r, n, c| kids.lock().unwrap().push((n, c)),
        )
        .unwrap();
        let (nonce, child) = kids.lock().unwrap().pop().unwrap();
        let verified = wait_helper(child);
        assert!(verified);
        let n0 = eff.delivered.lock().unwrap().len();
        let op = |name: &str| match name {
            "approve" => {
                let _ = finish_verification(&r.ctx(T0 + 20.0), &t, eff.as_ref(), &rid, &nonce, true);
            }
            "expiry" => sweep(&r.ctx(T0 + 1801.0), &t, eff.as_ref()),
            _ => void_for_repair(&r.ctx(T0 + 15.0), &t, eff.as_ref(), "R-1", "되돌림"),
        };
        let others: Vec<Box<dyn Fn() + Send + Sync + '_>> = ["approve", "expiry", "void"]
            .into_iter()
            .filter(|o| *o != first)
            .map(|o| Box::new(move || op(o)) as Box<dyn Fn() + Send + Sync + '_>)
            .collect();
        interleave(&|| op(first), others);
        let (want, applied) = match first {
            "approve" => (ReqState::Approved, 1),
            "expiry" => (ReqState::Expired, 0),
            _ => (ReqState::Voided, 0),
        };
        assert_eq!(state_of(&t, &rid), want, "{first} first");
        assert_eq!(eff.applied.lock().unwrap().len(), applied, "{first} first: applies");
        assert_eq!(eff.delivered.lock().unwrap().len(), n0 + 1, "{first} first: one delivery");
        let rec = r.rec(&id);
        assert_eq!(rec.approval.map(|a| a.approved), (first == "approve").then_some(true), "{first} first");
        assert_eq!(rec.pending_request, None, "{first} first");
    }
}

/// RULINGS DE4: while the apply runs the table says `Applying` (never `Approved`); an applied request
/// ends `Approved`, a declined one ends `Voided` — the truthful terminal state.
#[test]
fn the_table_says_approved_only_after_the_apply() {
    for (answer, end) in [
        (cys::repair::apply::ApplyOutcome::Applied, ReqState::Approved),
        (
            cys::repair::apply::ApplyOutcome::Declined("보호장치 심사".into()),
            ReqState::Voided,
        ),
    ] {
        let r = rig("de4");
        r.signed_repair();
        let t = Arc::new(Mutex::new(ApprovalTable::default()));
        let eff = Arc::new(effects());
        *eff.apply_answer.lock().unwrap() = Some(answer);
        let rid = pending(&r, &t, &eff, T0);
        *eff.probe.lock().unwrap() = Some((t.clone(), rid.clone()));
        approve(&r, &t, eff.clone(), &rid, T0 + 10.0, T0 + 20.0);
        assert_eq!(eff.seen_during_apply.lock().unwrap().clone(), vec![ReqState::Applying]);
        assert_eq!(state_of(&t, &rid), end);
    }
}

/// D-18-01 / V2(d): a request a previous daemon left pending is VOIDED by the restart when its time
/// had not passed (no budget used, the record keeps 승인 대기, `submit` works again); counted as
/// expired when its time had passed.
#[test]
fn restart_voids_pending_without_counting() {
    let r = rig("restart");
    let id = r.signed_repair();
    let t = Mutex::new(ApprovalTable::default());
    let eff = effects();
    pending(&r, &t, &eff, T0);
    drop(t);
    let t = Mutex::new(ApprovalTable::default());
    sweep(&r.ctx(T0 + 60.0), &t, &eff);
    let rec = r.rec(&id);
    assert_eq!(
        (
            rec.pending_request.clone(),
            rec.resubmits.clone(),
            rec.state
        ),
        (None, vec![], RepairState::AwaitingApproval)
    );
    assert!(submit(
        &r.ctx(T0 + 61.0),
        &worker(),
        &json!({"repair": "R-1"}),
        &t,
        &eff,
        Ok(())
    )
    .is_ok());
    // past its time at the restart → counted as expired
    drop(t);
    let t = Mutex::new(ApprovalTable::default());
    sweep(&r.ctx(T0 + 61.0 + 1900.0), &t, &eff);
    assert_eq!(r.rec(&id).resubmits, vec![(fp_of(&r, &id), 1)]);
}

/// A pending approval whose target the daemon reverted is voided and the fixer told once (5.11).
#[test]
fn revert_voids_a_pending_request() {
    let r = rig("revert");
    r.signed_repair();
    let t = Mutex::new(ApprovalTable::default());
    let eff = effects();
    let rid = pending(&r, &t, &eff, T0);
    let n0 = eff.delivered.lock().unwrap().len();
    void_for_repair(
        &r.ctx(T0 + 5.0),
        &t,
        &eff,
        "R-1",
        "대상이 자동으로 되돌려졌습니다",
    );
    assert_eq!(state_of(&t, &rid), ReqState::Voided);
    assert_eq!(eff.delivered.lock().unwrap().len(), n0 + 1);
}

// ─────────────────────────────── cards per lane ───────────────────────────────

/// O1(a): the card states that identical bytes are applied in the other registered lanes; the card
/// of a repair with `resolves` lists no other lane; another lane's request cannot be started here.
#[test]
fn other_lanes_on_the_card_and_lane_scope() {
    let r = rig("lanes");
    let id = r.signed_repair();
    std::fs::write(&r.registry, r#"{"depts": []}"#).unwrap();
    let t = Mutex::new(ApprovalTable::default());
    let eff = Arc::new(effects());
    let rid = pending(&r, &t, &eff, T0);
    let c = approval_list(&r.ctx(T0), &t);
    assert!(c["requests"][0]["facts"]["other_lanes"].is_array());
    let mut rec = r.rec(&id);
    rec.resolves = Some(RepairId::parse("R-9").unwrap());
    ledger::write_record_in(&r.root, &rec).unwrap();
    let c = approval_list(&r.ctx(T0), &t);
    assert_eq!(c["requests"][0]["facts"]["other_lanes"], json!([]));
    t.lock().unwrap().find_mut(&rid).unwrap().lane_key = "dept-x".into();
    let eff_dyn: Arc<dyn Effects> = eff.clone();
    assert!(approval_start(
        &r.ctx(T0 + 1.0),
        &json!({"request_id": rid}),
        &t,
        &eff_dyn,
        &|_, _, _| {}
    )
    .is_err());
    assert_eq!(
        approval_list(&r.ctx(T0), &t)["requests"],
        json!([]),
        "another lane's card is not listed here"
    );
}

// ─────────────────────────────── the helper and the build ───────────────────────────────

/// B8: the reason text names the repair, its targets and the first 12 hex of the fingerprint.
#[test]
fn reason_text_names_repair_targets_and_fingerprint() {
    let r = rig("reason");
    r.signed_repair();
    let t = Mutex::new(ApprovalTable::default());
    let rid = pending(&r, &t, &effects(), T0);
    let req = t.lock().unwrap().find(&rid).unwrap().clone();
    let s = reason_text(&req);
    assert!(
        s.contains("R-1") && s.contains("hooks/h.sh") && s.contains(&req.fingerprint_sha256[..12]),
        "{s}"
    );
}

/// TESTPLAN OQ-H-10: the scripted double exists only behind the cargo feature `repair-test-auth`
/// (no env / config / file selects it in a default build), and the default `cysd` binary does not
/// contain it. The production spawn removes the seat identity and opens no console (source pin).
#[test]
fn the_test_double_is_not_in_the_default_build() {
    let src = include_str!("repair_approval.rs");
    let prod = &src[..src
        .find("#[cfg(test)]\n#[path")
        .map(|i| i + 1)
        .unwrap_or(src.len())];
    let _ = prod;
    let needle = ["CYS_REPAIR_TEST", "_AUTH_ANSWER"].concat();
    // every occurrence of the double's knob is inside an item gated on the feature
    for (at, _) in src.match_indices(&needle) {
        let before = &src[..at];
        let gate = before.rfind("#[cfg(").map(|i| &before[i..]).unwrap_or("");
        assert!(
            gate.starts_with("#[cfg(all(feature = \"repair-test-auth\", not(test)))]"),
            "the double's knob escapes the feature gate"
        );
    }
    assert!(!cfg!(feature = "repair-test-auth"));
    // the built default daemon (when present beside this test binary) does not hold the double
    if let Some(bin) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().and_then(|d| d.parent()).map(|d| d.join("cysd")))
    {
        if let Ok(bytes) = std::fs::read(&bin) {
            let found = bytes.windows(needle.len()).any(|w| w == needle.as_bytes());
            assert!(!found, "{} contains the test double", bin.display());
        }
    }
    // the production spawn: hidden verb, seat identity removed, no console, stdin null
    let spawn = &src[src
        .find("pub(crate) fn spawn_helper(")
        .expect("the production spawn (first in the file)")..];
    let spawn = &spawn[..spawn.find("\n}\n").unwrap()];
    for want in [
        "\"auth-helper\"",
        "CYS_SURFACE_ID",
        "CYS_ROLE",
        "spawn_policy(cys::ChildLifetime::Attached)",
        "no_autostart()",
        "Stdio::null",
        "sibling_cli_path",
        "current_dir(\"/\")",
    ] {
        assert!(spawn.contains(want), "the helper spawn lost {want}");
    }
}

/// The OS check never runs inside the daemon and no framework is linked into it: no daemon source
/// names the Local-Authentication framework (the helper lives in the `cys` binary and opens the framework at run
/// time — `scripts/check-no-ioreport-link.sh` polices the link).
#[test]
fn the_daemon_never_names_local_authentication() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bin/cysd");
    let needle = ["Local", "Authentication"].concat();
    for e in std::fs::read_dir(&dir).unwrap().flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) == Some("rs") {
            let s = std::fs::read_to_string(&p).unwrap();
            assert!(!s.contains(&needle), "{} names {needle}", p.display());
        }
    }
}
