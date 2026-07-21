use crate::browser_runtime::{EngineKey, ParsedRuntimeState};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProtocolRequirement {
    pub major: u16,
    pub min_minor: u16,
    pub max_minor: u16,
    pub required_capabilities: BTreeSet<String>,
}

impl ProtocolRequirement {
    pub fn new<I, S>(major: u16, min_minor: u16, max_minor: u16, capabilities: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            major,
            min_minor,
            max_minor,
            required_capabilities: capabilities.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompatibilityRequirement {
    pub runtime_id: String,
    pub protocol: ProtocolRequirement,
    pub engine_key: EngineKey,
}

impl CompatibilityRequirement {
    pub fn new(
        runtime_id: impl Into<String>,
        protocol: ProtocolRequirement,
        engine_key: EngineKey,
    ) -> Self {
        Self { runtime_id: runtime_id.into(), protocol, engine_key }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CompatibilityIssue {
    LegacyState,
    RuntimeId,
    ProtocolMajor,
    ProtocolMinorRange,
    RequiredCapability(String),
    PeerRequiredCapability(String),
    EngineKey,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Compatibility {
    Compatible,
    Incompatible(Vec<CompatibilityIssue>),
}

pub fn evaluate_compatibility(
    parsed: &ParsedRuntimeState,
    required: &CompatibilityRequirement,
) -> Compatibility {
    let ParsedRuntimeState::V2(state) = parsed else {
        return Compatibility::Incompatible(vec![CompatibilityIssue::LegacyState]);
    };
    let mut issues = Vec::new();
    if state.runtime_id != required.runtime_id {
        issues.push(CompatibilityIssue::RuntimeId);
    }
    if state.protocol.major != required.protocol.major {
        issues.push(CompatibilityIssue::ProtocolMajor);
    } else if state.protocol.max_minor < required.protocol.min_minor
        || required.protocol.max_minor < state.protocol.min_minor
    {
        issues.push(CompatibilityIssue::ProtocolMinorRange);
    }
    for capability in &required.protocol.required_capabilities {
        if !state.protocol.capabilities.contains(capability) {
            issues.push(CompatibilityIssue::RequiredCapability(capability.clone()));
        }
    }
    for capability in &state.protocol.required_capabilities {
        if !required.protocol.required_capabilities.contains(capability) {
            issues.push(CompatibilityIssue::PeerRequiredCapability(capability.clone()));
        }
    }
    if state.engine_key != required.engine_key {
        issues.push(CompatibilityIssue::EngineKey);
    }
    if issues.is_empty() { Compatibility::Compatible } else { Compatibility::Incompatible(issues) }
}
