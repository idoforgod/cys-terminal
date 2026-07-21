//! Small, typed lifecycle contracts owned by cysd's lazy Browser authority broker.
//! The policy is deliberately pure so retry/update behaviour can be tested
//! without spawning a browser or mutating a signed bundle.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub max_elapsed_ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetryState {
    pub attempts: u32,
    pub started_ms: u64,
    pub cancelled: bool,
}

impl RetryPolicy {
    pub fn allows(&self, state: RetryState, now_ms: u64) -> bool {
        !state.cancelled
            && state.attempts < self.max_attempts
            && now_ms.saturating_sub(state.started_ms) < self.max_elapsed_ms
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UpdatePhase {
    Stage,
    Verify,
    Select,
    Health,
    Commit,
    Rollback,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UpdateJournal {
    pub generation: u64,
    pub phase: UpdatePhase,
}

impl UpdateJournal {
    pub fn new(generation: u64) -> Self {
        Self {
            generation,
            phase: UpdatePhase::Stage,
        }
    }
    pub fn advance(&mut self, phase: UpdatePhase) {
        self.phase = phase;
    }
    pub fn rollback(&mut self) {
        self.phase = UpdatePhase::Rollback;
    }
}
