//! S-a legacy submit association (DESIGN-S-FROZEN-R3 §4.5 orphan legacy body rule S3-5;
//! HEAD-R3-RULING S4-3; HEAD-RULINGS-W4-1 D3 as amended by HEAD-RULINGS-W4-2).
//!
//! An automatic body that arrives WITHOUT its own submit phase (the legacy two-step `cys send` then
//! `cys send-key Return`) is never typed on its own. It is held untyped here. A legacy Return from
//! the SAME daemon-resolved caller to the SAME seat generation within [`LEGACY_ASSOCIATION_WINDOW_MS`]
//! of the body's FULL receipt binds to it, and the daemon then types body + Return as one owned
//! request. The deadline is fixed once, at receipt: neither load, later calls, nor other held
//! bodies can widen it (the association must not become a spoofing window). A caller holds at most
//! one body per seat generation: a newer body from the same caller supersedes the older one, so a
//! legacy Return never has two candidates to guess between. Expiry and supersession are explicit
//! undelivered dispositions, never typing.

use std::time::{Duration, Instant};

/// S4-3 association window, from FULL receipt of the body. D3 locked 10,000 ms before any
/// measurement; HEAD-RULINGS-W4-2 amended it to 60,000 ms on a measurement of REAL caller
/// behaviour (a 34.1 s split send/Return in this lane that 10,000 ms would have dropped) -
/// LOCK-W4-AMEND-1-D3-20261016.json.
pub const LEGACY_ASSOCIATION_WINDOW_MS: u64 = 60_000;

/// Held bodies per seat; a full table refuses the new body (the caller keeps it).
pub const MAX_HELD_PER_SURFACE: usize = 16;

/// The caller a held body belongs to (IMPL-PLAN I4 as narrowed by C5B-PLAN I9), in this order:
/// the daemon-resolved caller surface; else the request's self-declared `from` seat (the same
/// `Claimed` grade the Return-absorb tickets use for cross-socket callers - it can only get a body
/// the caller's ACL already admitted submitted, which a bare Return could always do); else the
/// kernel peer's parent process (pid + native birth, so a reused pid is another caller).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum CallerKey {
    Surface(u64),
    Claimed(u64),
    Process { parent_pid: u32, parent_birth_us: u64 },
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
    /// The caller's text exactly as admitted (a queued Return re-enqueues it; the queue fences it).
    pub text: String,
    /// The direct path's paste-fence decision made at admission (direct send bytes = fenced head).
    pub fenced: bool,
}

impl HeldBody {
    pub fn new(request_id: String, caller: CallerKey, generation: u64, received_at: Instant, text: String, fenced: bool) -> HeldBody {
        let deadline = received_at + Duration::from_millis(LEGACY_ASSOCIATION_WINDOW_MS);
        HeldBody { request_id, caller, generation, received_at, deadline, text, fenced }
    }
}

/// Outcome of a legacy Return's association attempt.
#[derive(Debug, PartialEq, Eq)]
pub enum Associate {
    /// The held body of this caller and generation, inside its window: type body + Return.
    Bound(HeldBody),
    /// Nothing to bind: the Return is an ordinary AgentControl key (with its own full checks).
    NoBody,
}

/// Per-seat table. All times are monotonic and supplied by the caller, so tests control the clock.
#[derive(Debug, Default)]
pub struct AssociationTable {
    held: Vec<HeldBody>,
}

impl AssociationTable {
    /// Hold a body. A body of the same caller and generation already held is superseded and
    /// returned (`Ok(Some(old))`: the caller records it as undelivered). A full table refuses the
    /// new body and gives it back (`Err`, no silent eviction of another caller's body).
    pub fn hold(&mut self, body: HeldBody) -> Result<Option<HeldBody>, HeldBody> {
        let mine = self.held.iter().position(|b| b.caller == body.caller && b.generation == body.generation);
        if mine.is_none() && self.held.len() >= MAX_HELD_PER_SURFACE {
            return Err(body);
        }
        let old = mine.map(|i| self.held.remove(i));
        self.held.push(body);
        Ok(old)
    }

    /// Remove and return every body whose window has closed (`now > deadline`): each becomes an
    /// explicit undelivered disposition at the caller; none is ever typed.
    pub fn expire(&mut self, now: Instant) -> Vec<HeldBody> {
        let (gone, keep): (Vec<HeldBody>, Vec<HeldBody>) = self.held.drain(..).partition(|b| now > b.deadline);
        self.held = keep;
        gone
    }

    /// Remove and return every body held for another seat generation (the agent changed): each is
    /// an undelivered disposition; none may bind to the new agent.
    pub fn take_stale(&mut self, generation: u64) -> Vec<HeldBody> {
        let (gone, keep): (Vec<HeldBody>, Vec<HeldBody>) = self.held.drain(..).partition(|b| b.generation != generation);
        self.held = keep;
        gone
    }

    /// Remove and return every held body (a clear-first write erases the composer line every one of
    /// them would have been typed into under the old two-step).
    pub fn drain_all(&mut self) -> Vec<HeldBody> {
        std::mem::take(&mut self.held)
    }

    /// Put a body back after a bind-time refusal. Its deadline is the one fixed at receipt (never
    /// widened); a body whose caller already holds a newer one, or that no longer fits, is returned.
    pub fn restore(&mut self, body: HeldBody) -> Result<(), HeldBody> {
        if self.held.len() >= MAX_HELD_PER_SURFACE
            || self.held.iter().any(|b| b.caller == body.caller && b.generation == body.generation)
        {
            return Err(body);
        }
        self.held.push(body);
        Ok(())
    }

    /// A legacy (untokened) Return from `caller` for seat `generation` at `now`. Expired bodies never
    /// bind (they are left for [`Self::expire`]); a body of another caller or generation never binds.
    /// [`Self::hold`] keeps at most one body per caller and generation, so there is no candidate set.
    pub fn take_for_return(&mut self, caller: &CallerKey, generation: u64, now: Instant) -> Associate {
        match self.held.iter().position(|b| b.caller == *caller && b.generation == generation && now <= b.deadline) {
            Some(i) => Associate::Bound(self.held.remove(i)),
            None => Associate::NoBody,
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

    /// Tests only: make every held body `by` old now (receipt moved back; deadline = receipt + window, as at hold).
    #[cfg(test)]
    pub fn backdate(&mut self, by: Duration) {
        let received = Instant::now().checked_sub(by).expect("monotonic clock older than the backdate");
        for b in &mut self.held {
            b.received_at = received;
            b.deadline = received + Duration::from_millis(LEGACY_ASSOCIATION_WINDOW_MS);
        }
    }

    /// The held bodies (for the durable record of the held set, C7e).
    pub fn records(&self) -> impl Iterator<Item = &HeldBody> {
        self.held.iter()
    }

    #[cfg_attr(not(test), allow(dead_code))]
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

    /// The amended D3 bar (HEAD-RULINGS-W4-2): 60,000 ms from full receipt.
    #[test]
    fn the_window_is_the_amended_sixty_seconds() {
        assert_eq!(LEGACY_ASSOCIATION_WINDOW_MS, 60_000);
    }

    /// D3 as amended: a Return at 59,999 ms binds; at 60,001 ms it does not, and that body expires
    /// as an undelivered disposition instead of being typed.
    #[test]
    fn return_binds_at_59999_ms_and_not_at_60001_ms() {
        let t0 = Instant::now();
        let me = CallerKey::Surface(7);
        let mut t = AssociationTable::default();
        assert_eq!(t.hold(body("a", me.clone(), 1, t0)), Ok(None));
        match t.take_for_return(&me, 1, t0 + Duration::from_millis(59_999)) {
            Associate::Bound(b) => assert_eq!(b.request_id, "a"),
            other => panic!("{other:?}"),
        }
        assert_eq!(t.hold(body("b", me.clone(), 1, t0)), Ok(None));
        assert_eq!(t.take_for_return(&me, 1, t0 + Duration::from_millis(60_001)), Associate::NoBody);
        assert_eq!(t.len(), 1, "a late Return neither binds nor drops the body");
        assert!(t.expire(t0 + W).is_empty(), "the window end itself is still inside");
        let gone = t.expire(t0 + W + MS);
        assert_eq!(gone.iter().map(|b| b.request_id.as_str()).collect::<Vec<_>>(), ["b"]);
        assert_eq!(t.len(), 0);
    }

    /// The window does not widen under load: the deadline is fixed at full receipt and does not
    /// move with later calls, retries or other callers' held bodies.
    #[test]
    fn deadline_is_fixed_at_receipt_and_never_widens() {
        let t0 = Instant::now();
        let me = CallerKey::Surface(7);
        let mut t = AssociationTable::default();
        t.hold(body("a", me.clone(), 1, t0)).unwrap();
        // Load: other callers' bodies and failed association attempts in between.
        for i in 0..10u64 {
            t.hold(body(&format!("x{i}"), CallerKey::Surface(100 + i), 1, t0 + W / 2)).unwrap();
            assert_eq!(t.take_for_return(&CallerKey::Surface(999), 1, t0 + W / 2), Associate::NoBody);
        }
        let held: Vec<&HeldBody> = t.held.iter().filter(|b| b.request_id == "a").collect();
        assert_eq!(held[0].deadline, t0 + W, "deadline moved");
        assert_eq!(t.take_for_return(&me, 1, t0 + W + MS), Associate::NoBody);
    }

    /// The SAME authenticated caller and the SAME seat generation only.
    #[test]
    fn association_requires_same_caller_and_same_generation() {
        let t0 = Instant::now();
        let me = CallerKey::Process { parent_pid: 4242, parent_birth_us: 1_000_000 };
        let reused_pid = CallerKey::Process { parent_pid: 4242, parent_birth_us: 2_000_000 };
        let mut t = AssociationTable::default();
        t.hold(body("a", me.clone(), 3, t0)).unwrap();
        assert_eq!(t.take_for_return(&reused_pid, 3, t0), Associate::NoBody, "a reused parent pid is another caller");
        assert_eq!(t.take_for_return(&CallerKey::Surface(1), 3, t0), Associate::NoBody);
        assert_eq!(t.take_for_return(&CallerKey::Claimed(1), 3, t0), Associate::NoBody, "a claimed seat is not that process");
        assert_eq!(t.take_for_return(&me, 4, t0), Associate::NoBody, "a new seat generation never binds an old body");
        assert_eq!(t.ids(), ["a"]);
    }

    /// Head condition (2) at table level: A without Return, then B from the same caller -> A is
    /// superseded (handed back for its undelivered event), the Return binds B, nothing is left to
    /// bind twice.
    #[test]
    fn a_newer_body_from_the_same_caller_supersedes_and_the_return_binds_it() {
        let t0 = Instant::now();
        let me = CallerKey::Surface(7);
        let mut t = AssociationTable::default();
        assert_eq!(t.hold(body("a", me.clone(), 1, t0)), Ok(None));
        let old = t.hold(body("b", me.clone(), 1, t0 + MS)).unwrap().expect("A superseded");
        assert_eq!(old.request_id, "a");
        assert_eq!(t.ids(), ["b"], "one held body per caller and generation");
        match t.take_for_return(&me, 1, t0 + 2 * MS) {
            Associate::Bound(b) => assert_eq!(b.request_id, "b"),
            other => panic!("{other:?}"),
        }
        assert_eq!(t.take_for_return(&me, 1, t0 + 3 * MS), Associate::NoBody, "a second Return binds nothing");
    }

    /// Head condition (3): a body from a DIFFERENT caller (or the same caller in another seat
    /// generation) never supersedes.
    #[test]
    fn a_different_caller_or_generation_never_supersedes() {
        let t0 = Instant::now();
        let mut t = AssociationTable::default();
        t.hold(body("a", CallerKey::Surface(7), 1, t0)).unwrap();
        assert_eq!(t.hold(body("b", CallerKey::Surface(8), 1, t0)), Ok(None));
        assert_eq!(t.hold(body("c", CallerKey::Surface(7), 2, t0)), Ok(None));
        assert_eq!(t.ids(), ["a", "b", "c"]);
    }

    #[test]
    fn stale_generations_and_clear_drain_and_restore_keeps_the_receipt_deadline() {
        let t0 = Instant::now();
        let mut t = AssociationTable::default();
        t.hold(body("a", CallerKey::Surface(7), 1, t0)).unwrap();
        t.hold(body("b", CallerKey::Surface(8), 2, t0)).unwrap();
        assert_eq!(t.take_stale(2).iter().map(|b| b.request_id.as_str()).collect::<Vec<_>>(), ["a"]);
        assert_eq!(t.ids(), ["b"]);
        let Associate::Bound(b) = t.take_for_return(&CallerKey::Surface(8), 2, t0 + W / 2) else { panic!() };
        t.restore(b).unwrap();
        assert_eq!(t.held[0].deadline, t0 + W, "a refused bind puts the body back with its receipt deadline");
        t.hold(body("c", CallerKey::Surface(8), 2, t0 + MS)).unwrap();
        let Associate::Bound(c) = t.take_for_return(&CallerKey::Surface(8), 2, t0 + 2 * MS) else { panic!() };
        t.hold(body("d", CallerKey::Surface(8), 2, t0 + 3 * MS)).unwrap();
        assert_eq!(t.restore(c).unwrap_err().request_id, "c", "a newer body of that caller wins");
        assert_eq!(t.drain_all().len(), 1);
        assert_eq!(t.len(), 0);
    }

    #[test]
    fn token_binds_its_own_body_inside_the_window_only() {
        let t0 = Instant::now();
        let mut t = AssociationTable::default();
        t.hold(body("a", CallerKey::Surface(7), 1, t0)).unwrap();
        t.hold(body("b", CallerKey::Surface(8), 1, t0)).unwrap();
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
        // A caller already holding a body may still replace it in a full table.
        let old = t.hold(body("h0b", CallerKey::Surface(0), 1, t0)).unwrap().unwrap();
        assert_eq!(old.request_id, "h0");
        assert_eq!(t.len(), MAX_HELD_PER_SURFACE);
    }
}
