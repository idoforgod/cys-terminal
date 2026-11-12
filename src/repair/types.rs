//! Shared repair types (C1, C3 error codes, U2 target states) — T0 skeleton.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

/// A repair number `R-<n>` (O-C1-2: monotonic, allocated by an exclusive directory create).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RepairId(String);

impl RepairId {
    /// Parses `R-<n>`. Stub in T0 (body: T3).
    pub fn parse(s: &str) -> Result<Self, RepairError> {
        let _ = s;
        Err(RepairError::NotImplemented("RepairId::parse"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A lane = one daemon's socket, named by the plain lane key (`base` or the sanitised socket key;
/// RULINGS I7 — no incarnation in the key).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LaneKey(pub String);

impl LaneKey {
    /// The lane of a daemon socket — exactly `crate::lane::lane_key` (the single owner of the rule).
    pub fn of_socket(p: &Path) -> Self {
        LaneKey(crate::lane::lane_key(p))
    }
}

/// The eight repair states (D:616 · C1 1.3); wire names N-1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepairState {
    InProgress,
    AwaitingApproval,
    Probation,
    InUse,
    ReopenedByAudit,
    Superseded,
    Reverted,
    UpstreamFixed,
}

impl RepairState {
    pub fn label_ko(&self) -> &'static str {
        match self {
            RepairState::InProgress => "진행 중",
            RepairState::AwaitingApproval => "승인 대기",
            RepairState::Probation => "시험 기간",
            RepairState::InUse => "사용 중",
            RepairState::ReopenedByAudit => "감사 지적으로 다시 열림",
            RepairState::Superseded => "다른 수리로 대체됨",
            RepairState::Reverted => "되돌려짐",
            RepairState::UpstreamFixed => "원본에 반영됨",
        }
    }

    /// Whether a repair in this state closes its grant claim — false only for the three
    /// "being worked" states (RULINGS F1 / F7 / G2).
    pub fn closes_for_grant(&self) -> bool {
        match self {
            RepairState::Probation
            | RepairState::InUse
            | RepairState::Reverted
            | RepairState::UpstreamFixed
            | RepairState::Superseded => true,
            RepairState::InProgress
            | RepairState::AwaitingApproval
            | RepairState::ReopenedByAudit => false,
        }
    }
}

/// Per-lane application state (D:954 · C1 1.4); wire names N-1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaneState {
    Pending,
    Applied,
    Verified,
    Conflict,
}

impl LaneState {
    pub fn label_ko(&self) -> &'static str {
        match self {
            LaneState::Pending => "대기",
            LaneState::Applied => "적용됨",
            LaneState::Verified => "검증됨",
            LaneState::Conflict => "충돌",
        }
    }
}

/// Classification of a repair (D:611); wire names N-1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Classification {
    General,
    RuleDocument,
    BootCritical,
    ProtectionRelated,
    DaemonSource,
}

impl Classification {
    pub fn label_ko(&self) -> &'static str {
        match self {
            Classification::General => "일반",
            Classification::RuleDocument => "규칙 문서",
            Classification::BootCritical => "기동 필수",
            Classification::ProtectionRelated => "보호장치 관련",
            Classification::DaemonSource => "데몬 소스(2차)",
        }
    }
}

/// What started a repair (D:607); wire names N-1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    OwnerOrder,
    Grant,
    Update,
}

impl Trigger {
    pub fn label_ko(&self) -> &'static str {
        match self {
            Trigger::OwnerOrder => "주인의 \"고쳐라\"",
            Trigger::Grant => "자동 수리 허락 아래의 발견",
            Trigger::Update => "업데이트 때의 충돌·재적용",
        }
    }
}

/// Lock owner = lane + seat + daemon start incarnation — never a CLI process id (C1 1.12).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockOwner {
    pub lane: LaneKey,
    pub seat: u64,
    pub incarnation: String,
}

/// A repair lock's lease (O-C1-5). `claim` = the grant claim the lock is bound to (RULINGS G1/G2);
/// `None` for owner-ordered and update repairs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Lease {
    pub owner: LockOwner,
    pub expires_at: f64,
    pub claim: Option<u32>,
}

/// Lease length: 2 h (O-C1-5).
pub const LEASE_SECS: u64 = 7_200;

/// Identity of a live entry in an operation record: volume UUID + inode + content hash (RULINGS E6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub volume_uuid: String,
    pub inode: u64,
    pub sha256: String,
}

/// What is signed, approved and compared is a target's STATE (RULINGS U2; V1 removed the hook state).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetState {
    Absent,
    /// `sha256` of the raw bytes (lowercase hex); `mode` = permission bits (`st_mode & 0o7777`),
    /// the constant 0 on Windows.
    File {
        sha256: String,
        mode: u32,
    },
    Link {
        target: String,
    },
}

impl TargetState {
    /// Reads a target's state with `lstat` semantics — a symbolic link is reported as a link and
    /// never followed. A regular file is opened without following links and re-checked through
    /// its open handle, so a swap between the `lstat` and the open cannot make this read follow a
    /// link. A missing path is `Absent`; a directory or any other kind of entry is refused.
    pub fn read(path: &Path) -> Result<TargetState, RepairError> {
        let meta = match std::fs::symlink_metadata(path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(TargetState::Absent),
            Err(e) => return Err(RepairError::Io(format!("{}: {e}", path.display()))),
        };
        let kind = meta.file_type();
        if kind.is_symlink() {
            let target = std::fs::read_link(path)
                .map_err(|e| RepairError::Io(format!("{}: {e}", path.display())))?;
            let target = target.into_os_string().into_string().map_err(|_| {
                RepairError::Io(format!(
                    "{}: link target is not valid UTF-8",
                    path.display()
                ))
            })?;
            return Ok(TargetState::Link { target });
        }
        if !kind.is_file() {
            return Err(RepairError::Precondition(format!(
                "{}: not a regular file or symbolic link",
                path.display()
            )));
        }
        let mut file = open_no_follow(path)
            .map_err(|e| RepairError::Io(format!("{}: {e}", path.display())))?;
        let opened = file
            .metadata()
            .map_err(|e| RepairError::Io(format!("{}: {e}", path.display())))?;
        if !opened.file_type().is_file() {
            return Err(RepairError::Precondition(format!(
                "{}: changed while being read (no longer a regular file)",
                path.display()
            )));
        }
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = std::io::Read::read(&mut file, &mut buf)
                .map_err(|e| RepairError::Io(format!("{}: {e}", path.display())))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        Ok(TargetState::File {
            sha256: hex(&hasher.finalize()),
            mode: permission_bits(&opened),
        })
    }

    /// `absent` | `file:<sha256>:<mode as 4 octal digits>` | `link:<target>` (RULINGS U2).
    pub fn canon(&self) -> String {
        match self {
            TargetState::Absent => "absent".to_string(),
            TargetState::File { sha256, mode } => format!("file:{sha256}:{mode:04o}"),
            TargetState::Link { target } => format!("link:{target}"),
        }
    }
}

#[cfg(unix)]
fn open_no_follow(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
}

#[cfg(not(unix))]
fn open_no_follow(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    // FILE_FLAG_OPEN_REPARSE_POINT: open a reparse point (link) itself instead of its target.
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

#[cfg(unix)]
fn permission_bits(meta: &std::fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o7777
}

#[cfg(not(unix))]
fn permission_bits(_meta: &std::fs::Metadata) -> u32 {
    0
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// THE fingerprint (RULINGS T2 as amended by U2; V1: file targets only) — the sha256 (lowercase
/// hex) of the lines `rel NUL canon LF`, the lines sorted bytewise. The one definition for the
/// attestations, the approval request, the apply, the origin's triple and identical propagation.
/// Every entry given is hashed: a repair's hook lines (`ChangeKind::HookUnregister`) are skipped by
/// the caller that walks the record (RULINGS W3(b)), not here.
pub fn fingerprint(targets: &[(String, TargetState)]) -> String {
    let mut lines: Vec<Vec<u8>> = targets
        .iter()
        .map(|(rel, state)| {
            let mut line = Vec::with_capacity(rel.len() + 80);
            line.extend_from_slice(rel.as_bytes());
            line.push(0);
            line.extend_from_slice(state.canon().as_bytes());
            line.push(b'\n');
            line
        })
        .collect();
    lines.sort();
    let mut hasher = Sha256::new();
    for line in &lines {
        hasher.update(line);
    }
    hex(&hasher.finalize())
}

/// Errors of the repair module (O-C3-6, B3, N-2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepairError {
    NotImplemented(&'static str),
    RoleRefused(String),
    OldDaemon,
    Locked(String),
    Precondition(String),
    CheckFailed(String),
    CheckTimeout(String),
    PlatformPending(&'static str),
    Corrupt(String),
    Superseded(String),
    Declined(String),
    Io(String),
}

impl RepairError {
    /// The daemon error code of this error (O-C3-6: new `repair_*` codes, reused `acl_denied`,
    /// `method_not_found`, `target_changed`; `io` as handlers.rs already answers it).
    pub fn code(&self) -> &'static str {
        match self {
            RepairError::NotImplemented(_) => ERR_NOT_IMPLEMENTED,
            RepairError::RoleRefused(_) => "acl_denied",
            RepairError::OldDaemon => "method_not_found",
            RepairError::Locked(_) => ERR_LOCKED,
            RepairError::Precondition(_) => ERR_PRECONDITION,
            RepairError::CheckFailed(_) => ERR_CHECK_FAILED,
            // RULINGS B3: at apply / commit time a check timeout IS a failure.
            RepairError::CheckTimeout(_) => ERR_CHECK_FAILED,
            RepairError::PlatformPending(_) => ERR_PLATFORM_PENDING,
            RepairError::Corrupt(_) => ERR_CORRUPT,
            RepairError::Superseded(_) => "target_changed",
            RepairError::Declined(_) => ERR_PRECONDITION,
            RepairError::Io(_) => "io",
        }
    }

    /// The `cys repair` exit code of this error (O-C3-6 range 92–99; `Io` = the CLI's generic 1).
    pub fn exit_code(&self) -> i32 {
        match self {
            RepairError::NotImplemented(_) => EXIT_REPAIR_NOT_IMPLEMENTED,
            RepairError::RoleRefused(_) => EXIT_REPAIR_ROLE_REFUSED,
            RepairError::OldDaemon => EXIT_REPAIR_OLD_DAEMON,
            RepairError::Locked(_) => EXIT_REPAIR_LOCKED,
            RepairError::Precondition(_) => EXIT_REPAIR_PRECONDITION,
            RepairError::CheckFailed(_) => EXIT_REPAIR_CHECK_FAILED,
            RepairError::CheckTimeout(_) => EXIT_REPAIR_CHECK_FAILED,
            RepairError::PlatformPending(_) => EXIT_REPAIR_PLATFORM_PENDING,
            RepairError::Corrupt(_) => EXIT_REPAIR_PRECONDITION,
            RepairError::Superseded(_) => EXIT_REPAIR_LOCKED,
            RepairError::Declined(_) => EXIT_REPAIR_PRECONDITION,
            RepairError::Io(_) => 1,
        }
    }
}

/// `cys repair` exit codes (O-C3-6; 99 = N-2). 0 = success.
pub const EXIT_REPAIR_ROLE_REFUSED: i32 = 92;
pub const EXIT_REPAIR_OLD_DAEMON: i32 = 93;
pub const EXIT_REPAIR_LOCKED: i32 = 94;
pub const EXIT_REPAIR_PRECONDITION: i32 = 95;
pub const EXIT_REPAIR_CHECK_FAILED: i32 = 96;
pub const EXIT_REPAIR_PLATFORM_PENDING: i32 = 97;
pub const EXIT_REPAIR_DAEMON_UNREACHABLE: i32 = 98;
pub const EXIT_REPAIR_NOT_IMPLEMENTED: i32 = 99;

/// New daemon error codes of the `repair.*` namespace (O-C3-6, N-2).
pub const ERR_NOT_IMPLEMENTED: &str = "repair_not_implemented";
pub const ERR_LOCKED: &str = "repair_locked";
pub const ERR_PRECONDITION: &str = "repair_precondition";
pub const ERR_CHECK_FAILED: &str = "repair_check_failed";
pub const ERR_PLATFORM_PENDING: &str = "repair_pending_platform";
pub const ERR_CORRUPT: &str = "repair_corrupt";

#[cfg(test)]
mod tests {
    use super::*;

    /// Korean labels exactly as the design (D:616 states, D:954 lane states, D:611 classes,
    /// D:607 triggers) and the N-1 wire names (serde).
    #[test]
    fn labels_and_wire_names() {
        let wire = |v: serde_json::Value| {
            v.as_str()
                .map(str::to_string)
                .unwrap_or_else(|| panic!("not a string: {v}"))
        };
        let states = [
            (RepairState::InProgress, "in_progress", "진행 중"),
            (
                RepairState::AwaitingApproval,
                "awaiting_approval",
                "승인 대기",
            ),
            (RepairState::Probation, "probation", "시험 기간"),
            (RepairState::InUse, "in_use", "사용 중"),
            (
                RepairState::ReopenedByAudit,
                "reopened_by_audit",
                "감사 지적으로 다시 열림",
            ),
            (RepairState::Superseded, "superseded", "다른 수리로 대체됨"),
            (RepairState::Reverted, "reverted", "되돌려짐"),
            (
                RepairState::UpstreamFixed,
                "upstream_fixed",
                "원본에 반영됨",
            ),
        ];
        for (s, w, ko) in states {
            assert_eq!(wire(serde_json::to_value(s).unwrap()), w);
            assert_eq!(s.label_ko(), ko);
            assert_eq!(
                serde_json::from_value::<RepairState>(serde_json::json!(w)).unwrap(),
                s
            );
        }
        let lanes = [
            (LaneState::Pending, "pending", "대기"),
            (LaneState::Applied, "applied", "적용됨"),
            (LaneState::Verified, "verified", "검증됨"),
            (LaneState::Conflict, "conflict", "충돌"),
        ];
        for (s, w, ko) in lanes {
            assert_eq!(wire(serde_json::to_value(s).unwrap()), w);
            assert_eq!(s.label_ko(), ko);
            assert_eq!(
                serde_json::from_value::<LaneState>(serde_json::json!(w)).unwrap(),
                s
            );
        }
        let classes = [
            (Classification::General, "general", "일반"),
            (Classification::RuleDocument, "rule_document", "규칙 문서"),
            (Classification::BootCritical, "boot_critical", "기동 필수"),
            (
                Classification::ProtectionRelated,
                "protection_related",
                "보호장치 관련",
            ),
            (
                Classification::DaemonSource,
                "daemon_source",
                "데몬 소스(2차)",
            ),
        ];
        for (c, w, ko) in classes {
            assert_eq!(wire(serde_json::to_value(c).unwrap()), w);
            assert_eq!(c.label_ko(), ko);
            assert_eq!(
                serde_json::from_value::<Classification>(serde_json::json!(w)).unwrap(),
                c
            );
        }
        let triggers = [
            (Trigger::OwnerOrder, "owner_order", "주인의 \"고쳐라\""),
            (Trigger::Grant, "grant", "자동 수리 허락 아래의 발견"),
            (Trigger::Update, "update", "업데이트 때의 충돌·재적용"),
        ];
        for (t, w, ko) in triggers {
            assert_eq!(wire(serde_json::to_value(t).unwrap()), w);
            assert_eq!(t.label_ko(), ko);
            assert_eq!(
                serde_json::from_value::<Trigger>(serde_json::json!(w)).unwrap(),
                t
            );
        }
    }

    /// The 5 states that close a grant claim and the 3 "being worked" states (RULINGS F1, G2).
    #[test]
    fn closes_for_grant_table() {
        let closed = [
            RepairState::Probation,
            RepairState::InUse,
            RepairState::Reverted,
            RepairState::UpstreamFixed,
            RepairState::Superseded,
        ];
        let worked = [
            RepairState::InProgress,
            RepairState::AwaitingApproval,
            RepairState::ReopenedByAudit,
        ];
        for s in closed {
            assert!(s.closes_for_grant(), "{s:?} must close the grant claim");
        }
        for s in worked {
            assert!(
                !s.closes_for_grant(),
                "{s:?} is being worked — must not close the grant claim"
            );
        }
        assert_eq!(closed.len() + worked.len(), 8);
    }

    /// `code()` / `exit_code()` table; the eight exit codes are 92..=99 and distinct; the CLI
    /// source defines no other `const EXIT_*: i32 = 92..99`.
    #[test]
    fn error_and_exit_codes() {
        let s = || "x".to_string();
        let table: [(RepairError, &str, i32); 12] = [
            (
                RepairError::NotImplemented("f"),
                "repair_not_implemented",
                99,
            ),
            (RepairError::RoleRefused(s()), "acl_denied", 92),
            (RepairError::OldDaemon, "method_not_found", 93),
            (RepairError::Locked(s()), "repair_locked", 94),
            (RepairError::Precondition(s()), "repair_precondition", 95),
            (RepairError::CheckFailed(s()), "repair_check_failed", 96),
            (RepairError::CheckTimeout(s()), "repair_check_failed", 96),
            (
                RepairError::PlatformPending("p"),
                "repair_pending_platform",
                97,
            ),
            (RepairError::Corrupt(s()), "repair_corrupt", 95),
            (RepairError::Superseded(s()), "target_changed", 94),
            (RepairError::Declined(s()), "repair_precondition", 95),
            (RepairError::Io(s()), "io", 1),
        ];
        for (e, code, exit) in &table {
            assert_eq!(e.code(), *code, "{e:?}");
            assert_eq!(e.exit_code(), *exit, "{e:?}");
        }
        let exits = [
            EXIT_REPAIR_ROLE_REFUSED,
            EXIT_REPAIR_OLD_DAEMON,
            EXIT_REPAIR_LOCKED,
            EXIT_REPAIR_PRECONDITION,
            EXIT_REPAIR_CHECK_FAILED,
            EXIT_REPAIR_PLATFORM_PENDING,
            EXIT_REPAIR_DAEMON_UNREACHABLE,
            EXIT_REPAIR_NOT_IMPLEMENTED,
        ];
        assert_eq!(exits, [92, 93, 94, 95, 96, 97, 98, 99]);
        let mut sorted = exits.to_vec();
        sorted.dedup();
        assert_eq!(sorted.len(), 8, "exit codes are not distinct");
        assert_eq!(
            [
                ERR_NOT_IMPLEMENTED,
                ERR_LOCKED,
                ERR_PRECONDITION,
                ERR_CHECK_FAILED,
                ERR_PLATFORM_PENDING,
                ERR_CORRUPT
            ],
            [
                "repair_not_implemented",
                "repair_locked",
                "repair_precondition",
                "repair_check_failed",
                "repair_pending_platform",
                "repair_corrupt"
            ]
        );
        // The 92–99 range belongs to `cys repair` alone (O-C3-6 checked it unused at the tag).
        let cli = include_str!("../bin/cys.rs");
        assert!(
            cli.len() > 1_000_000,
            "src/bin/cys.rs is not the real CLI source"
        );
        let re = regex::Regex::new(r"const EXIT_[A-Z0-9_]*: i32 = 9[2-9]\b").unwrap();
        let hits: Vec<&str> = re.find_iter(cli).map(|m| m.as_str()).collect();
        assert!(
            hits.is_empty(),
            "src/bin/cys.rs defines another exit code in 92–99: {hits:?}"
        );
    }

    /// The fixed vectors of RULINGS T2 / U2 (re-pinned by V1: no hook line). The hex values were
    /// computed independently (Python hashlib) from the same lines.
    #[test]
    fn fingerprint_vectors() {
        let abc = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".to_string();
        let file = |mode| {
            (
                "bin/javis_a.py".to_string(),
                TargetState::File {
                    sha256: abc.clone(),
                    mode,
                },
            )
        };
        let absent = ("bin/gone.py".to_string(), TargetState::Absent);
        let link = (
            "hooks/x.sh".to_string(),
            TargetState::Link {
                target: "../bin/x.sh".to_string(),
            },
        );
        assert_eq!(file(0o644).1.canon(), format!("file:{abc}:0644"));
        assert_eq!(file(0o755).1.canon(), format!("file:{abc}:0755"));
        assert_eq!(link.1.canon(), "link:../bin/x.sh");
        assert_eq!(absent.1.canon(), "absent");
        // one file
        assert_eq!(
            fingerprint(&[file(0o644)]),
            "f5b92c8d43879fe721f91b365d894cb5334d38600a2fe1f2d981c3f76abbefd2"
        );
        // one mode-only change (same bytes, exec bit set) — a different fingerprint
        assert_eq!(
            fingerprint(&[file(0o755)]),
            "e3b49a6231c3e444b4ab1f6008b0db8c2090301cdd3a5da2b70b582716f4e69f"
        );
        // one link
        assert_eq!(
            fingerprint(&[link.clone()]),
            "09037962f6f2dde2c522bc3ae7eb97f011be0ea14bb440e8e0cafa817f7698a9"
        );
        // one absent
        assert_eq!(
            fingerprint(&[absent.clone()]),
            "30226a2b55abdba20d3d94dcd3ffbc43bde6e28bdf6f5cc1eda902991466a69a"
        );
        // two entries, given in either order
        let two = "0227d03ed329ae9225f0ea36183bfb4fd5589f4b54b432c6aab234e7f541ea64";
        assert_eq!(fingerprint(&[file(0o644), absent.clone()]), two);
        assert_eq!(fingerprint(&[absent, file(0o644)]), two);
        // no targets = sha256 of nothing
        assert_eq!(
            fingerprint(&[]),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    /// `TargetState::read` on a file, an exec-bit file, a symlink (not followed) and a missing
    /// path gives the four `canon` forms; `PreImage::state()` agrees with what was read (U2).
    #[test]
    fn target_state_read_and_canon() {
        use crate::repair::ledger::PreImage;
        let dir = std::env::temp_dir().join(format!(
            "cys-repair-tstate-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let abc = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        let plain = dir.join("plain.txt");
        std::fs::write(&plain, b"abc").unwrap();
        let missing = dir.join("missing.txt");
        assert_eq!(TargetState::read(&missing).unwrap(), TargetState::Absent);
        assert_eq!(TargetState::read(&missing).unwrap().canon(), "absent");
        let as_pre = |s: &TargetState| match s {
            TargetState::Absent => PreImage::Absent,
            TargetState::File { sha256, mode } => PreImage::Slot {
                slot_dir: dir.join("slot"),
                sha256: sha256.clone(),
                mode: *mode,
                link_target: None,
            },
            TargetState::Link { target } => PreImage::Slot {
                slot_dir: dir.join("slot"),
                sha256: String::new(),
                mode: 0,
                link_target: Some(target.clone()),
            },
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&plain, std::fs::Permissions::from_mode(0o644)).unwrap();
            let exec = dir.join("exec.sh");
            std::fs::write(&exec, b"abc").unwrap();
            std::fs::set_permissions(&exec, std::fs::Permissions::from_mode(0o755)).unwrap();
            let link = dir.join("link");
            std::os::unix::fs::symlink("plain.txt", &link).unwrap();
            let dangling = dir.join("dangling");
            std::os::unix::fs::symlink("nowhere/at/all", &dangling).unwrap();
            let cases = [
                (&plain, format!("file:{abc}:0644")),
                (&exec, format!("file:{abc}:0755")),
                (&link, "link:plain.txt".to_string()),
                (&dangling, "link:nowhere/at/all".to_string()),
                (&missing, "absent".to_string()),
            ];
            for (p, want) in &cases {
                let got = TargetState::read(p).unwrap();
                assert_eq!(got.canon(), *want, "{}", p.display());
                assert_eq!(
                    as_pre(&got).state(),
                    got,
                    "PreImage::state disagrees for {}",
                    p.display()
                );
            }
            // a directory is not a target
            assert!(matches!(
                TargetState::read(&dir),
                Err(RepairError::Precondition(_))
            ));
        }
        #[cfg(not(unix))]
        {
            let got = TargetState::read(&plain).unwrap();
            assert_eq!(got.canon(), format!("file:{abc}:0000"));
            assert_eq!(as_pre(&got).state(), got);
            assert!(matches!(
                TargetState::read(&dir),
                Err(RepairError::Precondition(_))
            ));
        }
        assert_eq!(PreImage::Absent.state(), TargetState::Absent);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
