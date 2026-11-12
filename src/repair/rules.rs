//! Role rule blocks delivered by the binary (C6 · design §4-1). T0: empty constants and a stub;
//! T1a fills the texts and `rules_text` (printed by the LOCAL verb `cys repair rules --role <role>`,
//! RULINGS B10).

use super::roles::RoleBlock;
use super::types::RepairError;

/// Common to every role (C6 6.3). Empty until T1a.
pub const RULES_COMMON: &str = "";
/// master · CEO block (C6 6.2). Empty until T1a.
pub const RULES_MASTER_CEO: &str = "";
/// worker block (C6 6.2). Empty until T1a.
pub const RULES_WORKER: &str = "";
/// CSO block (C6 6.2). Empty until T1a.
pub const RULES_CSO: &str = "";
/// reviewer block — both duties in one block (C6 6.2, O-C6-2). Empty until T1a.
pub const RULES_REVIEWER: &str = "";

/// The text printed for a role block. Stub in T0 (body: T1a).
pub fn rules_text(b: RoleBlock) -> Result<String, RepairError> {
    let _ = b;
    Err(RepairError::NotImplemented("rules::rules_text"))
}
