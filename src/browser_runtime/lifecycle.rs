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
    generation: u64,
    phase: UpdatePhase,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UpdateJournalError {
    InvalidGeneration,
    InvalidTransition { from: UpdatePhase, to: UpdatePhase },
}

impl std::fmt::Display for UpdateJournalError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidGeneration => formatter.write_str("runtime generation must be positive"),
            Self::InvalidTransition { from, to } => {
                write!(
                    formatter,
                    "invalid runtime update transition: {from:?} -> {to:?}"
                )
            }
        }
    }
}

impl std::error::Error for UpdateJournalError {}

fn invalid_transition(from: UpdatePhase, to: UpdatePhase) -> UpdateJournalError {
    UpdateJournalError::InvalidTransition { from, to }
}

impl UpdateJournal {
    pub fn new(generation: u64) -> Result<Self, UpdateJournalError> {
        if generation == 0 {
            return Err(UpdateJournalError::InvalidGeneration);
        }
        Ok(Self {
            generation,
            phase: UpdatePhase::Stage,
        })
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn phase(&self) -> UpdatePhase {
        self.phase
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self.phase, UpdatePhase::Commit | UpdatePhase::Rollback)
    }

    pub fn advance(&mut self, phase: UpdatePhase) -> Result<(), UpdateJournalError> {
        let expected = match self.phase {
            UpdatePhase::Stage => UpdatePhase::Verify,
            UpdatePhase::Verify => UpdatePhase::Select,
            UpdatePhase::Select => UpdatePhase::Health,
            UpdatePhase::Health => UpdatePhase::Commit,
            UpdatePhase::Commit | UpdatePhase::Rollback => {
                return Err(invalid_transition(self.phase, phase));
            }
        };
        if phase != expected {
            return Err(invalid_transition(self.phase, phase));
        }
        self.phase = phase;
        Ok(())
    }

    pub fn rollback(&mut self) -> Result<(), UpdateJournalError> {
        if self.is_terminal() {
            return Err(invalid_transition(self.phase, UpdatePhase::Rollback));
        }
        self.phase = UpdatePhase::Rollback;
        Ok(())
    }
}
