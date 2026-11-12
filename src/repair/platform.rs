//! The ONE platform switch of the repair feature (C12 O-C12-1 · RULINGS B9). Every track calls
//! these four functions and never `cfg!(windows)` directly. Compile-time values plus one constant —
//! no environment variable is read (no agent can flip the switch).

/// How a live pack file is replaced (C2 / C12 12.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyForm {
    /// macOS: atomic exchange (rule one).
    Exchange,
    /// Windows: deposit the live file into the vault first, then place without replace.
    DepositFirst,
    /// Any other target (interpretation in O-C12-1).
    Unsupported,
}

/// Whether boot-critical / protection-related repairs (the final-approval channel) are available.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootCriticalGate {
    Enabled,
    /// Not available; the reason is told to the user (C12 12.2).
    Pending(&'static str),
}

/// What the hourly unapproved-change sweep (M10) does with a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum M10Mode {
    Revert,
    ReportOnly,
}

/// RULINGS B9: macOS `boot_critical_enabled()` = Enabled because design §8-6 item 1 was measured
/// PASS on every daemon launch path (impl/measure/Ma-helper-auth-prompt.md). Items 2–4 and the
/// Developer-ID-signed helper are measured again at the release gate; if that fails this constant is
/// changed to `false` by a code change (→ `Pending`) and the fallback order of design §4-7 applies.
pub const MAC_HELPER_RELEASE_GATE_PASSED: bool = true;

const PENDING_MAC_RELEASE_GATE: &str =
    "맥 사용자 확인 도우미가 릴리스 관문 검증을 통과하지 못해 기동 필수 파일 수리는 보류됩니다";
const PENDING_WINDOWS: &str =
    "윈도우에서는 기동 필수 파일 수리가 보류됩니다(Windows Hello·계정 암호 확인과 앱 식별이 아직 실측되지 않았습니다)";
const PENDING_OTHER: &str = "이 운영체제에서는 기동 필수 파일 수리를 지원하지 않습니다";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HostOs {
    MacOs,
    Windows,
    Other,
}

const HOST: HostOs = if cfg!(target_os = "macos") {
    HostOs::MacOs
} else if cfg!(windows) {
    HostOs::Windows
} else {
    HostOs::Other
};

/// The B9 table, one row per OS (pure; the four public functions read the host's row).
fn table(os: HostOs) -> (ApplyForm, bool, BootCriticalGate, M10Mode) {
    match os {
        HostOs::MacOs => (
            ApplyForm::Exchange,
            true,
            if MAC_HELPER_RELEASE_GATE_PASSED {
                BootCriticalGate::Enabled
            } else {
                BootCriticalGate::Pending(PENDING_MAC_RELEASE_GATE)
            },
            M10Mode::Revert,
        ),
        HostOs::Windows => (
            ApplyForm::DepositFirst,
            false,
            BootCriticalGate::Pending(PENDING_WINDOWS),
            M10Mode::ReportOnly,
        ),
        HostOs::Other => (
            ApplyForm::Unsupported,
            false,
            BootCriticalGate::Pending(PENDING_OTHER),
            M10Mode::ReportOnly,
        ),
    }
}

pub fn apply_form() -> ApplyForm {
    table(HOST).0
}

pub fn auto_revert_enabled() -> bool {
    table(HOST).1
}

pub fn boot_critical_enabled() -> BootCriticalGate {
    table(HOST).2
}

pub fn m10_mode() -> M10Mode {
    table(HOST).3
}

#[cfg(test)]
mod tests {
    use super::*;

    /// B9 values per OS; the public functions return the host's row; no environment variable
    /// changes any value (behaviourally, and no env read exists in the production source).
    #[test]
    fn b9_values() {
        assert_eq!(
            table(HostOs::MacOs),
            (
                ApplyForm::Exchange,
                true,
                BootCriticalGate::Enabled,
                M10Mode::Revert
            )
        );
        let (form, revert, gate, m10) = table(HostOs::Windows);
        assert_eq!(
            (form, revert, m10),
            (ApplyForm::DepositFirst, false, M10Mode::ReportOnly)
        );
        assert!(matches!(gate, BootCriticalGate::Pending(r) if !r.is_empty()));
        let (form, revert, gate, m10) = table(HostOs::Other);
        assert_eq!(
            (form, revert, m10),
            (ApplyForm::Unsupported, false, M10Mode::ReportOnly)
        );
        assert!(matches!(gate, BootCriticalGate::Pending(r) if !r.is_empty()));

        let host = if cfg!(target_os = "macos") {
            HostOs::MacOs
        } else if cfg!(windows) {
            HostOs::Windows
        } else {
            HostOs::Other
        };
        let now = || {
            (
                apply_form(),
                auto_revert_enabled(),
                boot_critical_enabled(),
                m10_mode(),
            )
        };
        assert_eq!(now(), table(host));
        if cfg!(target_os = "macos") {
            assert_eq!(
                now(),
                (
                    ApplyForm::Exchange,
                    true,
                    BootCriticalGate::Enabled,
                    M10Mode::Revert
                )
            );
        }
        if cfg!(windows) {
            assert_eq!(
                (apply_form(), auto_revert_enabled(), m10_mode()),
                (ApplyForm::DepositFirst, false, M10Mode::ReportOnly)
            );
            assert!(matches!(
                boot_critical_enabled(),
                BootCriticalGate::Pending(_)
            ));
        }

        // Behaviour: plausible knob names change nothing.
        let before = now();
        {
            let _lock = crate::pack::PACK_ENV_LOCK
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let _g: Vec<crate::pack::EnvGuard> = [
                "CYS_REPAIR_PLATFORM",
                "CYS_REPAIR_APPLY_FORM",
                "CYS_REPAIR_AUTO_REVERT",
                "CYS_REPAIR_BOOT_CRITICAL",
                "CYS_REPAIR_M10_MODE",
                "CYS_REPAIR",
            ]
            .iter()
            .map(|k| crate::pack::EnvGuard::set(k, "windows"))
            .collect();
            assert_eq!(
                now(),
                before,
                "an environment variable changed the platform switch"
            );
        }
        // Structure: the production part of this file reads no environment at all.
        let src = include_str!("platform.rs");
        let prod = &src[..src.find("#[cfg(test)]").expect("test module boundary")];
        for needle in ["env::", "env_compat", "var_os", "option_env!", "env!("] {
            assert!(
                !prod.contains(needle),
                "platform.rs production code reads the environment ({needle})"
            );
        }
    }
}
