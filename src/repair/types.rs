//! Shared repair types (C1, C3 error codes, U2 target states) — T0 skeleton.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

/// A repair number `R-<n>` (O-C1-2: monotonic, allocated by an exclusive directory create).
/// RULINGS Z2(c): a value only ever comes from [`RepairId::parse`] — deserialisation goes through it,
/// so nothing that is not `R-<n>` can become a path component. Z2(d): ordering is numeric.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct RepairId(String);

/// The number of a canonical `R-<n>` (n ≥ 1, decimal, no sign, no leading zero, fits `u64`).
fn repair_number(s: &str) -> Option<u64> {
    let digits = s.strip_prefix("R-")?;
    if digits.is_empty()
        || digits.len() > 20
        || digits.starts_with('0')
        || !digits.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    digits.parse::<u64>().ok()
}

impl RepairId {
    /// Parses `R-<n>` (canonical spelling only: `R-07`, `R-0`, `r-7`, `R-7 ` are refused, so one
    /// number has exactly one directory name).
    pub fn parse(s: &str) -> Result<Self, RepairError> {
        match repair_number(s) {
            Some(_) => Ok(RepairId(s.to_string())),
            None => Err(RepairError::Precondition(format!(
                "수리 번호는 R-<숫자> 형식이어야 합니다: {s:?}"
            ))),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for RepairId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        RepairId::parse(&s).map_err(|_| serde::de::Error::custom(format!("not a repair id: {s:?}")))
    }
}

impl Ord for RepairId {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Both are canonical (only `parse` builds a value), so equal numbers ⇔ equal strings.
        (repair_number(&self.0), &self.0).cmp(&(repair_number(&other.0), &other.0))
    }
}

impl PartialOrd for RepairId {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
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
            // RULINGS Z2(a): only the permission bits — a raw `st_mode` read back from a record
            // must not produce 5–6 digits.
            TargetState::File { sha256, mode } => format!("file:{sha256}:{:04o}", mode & 0o7777),
            TargetState::Link { target } => format!("link:{target}"),
        }
    }
}

/// RULINGS Z2(b): `O_NONBLOCK` beside `O_NOFOLLOW` — a FIFO swapped in after the `lstat` must not
/// block a daemon thread on `open`; the is-file re-check on the opened handle then refuses it.
#[cfg(unix)]
fn open_no_follow(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
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

/// RULINGS AU4 / BJ3: whether a pack-relative target is one of the pack's locked security files
/// (`crate::pack::SYSTEM_LOCKED` — `trusted-keys.json`): never a repair target. Compared after full
/// case folding (case-insensitive APFS / NTFS open the same file under another spelling).
pub fn is_system_locked(rel: &str) -> bool {
    system_locked_entry(rel).is_some()
}

/// The `SYSTEM_LOCKED` entry `rel` names after full case folding ([`fold_name`] — RULINGS BJ3:
/// `trusted-\u{212A}eys.json`, `tru\u{017F}ted-keys.json` ARE the key file on APFS / NTFS), in the
/// pack's own spelling.
pub fn system_locked_entry(rel: &str) -> Option<&'static str> {
    let folded = fold_name(rel);
    crate::pack::SYSTEM_LOCKED
        .iter()
        .copied()
        .find(|l| fold_name(l) == folded)
}

/// RULINGS BJ3 — the ONE name comparison of protected names: Unicode case folding approximated by
/// upper- then lower-casing every character (`ſ` → `S` → `s`, the Kelvin sign `K` → `k`, `ß` →
/// `ss`), so every spelling a case-insensitive file system opens as the same name compares equal.
pub fn fold_name(s: &str) -> String {
    s.to_uppercase().to_lowercase()
}

/// Whether a no-follow stat result is a link for the repair rules: a symbolic link, and on Windows
/// any reparse point (a junction, a mount point, a cloud placeholder …) — RULINGS AW3 (b).
fn is_link_like(meta: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    meta.file_type().is_symlink()
}

/// RULINGS AW3 (b) — the ONE parent-chain check (the daemon's `begin` / `record` / `commit` and
/// T6's apply call it): every PARENT component of `rel` under `pack`, from the pack root down, is
/// read with a no-follow stat; a symbolic link (on Windows any reparse point) or a component that
/// is not a directory refuses. A missing component ends the walk (nothing below it exists). The
/// leaf itself may be a link (U2) and is not checked; the pack root itself is not checked.
pub fn parent_chain_link(pack: &Path, rel: &str) -> Result<(), RepairError> {
    let parts: Vec<&str> = rel.split(['/', '\\']).filter(|p| !p.is_empty()).collect();
    let mut at = pack.to_path_buf();
    for (i, part) in parts.iter().enumerate() {
        if i + 1 == parts.len() {
            break; // the leaf
        }
        at.push(part);
        let meta = match std::fs::symlink_metadata(&at) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => {
                return Err(RepairError::Precondition(format!(
                    "{rel}: 상위 경로 {} 를 확인할 수 없습니다 — {e}",
                    parts[..=i].join("/")
                )))
            }
        };
        if is_link_like(&meta) {
            return Err(RepairError::Precondition(format!(
                "{rel}: 상위 경로 {} 가 링크(또는 연결 지점 · 클라우드 동기화 폴더의 자리표시)입니다 — 링크를 거친 경로는 수리 대상이 아닙니다(RULINGS AW3)",
                parts[..=i].join("/")
            )));
        }
        if !meta.is_dir() {
            return Err(RepairError::Precondition(format!(
                "{rel}: 상위 경로 {} 가 폴더가 아닙니다",
                parts[..=i].join("/")
            )));
        }
    }
    Ok(())
}

/// The pack roots of every lane of this installation as seen from one lane's pack: the pack itself,
/// and in its parent (the cys base) `pack` and every `pack-dept-*` (the same rule as
/// `factory_reset::command_points_into_pack`). Each root also in its canonical form.
pub fn lane_pack_roots(pack: &Path) -> Vec<std::path::PathBuf> {
    let mut roots = vec![pack.to_path_buf()];
    if let Some(base) = pack.parent() {
        if let Ok(rd) = std::fs::read_dir(base) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if name == "pack" || name.starts_with("pack-dept-") {
                    roots.push(e.path());
                }
            }
        }
    }
    let canon: Vec<std::path::PathBuf> = roots
        .iter()
        .filter_map(|r| std::fs::canonicalize(r).ok())
        .collect();
    roots.extend(canon);
    roots.sort();
    roots.dedup();
    roots
}

/// RULINGS AW3 (c) / BJ1: `None` when a candidate link at `pack/<rel>` pointing to `target` is
/// allowed; otherwise the reason. The target is resolved PHYSICALLY, as the operating system opens
/// it: from the link's own canonical parent, component by component, every existing component
/// canonicalised (so a link inside the target is followed BEFORE a later `..` is applied — no
/// textual `..` clean-up). Refused when (i) the longest existing prefix lies inside any lane's pack
/// root ([`lane_pack_roots`]), (ii) the not-yet-existing rest contains `..`, (iii) the fully
/// resolved target is a directory or lies inside a pack, or the target cannot be resolved (a link
/// loop, an unreadable component — fail closed). A link to a user file outside the packs (also a
/// file that does not exist yet) stays allowed: a repair never creates an alias of a pack path.
pub fn link_target_refusal(pack: &Path, rel: &str, target: &str) -> Option<String> {
    use std::path::{Component, PathBuf};
    let roots = lane_pack_roots(pack);
    let inside = |p: &Path| roots.iter().any(|r| p.starts_with(r));
    let in_pack = || {
        Some(format!(
            "{rel}: 링크가 팩 안의 경로({target})를 가리킵니다 — 수리는 팩 경로의 별칭을 만들지 않습니다(RULINGS AW3)"
        ))
    };
    let unresolved = |why: String| {
        Some(format!(
            "{rel}: 링크가 가리키는 곳({target})을 끝까지 확인할 수 없어 받지 않습니다 — {why}"
        ))
    };
    let live = pack.join(rel);
    let start = match std::fs::canonicalize(live.parent().unwrap_or(pack)) {
        Ok(p) => p,
        Err(e) => return unresolved(e.to_string()),
    };
    let t = Path::new(target);
    let raw = if t.is_absolute() {
        t.to_path_buf()
    } else {
        start.join(t)
    };
    let mut cur = PathBuf::new();
    let mut rest: Vec<Component<'_>> = Vec::new();
    for c in raw.components() {
        if !rest.is_empty() {
            rest.push(c);
            continue;
        }
        match c {
            Component::Prefix(_) | Component::RootDir => cur.push(c.as_os_str()),
            Component::CurDir => {}
            // `cur` is canonical (every link in it already followed): its parent is the physical one
            Component::ParentDir => {
                cur.pop();
            }
            Component::Normal(name) => {
                let next = cur.join(name);
                match std::fs::symlink_metadata(&next) {
                    Ok(_) => match std::fs::canonicalize(&next) {
                        Ok(resolved) => cur = resolved,
                        Err(e) => return unresolved(e.to_string()),
                    },
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => rest.push(c),
                    Err(e) => return unresolved(e.to_string()),
                }
            }
        }
    }
    // (i) the longest existing prefix, canonical
    if inside(cur.as_path()) {
        return in_pack();
    }
    // (ii) `..` in the part that does not exist yet (the OS would resolve it after creation)
    if rest.iter().any(|c| matches!(c, Component::ParentDir)) {
        return Some(format!(
            "{rel}: 링크 대상({target})의 아직 없는 부분에 '..' 가 있습니다 — 받지 않습니다(RULINGS AW3)"
        ));
    }
    // (iii) the fully resolved target
    let mut full = cur.clone();
    for c in &rest {
        if let Component::Normal(name) = c {
            full.push(name);
        }
    }
    if rest.is_empty() && std::fs::metadata(&full).is_ok_and(|m| m.is_dir()) {
        return Some(format!(
            "{rel}: 링크가 폴더({target})를 가리킵니다 — 수리는 폴더를 가리키는 링크를 만들지 않습니다(RULINGS AW3)"
        ));
    }
    if inside(full.as_path()) {
        return in_pack();
    }
    None
}

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

/// RULINGS Z2 — the five small corrections T3 carries to the T0 library (each with a test).
#[cfg(test)]
mod z2_tests {
    use super::*;

    /// Z2(a): `canon` formats only the permission bits of a raw `st_mode`.
    #[test]
    fn canon_masks_mode_bits() {
        let raw = TargetState::File {
            sha256: "ab".into(),
            mode: 0o100_755, // S_IFREG | 0755 as `st_mode` reports it
        };
        assert_eq!(raw.canon(), "file:ab:0755");
        let setuid = TargetState::File {
            sha256: "ab".into(),
            mode: 0o104_755,
        };
        assert_eq!(setuid.canon(), "file:ab:4755");
        // the fingerprint follows the canon, so a raw mode and its bits fingerprint alike
        assert_eq!(
            fingerprint(&[("a".into(), raw)]),
            fingerprint(&[(
                "a".into(),
                TargetState::File {
                    sha256: "ab".into(),
                    mode: 0o755
                }
            )])
        );
    }

    /// Z2(b): the no-follow open does not block on a FIFO (O_NONBLOCK), and `read` refuses a FIFO.
    /// Without O_NONBLOCK the open of a FIFO with no writer blocks forever — the thread below would
    /// not answer within the deadline.
    #[cfg(unix)]
    #[test]
    fn read_opens_nonblocking() {
        let dir =
            std::env::temp_dir().join(format!("cys-repair-z2b-{}-{}", std::process::id(), line!()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let fifo = dir.join("pipe");
        let c = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
        // SAFETY: a valid NUL-terminated path; mkfifo only creates the node.
        assert_eq!(
            unsafe { libc::mkfifo(c.as_ptr(), 0o600) },
            0,
            "mkfifo failed"
        );
        let (tx, rx) = std::sync::mpsc::channel();
        let f2 = fifo.clone();
        std::thread::spawn(move || {
            let r = open_no_follow(&f2).map(|f| f.metadata().map(|m| m.file_type().is_fifo()));
            let _ = tx.send(r.is_ok());
        });
        let answered = rx.recv_timeout(std::time::Duration::from_secs(5));
        if answered.is_err() {
            // unblock the stuck opener before failing (open for writing releases it)
            let _ = std::fs::OpenOptions::new().write(true).open(&fifo);
        }
        assert_eq!(
            answered,
            Ok(true),
            "open of a FIFO blocked — O_NONBLOCK missing"
        );
        assert!(matches!(
            TargetState::read(&fifo),
            Err(RepairError::Precondition(_))
        ));
        let src = include_str!("types.rs");
        let prod = &src[..src.find("#[cfg(test)]").unwrap()];
        assert!(prod.contains("libc::O_NOFOLLOW | libc::O_NONBLOCK"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    use std::os::unix::fs::FileTypeExt;

    /// Z2(c): `RepairId` deserialises through `parse` — nothing that is not `R-<n>` becomes an id.
    #[test]
    fn repair_id_deserialises_through_parse() {
        for ok in ["R-1", "R-12", "R-18446744073709551615"] {
            let id: RepairId = serde_json::from_value(serde_json::json!(ok)).unwrap();
            assert_eq!(id.as_str(), ok);
            assert_eq!(RepairId::parse(ok).unwrap(), id);
            assert_eq!(serde_json::to_value(&id).unwrap(), serde_json::json!(ok));
        }
        for bad in [
            "",
            "R-",
            "R-0",
            "R-07",
            "r-7",
            "R7",
            "R-7 ",
            " R-7",
            "R-+7",
            "R--7",
            "R-7a",
            "R-1/..",
            "../R-1",
            "R-1\u{0}",
            "R-18446744073709551616",
            "R-１",
            "staging",
            "intake",
        ] {
            assert!(RepairId::parse(bad).is_err(), "parse accepted {bad:?}");
            assert!(
                serde_json::from_value::<RepairId>(serde_json::json!(bad)).is_err(),
                "deserialise accepted {bad:?}"
            );
        }
        // inside a record, a bad id makes the whole record unreadable (O-C1-6), never a path
        assert!(serde_json::from_value::<RepairId>(serde_json::json!(7)).is_err());
    }

    /// Z2(d): ordering is numeric — `R-10` is after `R-9`.
    #[test]
    fn repair_id_orders_numerically() {
        let p = |s: &str| RepairId::parse(s).unwrap();
        assert!(p("R-9") < p("R-10"));
        assert!(p("R-2") < p("R-100"));
        assert!(p("R-100") > p("R-99"));
        let mut v = vec![p("R-10"), p("R-9"), p("R-1"), p("R-100"), p("R-11")];
        v.sort();
        let got: Vec<&str> = v.iter().map(|i| i.as_str()).collect();
        assert_eq!(got, ["R-1", "R-9", "R-10", "R-11", "R-100"]);
        assert_eq!(p("R-5").cmp(&p("R-5")), std::cmp::Ordering::Equal);
    }
}

#[cfg(test)]
mod t3c_tests {
    use super::*;

    #[cfg(unix)]
    fn tmp(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("cys-t3c-types-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// RULINGS AU4: the locked key file, any case; nothing else.
    #[test]
    fn system_locked_is_the_pack_constant() {
        for rel in crate::pack::SYSTEM_LOCKED {
            assert!(is_system_locked(rel));
            assert!(is_system_locked(&rel.to_ascii_uppercase()));
        }
        assert!(is_system_locked("Trusted-Keys.JSON"));
        // RULINGS BJ3: spellings a case-insensitive file system opens as the key file
        for rel in [
            "trusted-\u{212A}eys.json",
            "tru\u{017F}ted-keys.json",
            "TRU\u{017F}TED-KEYS.JSON",
        ] {
            assert_eq!(system_locked_entry(rel), Some("trusted-keys.json"), "{rel}");
        }
        assert_eq!(fold_name("Stra\u{00DF}e"), "strasse");
        for rel in ["bin/trusted-keys.json", "trusted-keys.json.bak", "bin/a.py"] {
            assert!(!is_system_locked(rel), "{rel}");
        }
    }

    /// RULINGS AW3 (b): a link anywhere in the parent chain refuses; the leaf may be a link; a
    /// missing parent ends the walk; a file used as a parent refuses.
    #[cfg(unix)]
    #[test]
    fn parent_chain_links_refused() {
        let base = tmp("chain");
        let pack = base.join("pack");
        std::fs::create_dir_all(pack.join("hooks/sub")).unwrap();
        std::fs::write(pack.join("hooks/guard.sh"), b"x").unwrap();
        std::os::unix::fs::symlink(pack.join("hooks"), pack.join("x")).unwrap();
        std::os::unix::fs::symlink("sub", pack.join("hooks/rel-link")).unwrap();
        std::os::unix::fs::symlink(pack.join("hooks/guard.sh"), pack.join("hooks/leaf")).unwrap();
        assert!(parent_chain_link(&pack, "hooks/guard.sh").is_ok());
        assert!(
            parent_chain_link(&pack, "hooks/leaf").is_ok(),
            "the leaf may be a link (U2)"
        );
        assert!(
            parent_chain_link(&pack, "hooks/new/deeper/file.sh").is_ok(),
            "missing parents"
        );
        let e = parent_chain_link(&pack, "x/guard.sh").unwrap_err();
        assert!(format!("{e:?}").contains("링크"), "{e:?}");
        assert!(parent_chain_link(&pack, "hooks/rel-link/a.sh").is_err());
        assert!(
            parent_chain_link(&pack, "hooks/guard.sh/a").is_err(),
            "a file as a parent"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// RULINGS AW3 (c): a link to a directory, or into any lane's pack, is refused; a link to a user
    /// file outside the packs is allowed.
    #[cfg(unix)]
    #[test]
    fn link_targets_into_packs_or_directories_refused() {
        let base = tmp("links");
        let pack = base.join("pack");
        let dept = base.join("pack-dept-d1");
        let user = base.join("user");
        std::fs::create_dir_all(pack.join("bin")).unwrap();
        std::fs::create_dir_all(dept.join("hooks")).unwrap();
        std::fs::create_dir_all(&user).unwrap();
        std::fs::write(pack.join("hooks.sh"), b"x").unwrap();
        std::fs::write(dept.join("hooks/guard.sh"), b"x").unwrap();
        std::fs::write(user.join("notes.txt"), b"x").unwrap();
        let roots = lane_pack_roots(&pack);
        assert!(
            roots.iter().any(|r| r.ends_with("pack-dept-d1")),
            "{roots:?}"
        );
        // into this pack (relative and absolute), into another lane's pack, a directory
        for t in [
            "../hooks.sh".to_string(),
            pack.join("hooks.sh").display().to_string(),
            dept.join("hooks/guard.sh").display().to_string(),
            user.display().to_string(),
            "..".to_string(),
            pack.join("bin/not-yet.py").display().to_string(),
        ] {
            assert!(
                link_target_refusal(&pack, "bin/a.py", &t).is_some(),
                "{t} allowed"
            );
        }
        // a user file outside the packs (also a dangling one)
        for t in [
            user.join("notes.txt").display().to_string(),
            user.join("missing.txt").display().to_string(),
            "../../user/notes.txt".to_string(),
        ] {
            assert!(
                link_target_refusal(&pack, "bin/a.py", &t).is_none(),
                "{t} refused"
            );
        }
        // RULINGS BJ1: an outside link followed by `..` resolves physically into the pack's hooks
        std::fs::create_dir_all(pack.join("hooks")).unwrap();
        std::fs::write(pack.join("hooks/guard.sh"), b"x").unwrap();
        let outside = base.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(pack.join("hooks"), outside.join("lnk")).unwrap();
        let sneaky = format!("{}/lnk/../hooks/guard.sh", outside.display());
        let why =
            link_target_refusal(&pack, "bin/helper.sh", &sneaky).expect("the alias was allowed");
        assert!(why.contains("팩 안의 경로"), "{why}");
        // a missing path two levels deep inside a pack; `..` in the part that does not exist yet
        for t in [
            pack.join("bin/new/deeper/x.py").display().to_string(),
            format!("{}/missing/../../pack/hooks.sh", user.display()),
        ] {
            assert!(
                link_target_refusal(&pack, "bin/a.py", &t).is_some(),
                "{t} allowed"
            );
        }
        // `pack-notes` is not a pack (boundary match)
        let notes = base.join("pack-notes");
        std::fs::create_dir_all(&notes).unwrap();
        std::fs::write(notes.join("n.txt"), b"x").unwrap();
        assert!(link_target_refusal(
            &pack,
            "bin/a.py",
            &notes.join("n.txt").display().to_string()
        )
        .is_none());
        let _ = std::fs::remove_dir_all(&base);
    }
}
