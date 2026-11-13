//! Track T4 daemon tests (`src/bin/cysd/repair_grant_tests.rs`, declared test-only from `repair.rs`
//! through `#[path]`) — the owner-decision verbs, the rule-document branch of `begin`, `rule_doc_allowed`
//! and the CSO revision-marker refusal, all through `handle_with` (the role table first, then the body).
//! Every test builds its own ledger root and pack under the temporary directory; the per-seat prompt
//! record is filled with T3's own `apply_prompt` (as `hook.machine_origin` does); the grant claim runs
//! T4's real start protocol on the test root (`grant::try_claim_in`).

use super::*;
use cys::repair::boot_critical::CheckKind;
use cys::repair::grant::{self, ClaimEnv, ClaimStep, GrantDecision, GrantState, HookAction};
use cys::repair::ledger::{self, RepairRecord};
use cys::repair::owner_origin as oo;
use cys::repair::roles::CallerKind;
use cys::repair::types::{Classification, LockOwner, RepairId, RepairState, Trigger};
use serde_json::json;
use std::sync::atomic::{AtomicU64, Ordering};

const DAY: &str = "2026-11-13";
static SEQ: AtomicU64 = AtomicU64::new(0);

struct Rig {
    root: PathBuf,
    pack: PathBuf,
    registry: PathBuf,
    mutexes: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    prompts: Mutex<HashMap<u64, PromptOrigin>>,
    lane: LaneKey,
    inc: String,
}

const CSO_HEAD: &str = "# CSO ABSOLUTE DIRECTIVE\n<!-- cso-directive-rev: 2026-09-06-alert-inbox -->\n\nbody line\n";

fn rig(tag: &str) -> Rig {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("cys-t4d-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let pack = dir.join("pack");
    std::fs::create_dir_all(pack.join("bin")).unwrap();
    std::fs::create_dir_all(pack.join("directives")).unwrap();
    for f in ["bin/a.py", "bin/b.py", "bin/c.py"] {
        std::fs::write(pack.join(f), b"print('x')\n").unwrap();
    }
    std::fs::write(pack.join("soul.md"), b"# soul\nowner name\n").unwrap();
    std::fs::write(pack.join("directives/MASTER_DIRECTIVE.md"), b"# master\n").unwrap();
    std::fs::write(pack.join("directives/CSO_DIRECTIVE.md"), CSO_HEAD).unwrap();
    Rig {
        root: dir.join("local").join("repairs"),
        pack,
        registry: dir.join("depts.json"),
        mutexes: Mutex::new(HashMap::new()),
        prompts: Mutex::new(HashMap::new()),
        lane: LaneKey("base".into()),
        inc: "inc-1".into(),
    }
}

fn general(_rel: &str) -> Result<Classification, RepairError> {
    Ok(Classification::General)
}

fn pass(_rel: &str, _live: &Path, _d: std::time::Duration) -> Result<CheckReport, RepairError> {
    Ok(CheckReport {
        passed: true,
        timed_out: false,
        failure: String::new(),
        check_ms: 1,
        reference_kind: CheckKind::NonePrerehearsal,
        reference_ms: 1,
    })
}

fn cont(_: ClaimStep) -> HookAction {
    HookAction::Continue
}

impl Rig {
    fn call_with(
        &self,
        now: f64,
        classify: &dyn Fn(&str) -> Result<Classification, RepairError>,
        who: &Caller,
        sub: &str,
        p: Value,
    ) -> Result<Value, VerbError> {
        let clock = move || now;
        let today = || DAY.to_string();
        let claim = |o: &LockOwner, id: &RepairId| {
            grant::try_claim_in(
                &ClaimEnv {
                    root: &self.root,
                    clock: &clock,
                    today: &today,
                    hook: &cont,
                },
                o,
                id,
            )
        };
        let ctx = Ctx {
            root: self.root.clone(),
            lane: self.lane.clone(),
            incarnation: self.inc.clone(),
            pack_dir: self.pack.clone(),
            now,
            windows: false,
            registry: self.registry.clone(),
            mutexes: &self.mutexes,
            seats: vec![(7, "reviewer-codex".into())],
            classify,
            check: &pass,
            hook_step: &cys::repair::apply::hook_lines_step,
            claim: &claim,
            prompts: &self.prompts,
        };
        handle_with(&ctx, who, sub, &p)
    }
    fn call(&self, now: f64, who: &Caller, sub: &str, p: Value) -> Result<Value, VerbError> {
        self.call_with(now, &general, who, sub, p)
    }
    /// What `hook.machine_origin` records for a prompt to seat `sid` (T3's own pure function).
    fn prompt(&self, sid: u64, text: &str, origin: &str, at: f64) {
        let mut m = self.prompts.lock().unwrap();
        let e = m.entry(sid).or_default();
        apply_prompt(e, &cys::mission_gate::digest_normalized(text), text, origin, at);
    }
    fn decision(&self) -> Option<GrantDecision> {
        grant::read_decision_in(&self.root).unwrap()
    }
    fn ids(&self) -> Vec<RepairId> {
        ledger::list_ids_in(&self.root)
    }
    fn rec(&self, id: &str) -> RepairRecord {
        ledger::read_record_in(&self.root, &RepairId::parse(id).unwrap())
            .unwrap()
            .unwrap()
    }
    fn grant_on(&self, cap: u32) {
        grant::write_decision_in(
            &self.root,
            &GrantDecision {
                schema: 1,
                state: GrantState::On,
                daily_cap: cap,
                decided_at: 1.0,
                decided_lane: self.lane.clone(),
                decided_seat: 1,
                prompt_digest: "d".into(),
                prompt_excerpt: "예".into(),
                paused_at: None,
                resumed_at: None,
            },
        )
        .unwrap();
    }
    fn claims(&self) -> usize {
        std::fs::read_dir(self.root.join("grant-seq"))
            .map(|rd| {
                rd.flatten()
                    .filter(|e| e.file_name().to_string_lossy().ends_with(".json"))
                    .count()
            })
            .unwrap_or(0)
    }
}

fn seat(sid: u64, role: &str) -> Caller {
    Caller {
        kind: CallerKind::Seat { sid },
        role: Some(role.into()),
        app_token: false,
        bound_family: role_family(role),
    }
}

fn now0() -> f64 {
    cys::repair::ledger::now_epoch()
}

const MASTER: u64 = 1;

fn master() -> Caller {
    seat(MASTER, "master")
}

/// The master gets a token for `rel` the Q2 (a) way: the owner typed "고쳐라 <rel>".
fn token_for(r: &Rig, rel: &str, t: f64) -> String {
    r.prompt(MASTER, &format!("고쳐라 {rel}"), "human", t);
    let v = r
        .call(t + 1.0, &master(), "owner_token", json!({"path": rel}))
        .unwrap();
    v["token"].as_str().unwrap().to_string()
}

// ───────────────────────────── D:1105 (7) through `begin --trigger grant` ─────────────────────────────

/// Off → 0 starts (also with open defects in the ledger — a boot without a mission): the worker's
/// `begin --trigger grant` is refused with the off sentence; no record, no claim.
#[test]
fn grant_off_refuses_every_autonomous_begin() {
    let r = rig("off");
    let w = seat(3, "worker");
    let t = now0();
    assert!(r
        .call(t, &w, "report", json!({"file": "bin/a.py", "error": "SyntaxError: x"}))
        .is_ok());
    let e = r
        .call(t, &w, "begin", json!({"trigger": "grant", "files": ["bin/a.py"]}))
        .unwrap_err();
    assert_eq!(e.code, "repair_precondition");
    assert!(e.message.contains(grant::GRANT_OFF_KO), "{}", e.message);
    assert!(r.ids().is_empty());
    assert_eq!(r.claims(), 0);
    // the owner's "고쳐라" (owner-order) is never governed by the grant
    assert!(r
        .call(t, &w, "begin", json!({"trigger": "owner_order", "files": ["bin/a.py"]}))
        .is_ok());
    assert_eq!(r.claims(), 0);
}

/// On → one concurrent (another worker's grant start is `Busy`), stops at the daily cap of distinct
/// repairs; `begin` answers T3's sentences for T4's verdicts.
#[test]
fn grant_on_one_at_a_time_and_daily_cap() {
    let r = rig("on");
    r.grant_on(2);
    let t = now0();
    let a = r
        .call(t, &seat(3, "worker"), "begin", json!({"trigger": "grant", "files": ["bin/a.py"]}))
        .unwrap();
    let a_id = a["repair"].as_str().unwrap().to_string();
    assert_eq!(r.rec(&a_id).lock.unwrap().claim, Some(1));
    let e = r
        .call(t, &seat(4, "worker-2"), "begin", json!({"trigger": "grant", "files": ["bin/b.py"]}))
        .unwrap_err();
    assert_eq!(
        (e.code.as_str(), e.message.as_str()),
        ("repair_locked", "repair.begin: 다른 자동 수리가 진행 중입니다(동시 1건)")
    );
    assert_eq!(r.ids().len(), 1);
    // A is committed (closed state, no lock): the next start closes claim 1
    let mut x = r.rec(&a_id);
    x.state = RepairState::Probation;
    x.lock = None;
    ledger::write_record_in(&r.root, &x).unwrap();
    let b = r
        .call(t, &seat(4, "worker-2"), "begin", json!({"trigger": "grant", "files": ["bin/b.py"]}))
        .unwrap();
    let b_id = b["repair"].as_str().unwrap().to_string();
    assert_eq!(r.rec(&b_id).lock.unwrap().claim, Some(2));
    let mut x = r.rec(&b_id);
    x.state = RepairState::Reverted;
    x.lock = None;
    ledger::write_record_in(&r.root, &x).unwrap();
    let e = r
        .call(t, &seat(5, "worker-3"), "begin", json!({"trigger": "grant", "files": ["bin/c.py"]}))
        .unwrap_err();
    assert_eq!(
        (e.code.as_str(), e.message.as_str()),
        ("repair_precondition", "repair.begin: 오늘의 자동 수리 한도에 이르렀습니다")
    );
    assert_eq!(r.claims(), 2, "failures count, the cap is never exceeded");
}

/// Owner input → immediate stop (B4): while paused a grant start is refused; an owner-ordered repair is
/// never paused; an R′ (`--resolves`) of a grant repair takes no claim and is not paused (R8).
#[test]
fn owner_input_pauses_grant_starts_only() {
    let r = rig("pause");
    r.grant_on(5);
    let t = now0();
    let w = seat(3, "worker");
    let a = r
        .call(t, &w, "begin", json!({"trigger": "grant", "files": ["bin/a.py"]}))
        .unwrap();
    let a_id = a["repair"].as_str().unwrap().to_string();
    assert_eq!(r.claims(), 1);
    // A is committed (its claim becomes closable) — the next grant start is judged on the pause only
    let mut x = r.rec(&a_id);
    x.state = RepairState::Probation;
    x.lock = None;
    ledger::write_record_in(&r.root, &x).unwrap();
    assert!(grant::note_owner_input_in(&r.root, &r.lane, t + 1.0).unwrap());
    let e = r
        .call(t + 2.0, &seat(4, "worker-2"), "begin", json!({"trigger": "grant", "files": ["bin/b.py"]}))
        .unwrap_err();
    assert_eq!(e.message, "repair.begin: 자동 수리 허락이 일시정지 중입니다");
    assert!(!grant::may_open_new_round_in(&r.root, t + 2.0).unwrap());
    assert!(r
        .call(t + 2.0, &seat(4, "worker-2"), "begin", json!({"trigger": "owner_order", "files": ["bin/b.py"]}))
        .is_ok());
    let v = r
        .call(
            t + 3.0,
            &seat(5, "worker-3"),
            "begin",
            json!({"trigger": "grant", "files": ["bin/c.py"], "resolves": a_id}),
        )
        .unwrap();
    assert_eq!(v["resolves"], json!(a_id));
    assert_eq!(r.claims(), 1, "an R′ takes no claim and no cap unit");
}

/// K3: rounds 2…10 of one grant repair run under the SAME claim — no new claim file, no cap use,
/// never `Busy` against its own claim (a retried `begin` of the holder finds its own start); while
/// paused (B4) a new round is refused. (T3's `record` asks `grant::may_open_new_round` on the
/// production root, so this test points `CYS_LOCAL_DIR` at the rig under the env lock.)
#[test]
fn k3_rounds_run_under_the_same_claim() {
    let _g = crate::governance::PACK_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let r = rig("k3");
    let _env = cys::pack::EnvGuard::set("CYS_LOCAL_DIR", r.root.parent().unwrap());
    assert_eq!(cys::repair::paths::repairs_root(), r.root);
    r.grant_on(1);
    let t = now0();
    let w = seat(3, "worker");
    let id = r
        .call(t, &w, "begin", json!({"trigger": "grant", "files": ["bin/a.py"]}))
        .unwrap()["repair"]
        .as_str()
        .unwrap()
        .to_string();
    let rv = seat(7, "reviewer-codex");
    for round in 1..=10u32 {
        let at = t + f64::from(round) * 10.0;
        std::fs::write(r.pack.join("bin/a.py"), format!("print({round})\n")).unwrap();
        let v = r
            .call(at, &w, "record", json!({"repair": id, "note": format!("round {round}")}))
            .unwrap();
        assert_eq!(v["attempt"], json!(round), "{v}");
        r.call(at + 1.0, &rv, "attest_verify", json!({"repair": id, "verdict": "fail", "note": "again"}))
            .unwrap();
        assert_eq!(r.claims(), 1, "round {round}: a round takes no claim");
        assert_eq!(r.rec(&id).lock.unwrap().claim, Some(1));
        // the holder's retried begin finds its own start — never Busy against its own claim
        let again = r
            .call(at + 2.0, &w, "begin", json!({"trigger": "grant", "files": ["bin/a.py"]}))
            .unwrap();
        assert_eq!(again["begin"], "already-held");
    }
    assert_eq!(r.claims(), 1);
    // the cap (1) is not used up by the rounds: the decision is unchanged, and the pause refuses a
    // new round (B4)
    let r2 = rig("k3-pause");
    let _env2 = cys::pack::EnvGuard::set("CYS_LOCAL_DIR", r2.root.parent().unwrap());
    r2.grant_on(5);
    let id2 = r2
        .call(t, &w, "begin", json!({"trigger": "grant", "files": ["bin/b.py"]}))
        .unwrap()["repair"]
        .as_str()
        .unwrap()
        .to_string();
    std::fs::write(r2.pack.join("bin/b.py"), b"print(1)\n").unwrap();
    r2.call(t + 1.0, &w, "record", json!({"repair": id2, "note": "r1"})).unwrap();
    r2.call(t + 2.0, &rv, "attest_verify", json!({"repair": id2, "verdict": "fail", "note": "x"}))
        .unwrap();
    assert!(grant::note_owner_input_in(&r2.root, &r2.lane, now0()).unwrap());
    let e = r2
        .call(t + 3.0, &w, "record", json!({"repair": id2, "note": "r2"}))
        .unwrap_err();
    assert!(e.message.contains("새 라운드를 열 수 없습니다"), "{}", e.message);
    assert!(e.message.contains("일시정지"), "{}", e.message);
    assert_eq!(r2.rec(&id2).attempt, 1);
    // CH7: with the grant OFF the refusal says so (not "일시정지")
    let mut d = r2.decision().unwrap();
    d.state = GrantState::Off;
    grant::write_decision_in(&r2.root, &d).unwrap();
    let e = r2
        .call(t + 4.0, &w, "record", json!({"repair": id2, "note": "r2"}))
        .unwrap_err();
    assert!(e.message.contains("자동 수리 허락이 꺼져 있습니다"), "{}", e.message);
    assert!(!e.message.contains("일시정지"), "{}", e.message);
    assert_eq!(r2.rec(&id2).attempt, 1);
}

// ───────────────────────────── grant on / off (Q1, T3, T5, U4) ─────────────────────────────

/// `grant on` only after `grant ask` and the owner's FIRST typed answer, affirmative, with no machine
/// prompt between, within 30 minutes; every refusal names its reason; nothing else is written and no
/// `R-*` record is touched (F10).
#[test]
fn grant_on_needs_the_registered_question_and_an_owner_typed_yes() {
    let r = rig("granton");
    let t = now0();
    let on = |at: f64| r.call(at, &master(), "grant", json!({"mode": "on"}));
    // no question → the step is named (T5)
    let e = on(t).unwrap_err();
    assert_eq!(e.code, "repair_precondition");
    assert!(e.message.contains("먼저 `cys repair grant ask`로 질문을 등록하세요"), "{}", e.message);
    // a machine "예" after the question → off
    r.call(t, &master(), "grant_ask", json!({})).unwrap();
    r.prompt(MASTER, "예", "machine", t + 1.0);
    assert!(on(t + 2.0).unwrap_err().message.contains("기계"));
    // an unknown-origin prompt counts as not the owner's (fail closed)
    r.call(t + 3.0, &master(), "grant_ask", json!({})).unwrap();
    r.prompt(MASTER, "예", "unknown", t + 4.0);
    assert!(on(t + 5.0).is_err());
    // an unrelated owner line → off
    r.call(t + 6.0, &master(), "grant_ask", json!({})).unwrap();
    r.prompt(MASTER, "오늘 일정 정리해줘", "human", t + 7.0);
    r.prompt(MASTER, "예", "human", t + 8.0); // not the FIRST answer
    assert!(on(t + 9.0).unwrap_err().message.contains("허락"));
    // an answer typed BEFORE the question is ignored
    r.prompt(MASTER, "예", "human", t + 10.0);
    r.call(t + 11.0, &master(), "grant_ask", json!({})).unwrap();
    assert!(on(t + 12.0).unwrap_err().message.contains("아직 없습니다"));
    // a 31-minute-old question has lapsed
    r.prompt(MASTER, "예", "human", t + 13.0);
    assert!(on(t + 11.0 + 31.0 * 60.0).unwrap_err().message.contains("만료"));
    assert_eq!(r.decision(), None, "no refused call wrote the decision");
    // the owner's typed "네" → on
    r.call(t + 2_000.0, &master(), "grant_ask", json!({})).unwrap();
    r.prompt(MASTER, "네", "human", t + 2_001.0);
    let v = r
        .call(t + 2_002.0, &master(), "grant", json!({"mode": "on", "daily_cap": 3}))
        .unwrap();
    assert_eq!(v["grant"], "on");
    let d = r.decision().unwrap();
    assert_eq!((d.state, d.daily_cap, d.decided_seat), (GrantState::On, 3, MASTER));
    assert_eq!(d.decided_lane, r.lane);
    assert_eq!(d.prompt_excerpt, "네");
    assert_eq!(d.prompt_digest, cys::mission_gate::digest_normalized("네"));
    assert_eq!(d.paused_at, Some(t + 2_001.0), "the answer is an owner input (B4)");
    assert!(r.prompts.lock().unwrap()[&MASTER].question.is_none(), "the question is consumed");
    assert!(on(t + 2_003.0).is_err(), "one answer turns it on once");
    assert!(r.ids().is_empty(), "no R-* record");
    // a bad cap is refused
    let e = r
        .call(t, &master(), "grant", json!({"mode": "on", "daily_cap": 0}))
        .unwrap_err();
    assert_eq!(e.code, "invalid_params");
}

/// U4(b): an owner "예" to an OWNER-TOKEN question never turns the grant on.
#[test]
fn a_token_questions_yes_is_not_a_grant() {
    let r = rig("u4b");
    let t = now0();
    r.call(t, &master(), "owner_token_ask", json!({"path": "soul.md"}))
        .unwrap();
    r.prompt(MASTER, "예", "human", t + 1.0);
    let e = r
        .call(t + 2.0, &master(), "grant", json!({"mode": "on"}))
        .unwrap_err();
    assert!(e.message.contains("규칙 문서 표 질문"), "{}", e.message);
    assert_eq!(r.decision(), None);
}

/// `grant off` needs an owner-typed prompt after the last decision (the safe direction, Q1).
#[test]
fn grant_off_after_an_owner_typed_prompt() {
    let r = rig("off2");
    let t = now0();
    r.call(t, &master(), "grant_ask", json!({})).unwrap();
    r.prompt(MASTER, "응", "human", t + 1.0);
    r.call(t + 2.0, &master(), "grant", json!({"mode": "on"}))
        .unwrap();
    let off = |at: f64| r.call(at, &master(), "grant", json!({"mode": "off"}));
    assert!(off(t + 3.0).is_err(), "no owner input after the decision");
    r.prompt(MASTER, "worker report", "machine", t + 4.0);
    assert!(off(t + 5.0).is_err(), "a machine prompt is not the owner's");
    r.prompt(MASTER, "자동 수리 꺼", "human", t + 6.0);
    assert_eq!(off(t + 7.0).unwrap()["grant"], "off");
    assert_eq!(r.decision().unwrap().state, GrantState::Off);
}

/// J1 / AK1: the owner-decision verbs are a master seat's only (a worker is refused; a worker pane
/// re-declared master is refused with the way back).
#[test]
fn owner_decision_verbs_master_seat_only() {
    let r = rig("roles");
    for verb in ["grant", "grant_ask", "owner_token", "owner_token_ask"] {
        let e = r
            .call(now0(), &seat(3, "worker"), verb, json!({"mode": "on", "path": "soul.md"}))
            .unwrap_err();
        assert_eq!(e.code, "acl_denied", "{verb}");
        let redeclared = Caller {
            kind: CallerKind::Seat { sid: 9 },
            role: Some("master".into()),
            app_token: false,
            bound_family: Some(RoleFamily::Worker),
        };
        let e = r
            .call(now0(), &redeclared, verb, json!({"mode": "on", "path": "soul.md"}))
            .unwrap_err();
        assert!(e.message.contains("AD1"), "{verb}: {}", e.message);
    }
}

// ───────────────────────────── the owner-instruction token (Q2, T3, U4) ─────────────────────────────

/// "고쳐라 soul.md" → a token for `soul.md` only; a machine or unknown prompt, a seat with no owner
/// input (codex / agy — no prompt hook), another document, a non-rule file → no token; nothing but the
/// token file is written.
#[test]
fn owner_token_only_for_the_document_the_owner_named() {
    let r = rig("token");
    let t = now0();
    let tok = |sid: u64, path: &str| {
        r.call(t + 1.0, &seat(sid, "master"), "owner_token", json!({"path": path}))
    };
    // a seat whose agent has no prompt hook has no record at all
    assert!(tok(MASTER, "soul.md").unwrap_err().message.contains("기록이 없습니다"));
    r.prompt(MASTER, "고쳐라 soul.md", "machine", t);
    assert!(tok(MASTER, "soul.md").is_err(), "machine-origin prompt → no token");
    r.prompt(MASTER, "고쳐라 soul.md", "unknown", t);
    assert!(tok(MASTER, "soul.md").is_err(), "unknown origin → no token");
    r.prompt(MASTER, "고쳐라 soul.md", "human", t);
    let v = tok(MASTER, "soul.md").unwrap();
    let token = v["token"].as_str().unwrap();
    assert!(oo::is_token_shape(token));
    assert_eq!(v["ticket_line"], format!("주인 지시 표: {token} (규칙 문서 soul.md)"));
    let rec = oo::read_token_in(&r.root, &oo::token_sha256(token)).unwrap().unwrap();
    assert_eq!(rec.paths, ["soul.md"]);
    assert_eq!((rec.lane_key.clone(), rec.issued_to_seat), (r.lane.clone(), MASTER));
    assert_eq!(rec.expires_at, t + 1.0 + 86_400.0);
    assert_eq!(rec.used_by_repair, None);
    assert!(!std::fs::read_to_string(oo::token_path_in(&r.root, &rec.token_sha256))
        .unwrap()
        .contains(token), "the plain token is never stored");
    let e = tok(MASTER, "directives/MASTER_DIRECTIVE.md").unwrap_err();
    assert!(e.message.contains("이름으로 부르지"), "{}", e.message);
    assert_eq!(tok(MASTER, "bin/a.py").unwrap_err().code, "invalid_params");
    // an absolute path inside the pack names the same document
    let abs = r.pack.join("soul.md");
    assert!(tok(MASTER, abs.to_str().unwrap()).is_ok());
    assert!(r.ids().is_empty(), "no R-* record");
}

/// Q2 (b) / U4: after `owner-token ask --path soul.md` the owner's affirmative answer gives a token for
/// soul.md — never for MASTER_DIRECTIVE.md.
#[test]
fn owner_token_by_registered_question() {
    let r = rig("token-ask");
    let t = now0();
    r.prompt(MASTER, "규칙 하나 바꿔야겠다", "human", t);
    r.call(t + 1.0, &master(), "owner_token_ask", json!({"path": "soul.md"}))
        .unwrap();
    r.prompt(MASTER, "네", "human", t + 2.0);
    let e = r
        .call(t + 3.0, &master(), "owner_token", json!({"path": "directives/MASTER_DIRECTIVE.md"}))
        .unwrap_err();
    assert!(e.message.contains("다른 문서"), "{}", e.message);
    let v = r
        .call(t + 3.0, &master(), "owner_token", json!({"path": "soul.md"}))
        .unwrap();
    assert_eq!(v["basis"], "answered_question");
    assert!(r.prompts.lock().unwrap()[&MASTER].question.is_none());
    // a question about a non-rule file cannot be registered
    assert_eq!(
        r.call(t, &master(), "owner_token_ask", json!({"path": "bin/a.py"}))
            .unwrap_err()
            .code,
        "invalid_params"
    );
}

// ───────────────────────────── the rule-document branch of `begin` (C10 10.6) ─────────────────────────────

/// With a valid token the worker starts the rule-document repair: classification `RuleDocument`, the
/// pre-image copied to the vault (the backup), the token bound (record + token file);
/// `rule_doc_allowed` answers true only for the lock holder and the covered path.
#[test]
fn rule_document_begin_with_a_token_and_rule_doc_allowed() {
    let r = rig("rdoc");
    let t = now0();
    let token = token_for(&r, "soul.md", t);
    let w = seat(3, "worker");
    let v = r
        .call(t + 2.0, &w, "begin", json!({"trigger": "owner_order", "files": ["soul.md"], "owner_token": token}))
        .unwrap();
    assert_eq!(v["owner_token_bound"], true);
    let id = v["repair"].as_str().unwrap().to_string();
    let rec = r.rec(&id);
    assert_eq!(rec.classification, Classification::RuleDocument);
    assert_eq!(rec.trigger, Trigger::OwnerOrder);
    assert_eq!(rec.owner_token_sha256.as_deref(), Some(oo::token_sha256(&token).as_str()));
    assert!(matches!(rec.files[0].pre, cys::repair::ledger::PreImage::Slot { .. }), "backed up");
    let tf = oo::read_token_in(&r.root, &oo::token_sha256(&token)).unwrap().unwrap();
    assert_eq!(tf.used_by_repair.as_ref().map(|x| x.as_str()), Some(id.as_str()));
    let allowed = |who: &Caller, path: &str| {
        r.call(t + 3.0, who, "rule_doc_allowed", json!({"path": path}))
            .unwrap()["allowed"]
            .as_bool()
            .unwrap()
    };
    assert!(allowed(&w, "soul.md"));
    assert!(allowed(&w, r.pack.join("soul.md").to_str().unwrap()), "the tool's absolute path");
    assert!(allowed(&w, "SOUL.md"));
    assert!(!allowed(&seat(4, "worker-2"), "soul.md"), "another seat holds no lock");
    assert!(!allowed(&w, "directives/MASTER_DIRECTIVE.md"));
    assert!(!allowed(&w, "bin/a.py"));
    let human = Caller {
        kind: CallerKind::Human,
        role: None,
        app_token: false,
        bound_family: None,
    };
    assert!(!allowed(&human, "soul.md"));
    // after expiry the hook stops allowing
    assert!(!r
        .call(t + 1.0 + 86_400.0, &w, "rule_doc_allowed", json!({"path": "soul.md"}))
        .unwrap()["allowed"]
        .as_bool()
        .unwrap());
}

/// Refusals of the rule-document branch — each names the file, none writes anything: no token,
/// malformed, unknown, another document, expired, another lane, a grant / update trigger, a mix with
/// a general file, `--joins`; `--owner-token` on a general repair is invalid.
#[test]
fn rule_document_begin_refusals() {
    let r = rig("rdoc-no");
    let t = now0();
    let token = token_for(&r, "soul.md", t);
    let w = seat(3, "worker");
    let begin = |at: f64, p: Value| r.call(at, &w, "begin", p);
    let cases: Vec<(Value, &str)> = vec![
        (json!({"trigger": "owner_order", "files": ["soul.md"]}), "주인 지시 표가 필요합니다"),
        (json!({"trigger": "owner_order", "files": ["soul.md"], "owner_token": "ot-bad"}), "형식이 아닙니다"),
        (json!({"trigger": "owner_order", "files": ["soul.md"], "owner_token": "ot-aaaaaaaaaaaaaaaaaaaaaaaaaa"}), "발급된 적이 없습니다"),
        (json!({"trigger": "owner_order", "files": ["directives/MASTER_DIRECTIVE.md"], "owner_token": token}), "덮지 않습니다"),
        (json!({"trigger": "grant", "files": ["soul.md"], "owner_token": token}), "주인의 지시로만"),
        (json!({"trigger": "update", "files": ["soul.md"], "owner_token": token}), "주인의 지시로만"),
        (json!({"trigger": "owner_order", "files": ["soul.md", "bin/a.py"], "owner_token": token}), "섞지 마세요"),
        (json!({"trigger": "owner_order", "files": ["soul.md"], "owner_token": token, "joins": "R-1"}), "병합 모드"),
    ];
    for (p, want) in cases {
        let e = begin(t + 2.0, p.clone()).unwrap_err();
        assert_eq!(e.code, "repair_precondition", "{p}: {}", e.message);
        assert!(e.message.contains(want), "{p}: {}", e.message);
        assert!(e.message.contains("soul.md") || e.message.contains("MASTER_DIRECTIVE"), "{p}: names the file: {}", e.message);
    }
    let e = begin(t + 1.0 + 86_400.0, json!({"trigger": "owner_order", "files": ["soul.md"], "owner_token": token})).unwrap_err();
    assert!(e.message.contains("유효 기간"), "{}", e.message);
    let mut other = rig("rdoc-lane");
    other.root = r.root.clone();
    other.pack = r.pack.clone();
    other.lane = LaneKey("dept-2".into());
    let e = other
        .call(t + 2.0, &w, "begin", json!({"trigger": "owner_order", "files": ["soul.md"], "owner_token": token}))
        .unwrap_err();
    assert!(e.message.contains("부서 base"), "{}", e.message);
    let e = begin(t + 2.0, json!({"trigger": "owner_order", "files": ["bin/a.py"], "owner_token": token})).unwrap_err();
    assert_eq!(e.code, "invalid_params");
    assert!(r.ids().is_empty(), "a refused begin wrote a record");
    // the classification of the compiled list still applies: a rule document that is also listed is refused
    let listed = |_rel: &str| -> Result<Classification, RepairError> { Ok(Classification::BootCritical) };
    let e = r
        .call_with(t + 2.0, &listed, &w, "begin", json!({"trigger": "owner_order", "files": ["soul.md"], "owner_token": token}))
        .unwrap_err();
    assert!(e.message.contains("기동 필수"), "{}", e.message);
    assert!(r.ids().is_empty());
}

/// One token, one repair (O-C10-2): another seat picks the SAME repair up after the holder's daemon
/// restarted; a new repair with a used token is refused.
#[test]
fn a_token_serves_one_repair() {
    let r = rig("rdoc-once");
    let t = now0();
    let token = token_for(&r, "soul.md", t);
    let p = json!({"trigger": "owner_order", "files": ["soul.md"], "owner_token": token});
    let id = r.call(t + 2.0, &seat(3, "worker"), "begin", p.clone()).unwrap()["repair"]
        .as_str()
        .unwrap()
        .to_string();
    // held by the live lock of seat 3
    assert_eq!(
        r.call(t + 3.0, &seat(4, "worker-2"), "begin", p.clone()).unwrap_err().code,
        "repair_locked"
    );
    // the daemon restarts: the lock is released; seat 4 picks the same repair up with the same token
    let mut r2 = rig("rdoc-once-2");
    r2.root = r.root.clone();
    r2.pack = r.pack.clone();
    r2.inc = "inc-2".into();
    let v = r2.call(t + 4.0, &seat(4, "worker-2"), "begin", p.clone()).unwrap();
    assert_eq!((v["repair"].as_str().unwrap(), v["begin"].as_str().unwrap()), (id.as_str(), "picked-up"));
    // the repair is in force; a NEW repair with the used token is refused
    let mut x = r.rec(&id);
    x.state = RepairState::Probation;
    x.lock = None;
    ledger::write_record_in(&r.root, &x).unwrap();
    let e = r2.call(t + 5.0, &seat(4, "worker-2"), "begin", p).unwrap_err();
    assert!(e.message.contains(&format!("이미 수리 {id}")), "{}", e.message);
    assert_eq!(r.ids().len(), 1);
}

// ───────────────────────────── the CSO revision marker (AQ8) ─────────────────────────────

/// AQ8: `record` and `commit` refuse a CSO-directive candidate whose first 20 lines lost the marker the
/// pre-image has, with the sentence naming the marker and why; a candidate that keeps it is recorded.
#[test]
fn cso_marker_refused_at_record_and_commit() {
    let r = rig("cso");
    let t = now0();
    let rel = "directives/CSO_DIRECTIVE.md";
    let token = token_for(&r, rel, t);
    let w = seat(3, "worker");
    let id = r
        .call(t + 2.0, &w, "begin", json!({"trigger": "owner_order", "files": [rel], "owner_token": token}))
        .unwrap()["repair"]
        .as_str()
        .unwrap()
        .to_string();
    let live = r.pack.join(rel);
    std::fs::write(&live, "# CSO ABSOLUTE DIRECTIVE\n\nbody line changed\n").unwrap();
    let e = r
        .call(t + 3.0, &w, "record", json!({"repair": id, "note": "drop marker"}))
        .unwrap_err();
    assert_eq!(e.code, "repair_precondition");
    assert!(e.message.contains(oo::CSO_DIRECTIVE_REV_MARKER) && e.message.contains("AQ8"), "{}", e.message);
    assert!(r.rec(&id).worker_record.is_none(), "nothing recorded");
    // the marker moved within the head is kept
    std::fs::write(&live, format!("# CSO\n\n{}\nbody line changed\n", oo::CSO_DIRECTIVE_REV_MARKER)).unwrap();
    r.call(t + 4.0, &w, "record", json!({"repair": id, "note": "keep marker"}))
        .unwrap();
    r.call(
        t + 5.0,
        &seat(7, "reviewer-codex"),
        "attest_verify",
        json!({"repair": id, "verdict": "pass", "note": "ok"}),
    )
    .unwrap();
    // the live file loses the marker between the verification and the commit
    std::fs::write(&live, "# CSO\nbody line changed\n").unwrap();
    let e = r
        .call(t + 6.0, &w, "commit", json!({"repair": id}))
        .unwrap_err();
    assert!(e.message.contains(oo::CSO_DIRECTIVE_REV_MARKER), "{}", e.message);
    assert_eq!(r.rec(&id).state, RepairState::InProgress);
}

/// AQ8 applies only where the pre-image had the marker (an old installation's directive without it
/// is not held to it).
#[test]
fn cso_marker_not_required_when_the_pre_image_lacks_it() {
    let r = rig("cso-old");
    let t = now0();
    let rel = "directives/CSO_DIRECTIVE.md";
    std::fs::write(r.pack.join(rel), "# old CSO directive\n").unwrap();
    let token = token_for(&r, rel, t);
    let w = seat(3, "worker");
    let id = r
        .call(t + 2.0, &w, "begin", json!({"trigger": "owner_order", "files": [rel], "owner_token": token}))
        .unwrap()["repair"]
        .as_str()
        .unwrap()
        .to_string();
    std::fs::write(r.pack.join(rel), "# old CSO directive, fixed\n").unwrap();
    assert!(r
        .call(t + 3.0, &w, "record", json!({"repair": id, "note": "fix"}))
        .is_ok());
}

// ───────────────────────────── F1 / S2 ─────────────────────────────

/// F1 / S2: T4's daemon code sends nothing to a queue, an alert route or the event bus — a refusal or a
/// decision is an answer only (with the grant ON nothing reaches a master's queue outside the digest).
#[test]
fn t4_daemon_code_sends_nothing_to_queues() {
    let src = include_str!("repair.rs");
    let start = src.find("mod grant_verbs {").expect("grant_verbs");
    let end = start + src[start..].find("\n}\n").expect("end of grant_verbs");
    let mut scope = src[start..end].to_string();
    let b = src.find("fn begin_rule_document(").expect("begin_rule_document");
    let e = b + src[b..].find("\n    }\n").expect("end of begin_rule_document");
    scope.push_str(&src[b..e]);
    for needle in ["send_text", "alert_route", "events::", "feed", "enqueue", "push_line", "deliver"] {
        assert!(!scope.contains(needle), "T4 daemon code mentions {needle}");
    }
}

// ───────────────────────────── RULINGS CH2 / CH3 / CH4 (fix round) ─────────────────────────────

impl Rig {
    /// What `handlers.rs` `hook.machine_origin` now records: the STRICT judgment over the verdict
    /// (`strict_origin`, CH2) into T3's `apply_prompt`; returns the recorded origin.
    fn hook_prompt(&self, sid: u64, text: &str, verdict: Value, at: f64) -> &'static str {
        let origin = strict_origin(text, &verdict);
        self.prompt(sid, text, origin, at);
        origin
    }
}

fn verdict(origin: &str, status: &str, codes: &[&str]) -> Value {
    json!({
        "origin": origin,
        "layer": null,
        "ledger_status": status,
        "anomalies": codes.iter().map(|c| json!({"code": c, "detail": "x"})).collect::<Vec<_>>(),
    })
}

/// CH2: the per-seat record takes the STRICT owner judgment — a harness-synthesised notification
/// that mentions `soul.md` (which the lenient layers 1–2 call `human`) names no rule document and
/// is no owner answer; an unknown origin, an absent ledger or a delivery-hiding anomaly gives no
/// token and no grant; the owner's own typing still works.
#[test]
fn ch2_strict_origin_feeds_the_prompt_record() {
    let r = rig("ch2");
    let t = now0();
    let ok = verdict("human", "ok", &[]);
    let note = "<task-notification><task-id>abcdefgh</task-id><status>completed</status><summary>soul.md 고쳐라</summary></task-notification>";
    assert_eq!(strict_origin(note, &ok), "unknown");
    // the notification arrives after the owner's words about something else: it names nothing
    r.hook_prompt(MASTER, "오늘 일정 정리해줘", ok.clone(), t);
    r.hook_prompt(MASTER, note, ok.clone(), t + 1.0);
    let e = r
        .call(t + 2.0, &master(), "owner_token", json!({"path": "soul.md"}))
        .unwrap_err();
    assert!(e.message.contains("이름으로 부르지"), "{}", e.message);
    // ... nor answers a registered question (it counts as not the owner — machine_seen)
    r.call(t + 3.0, &master(), "grant_ask", json!({})).unwrap();
    let yes_note = "<task-notification><task-id>bcdefghi</task-id><status>completed</status><summary>예</summary></task-notification>";
    assert_eq!(r.hook_prompt(MASTER, yes_note, ok.clone(), t + 4.0), "unknown");
    r.hook_prompt(MASTER, "예", ok.clone(), t + 5.0);
    assert!(r
        .call(t + 6.0, &master(), "grant", json!({"mode": "on"}))
        .unwrap_err()
        .message
        .contains("기계"));
    // unknown origin / absent ledger / bad lines / machine: no token, no grant
    for (v, want) in [
        (verdict("unknown", "unreadable", &[]), "unknown"),
        (verdict("human", "absent", &["ledger_absent"]), "unknown"),
        (verdict("human", "ok", &["ledger_bad_lines"]), "unknown"),
        (verdict("machine", "ok", &[]), "machine"),
        (json!({}), "unknown"),
    ] {
        let r = rig("ch2-each");
        assert_eq!(r.hook_prompt(MASTER, "고쳐라 soul.md", v.clone(), t), want, "{v}");
        assert!(r
            .call(t + 1.0, &master(), "owner_token", json!({"path": "soul.md"}))
            .is_err(), "{v}: a token");
        r.call(t + 2.0, &master(), "grant_ask", json!({})).unwrap();
        r.hook_prompt(MASTER, "예", v.clone(), t + 3.0);
        assert!(r
            .call(t + 4.0, &master(), "grant", json!({"mode": "on"}))
            .is_err(), "{v}: a grant");
        assert_eq!(r.decision(), None);
    }
    // the owner's own typing (a rotated ledger is ordinary) still works
    let r = rig("ch2-owner");
    let typed = verdict("human", "ok", &["ledger_rotated"]);
    assert_eq!(r.hook_prompt(MASTER, "고쳐라 soul.md", typed.clone(), t), "human");
    assert!(r
        .call(t + 1.0, &master(), "owner_token", json!({"path": "soul.md"}))
        .is_ok());
    r.call(t + 2.0, &master(), "grant_ask", json!({})).unwrap();
    r.hook_prompt(MASTER, "예", typed, t + 3.0);
    assert_eq!(
        r.call(t + 4.0, &master(), "grant", json!({"mode": "on"}))
            .unwrap()["grant"],
        "on"
    );
}

/// CH2 / CH3 wiring pins: `hook.machine_origin` passes the STRICT judgment to the record (never the
/// lenient origin) and marks owner input only when it says owner-typed; the operator-token send path
/// marks it under `human_verified`; exactly these two calls exist.
#[test]
fn ch2_ch3_handler_wiring_pins() {
    let handlers = include_str!("handlers.rs");
    let arm = &handlers[handlers.find("\"hook.machine_origin\" => {").unwrap()..];
    let arm = &arm[..arm.find("\"boot.exit_ack\" =>").unwrap()];
    assert!(arm.contains("let owner = crate::repair::strict_origin(&norm, &reply);"));
    assert!(arm.contains(
        "crate::repair::record_prompt_origin(daemon, sid, &norm, digest_in.as_deref(), Some(owner));"
    ));
    assert!(!arm.contains("reply[\"origin\"].as_str())"), "the lenient origin reaches the record");
    assert!(arm.contains(
        "if owner == \"human\" { crate::repair::grant_note_owner_input(daemon); }"
    ));
    let send = &handlers[handlers.find("let human_verified = human && !machine_origin").unwrap()..];
    let send = &send[..send.find("if !human_verified {").unwrap()];
    assert!(send.contains("if human_verified { crate::repair::grant_note_owner_input(daemon); }"));
    assert_eq!(handlers.matches("crate::repair::grant_note_owner_input(daemon)").count(), 2);
}

/// CH3: with the grant ON, owner input in the lane pauses NEW autonomous starts at once — and
/// nothing else: a running grant repair still commits, an owner-ordered repair still starts. The
/// daemon entry writes this lane's pause mark under its ledger root.
#[test]
fn ch3_owner_input_pauses_new_autonomous_starts_only() {
    let daemon = crate::team_gate_tests::tmp_daemon("t4-ch3", false);
    let mut r = rig("ch3");
    r.root = production_root(&daemon);
    r.lane = LaneKey::of_socket(&daemon.socket_path);
    let _ = std::fs::remove_dir_all(&r.root);
    r.grant_on(5);
    let t = now0();
    let w = seat(3, "worker");
    let id = r
        .call(t, &w, "begin", json!({"trigger": "grant", "files": ["bin/a.py"]}))
        .unwrap()["repair"]
        .as_str()
        .unwrap()
        .to_string();
    std::fs::write(r.pack.join("bin/a.py"), b"print('fixed')\n").unwrap();
    r.call(t + 1.0, &w, "record", json!({"repair": id, "note": "fix"})).unwrap();
    r.call(t + 2.0, &seat(7, "reviewer-codex"), "attest_verify", json!({"repair": id, "verdict": "pass", "note": "ok"}))
        .unwrap();
    // the owner types (hook path or app path — both call this)
    grant_note_owner_input(&daemon);
    assert!(grant::pause_path_in(&r.root, &r.lane).exists(), "the lane's pause mark");
    assert!(grant::is_paused_in(&r.root, now0()));
    // the running round still closes
    let v = r.call(now0(), &w, "commit", json!({"repair": id})).unwrap();
    assert_eq!(v["state"], json!("probation"), "{v}");
    // a NEW autonomous start is paused, an owner-ordered repair is not
    let e = r
        .call(now0(), &seat(4, "worker-2"), "begin", json!({"trigger": "grant", "files": ["bin/b.py"]}))
        .unwrap_err();
    assert_eq!(e.message, "repair.begin: 자동 수리 허락이 일시정지 중입니다");
    assert!(r
        .call(now0(), &seat(4, "worker-2"), "begin", json!({"trigger": "owner_order", "files": ["bin/b.py"]}))
        .is_ok());
    // grant OFF: owner input writes no mark (nothing to pause)
    let r2 = rig("ch3-off");
    note_owner_input_throttled(&r2.root, &r2.lane, now0());
    assert!(!grant::pause_path_in(&r2.root, &r2.lane).exists());
    let _ = std::fs::remove_dir_all(&r.root);
}

/// CH3: app typing arrives in many small sends — one lane's mark is rewritten at most once per
/// `PAUSE_MARK_MIN_GAP_SECS`; a later input moves it on.
#[test]
fn ch3_pause_mark_is_throttled_per_lane() {
    let r = rig("ch3-throttle");
    r.grant_on(5);
    let t = now0();
    note_owner_input_throttled(&r.root, &r.lane, t);
    let read = || -> f64 {
        let v: Value = serde_json::from_slice(&std::fs::read(grant::pause_path_in(&r.root, &r.lane)).unwrap()).unwrap();
        v["paused_at"].as_f64().unwrap()
    };
    assert_eq!(read(), t);
    note_owner_input_throttled(&r.root, &r.lane, t + 1.0);
    assert_eq!(read(), t, "within the gap: not rewritten");
    note_owner_input_throttled(&r.root, &r.lane, t + PAUSE_MARK_MIN_GAP_SECS + 1.0);
    assert_eq!(read(), t + PAUSE_MARK_MIN_GAP_SECS + 1.0);
}

/// CH4: the owner says "끄라", a machine prompt arrives, then the master runs `grant off` — still
/// accepted (the latest OWNER-typed prompt is kept beside the latest prompt of any origin).
#[test]
fn ch4_grant_off_after_a_machine_prompt() {
    let r = rig("ch4");
    let t = now0();
    let ok = verdict("human", "ok", &[]);
    r.call(t, &master(), "grant_ask", json!({})).unwrap();
    r.hook_prompt(MASTER, "응", ok.clone(), t + 1.0);
    r.call(t + 2.0, &master(), "grant", json!({"mode": "on"})).unwrap();
    r.hook_prompt(MASTER, "자동 수리 꺼", ok.clone(), t + 3.0);
    r.hook_prompt(MASTER, "[worker] 보고: 완료", verdict("machine", "ok", &[]), t + 4.0);
    assert_eq!(r.prompts.lock().unwrap()[&MASTER].origin, "machine");
    let v = r
        .call(t + 5.0, &master(), "grant", json!({"mode": "off"}))
        .unwrap();
    assert_eq!(v["grant"], "off");
    let d = r.decision().unwrap();
    assert_eq!(d.state, GrantState::Off);
    assert_eq!(d.prompt_digest, cys::mission_gate::digest_normalized("자동 수리 꺼"));
    // a machine prompt alone after a decision still turns nothing off
    r.call(t + 6.0, &master(), "grant_ask", json!({})).unwrap();
    r.hook_prompt(MASTER, "예", ok, t + 7.0);
    r.call(t + 8.0, &master(), "grant", json!({"mode": "on"})).unwrap();
    r.hook_prompt(MASTER, "[worker] 끄라", verdict("machine", "ok", &[]), t + 9.0);
    assert!(r.call(t + 10.0, &master(), "grant", json!({"mode": "off"})).is_err());
}
