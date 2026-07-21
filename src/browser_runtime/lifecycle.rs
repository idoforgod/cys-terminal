//! Small, typed lifecycle contracts owned by cysd's lazy Browser authority broker.
//! Runtime selection state lives outside immutable signed bundles and is changed
//! through an ordered, crash-recoverable journal.

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

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

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UpdateJournalError {
    InvalidGeneration,
    InvalidRuntimeId,
    InvalidTransition { from: UpdatePhase, to: UpdatePhase },
    AlreadyInitialized,
    NotInitialized,
    UpdateInProgress,
    GenerationNotIncreasing,
    CorruptState(String),
    Storage(String),
}

impl std::fmt::Display for UpdateJournalError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidGeneration => formatter.write_str("runtime generation must be positive"),
            Self::InvalidRuntimeId => formatter.write_str("runtime id must be lowercase sha256"),
            Self::InvalidTransition { from, to } => {
                write!(
                    formatter,
                    "invalid runtime update transition: {from:?} -> {to:?}"
                )
            }
            Self::AlreadyInitialized => {
                formatter.write_str("runtime selection already initialized")
            }
            Self::NotInitialized => formatter.write_str("runtime selection is not initialized"),
            Self::UpdateInProgress => formatter.write_str("runtime update journal already exists"),
            Self::GenerationNotIncreasing => {
                formatter.write_str("runtime selection generation must increase")
            }
            Self::CorruptState(message) => {
                write!(formatter, "corrupt runtime selection state: {message}")
            }
            Self::Storage(message) => {
                write!(formatter, "runtime selection storage failed: {message}")
            }
        }
    }
}

impl std::error::Error for UpdateJournalError {}

impl From<std::io::Error> for UpdateJournalError {
    fn from(error: std::io::Error) -> Self {
        Self::Storage(error.to_string())
    }
}

impl From<serde_json::Error> for UpdateJournalError {
    fn from(error: serde_json::Error) -> Self {
        Self::CorruptState(error.to_string())
    }
}

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

const SELECTION_FILE: &str = "active-runtime.json";
const JOURNAL_FILE: &str = "runtime-update-journal.json";
const SELECTION_SCHEMA: u32 = 1;
static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn valid_runtime_id(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RuntimeSelection {
    schema_version: u32,
    generation: u64,
    runtime_id: String,
}

impl RuntimeSelection {
    pub fn new(generation: u64, runtime_id: String) -> Result<Self, UpdateJournalError> {
        if generation == 0 {
            return Err(UpdateJournalError::InvalidGeneration);
        }
        if !valid_runtime_id(&runtime_id) {
            return Err(UpdateJournalError::InvalidRuntimeId);
        }
        Ok(Self {
            schema_version: SELECTION_SCHEMA,
            generation,
            runtime_id,
        })
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn runtime_id(&self) -> &str {
        &self.runtime_id
    }

    fn validate(self) -> Result<Self, UpdateJournalError> {
        if self.schema_version != SELECTION_SCHEMA {
            return Err(UpdateJournalError::CorruptState(
                "unsupported selection schema".into(),
            ));
        }
        Self::new(self.generation, self.runtime_id)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct StoredUpdateJournal {
    schema_version: u32,
    generation: u64,
    phase: UpdatePhase,
    previous_generation: u64,
    previous_runtime_id: String,
    candidate_runtime_id: String,
}

impl StoredUpdateJournal {
    fn validate(&self) -> Result<UpdateJournal, UpdateJournalError> {
        if self.schema_version != SELECTION_SCHEMA
            || self.previous_generation == 0
            || self.previous_generation >= self.generation
            || !valid_runtime_id(&self.previous_runtime_id)
            || !valid_runtime_id(&self.candidate_runtime_id)
            || self.previous_runtime_id == self.candidate_runtime_id
        {
            return Err(UpdateJournalError::CorruptState(
                "invalid runtime update journal".into(),
            ));
        }
        let mut state = UpdateJournal::new(self.generation)?;
        match self.phase {
            UpdatePhase::Stage => {}
            UpdatePhase::Verify => state.advance(UpdatePhase::Verify)?,
            UpdatePhase::Select => {
                state.advance(UpdatePhase::Verify)?;
                state.advance(UpdatePhase::Select)?;
            }
            UpdatePhase::Health => {
                state.advance(UpdatePhase::Verify)?;
                state.advance(UpdatePhase::Select)?;
                state.advance(UpdatePhase::Health)?;
            }
            UpdatePhase::Commit => {
                state.advance(UpdatePhase::Verify)?;
                state.advance(UpdatePhase::Select)?;
                state.advance(UpdatePhase::Health)?;
                state.advance(UpdatePhase::Commit)?;
            }
            UpdatePhase::Rollback => state.rollback()?,
        }
        Ok(state)
    }
}

#[derive(Clone, Debug)]
pub struct RuntimeSelectionStore {
    root: PathBuf,
}

impl RuntimeSelectionStore {
    pub fn open(root: impl AsRef<Path>) -> Result<Self, UpdateJournalError> {
        let root = root.as_ref();
        if !root.is_absolute() {
            return Err(UpdateJournalError::Storage(
                "runtime selection root must be absolute".into(),
            ));
        }
        if root.exists() {
            let metadata = std::fs::symlink_metadata(root)?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(UpdateJournalError::Storage(
                    "runtime selection root must be an unlinked directory".into(),
                ));
            }
        } else {
            std::fs::create_dir_all(root)?;
        }
        let store = Self {
            root: root.to_path_buf(),
        };
        store.recover()?;
        Ok(store)
    }

    pub fn initialize(&self, selection: RuntimeSelection) -> Result<(), UpdateJournalError> {
        if self.selection_path().exists() || self.journal_path().exists() {
            return Err(UpdateJournalError::AlreadyInitialized);
        }
        atomic_write_json(&self.root, &self.selection_path(), &selection)
    }

    pub fn current(&self) -> Result<RuntimeSelection, UpdateJournalError> {
        if !self.selection_path().exists() {
            return Err(UpdateJournalError::NotInitialized);
        }
        read_json::<RuntimeSelection>(&self.selection_path())?.validate()
    }

    pub fn begin(
        &self,
        generation: u64,
        candidate_runtime_id: String,
    ) -> Result<RuntimeSelectionUpdate, UpdateJournalError> {
        if self.journal_path().exists() {
            return Err(UpdateJournalError::UpdateInProgress);
        }
        if !valid_runtime_id(&candidate_runtime_id) {
            return Err(UpdateJournalError::InvalidRuntimeId);
        }
        let current = self.current()?;
        if generation <= current.generation || candidate_runtime_id == current.runtime_id {
            return Err(UpdateJournalError::GenerationNotIncreasing);
        }
        let state = UpdateJournal::new(generation)?;
        let journal = StoredUpdateJournal {
            schema_version: SELECTION_SCHEMA,
            generation,
            phase: UpdatePhase::Stage,
            previous_generation: current.generation,
            previous_runtime_id: current.runtime_id,
            candidate_runtime_id,
        };
        atomic_write_json(&self.root, &self.journal_path(), &journal)?;
        Ok(RuntimeSelectionUpdate {
            store: self.clone(),
            state,
            journal,
        })
    }

    fn selection_path(&self) -> PathBuf {
        self.root.join(SELECTION_FILE)
    }

    fn journal_path(&self) -> PathBuf {
        self.root.join(JOURNAL_FILE)
    }

    fn remove_journal(&self) -> Result<(), UpdateJournalError> {
        match std::fs::remove_file(self.journal_path()) {
            Ok(()) => sync_directory(&self.root)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        Ok(())
    }

    fn recover(&self) -> Result<(), UpdateJournalError> {
        if !self.journal_path().exists() {
            return Ok(());
        }
        let mut journal = read_json::<StoredUpdateJournal>(&self.journal_path())?;
        journal.validate()?;
        match journal.phase {
            UpdatePhase::Commit => {
                let current = self.current()?;
                if current.generation != journal.generation
                    || current.runtime_id != journal.candidate_runtime_id
                {
                    return Err(UpdateJournalError::CorruptState(
                        "committed journal disagrees with active selection".into(),
                    ));
                }
            }
            UpdatePhase::Rollback => {
                let previous = self.previous_selection(&journal)?;
                atomic_write_json(&self.root, &self.selection_path(), &previous)?;
            }
            _ => {
                let previous = self.previous_selection(&journal)?;
                atomic_write_json(&self.root, &self.selection_path(), &previous)?;
                journal.phase = UpdatePhase::Rollback;
                atomic_write_json(&self.root, &self.journal_path(), &journal)?;
            }
        }
        self.remove_journal()
    }

    fn previous_selection(
        &self,
        journal: &StoredUpdateJournal,
    ) -> Result<RuntimeSelection, UpdateJournalError> {
        RuntimeSelection::new(
            journal.previous_generation,
            journal.previous_runtime_id.clone(),
        )
    }
}

pub struct RuntimeSelectionUpdate {
    store: RuntimeSelectionStore,
    state: UpdateJournal,
    journal: StoredUpdateJournal,
}

impl RuntimeSelectionUpdate {
    pub fn phase(&self) -> UpdatePhase {
        self.state.phase()
    }

    pub fn advance(&mut self, phase: UpdatePhase) -> Result<(), UpdateJournalError> {
        let mut next = self.state;
        next.advance(phase)?;
        match phase {
            UpdatePhase::Select => {
                let candidate = RuntimeSelection::new(
                    self.journal.generation,
                    self.journal.candidate_runtime_id.clone(),
                )?;
                atomic_write_json(&self.store.root, &self.store.selection_path(), &candidate)?;
            }
            UpdatePhase::Health | UpdatePhase::Commit => {
                let current = self.store.current()?;
                if current.generation != self.journal.generation
                    || current.runtime_id != self.journal.candidate_runtime_id
                {
                    return Err(UpdateJournalError::CorruptState(
                        "candidate is not the active runtime selection".into(),
                    ));
                }
            }
            UpdatePhase::Verify => {}
            UpdatePhase::Stage | UpdatePhase::Rollback => unreachable!("ordered state rejected"),
        }
        self.journal.phase = phase;
        atomic_write_json(&self.store.root, &self.store.journal_path(), &self.journal)?;
        self.state = next;
        if phase == UpdatePhase::Commit {
            self.store.remove_journal()?;
        }
        Ok(())
    }

    pub fn rollback(&mut self) -> Result<(), UpdateJournalError> {
        let mut next = self.state;
        next.rollback()?;
        let previous = self.store.previous_selection(&self.journal)?;
        atomic_write_json(&self.store.root, &self.store.selection_path(), &previous)?;
        self.journal.phase = UpdatePhase::Rollback;
        atomic_write_json(&self.store.root, &self.store.journal_path(), &self.journal)?;
        self.state = next;
        self.store.remove_journal()
    }
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, UpdateJournalError> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(UpdateJournalError::CorruptState(
            "selection state must be a regular unlinked file".into(),
        ));
    }
    let bytes = std::fs::read(path)?;
    if bytes.len() > 64 * 1024 {
        return Err(UpdateJournalError::CorruptState(
            "selection state exceeds 64 KiB".into(),
        ));
    }
    Ok(serde_json::from_slice(&bytes)?)
}

fn atomic_write_json<T: Serialize>(
    root: &Path,
    target: &Path,
    value: &T,
) -> Result<(), UpdateJournalError> {
    let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| UpdateJournalError::Storage("invalid selection filename".into()))?;
    let temporary = root.join(format!(".{name}.{}.{}.tmp", std::process::id(), sequence));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| -> Result<(), UpdateJournalError> {
        use std::io::Write;
        let mut file = options.open(&temporary)?;
        let mut bytes = serde_json::to_vec_pretty(value)?;
        bytes.push(b'\n');
        file.write_all(&bytes)?;
        file.sync_all()?;
        replace_file(&temporary, target)?;
        sync_directory(root)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::fs::rename(source, destination)
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let success = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if success == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> std::io::Result<()> {
    std::fs::File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> std::io::Result<()> {
    Ok(())
}
