// Wired into surface.send_text / surface.send_key by the next S-a commit; until then only tests use it.
#![allow(dead_code)]
//! S-a legacy submit association (DESIGN-S-FROZEN-R3 §4.5 orphan legacy body rule S3-5;
//! HEAD-R3-RULING S4-3; HEAD-RULINGS-W4-1 D3).
//!
//! An automatic body that arrives WITHOUT its own submit phase (the legacy two-step `cys send` then
//! `cys send-key Return`) is never typed on its own. It is held untyped here. A legacy Return from
//! the SAME daemon-resolved caller to the SAME seat generation within [`LEGACY_ASSOCIATION_WINDOW_MS`]
//! of the body's FULL receipt binds to it, and the daemon then types body + Return as one owned
//! request. The deadline is fixed once, at receipt: neither load, later calls, nor other held
//! bodies can widen it (the association must not become a spoofing window). Ambiguity refuses
//! instead of guessing; expiry is an explicit undelivered disposition, never typing.

use std::time::{Duration, Instant};

/// S4-3 association window, from FULL receipt of the body (D3; locked in
/// LOCK-W4-WINDOWS-AND-N-20261016.json before any measurement).
pub const LEGACY_ASSOCIATION_WINDOW_MS: u64 = 10_000;

/// Held bodies per seat; a full table refuses the new body (the caller keeps it).
pub const MAX_HELD_PER_SURFACE: usize = 16;

/// The authenticated caller (IMPL-PLAN I4): the daemon-resolved caller surface, else the kernel
/// peer's parent process identity derived by the daemon. Never a client-supplied claim.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum CallerKey {
    Surface(u64),
    Process { uid: u32, parent_pid: u32, parent_birth_us: u64 },
}

/// One untyped body awaiting its submit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeldBody {
    /// The request token returned at admission (a migrated caller attaches its Return by it).
    pub request_id: String,
    pub caller: CallerKey,
    /// Seat generation the body was admitted for (binding / conversation epoch).
    pub generation: u64,
    pub received_at: Instant,
    pub deadline: Instant,
    pub text: String,
    pub clear_first: bool,
}

impl HeldBody {
    pub fn new(request_id: String, caller: CallerKey, generation: u64, received_at: Instant, text: String, clear_first: bool) -> HeldBody {
        let deadline = received_at + Duration::from_millis(LEGACY_ASSOCIATION_WINDOW_MS);
        HeldBody { request_id, caller, generation, received_at, deadline, text, clear_first }
    }
}

/// Outcome of a legacy Return's association attempt.
#[derive(Debug, PartialEq, Eq)]
pub enum Associate {
    /// Exactly one held body of this caller and generation inside its window: type body + Return.
    Bound(HeldBody),
    /// Nothing to bind: the Return is an ordinary AgentControl key (with its own full checks).
    NoBody,
    /// More than one candidate: refuse the key (recorded, zero bytes); every body stays held.
    Ambiguous(Vec<String>),
}

/// Per-seat table. All times are monotonic and supplied by the caller, so tests control the clock.
#[derive(Debug, Default)]
pub struct AssociationTable {
    held: Vec<HeldBody>,
}

impl AssociationTable {
    /// Hold a body. A full table refuses it and gives it back (no silent eviction).
    pub fn hold(&mut self, body: HeldBody) -> Result<(), HeldBody> {
        if self.held.len() >= MAX_HELD_PER_SURFACE {
            return Err(body);
        }
        self.held.push(body);
        Ok(())
    }

    /// Remove and return every body whose window has closed (`now > deadline`): each becomes an
    /// explicit undelivered disposition at the caller; none is ever typed.
    pub fn expire(&mut self, now: Instant) -> Vec<HeldBody> {
        let (gone, keep): (Vec<HeldBody>, Vec<HeldBody>) = self.held.drain(..).partition(|b| now > b.deadline);
        self.held = keep;
        gone
    }

    /// A legacy (untokened) Return from `caller` for seat `generation` at `now`. Expired bodies never
    /// bind (they are left for [`Self::expire`]); a body of another caller or generation never binds.
    pub fn take_for_return(&mut self, caller: &CallerKey, generation: u64, now: Instant) -> Associate {
        let candidates: Vec<usize> = self
            .held
            .iter()
            .enumerate()
            .filter(|(_, b)| b.caller == *caller && b.generation == generation && now <= b.deadline)
            .map(|(i, _)| i)
            .collect();
        match candidates.as_slice() {
            [] => Associate::NoBody,
            [i] => Associate::Bound(self.held.remove(*i)),
            many => Associate::Ambiguous(many.iter().map(|&i| self.held[i].request_id.clone()).collect()),
        }
    }

    /// A migrated caller's Return carrying the admission token: binds that body only, inside its
    /// window and for the same seat generation.
    pub fn take_by_token(&mut self, request_id: &str, generation: u64, now: Instant) -> Option<HeldBody> {
        let i = self
            .held
            .iter()
            .position(|b| b.request_id == request_id && b.generation == generation && now <= b.deadline)?;
        Some(self.held.remove(i))
    }

    pub fn len(&self) -> usize {
        self.held.len()
    }

    pub fn ids(&self) -> Vec<String> {
        self.held.iter().map(|b| b.request_id.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: Duration = Duration::from_millis(LEGACY_ASSOCIATION_WINDOW_MS);
    const MS: Duration = Duration::from_millis(1);

    fn body(id: &str, caller: CallerKey, generation: u64, at: Instant) -> HeldBody {
        HeldBody::new(id.into(), caller, generation, at, format!("body {id}"), false)
    }

    /// D3: the Return at exactly the window end binds; at window + 1 ms it does not, and that body
    /// expires as an undelivered disposition instead of being typed.
    #[test]
    fn return_binds_inside_the_window_and_not_at_window_plus_one_ms() {
        let t0 = Instant::now();
        let me = CallerKey::Surface(7);
        let mut t = AssociationTable::default();
        t.hold(body("a", me.clone(), 1, t0)).unwrap();
        match t.take_for_return(&me, 1, t0 + W) {
            Associate::Bound(b) => assert_eq!(b.request_id, "a"),
            other => panic!("{other:?}"),
        }
        t.hold(body("b", me.clone(), 1, t0)).unwrap();
        assert_eq!(t.take_for_return(&me, 1, t0 + W + MS), Associate::NoBody);
        assert_eq!(t.len(), 1, "a late Return neither binds nor drops the body");
        let gone = t.expire(t0 + W + MS);
        assert_eq!(gone.iter().map(|b| b.request_id.as_str()).collect::<Vec<_>>(), ["b"]);
        assert_eq!(t.len(), 0);
    }

    /// D3: the window does not widen under load — the deadline is fixed at full receipt and does not
    /// move with later calls, retries or other held bodies.
    #[test]
    fn deadline_is_fixed_at_receipt_and_never_widens() {
        let t0 = Instant::now();
        let me = CallerKey::Surface(7);
        let mut t = AssociationTable::default();
        t.hold(body("a", me.clone(), 1, t0)).unwrap();
        // Load: many other callers' bodies and failed association attempts in between.
        for i in 0..10u64 {
            t.hold(body(&format!("x{i}"), CallerKey::Surface(100 + i), 1, t0 + W / 2)).unwrap();
            assert_eq!(t.take_for_return(&CallerKey::Surface(999), 1, t0 + W / 2), Associate::NoBody);
        }
        let held: Vec<&HeldBody> = t.held.iter().filter(|b| b.request_id == "a").collect();
        assert_eq!(held[0].deadline, t0 + W, "deadline moved");
        assert_eq!(t.take_for_return(&me, 1, t0 + W + MS), Associate::NoBody);
    }

    /// The SAME authenticated caller and the SAME seat generation only; ambiguity refuses.
    #[test]
    fn association_requires_same_caller_same_generation_and_a_unique_candidate() {
        let t0 = Instant::now();
        let me = CallerKey::Process { uid: 501, parent_pid: 4242, parent_birth_us: 1_000_000 };
        let reused_pid = CallerKey::Process { uid: 501, parent_pid: 4242, parent_birth_us: 2_000_000 };
        let mut t = AssociationTable::default();
        t.hold(body("a", me.clone(), 3, t0)).unwrap();
        assert_eq!(t.take_for_return(&reused_pid, 3, t0), Associate::NoBody, "a reused parent pid is another caller");
        assert_eq!(t.take_for_return(&CallerKey::Surface(1), 3, t0), Associate::NoBody);
        assert_eq!(t.take_for_return(&me, 4, t0), Associate::NoBody, "a new seat generation never binds an old body");
        t.hold(body("b", me.clone(), 3, t0)).unwrap();
        assert_eq!(t.take_for_return(&me, 3, t0), Associate::Ambiguous(vec!["a".into(), "b".into()]));
        assert_eq!(t.len(), 2, "ambiguity refuses without dropping or guessing");
    }

    #[test]
    fn token_binds_its_own_body_inside_the_window_only() {
        let t0 = Instant::now();
        let me = CallerKey::Surface(7);
        let mut t = AssociationTable::default();
        t.hold(body("a", me.clone(), 1, t0)).unwrap();
        t.hold(body("b", me.clone(), 1, t0)).unwrap();
        assert_eq!(t.take_by_token("b", 1, t0 + W).map(|b| b.request_id), Some("b".into()));
        assert_eq!(t.take_by_token("a", 2, t0), None, "token for another generation");
        assert_eq!(t.take_by_token("a", 1, t0 + W + MS), None, "token after the window");
        assert_eq!(t.ids(), ["a"]);
    }

    #[test]
    fn a_full_table_refuses_the_new_body_without_evicting() {
        let t0 = Instant::now();
        let mut t = AssociationTable::default();
        for i in 0..MAX_HELD_PER_SURFACE {
            t.hold(body(&format!("h{i}"), CallerKey::Surface(i as u64), 1, t0)).unwrap();
        }
        let back = t.hold(body("late", CallerKey::Surface(99), 1, t0)).unwrap_err();
        assert_eq!(back.request_id, "late");
        assert_eq!(t.len(), MAX_HELD_PER_SURFACE);
        assert!(t.ids().contains(&"h0".to_string()));
    }
}
