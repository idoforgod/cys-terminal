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
    // RULINGS JW2: shared, so that a send seam can probe the repair locks (try-lock)
    mutexes: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
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
    // RULINGS DE6: inside the replay's own scratch base, never HOME
    let dir = cys::repair::rehearsal::scratch_base()
        .join("tests.nosync")
        .join(format!("approval-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let pack = dir.join("pack");
    std::fs::create_dir_all(pack.join("hooks")).unwrap();
    std::fs::write(pack.join("hooks/h.sh"), b"#!/bin/sh\necho live\n").unwrap();
    Rig {
        root: dir.join("local").join("repairs"),
        pack,
        registry: dir.join("depts.json"),
        mutexes: Arc::new(Mutex::new(HashMap::new())),
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
            product_pause: &|| false,
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
        // T6 (AQ5; the field arrived with the rebase onto preview/batch6)
        reapplies: None,
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
    /// RULINGS GA8: the ledger's answer the double gives (None = a new row)
    intake_answer: Mutex<Option<crate::repair::Filed>>,
    step3_fail: Mutex<Option<String>>,
    step3_err: Mutex<Option<Step3Err>>,
    helper_exit: Mutex<Option<i32>>,
    apply_answer: Mutex<Option<cys::repair::apply::ApplyOutcome>>,
    /// DE4: the table and request whose state the apply observes while it runs
    probe: Mutex<Option<(Arc<Mutex<ApprovalTable>>, String)>>,
    /// RULINGS EL5: the test double of the seam (None = NotConsumed)
    consumed_answer: Mutex<Option<cys::repair::consumed::ApprovalUse>>,
    seen_during_apply: Mutex<Vec<ReqState>>,
    /// RULINGS JW2: probed at every send to the master — (the approval table free, the repair's lock
    /// free) by try-lock — and what it saw
    send_probe: Mutex<Option<Box<dyn Fn() -> (bool, bool) + Send + Sync>>>,
    sends_free: Mutex<Vec<(bool, bool)>>,
    /// RULINGS KX2: when set, the apply goes to the ENGINE (`apply::apply_candidate_locked` — the
    /// production lock + apply) over the rig's paths instead of the scripted answer
    engine: Mutex<Option<EngineApply>>,
    /// every bus event with its payload (`events` keeps the names)
    payloads: Mutex<Vec<(String, Value)>>,
}

type EngineApply = Box<
    dyn Fn(
            &RepairId,
            &[cys::repair::apply::CandidateFile],
            &cys::repair::apply::LifetimeFn<'_>,
        ) -> Result<cys::repair::apply::Bounded<cys::repair::apply::ApplyOutcome>, RepairError>
        + Send
        + Sync,
>;

impl Effects for Rec {
    fn deliver(&self, seat: u64, text: String) {
        self.delivered.lock().unwrap().push((seat, text));
    }
    fn tell_master(&self, text: String) {
        if let Some(p) = self.send_probe.lock().unwrap().as_ref() {
            self.sends_free.lock().unwrap().push(p());
        }
        self.master.lock().unwrap().push(text);
    }
    fn point_master(&self, text: String) {
        self.master.lock().unwrap().push(text);
    }
    fn publish(&self, name: &str, payload: Value) {
        self.events.lock().unwrap().push(name.to_string());
        self.payloads.lock().unwrap().push((name.to_string(), payload));
    }
    fn spawn_helper(
        &self,
        request_id: &str,
        nonce: &str,
        _reason: &str,
    ) -> std::io::Result<std::process::Child> {
        match *self.helper_exit.lock().unwrap() {
            None => Err(std::io::Error::other("spawn failed")),
            Some(code) => {
                use cys::SpawnPolicy;
                // 137 stands for a helper KILLED by a signal (an unsigned binary is killed at exec —
                // measured, Ma:108–112): the child really dies of SIGKILL; RULINGS JW1: exit 0 is
                // the helper whose approval call succeeded — it prints its request's ONE record
                let script = match code {
                    137 => "kill -KILL $$".to_string(),
                    0 => "printf 'CYS-AUTH-OK v1 request=%s nonce=%s\\n' \"$1\" \"$CYS_REPAIR_AUTH_NONCE\"; exit 0".to_string(),
                    c => format!("exit {c}"),
                };
                bind_helper(
                    std::process::Command::new("/bin/sh")
                        .args(["-c", &script, "sh", request_id])
                        .stdin(std::process::Stdio::null()),
                    nonce,
                )
                .spawn_policy(cys::ChildLifetime::Attached)
                .spawn()
            }
        }
    }
    fn apply(
        &self,
        id: &RepairId,
        _lane: &LaneKey,
        files: &[cys::repair::apply::CandidateFile],
        still_valid: &cys::repair::apply::LifetimeFn<'_>,
    ) -> Result<cys::repair::apply::Bounded<cys::repair::apply::ApplyOutcome>, RepairError> {
        if let Some(engine) = self.engine.lock().unwrap().as_ref() {
            return engine(id, files, still_valid);
        }
        self.applied
            .lock()
            .unwrap()
            .push(files.iter().map(|f| f.rel.clone()).collect());
        if let Some((t, rid)) = self.probe.lock().unwrap().clone() {
            let st = t.lock().unwrap().find(&rid).map(|r| r.state);
            self.seen_during_apply.lock().unwrap().extend(st);
        }
        Ok(cys::repair::apply::Bounded::Ran(
            self.apply_answer
                .lock()
                .unwrap()
                .clone()
                .unwrap_or(cys::repair::apply::ApplyOutcome::Applied),
        ))
    }
    fn step3(&self, _s3: &Step3Input) -> Result<Step3Ok, Step3Err> {
        if let Some(e) = self.step3_err.lock().unwrap().clone() {
            return Err(e);
        }
        match self.step3_fail.lock().unwrap().clone() {
            Some(why) => Err(why.into()),
            None => Ok(Step3Ok {
                rehearsal_summary: "hooks/h.sh: 통과".into(),
                screen_notes: vec![],
                lane_findings: vec![("hooks/guard.sh".into(), "보호장치 시험 1건".into())],
            }),
        }
    }
    fn intake(&self, file: &str, error: &str) -> crate::repair::Filed {
        self.intake
            .lock()
            .unwrap()
            .push((file.into(), error.into()));
        self.intake_answer
            .lock()
            .unwrap()
            .clone()
            .unwrap_or(crate::repair::Filed::New)
    }
    fn approval_consumed(
        &self,
        _root: &Path,
        _lane: &LaneKey,
        _approval_ref: &str,
    ) -> cys::repair::consumed::ApprovalUse {
        self.consumed_answer
            .lock()
            .unwrap()
            .clone()
            .unwrap_or(cys::repair::consumed::ApprovalUse::NotConsumed)
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
    let verified = wait_helper(child, rid, &nonce);
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

/// RULINGS JW1 (HL3 (ii)) — only a VALIDATED success record approves, whatever the exit code: the
/// PRODUCTION helper boundary (`bind_helper` + `wait_helper_for`) driven by stand-in helper programs.
/// (The old boundary returned the exit status alone: the first three stand-ins approved.)
#[test]
fn only_the_requests_success_record_approves() {
    use cys::SpawnPolicy;
    let (rid, nonce) = ("rq-00aa", "00112233445566778899aabbccddeeff");
    let run = |script: &str| -> bool {
        let child = bind_helper(
            std::process::Command::new("/bin/sh")
                .args(["-c", script, "sh", rid])
                .stdin(std::process::Stdio::null())
                .stderr(std::process::Stdio::null()),
            nonce,
        )
        .spawn_policy(cys::ChildLifetime::Attached)
        .spawn()
        .unwrap();
        wait_helper_for(child, Duration::from_secs(20), rid, nonce)
    };
    let record = "printf 'CYS-AUTH-OK v1 request=%s nonce=%s\\n' \"$1\" \"$CYS_REPAIR_AUTH_NONCE\"";
    for (name, script) in [
        ("malformed output + exit 0", "printf 'ok\\n'; exit 0".to_string()),
        ("empty output + exit 0", "exit 0".to_string()),
        ("another request's record + exit 0", "printf 'CYS-AUTH-OK v1 request=rq-00bb nonce=%s\\n' \"$CYS_REPAIR_AUTH_NONCE\"; exit 0".to_string()),
        ("the valid record + exit 1", format!("{record}; exit 1")),
        ("another nonce + exit 0", "printf 'CYS-AUTH-OK v1 request=%s nonce=ffffffffffffffffffffffffffffffff\\n' \"$1\"; exit 0".to_string()),
        ("two records + exit 0", format!("{record}; {record}; exit 0")),
        ("an over-long output + exit 0", format!("{record}; head -c 4096 /dev/zero; exit 0")),
    ] {
        assert!(!run(&script), "{name} approved");
    }
    assert!(run(&format!("{record}; exit 0")), "the valid record + exit 0 is the one approval");
    // the helper (the `cys` binary) prints exactly this form, from the nonce in this variable
    let cli = include_str!("../cys/repair_cli.rs");
    assert!(cli.contains("\"CYS-AUTH-OK v1 request={request} nonce={nonce}\""));
    assert_eq!(
        helper_success_record("rq-1", "ab"),
        format!("{HELPER_RECORD_TAG} request=rq-1 nonce=ab\n")
    );
    assert_eq!(HELPER_RECORD_TAG, "CYS-AUTH-OK v1");
    assert!(cli.contains(&format!("\"{HELPER_NONCE_VAR}\"")));
}

/// RULINGS KJ1 — the deadline is tested through the PRODUCTION boundary (`bind_helper`, a real pipe):
/// each stand-in ignores the limit and, left alone, WOULD end with this request's valid record and
/// exit 0 (an approval) — so the verdict itself tells a wait that honours the limit from one that
/// does not (RULINGS EZ1: no clock is asserted). (a) the record only after the limit; (b) the record,
/// then a hang; (c) the record, then its standard output closed while it lives; (d) the record and
/// exit 0 while a GRANDCHILD keeps the pipe open. Each: "not approved"; the direct child ended and
/// was reaped (DR3 — a zombie would still answer signal 0); in (b)–(d) the stand-in reached the
/// point after its record (a marker file — the shape was real, not killed early), and in (d) the
/// grandchild still held the pipe when the wait returned and is ended by its exact pid (also on a failure).
/// (Old expectation — this test's former body: a child spawned WITHOUT a piped output and
/// `exec sleep 30`, `!wait_helper_for(child, 500 ms, …)`, `!pid_alive(pid)`; the boundary answered
/// "no answer" at its first turn, so the deadline check was never reached and removing it left the
/// test green.)
#[cfg(unix)]
#[test]
fn helper_timeout_is_killed_and_not_approval() {
    use cys::repair::replay::{pid_alive, process_identity};
    use cys::SpawnPolicy;
    let (rid, nonce) = ("rq-00aa", "00112233445566778899aabbccddeeff");
    let r = rig("kj1");
    let gpid = r.dir.join("grandchild.pid");
    /// Ends (d)'s grandchild by the exact pid it wrote, also when an assertion above fails first.
    struct EndGrandchild(PathBuf);
    impl Drop for EndGrandchild {
        fn drop(&mut self) {
            if let Some(g) = std::fs::read_to_string(&self.0).ok().and_then(|t| t.trim().parse::<i32>().ok()) {
                // SAFETY: kill(2) of the grandchild this test started, by its exact pid
                unsafe {
                    libc::kill(g, libc::SIGKILL);
                }
            }
        }
    }
    let _end = EndGrandchild(gpid.clone());
    let record = "printf 'CYS-AUTH-OK v1 request=%s nonce=%s\\n' \"$1\" \"$CYS_REPAIR_AUTH_NONCE\"";
    let reached = |n: &str| r.dir.join(format!("reached-{n}"));
    let shapes = [
        (
            "a",
            "the record only after the limit",
            "exec /usr/bin/perl -e 'select(undef, undef, undef, 3); print \"CYS-AUTH-OK v1 request=$ARGV[0] nonce=$ENV{CYS_REPAIR_AUTH_NONCE}\\n\"; exit 0' \"$1\"".to_string(),
        ),
        (
            "b",
            "the record, then a hang",
            format!("{record}; : > '{}'; exec /bin/sleep 30", reached("b").display()),
        ),
        (
            "c",
            "the record, then the output closed while it lives",
            format!("{record}; exec >&-; : > '{}'; exec /bin/sleep 30", reached("c").display()),
        ),
        (
            "d",
            "the record and exit 0, a grandchild holding the pipe",
            format!(
                "(exec /bin/sleep 30) & echo $! > '{}'; {record}; : > '{}'; exit 0",
                gpid.display(),
                reached("d").display()
            ),
        ),
    ];
    for (tag, name, script) in shapes {
        let child = bind_helper(
            std::process::Command::new("/bin/sh")
                .args(["-c", &script, "sh", rid])
                .stdin(std::process::Stdio::null())
                .stderr(std::process::Stdio::null()),
            nonce,
        )
        .spawn_policy(cys::ChildLifetime::Attached)
        .spawn()
        .unwrap();
        let pid = child.id();
        assert!(
            !wait_helper_for(child, Duration::from_secs(1), rid, nonce),
            "({tag}) {name}: approved"
        );
        assert!(!pid_alive(pid), "({tag}) {name}: the direct child {pid} was not ended and reaped");
        if tag != "a" {
            assert!(reached(tag).exists(), "premise ({tag}) {name}: the record was not written");
        }
    }
    let g: u32 = std::fs::read_to_string(&gpid).unwrap().trim().parse().unwrap();
    assert!(process_identity(g).is_some(), "premise (d): the grandchild held the pipe past the limit");
    assert_eq!(HELPER_KILL_SECS, 120);
    assert_eq!(HELPER_INVALIDATE_SECS, 110);
}

/// RULINGS KQ1 — expiry wins over success: a helper that completes (its valid record, the end of its
/// output, exit 0) while the waiter sleeps across the limit is NOT approved when the waiter sees it at
/// or past the limit (the reviewer's sequence: the clock read at limit − 10 ms, the helper done during
/// the 20 ms pause, the next turn at the limit or past it). The ORDER is controlled, not timed: the
/// stand-in completes only when released, and the release happens inside the waiter's first reading
/// of the clock (`wait_helper_clocked`'s seam), which then waits — blocking, not polling — until the
/// stand-in has ended (not reaped). The control: the same completion seen just BEFORE the limit is the
/// one approval. (The old loop returned the success before it read the clock again: approved.)
#[cfg(unix)]
#[test]
fn a_success_seen_at_or_past_the_limit_is_not_approval() {
    use cys::SpawnPolicy;
    use std::io::Write;
    let (rid, nonce) = ("rq-00aa", "00112233445566778899aabbccddeeff");
    let limit = Duration::from_secs(HELPER_KILL_SECS);
    let script = "read go; printf 'CYS-AUTH-OK v1 request=%s nonce=%s\\n' \"$1\" \"$CYS_REPAIR_AUTH_NONCE\"; exit 0";
    let run = |seen_at: Duration| -> bool {
        let mut child = bind_helper(
            std::process::Command::new("/bin/sh")
                .args(["-c", script, "sh", rid])
                .stdin(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null()),
            nonce,
        )
        .spawn_policy(cys::ChildLifetime::Attached)
        .spawn()
        .unwrap();
        let pid = child.id();
        let mut release = child.stdin.take();
        let mut readings = 0u32;
        let mut clock = || -> Duration {
            readings += 1;
            if let Some(mut go) = release.take() {
                go.write_all(b"go\n").unwrap();
                drop(go);
                // SAFETY: waitid(2) on this test's own child, WEXITED | WNOWAIT: blocks until it has
                // ended and leaves it for the waiter to reap; `info` is zeroed and never read
                unsafe {
                    let mut info: libc::siginfo_t = std::mem::zeroed();
                    while libc::waitid(libc::P_PID, pid as libc::id_t, &mut info, libc::WEXITED | libc::WNOWAIT) != 0 {
                        assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(libc::EINTR));
                    }
                }
                return limit - Duration::from_millis(10);
            }
            seen_at
        };
        let approved = wait_helper_clocked(child, limit, rid, nonce, &mut clock);
        assert!(readings >= 1, "the waiter never read the clock");
        approved
    };
    assert!(!run(limit + Duration::from_millis(10)), "a success seen 10 ms past the limit approved");
    assert!(!run(limit), "a success seen AT the limit approved");
    assert!(
        run(limit - Duration::from_millis(5)),
        "control: the same completion seen just before the limit is the one approval"
    );
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

/// What one KX2 case left behind.
#[cfg(target_os = "macos")]
struct Kx2Seen {
    state: ReqState,
    rec_state: RepairState,
    approval: Option<ApprovalResult>,
    intent: Option<String>,
    live: (Vec<u8>, (u64, u64)),
    live_before: (Vec<u8>, (u64, u64)),
    staged: (Vec<u8>, (u64, u64), TargetState),
    staged_before: (Vec<u8>, (u64, u64), TargetState),
    resolved: Vec<Value>,
}

/// One approval through the production lock + apply (`apply::apply_candidate_locked`: the blocking
/// `flock` of `pack_apply_lock_path_for(pack)`, then the engine) over the rig's own repairs root and
/// pack, the protection screen and the file's own check passing. `hold` = another holder (a thread of
/// this test) holds the REAL lock on the file a SIBLING pack under the same parent names — the same
/// file. The owner's result is consumed at T0 + 20 s (inside the lifetime); the approving thread then
/// says it enters the apply; `during_wait` (from `expires_at`) is where the holder moves the injected
/// consumption clock BEFORE it releases the lock — so the clock moves after every check before the
/// apply and before the apply can pass the lock.
#[cfg(target_os = "macos")]
fn kx2_case(tag: &str, hold: bool, during_wait: Option<fn(f64) -> f64>) -> Kx2Seen {
    use cys::repair::apply as engine;
    use std::os::unix::fs::MetadataExt;
    use std::sync::atomic::AtomicU64;
    let r = rig(tag);
    let id = r.signed_repair();
    // the engine's own preconditions: a known pack generation and the live file's pre-image as
    // `begin` records it
    std::fs::write(r.pack.join(".pack-version"), "0.15.0\n").unwrap();
    let live = r.pack.join("hooks/h.sh");
    let pre = ledger::copy_preimage_in(&r.root, &id, &r.lane, "hooks/h.sh", &live, T0).unwrap();
    let mut rec = r.rec(&id);
    rec.files[0].pre = pre;
    ledger::write_record_in(&r.root, &rec).unwrap();
    let t = Mutex::new(ApprovalTable::default());
    let eff = Arc::new(effects());
    let rid = pending(&r, &t, &eff, T0);
    let entering: Arc<Mutex<Option<std::sync::mpsc::Sender<()>>>> = Arc::new(Mutex::new(None));
    {
        let (root, pack, lane, entering) = (r.root.clone(), r.pack.clone(), r.lane.clone(), entering.clone());
        *eff.engine.lock().unwrap() = Some(Box::new(move |id, files, still_valid| {
            if let Some(tx) = entering.lock().unwrap().take() {
                let _ = tx.send(());
            }
            let env = engine::OpEnv {
                root: &root,
                pack: &pack,
                lane: &lane,
                now: T0 + 20.0,
                form: cys::repair::platform::ApplyForm::Exchange,
                faults: &engine::NoFaults,
                own_check: &|_, _| None,
            };
            let pass = |_: &str, _: &Path, _: Duration| -> Result<engine::CheckOutcome, RepairError> {
                Ok(engine::CheckOutcome { result: cys::repair::rehearsal::CheckResult::Pass, ms: 1 })
            };
            let pass_reference = |_: cys::repair::boot_critical::CheckKind, _: Duration| -> Result<engine::CheckOutcome, RepairError> {
                Ok(engine::CheckOutcome { result: cys::repair::rehearsal::CheckResult::Pass, ms: 1 })
            };
            engine::apply_candidate_locked(&env, id, files, &|_, _, _| Ok(()), &pass, &pass_reference, still_valid)
        }));
    }
    // the owner clicks and the helper answers with this request's record
    let children: Mutex<Vec<(String, std::process::Child)>> = Mutex::new(Vec::new());
    let eff_dyn: Arc<dyn Effects> = eff.clone();
    approval_start(&r.ctx(T0 + 10.0), &json!({"request_id": rid}), &t, &eff_dyn, &|_r, n, c| {
        children.lock().unwrap().push((n, c))
    })
    .unwrap();
    let (nonce, child) = children.lock().unwrap().pop().expect("the helper was spawned");
    assert!(wait_helper(child, &rid, &nonce), "premise: the helper's record approves");
    let expires_at = t.lock().unwrap().find(&rid).unwrap().expires_at;
    let clock = Arc::new(AtomicU64::new((T0 + 20.0).to_bits()));
    CONSUMPTION_CLOCK_FOR_TEST.with(|c| *c.borrow_mut() = Some(clock.clone()));
    // the holder: the REAL lock, through the file a sibling pack under the same parent names
    let sibling = r.pack.parent().unwrap().join("pack-dept-x");
    let lock_file = cys::pack_apply_lock::pack_apply_lock_path_for(&sibling);
    assert_eq!(lock_file, cys::pack_apply_lock::pack_apply_lock_path_for(&r.pack), "premise: one lock file");
    let holder = hold.then(|| {
        let (held_tx, held_rx) = std::sync::mpsc::channel::<()>();
        let (enter_tx, enter_rx) = std::sync::mpsc::channel::<()>();
        *entering.lock().unwrap() = Some(enter_tx);
        let clock = clock.clone();
        let h = std::thread::spawn(move || {
            cys::pack_apply_lock::with_apply_lock(&lock_file, || {
                held_tx.send(()).unwrap();
                // the approving thread has passed every check before the apply and enters it; the
                // clock moves while the lock is still held
                if enter_rx.recv().is_ok() {
                    if let Some(to) = during_wait {
                        clock.store(to(expires_at).to_bits(), std::sync::atomic::Ordering::SeqCst);
                    }
                }
            })
            .unwrap();
        });
        held_rx.recv().unwrap();
        h
    });
    let id_of = |p: &Path| {
        let m = std::fs::symlink_metadata(p).unwrap();
        (m.dev(), m.ino())
    };
    let staged_path = r.staged(&id);
    let live_before = (std::fs::read(&live).unwrap(), id_of(&live));
    let staged_before = (
        std::fs::read(&staged_path).unwrap(),
        id_of(&staged_path),
        TargetState::read(&staged_path).unwrap(),
    );
    let res = finish_verification(&r.ctx(T0 + 20.0), &t, eff.as_ref(), &rid, &nonce, true);
    // a path that never entered the apply must not leave the holder waiting
    entering.lock().unwrap().take();
    if let Some(h) = holder {
        h.join().unwrap();
    }
    CONSUMPTION_CLOCK_FOR_TEST.with(|c| *c.borrow_mut() = None);
    assert!(res.is_ok(), "{tag}: {res:?}");
    let rec = r.rec(&id);
    let seen = Kx2Seen {
        state: state_of(&t, &rid),
        rec_state: rec.state,
        approval: rec.approval,
        intent: engine::read_intent_in(&r.root, &id, &r.lane).map(|i| i.state),
        live: (std::fs::read(&live).unwrap(), id_of(&live)),
        live_before,
        staged: (
            std::fs::read(&staged_path).unwrap(),
            id_of(&staged_path),
            TargetState::read(&staged_path).unwrap(),
        ),
        staged_before,
        resolved: eff
            .payloads
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, _)| n == "repair.approval.resolved")
            .map(|(_, p)| p["state"].clone())
            .collect(),
    };
    seen
}

/// RULINGS KX2 (whole-diff review pass 2, slice AUTH, M1; I3 / contract 5.5 "again at apply") — an
/// approval that waited for the SHARED pack-apply lock past its request's expiry applies NOTHING:
/// no apply intent, the live file's bytes and identity unchanged, the staged candidate untouched, and
/// the request ends exactly as the check before the apply ends it — `Expired`, the repair back to
/// 승인 대기, the approved record cleared (not `Voided`, not 진행 중) — with one resolved event saying
/// expired. Controls: the lock free → applied; the lock waited for with the clock still inside →
/// applied; the boundary — one tick before `expires_at` applied, AT `expires_at` expired (the side
/// of the check before the apply, `now >= expires_at`). (Old behaviour, fb75a072: nothing between
/// the lock and the first durable intent asked the lifetime — case (a) wrote its intent and
/// exchanged the live file.)
#[cfg(target_os = "macos")]
#[test]
fn an_approval_that_waited_for_the_pack_apply_lock_past_its_expiry_applies_nothing() {
    let expired = |tag: &str, s: &Kx2Seen| {
        assert_eq!(s.state, ReqState::Expired, "{tag}: the request");
        assert_eq!(s.rec_state, RepairState::AwaitingApproval, "{tag}: the repair back to 승인 대기");
        assert_eq!(s.approval, None, "{tag}: the approved record cleared");
        assert_eq!(s.intent, None, "{tag}: an apply intent was written");
        assert_eq!(s.live, s.live_before, "{tag}: the live file's bytes or identity changed");
        assert_eq!(s.staged, s.staged_before, "{tag}: the staged candidate changed");
        assert_eq!(s.resolved, vec![json!("expired")], "{tag}: the one resolved event");
    };
    let applied = |tag: &str, s: &Kx2Seen| {
        assert_eq!(s.state, ReqState::Approved, "{tag}: the request");
        assert_eq!(s.intent.as_deref(), Some("applied"), "{tag}: the apply intent");
        assert_eq!(s.live.0, s.staged_before.0, "{tag}: the candidate is live");
        assert_ne!(s.live.1, s.live_before.1, "{tag}: the live entry was exchanged");
        assert_eq!(s.resolved, vec![json!("approved")], "{tag}: the one resolved event");
    };
    // (a) the clock passes the expiry while the apply waits for another holder's lock
    expired("a", &kx2_case("kx2-a", true, Some(|e| e + 10.0)));
    // (b) the lock free → applied exactly as before
    applied("b", &kx2_case("kx2-b", false, None));
    // (c) the lock waited for, the clock still inside the lifetime
    applied("c", &kx2_case("kx2-c", true, Some(|e| e - 60.0)));
    // (d) the boundary instants
    applied("d-tick", &kx2_case("kx2-d1", true, Some(|e| f64::from_bits(e.to_bits() - 1))));
    expired("d-at", &kx2_case("kx2-d2", true, Some(|e| e)));
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
        wait_helper_for(c, Duration::from_secs(5), "rq-00", "00112233445566778899aabbccddeeff");
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

/// RULINGS DF4: a panic inside the watchdog's sweep (or a worker thread of this channel) is caught and
/// the caller goes on; no thread of this module is created by a call that panics when it cannot be.
#[test]
fn watchdog_sweep_and_threads_survive_panics() {
    assert!(!guarded("test", || panic!("boom")), "a panic is caught and reported");
    assert!(guarded("test", || {}));
    let src = include_str!("repair_approval.rs");
    assert!(
        !src.contains(concat!("std::thread::", "spawn(")),
        "every thread goes through spawn_thread (Builder::spawn, a handled error)"
    );
    let sweep_fn = src
        .split("pub(crate) fn watchdog_sweep(")
        .nth(1)
        .and_then(|t| t.split("\n}\n").next())
        .unwrap_or("");
    assert!(sweep_fn.contains("guarded("), "the watchdog's sweep is panic-isolated");
    let gov = include_str!("governance.rs");
    assert!(gov.contains("crate::repair_approval::watchdog_sweep(&daemon);"));
}

/// RULINGS DE1 / GA8: a refusal by the after-run comparison returns the candidate; step 3 itself wrote
/// its ONE intake row first (`intake` lists it) — `run_step3` files nothing again and sends the
/// master's pointer once, nothing per tick.
#[test]
fn comparison_refusal_files_one_intake_row_and_one_owner_line() {
    let r = rig("de1");
    r.signed_repair();
    let t = Mutex::new(ApprovalTable::default());
    let eff = effects();
    *eff.step3_err.lock().unwrap() = Some(Step3Err {
        why: "예행이 실제 환경의 항목을 다시 썼습니다(x)".into(),
        intake: vec![("hooks/h.sh".into(), "예행이 실제 환경의 항목을 다시 썼습니다(x)".into())],
        owner_line: Some("수리 R-1 … 돌려보냈습니다".into()),
    });
    let rid = pending(&r, &t, &eff, T0);
    assert_eq!(state_of(&t, &rid), ReqState::Voided);
    // RULINGS GA8 (old expectation: `eff.intake.len() == 1` — run_step3 filed the carried rows after
    // ending the request; the production step 3 writes them first, `step3_writes_the_row_first…`)
    assert_eq!(eff.intake.lock().unwrap().len(), 0, "the caller never files the rows again");
    assert_eq!(eff.master.lock().unwrap().len(), 1, "one owner line");
    sweep(&r.ctx(T0 + 60.0), &t, &eff);
    sweep(&r.ctx(T0 + 2000.0), &t, &eff);
    assert_eq!(eff.intake.lock().unwrap().len(), 0, "never again per tick");
    assert_eq!(eff.master.lock().unwrap().len(), 1);
}

/// RULINGS GA3: the rehearsal's pass is carried out WHOLE — a target that fails does not end it early.
/// Target A passes the first time and fails in the repeat (a marker of its own); target B writes a
/// live listed file on every run. The repeat still runs B, B writes again → REFUSED. (The old pass
/// returned at A's failure in the repeat, B never ran, the repeat counted as complete → "noted".)
#[test]
fn a_failing_target_does_not_end_the_repeat_early() {
    let r = rig("ga3");
    let id = RepairId::parse("R-1").unwrap();
    let live = r.pack.join("hooks/guard.sh");
    std::fs::write(&live, b"#!/bin/sh\nexit 0\n").unwrap();
    let marker = r.dir.join("a-ran-once");
    let staging = ledger::staging_dir_in(&r.root, &id);
    let a = "hooks/appbuild-gate.sh";
    let b = "hooks/grill-arm.sh";
    let mut states = Vec::new();
    for (rel, body) in [
        (a, format!("#!/bin/sh\nif [ -f '{0}' ]; then exit 1; fi\n: > '{0}'\nexit 0\n", marker.display())),
        (b, format!("#!/bin/sh\nprintf 'x\\n' >> '{}'\nexit 0\n", live.display())),
    ] {
        let p = staging.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, body).unwrap();
        cys::repair::replay::set_mode(&p, 0o755);
        states.push((rel.to_string(), TargetState::read(&p).unwrap()));
    }
    let s3 = Step3Input { root: r.root.clone(), repair: id, pack: r.pack.clone(), states, windows: false };
    let home = r.dir.join("realhome");
    std::fs::create_dir_all(&home).unwrap();
    let e = production_step3_with(&s3, &home, &|_, _| crate::repair::Filed::New).unwrap_err();
    // RULINGS GR3 (old expectation: `e.why.contains("다시 썼습니다")`): A's failing event leaves its
    // released-side unit without an outcome, so the repeat is now refused as incomplete; that B still
    // ran in the repeat (GA3) is read from the live file — written once per pass
    assert!(e.why.contains("끝나지 않아"), "{}", e.why);
    assert_eq!(
        std::fs::read_to_string(&live).unwrap().lines().filter(|l| *l == "x").count(),
        2,
        "B ran in both passes"
    );
}

/// RULINGS GR3 — the plan lists every execution UNIT (file × event × side) before the first starts.
/// A real multi-event hook (`hooks/cys-hook.sh`: PermissionRequest, PostToolUse, PreToolUse) passes
/// all its events in the first pass, the third writing a live listed file; in the repeat its FIRST
/// event fails, the replay returns there and its later events never run → the repeat is incomplete →
/// REFUSED, although the first pass succeeded. (The old completeness counted only the executions
/// that ran: `(Fail, complete)`, the repeat "complete", nothing written again → noted.)
#[test]
fn a_hook_failing_its_first_event_only_in_the_repeat_is_incomplete() {
    let r = rig("gr3");
    let id = RepairId::parse("R-1").unwrap();
    let live = r.pack.join("hooks/guard.sh");
    std::fs::write(&live, b"#!/bin/sh\nexit 0\n").unwrap();
    let count = r.dir.join("runs");
    let rel = "hooks/cys-hook.sh";
    assert_eq!(
        cys::repair::boot_critical::boot_critical_entry(rel).map(|e| e.check_arg),
        Some("PermissionRequest,PostToolUse,PreToolUse"),
        "the hook replays three events"
    );
    let staged = ledger::staging_dir_in(&r.root, &id).join(rel);
    std::fs::create_dir_all(staged.parent().unwrap()).unwrap();
    std::fs::write(
        &staged,
        format!(
            "#!/bin/sh\ncat >/dev/null\nn=$(cat '{c}' 2>/dev/null || echo 0)\nn=$((n+1))\necho \"$n\" > '{c}'\n[ \"$n\" -eq 4 ] && exit 1\n[ \"$n\" -eq 3 ] && printf 'x\\n' >> '{l}'\nexit 0\n",
            c = count.display(),
            l = live.display()
        ),
    )
    .unwrap();
    cys::repair::replay::set_mode(&staged, 0o755);
    let s3 = Step3Input {
        root: r.root.clone(),
        repair: id,
        pack: r.pack.clone(),
        states: vec![(rel.to_string(), TargetState::read(&staged).unwrap())],
        windows: false,
    };
    let home = r.dir.join("realhome");
    std::fs::create_dir_all(&home).unwrap();
    let e = production_step3_with(&s3, &home, &|_, _| crate::repair::Filed::New).unwrap_err();
    assert!(e.why.contains("끝나지 않아"), "{}", e.why);
    assert_eq!(
        std::fs::read_to_string(&count).unwrap().trim(),
        "4",
        "three events in the first pass, the repeat ended at its first"
    );
}

/// RULINGS EO5 (b) → GA8 (2): the owner's line for an escape refusal follows the ledger's answer that
/// the refusal's row is NEW — the same candidate refused again (a restart in between included: the
/// test keeps NO memory between the runs, only the ledger on disk) tells nobody again, while the row
/// is written each time. (Old form: an in-memory `escape_told` set, emptied by every restart; old
/// expectation `master.len() == 1, intake.len() == 2` over a canned step 3 — the counts below are the
/// same, now from the real ledger.)
#[test]
fn escape_line_once_per_candidate() {
    let r = rig("eo5b");
    let s3 = escaping_candidate(&r);
    let home = r.dir.join("realhome");
    std::fs::create_dir_all(&home).unwrap();
    let rows = Mutex::new(Vec::<crate::repair::Filed>::new());
    let ledger = |f: &str, l: &str| {
        let a = crate::repair::file_intake_in(&r.ctx(T0), f, l);
        rows.lock().unwrap().push(a.clone());
        a
    };
    let first = production_step3_with(&s3, &home, &ledger).unwrap_err();
    let again = production_step3_with(&s3, &home, &ledger).unwrap_err();
    assert!(first.owner_line.is_some(), "a new row: one pointer — {first:?}");
    assert!(again.owner_line.is_none(), "the ledger says not new: no second line — {again:?}");
    assert_eq!(
        *rows.lock().unwrap(),
        // RULINGS GR6 renamed the existing row's answer (`NotNew` → `Exists`; quota / excluded are
        // `Failed` now)
        vec![crate::repair::Filed::New, crate::repair::Filed::Exists],
        "the row is written each time"
    );
}

/// A candidate whose hook writes a LIVE listed file of the lane on every run (CZ3's sequence).
fn escaping_candidate(r: &Rig) -> Step3Input {
    let id = RepairId::parse("R-1").unwrap();
    let live = r.pack.join("hooks/guard.sh");
    std::fs::write(&live, b"#!/bin/sh\nexit 0\n").unwrap();
    let rel = "hooks/appbuild-gate.sh";
    let staged = ledger::staging_dir_in(&r.root, &id).join(rel);
    std::fs::create_dir_all(staged.parent().unwrap()).unwrap();
    std::fs::write(&staged, format!("#!/bin/sh\nprintf 'x\\n' >> '{}'\nexit 0\n", live.display())).unwrap();
    cys::repair::replay::set_mode(&staged, 0o755);
    Step3Input {
        root: r.root.clone(),
        repair: id,
        pack: r.pack.clone(),
        states: vec![(rel.to_string(), TargetState::read(&staged).unwrap())],
        windows: false,
    }
}

/// RULINGS GA8 (1) / (2): step 3 writes what its comparison found FIRST — before it returns, whatever
/// follows — and a write that fails marks nothing: no pointer then, and the next identical step 3
/// writes the row (new) and points once.
#[test]
fn step3_writes_the_row_first_and_a_failed_write_marks_nothing() {
    let r = rig("ga8-s3");
    let s3 = escaping_candidate(&r);
    let home = r.dir.join("realhome");
    std::fs::create_dir_all(&home).unwrap();
    let failed = production_step3_with(&s3, &home, &|_, _| {
        crate::repair::Filed::Failed("the ledger could not be written".into())
    })
    .unwrap_err();
    assert!(failed.owner_line.is_none(), "{failed:?}");
    assert_eq!(failed.intake.len(), 1, "the found list is carried: {failed:?}");
    let ledger = |f: &str, l: &str| crate::repair::file_intake_in(&r.ctx(T0), f, l);
    let next = production_step3_with(&s3, &home, &ledger).unwrap_err();
    assert!(next.owner_line.is_some(), "nothing was marked by the failure: {next:?}");
}

/// RULINGS GR6 — step 3: a one-off finding (written in the first pass only → "noted") whose intake row
/// could not be written stops step 3 — no approval follows; the answer says so and the found list
/// travels in it. (The old step 3 discarded the intake's answer and went on to the screen and the
/// card.)
#[test]
fn step3_refuses_a_finding_that_could_not_be_recorded() {
    let r = rig("gr6-s3");
    let id = RepairId::parse("R-1").unwrap();
    let live = r.pack.join("hooks/guard.sh");
    std::fs::write(&live, b"#!/bin/sh\nexit 0\n").unwrap();
    let marker = r.dir.join("wrote-once");
    let rel = "hooks/appbuild-gate.sh";
    let staged = ledger::staging_dir_in(&r.root, &id).join(rel);
    std::fs::create_dir_all(staged.parent().unwrap()).unwrap();
    std::fs::write(
        &staged,
        format!(
            "#!/bin/sh\nif [ ! -f '{m}' ]; then : > '{m}'; printf 'x\\n' >> '{l}'; fi\nexit 0\n",
            m = marker.display(),
            l = live.display()
        ),
    )
    .unwrap();
    cys::repair::replay::set_mode(&staged, 0o755);
    let s3 = Step3Input {
        root: r.root.clone(),
        repair: id,
        pack: r.pack.clone(),
        states: vec![(rel.to_string(), TargetState::read(&staged).unwrap())],
        windows: false,
    };
    let home = r.dir.join("realhome");
    std::fs::create_dir_all(&home).unwrap();
    let e = production_step3_with(&s3, &home, &|_, _| {
        crate::repair::Filed::Failed("injected: the ledger cannot be written".into())
    })
    .unwrap_err();
    assert!(e.why.contains(crate::repair::UNRECORDED_FINDING), "{}", e.why);
    assert!(e.why.contains("바뀌었으나"), "{}", e.why);
    assert_eq!(e.intake.len(), 1, "the found list travels in the answer: {e:?}");
    assert!(e.owner_line.is_none());
}

/// RULINGS GA8 — the worker's `check`: the comparison's row is written within the call (no helper
/// thread), the pointer follows only a NEW row and is sent once; a failed write sends nothing and
/// marks nothing; a noted difference is a row and never a pointer.
#[test]
fn check_comparison_writes_the_row_and_points_once() {
    use cys::repair::rehearsal::Comparison;
    let r = rig("ga8-check");
    let rel = "hooks/appbuild-gate.sh";
    let pointed = Mutex::new(Vec::<String>::new());
    let point = |t: String| pointed.lock().unwrap().push(t);
    let ledger = |f: &str, l: &str| crate::repair::file_intake_in(&r.ctx(T0), f, l);
    let refused = Comparison::Refused("예행이 실제 환경의 항목을 다시 썼습니다(hooks/guard.sh)".into());
    let fail = |_: &str, _: &str| crate::repair::Filed::Failed("injected".into());
    assert_eq!(
        check_consequence(rel, &refused, &fail, &point),
        Some(crate::repair::Filed::Failed("injected".into()))
    );
    assert!(pointed.lock().unwrap().is_empty(), "a failed write sends nothing");
    assert_eq!(check_consequence(rel, &refused, &ledger, &point), Some(crate::repair::Filed::New));
    assert_eq!(check_consequence(rel, &refused, &ledger, &point), Some(crate::repair::Filed::Exists));
    assert_eq!(pointed.lock().unwrap().len(), 1, "one pointer for one new row");
    let noted = Comparison::Noted("예행 동안 실제 환경의 항목이 바뀌었으나 다시 돌렸을 때는 바뀌지 않았습니다(x)".into());
    assert_eq!(check_consequence(rel, &noted, &ledger, &point), Some(crate::repair::Filed::New));
    assert_eq!(pointed.lock().unwrap().len(), 1, "a noted difference is no pointer");
    assert_eq!(check_consequence(rel, &Comparison::Clean, &ledger, &point), None);
    // the production consequence writes its row on the calling thread and keeps nothing in memory;
    // RULINGS JW2 (old expectation: `!body.contains("spawn_thread")`): only the POINTER goes out on a
    // thread of its own (no lock across a send), with no synchronous fallback
    let src = include_str!("repair_approval.rs");
    let body = src
        .split("pub(crate) fn comparison_consequence(")
        .nth(1)
        .and_then(|b| b.split("\n}\n").next())
        .unwrap();
    assert!(!body.contains("static "), "{body}");
    assert!(body.contains("&|f, l| crate::repair::file_intake(daemon, f, l)"), "{body}");
    let pointer = &body[body.find("&|text|").unwrap()..];
    assert!(
        pointer.contains("spawn_thread(") && pointer.matches("point_master_now(").count() == 1,
        "{pointer}"
    );
    // the pointer is one attempt: no retrying delivery, no loop
    let pm = src
        .split("fn point_master_now(")
        .nth(1)
        .and_then(|b| b.split("\n}\n").next())
        .unwrap();
    assert!(
        !pm.contains("deliver_once") && !pm.contains("for ") && !pm.contains("loop") && !pm.contains("while "),
        "{pm}"
    );
}

/// RULINGS GR5 — a worker's staged candidate far over the size limit (a 64 GiB sparse file) is refused
/// by `submit` at once with a sentence that says so (the old reader hashed it to its end while
/// holding the repair lock AND the approval table).
#[test]
fn submit_refuses_a_candidate_over_the_size_limit() {
    let r = rig("gr5-huge");
    let id = r.signed_repair();
    let t = Mutex::new(ApprovalTable::default());
    let eff = effects();
    let f = std::fs::OpenOptions::new().write(true).open(r.staged(&id)).unwrap();
    f.set_len(64 * 1024 * 1024 * 1024).unwrap();
    drop(f);
    let e = submit(&r.ctx(T0), &worker(), &json!({"repair": "R-1"}), &t, &eff, Ok(())).unwrap_err();
    assert!(e.message.contains("over the size limit"), "{}", e.message);
    assert!(t.lock().unwrap().requests.is_empty());
}

/// RULINGS GR5 — while `submit` reads a candidate (a reader made slow by a test double: it waits for
/// the test's release, no real time), the approval table is NOT held: the approval list and the sweep
/// answer. (The old `submit` read the candidate holding the table — both would have waited.)
#[test]
fn a_slow_candidate_read_holds_neither_the_list_nor_the_sweep() {
    let r = rig("gr5-slow");
    r.signed_repair();
    let t = Mutex::new(ApprovalTable::default());
    let eff = effects();
    // the restart's first sweep (which visits every repair under its lock) is done before
    sweep(&r.ctx(T0), &t, &eff);
    let (entered_tx, entered_rx) = std::sync::mpsc::channel::<()>();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    std::thread::scope(|s| {
        let h = s.spawn(|| {
            BEFORE_CANDIDATE_READ.with(|b| {
                *b.borrow_mut() = Some(Box::new(move || {
                    entered_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                }))
            });
            submit(&r.ctx(T0 + 1.0), &worker(), &json!({"repair": "R-1"}), &t, &eff, Ok(()))
        });
        entered_rx.recv().unwrap();
        // inside the candidate read now: the table is free, the list and the sweep answer
        assert!(t.try_lock().is_ok(), "the approval table is held during the candidate read");
        let l = approval_list(&r.ctx(T0 + 1.0), &t);
        assert!(l["requests"].as_array().is_some_and(|a| a.is_empty()), "{l}");
        sweep(&r.ctx(T0 + 2.0), &t, &eff);
        release_tx.send(()).unwrap();
        let out = h.join().unwrap();
        assert!(out.is_ok(), "{:?}", out.err().map(|e| e.message));
    });
    assert_eq!(t.lock().unwrap().requests.len(), 1);
}

/// RULINGS GA8 (2) on the unknown-approval finding: the repair is remembered as reported only after
/// the ledger answered — a failed write (the restart's first sweep) is tried again by the next submit,
/// then nothing more.
#[test]
fn unknown_finding_is_cached_only_after_the_ledger_answered() {
    let r = rig("ga8-unknown");
    let eff = effects();
    *eff.consumed_answer.lock().unwrap() =
        Some(cys::repair::consumed::ApprovalUse::Unknown("no evidence".into()));
    *eff.intake_answer.lock().unwrap() = Some(crate::repair::Filed::Failed("injected".into()));
    let (_id, _rid, t) = approved_then_restart(&r, &eff);
    let n0 = eff.intake.lock().unwrap().len();
    sweep(&r.ctx(T0 + 60.0), &t, &eff);
    let n1 = eff.intake.lock().unwrap().len();
    assert!(n1 > n0, "the finding was tried");
    assert!(t.lock().unwrap().unknown_reported.is_empty(), "a failed write marks nothing");
    *eff.intake_answer.lock().unwrap() = None;
    let submit_at = |at: f64| {
        let e = submit(&r.ctx(at), &worker(), &json!({"repair": "R-1"}), &t, &eff, Ok(())).unwrap_err();
        assert!(e.message.contains("알 수 없어"), "{}", e.message);
    };
    submit_at(T0 + 120.0);
    let n2 = eff.intake.lock().unwrap().len();
    assert_eq!(n2, n1 + 1, "tried again after the failure");
    assert!(t.lock().unwrap().unknown_reported.contains("R-1"), "remembered once the ledger answered");
    submit_at(T0 + 180.0);
    sweep(&r.ctx(T0 + 240.0), &t, &eff);
    assert_eq!(eff.intake.lock().unwrap().len(), n2, "then never again");
}

/// RULINGS CZ3: the rehearsal of step 3 runs INSIDE the scoped after-run comparison — a candidate hook
/// whose replay writes a LIVE listed file of the lane (by its absolute path) on every run is found:
/// the repeat writes it again → returned to the worker with one intake row and the owner's line,
/// before any screen runs. (The old step 3 compared nothing around its rehearsal.)
#[test]
fn step3_finds_a_rehearsal_writing_the_live_lane() {
    let r = rig("cz3");
    let id = RepairId::parse("R-1").unwrap();
    let live = r.pack.join("hooks/guard.sh");
    std::fs::write(&live, b"#!/bin/sh\nexit 0\n").unwrap();
    let rel = "hooks/appbuild-gate.sh";
    let staged = ledger::staging_dir_in(&r.root, &id).join(rel);
    std::fs::create_dir_all(staged.parent().unwrap()).unwrap();
    std::fs::write(
        &staged,
        format!("#!/bin/sh\nprintf 'x\\n' >> '{}'\nexit 0\n", live.display()),
    )
    .unwrap();
    cys::repair::replay::set_mode(&staged, 0o755);
    let s3 = Step3Input {
        root: r.root.clone(),
        repair: id.clone(),
        pack: r.pack.clone(),
        states: vec![(rel.to_string(), TargetState::read(&staged).unwrap())],
        windows: false,
    };
    let home = r.dir.join("realhome");
    std::fs::create_dir_all(&home).unwrap();
    let e = production_step3_with(&s3, &home, &|_, _| crate::repair::Filed::New).unwrap_err();
    assert!(e.why.contains("다시 썼습니다"), "{}", e.why);
    assert!(e.why.contains("guard.sh"), "{}", e.why);
    assert_eq!(e.intake.first().map(|(f, _)| f.as_str()), Some(rel));
    assert!(e.owner_line.as_deref().is_some_and(|l| l.contains("R-1")));
}

/// RULINGS EL4: a thread that cannot be made AND a helper slow to die, injected together — the
/// fallback reaps without blocking (WNOHANG + a fixed grace) and reports the pid it could not reap;
/// with the kill delivered the helper is reaped.
#[test]
fn thread_failure_and_a_slow_helper_never_block() {
    use cys::SpawnPolicy;
    let spawn = || {
        std::process::Command::new("/bin/sh")
            .args(["-c", "sleep 30"])
            .stdin(std::process::Stdio::null())
            .spawn_policy(cys::ChildLifetime::Attached)
            .spawn()
            .unwrap()
    };
    FAIL_SPAWN_FOR_TEST.with(|f| f.set(true));
    let child = spawn();
    let pid = child.id();
    // RULINGS EZ1: no real-clock premise — a blocking reap would return only when the helper's 30 s
    // sleep ended, reaped (`Err(None)`); the non-blocking one reports the pid it could not reap
    let r = start_waiter_or_reap(move || drop(child), pid, false);
    assert_eq!(r, Err(Some(pid)), "the slow helper is reported, not waited for");
    // the test's own clean-up: kill and reap that pid without blocking
    assert_eq!(kill_and_reap(pid, true), None);
    let child = spawn();
    let pid = child.id();
    assert_eq!(start_waiter_or_reap(move || drop(child), pid, true), Err(None));
    FAIL_SPAWN_FOR_TEST.with(|f| f.set(false));
}

/// RULINGS DF3: the report gate's set is the repairs the owner HAS A CARD for — not a request still
/// in step 3, not one whose step 3 refused (a fix-and-resubmit cycle), not an expired one.
#[test]
fn approval_cards_are_only_requests_past_step_3() {
    let r = rig("cards");
    r.signed_repair();
    let t = Mutex::new(ApprovalTable::default());
    let eff = effects();
    let lane = r.lane.0.clone();
    // in step 3: no card
    let (out, s3) = submit(&r.ctx(T0), &worker(), &json!({"repair": "R-1"}), &t, &eff, Ok(())).unwrap();
    let rid = out["request_id"].as_str().unwrap().to_string();
    assert!(approval_cards(&t, &lane).is_empty(), "a request in step 3 has no card");
    // step 3 refuses: still none, ever
    *eff.step3_fail.lock().unwrap() = Some("예행 검사 실패".into());
    run_step3(&r.ctx(T0), &t, &eff, &rid, &s3.unwrap());
    assert!(approval_cards(&t, &lane).is_empty(), "a refused step 3 offers nothing");
    // resubmitted, step 3 passes: the card
    *eff.step3_fail.lock().unwrap() = None;
    let rid = pending(&r, &t, &eff, T0 + 10.0);
    assert_eq!(approval_cards(&t, &lane), vec!["R-1".to_string()]);
    assert!(approval_cards(&t, "dept-other").is_empty(), "another lane's set");
    // expired: gone, although the record stays 승인 대기
    sweep(&r.ctx(T0 + 10.0 + 1801.0), &t, &eff);
    assert_eq!(state_of(&t, &rid), ReqState::Expired);
    assert!(approval_cards(&t, &lane).is_empty());
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

/// RULINGS JW2 — no lock across a send: the stale request's FINAL expiry, settled by a `submit` after a
/// restart (the budget used up), with the thread spawn failing (so the production effect would send
/// synchronously, right where it is called): at the moment of the send the approval table and the
/// repair's lock are BOTH free (probed by try-lock from the send seam). (The old `submit` called
/// `tell_master` inside both locks.)
#[test]
fn the_budget_notice_is_sent_after_both_locks_are_released() {
    let r = rig("jw2");
    let id = r.signed_repair();
    let eff = effects();
    let t = Mutex::new(ApprovalTable::default());
    pending(&r, &t, &eff, T0);
    drop(t);
    // E1 … E3 counted as in the CZ2 sequence, then a restart while S3 is pending
    let t = Mutex::new(ApprovalTable::default());
    let mut now = T0 + 1801.0;
    pending(&r, &t, &eff, now);
    sweep(&r.ctx(now + 1.0), &t, &eff);
    now += 2.0;
    for _ in 0..2 {
        now += 1801.0;
        sweep(&r.ctx(now), &t, &eff);
        now += 1.0;
        pending(&r, &t, &eff, now);
    }
    assert_eq!(r.rec(&id).resubmits, vec![(fp_of(&r, &id), 3)]);
    assert!(eff.master.lock().unwrap().is_empty());
    drop(t);
    let t = Arc::new(Mutex::new(ApprovalTable::default()));
    let (probe_table, probe_locks) = (t.clone(), r.mutexes.clone());
    *eff.send_probe.lock().unwrap() = Some(Box::new(move || {
        let table_free = probe_table.try_lock().is_ok();
        let repair_free = probe_locks
            .lock()
            .unwrap()
            .get("R-1")
            .map_or(true, |m| m.try_lock().is_ok());
        (table_free, repair_free)
    }));
    FAIL_SPAWN_FOR_TEST.with(|f| f.set(true));
    now += 1801.0;
    let e = submit(&r.ctx(now), &worker(), &json!({"repair": "R-1"}), &t, &eff, Ok(())).unwrap_err();
    FAIL_SPAWN_FOR_TEST.with(|f| f.set(false));
    assert!(e.message.contains("만료"), "{}", e.message);
    assert_eq!(r.rec(&id).resubmits, vec![(fp_of(&r, &id), 4)]);
    assert_eq!(eff.master.lock().unwrap().len(), 1, "the one line");
    assert_eq!(*eff.sends_free.lock().unwrap(), vec![(true, true)], "a lock was held across the send");
}

/// RULINGS KJ2 — "a notice is due" only after the settlement's write succeeded: the stale request's
/// FINAL expiry settled by a `submit` after a restart (the budget used up) while the record cannot be
/// written (its folder read-only) — repeated submits are refused, nothing is settled on disk and NO
/// notice is sent; the store recovers → the next submit saves the settlement and sends exactly ONE
/// line; a further submit sends none. (The old `submit` decided the notice before the write and
/// sent it although the write failed — so every later submit sent it again.)
#[cfg(unix)]
#[test]
fn the_budget_notice_waits_for_the_settlements_write() {
    use std::os::unix::fs::PermissionsExt;
    let r = rig("kj2");
    let id = r.signed_repair();
    let eff = effects();
    let t = Mutex::new(ApprovalTable::default());
    pending(&r, &t, &eff, T0);
    drop(t);
    // E1 … E3 counted as in the CZ2 sequence, then a restart while S3 is pending
    let t = Mutex::new(ApprovalTable::default());
    let mut now = T0 + 1801.0;
    pending(&r, &t, &eff, now);
    sweep(&r.ctx(now + 1.0), &t, &eff);
    now += 2.0;
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
    /// Gives the record's folder back its mode, also when an assertion fails first.
    struct Writable(PathBuf);
    impl Drop for Writable {
        fn drop(&mut self) {
            let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
        }
    }
    let folder = Writable(ledger::repair_dir_in(&r.root, &id));
    std::fs::set_permissions(&folder.0, std::fs::Permissions::from_mode(0o500)).unwrap();
    now += 1801.0;
    for n in 1..=3 {
        now += 1.0;
        let e = submit(&r.ctx(now), &worker(), &json!({"repair": "R-1"}), &t, &eff, Ok(())).unwrap_err();
        assert!(!e.message.contains("만료"), "submit {n}: refused for the budget although nothing was saved");
        assert_eq!(r.rec(&id).resubmits, vec![(fp_of(&r, &id), 3)], "submit {n}: settled on disk");
        assert!(eff.master.lock().unwrap().is_empty(), "submit {n}: a notice for an unsaved settlement");
    }
    drop(folder);
    now += 1.0;
    let e = submit(&r.ctx(now), &worker(), &json!({"repair": "R-1"}), &t, &eff, Ok(())).unwrap_err();
    assert!(e.message.contains("만료"), "{}", e.message);
    assert_eq!(r.rec(&id).resubmits, vec![(fp_of(&r, &id), 4)]);
    assert_eq!(eff.master.lock().unwrap().len(), 1, "exactly one line once the settlement is saved");
    now += 1.0;
    assert!(submit(&r.ctx(now), &worker(), &json!({"repair": "R-1"}), &t, &eff, Ok(())).is_err());
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

/// RULINGS DF5 / CZ2: the reconciliation clears an `approved` approval only when it was never
/// consumed — a request this table saw end `Approved` (applied) keeps it even if its record still
/// names the request (an end whose write failed).
#[test]
fn reconciliation_keeps_an_applied_approval() {
    let r = rig("recon-applied");
    let id = r.signed_repair();
    let eff = effects();
    let t = Mutex::new(ApprovalTable::default());
    let rid = pending(&r, &t, &eff, T0);
    let mut rec = r.rec(&id);
    rec.approval = Some(ApprovalResult {
        request_id: rid.clone(),
        fingerprint_sha256: fp_of(&r, &id),
        decided_at: T0 + 20.0,
        approved: true,
    });
    t.lock().unwrap().find_mut(&rid).unwrap().state = ReqState::Approved;
    let decided = reconcile_stale_pending(T0 + 30.0, &t.lock().unwrap(), "R-1", &mut rec, &|_| {
        cys::repair::consumed::ApprovalUse::Unknown("the table's own end decides".into())
    });
    assert_eq!(decided, Reconciled::Settled(None));
    assert_eq!(rec.pending_request, None);
    assert_eq!(rec.approval.map(|a| a.approved), Some(true), "the applied approval stays");
}

/// RULINGS EL5 — the record of a request whose approval was written, then a restart: the NEW table is
/// empty. Sets up R-1 with its pending request and an `approved` approval, and returns the fresh table.
fn approved_then_restart(r: &Rig, eff: &Rec) -> (RepairId, String, Mutex<ApprovalTable>) {
    let id = r.signed_repair();
    let t = Mutex::new(ApprovalTable::default());
    let rid = pending(r, &t, eff, T0);
    let mut rec = r.rec(&id);
    rec.approval = Some(ApprovalResult {
        request_id: rid.clone(),
        fingerprint_sha256: fp_of(r, &id),
        decided_at: T0 + 20.0,
        approved: true,
    });
    ledger::write_record_in(&r.root, &rec).unwrap();
    drop(t); // the crash
    (id, rid, Mutex::new(ApprovalTable::default()))
}

/// EL5 boundary 1 — crash BEFORE the apply: the seam answers NOT CONSUMED → CZ2's reconciliation (the
/// approval cleared; voided by the restart in time, not counted).
#[test]
fn crash_before_the_apply_is_reconciled_when_not_consumed() {
    let r = rig("el5-before");
    let eff = effects();
    *eff.consumed_answer.lock().unwrap() = Some(cys::repair::consumed::ApprovalUse::NotConsumed);
    let (id, _rid, t) = approved_then_restart(&r, &eff);
    sweep(&r.ctx(T0 + 60.0), &t, &eff);
    let rec = r.rec(&id);
    assert_eq!((rec.pending_request, rec.approval, rec.resubmits), (None, None, vec![]));
}

/// EL5 boundary 2 — crash AFTER the apply, before the settlement (past the request's deadline): the
/// seam answers CONSUMED → kept as applied: never deleted, never counted as an expiry, never usable
/// again (no request names it). (The old reconciliation, its table empty, deleted it and counted an
/// expiry.)
#[test]
fn crash_after_the_apply_keeps_the_consumed_approval() {
    let r = rig("el5-after");
    let eff = effects();
    *eff.consumed_answer.lock().unwrap() = Some(cys::repair::consumed::ApprovalUse::Consumed);
    let (id, rid, t) = approved_then_restart(&r, &eff);
    sweep(&r.ctx(T0 + 1900.0), &t, &eff);
    let rec = r.rec(&id);
    assert_eq!(rec.pending_request, None);
    assert_eq!(rec.resubmits, vec![], "never counted as an expiry");
    let a = rec.approval.expect("kept as applied");
    assert!(a.approved && a.request_id == rid);
    assert!(t.lock().unwrap().find(&rid).is_none(), "no request can use it again");
}

/// EL5 — UNKNOWN (the production answer until T6's body is merged): neither deleted nor usable, and
/// ONE finding — not one per sweep or per submit; a submit is refused meanwhile.
#[test]
fn unknown_consumption_is_neither_deleted_nor_usable_one_finding() {
    let r = rig("el5-unknown");
    let eff = effects();
    *eff.consumed_answer.lock().unwrap() =
        Some(cys::repair::consumed::ApprovalUse::Unknown("no evidence".into()));
    let (id, _rid, t) = approved_then_restart(&r, &eff);
    let before = r.rec(&id);
    let n0 = eff.intake.lock().unwrap().len();
    sweep(&r.ctx(T0 + 60.0), &t, &eff);
    let after = r.rec(&id);
    assert_eq!((after.pending_request.clone(), after.approval.clone()), (before.pending_request.clone(), before.approval.clone()));
    assert_eq!(eff.intake.lock().unwrap().len(), n0 + 1, "one finding");
    for k in 0..2 {
        let e = submit(&r.ctx(T0 + 70.0 + f64::from(k)), &worker(), &json!({"repair": "R-1"}), &t, &eff, Ok(())).unwrap_err();
        assert!(e.message.contains("알 수 없어"), "{}", e.message);
    }
    sweep(&r.ctx(T0 + 2000.0), &t, &eff);
    assert_eq!(eff.intake.lock().unwrap().len(), n0 + 1, "never one per sweep or submit");
    assert_eq!(r.rec(&id).approval, before.approval);
}

/// EL5 / EP: until T6's body is merged the seam's production answer is UNKNOWN.
#[test]
fn the_seam_answers_unknown_until_t6_fills_it() {
    let r = rig("el5-seam");
    assert!(matches!(
        cys::repair::consumed::approval_consumed_in(&r.root, &r.lane, "rq-0"),
        cys::repair::consumed::ApprovalUse::Unknown(_)
    ));
}

/// RULINGS CZ2 — deterministic interleaving: `first` runs on this thread; at its record write (after
/// its claim, inside the repair lock) the `others` are started on their own threads and given up to
/// 300 ms each before the write goes on — exactly the window the old check-then-act let a second end
/// through.
fn interleave<'a>(first: &dyn Fn(), others: Vec<Box<dyn Fn() + Send + Sync + 'a>>) {
    let (go_tx, go_rx) = std::sync::mpsc::channel::<()>();
    let go_rx = Mutex::new(go_rx);
    let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
    std::thread::scope(|s| {
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
        let verified = wait_helper(child, &rid, &nonce);
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
    // RULINGS CZ8: the artifact check runs on the EXACT default-build artifact the release ships —
    // the release mac leg's artifact gate (`scripts/check-no-ioreport-link.sh` on
    // `cys.app/Contents/MacOS/cysd`) refuses the double's knob in it (exit 1) and a missing or
    // unreadable artifact (exit 2: not measured is not passed); the harness's `ioreport-link` suite
    // runs the same gate on the default debug build of this source. (The old block here silently
    // passed when its guessed path was absent or unreadable.)
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let gate = std::fs::read_to_string(root.join("scripts/check-no-ioreport-link.sh")).unwrap();
    for want in [
        "knob=\"CYS_REPAIR_TEST\"\"_AUTH_ANSWER\"",
        "grep -a -F -q \"$knob\" \"$bin\"",
        "if [ ! -r \"$bin\" ]; then",
        "if [ ! -f \"$bin\" ]; then",
    ] {
        assert!(gate.contains(want), "the artifact gate lost: {want}");
    }
    let release = std::fs::read_to_string(root.join(".github/workflows/release.yml")).unwrap();
    assert!(
        release.contains("bash scripts/check-no-ioreport-link.sh \"$SRC/macos/cys.app/Contents/MacOS/cysd\""),
        "the release leg no longer runs the artifact gate on the shipped cysd"
    );
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
