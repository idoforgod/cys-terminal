//! Common ingress/writer intent labels, shared by producers and durable records.
//! These enums are DATA labels. Deserializing one never grants authority.
use serde::{Serialize,Deserialize};
#[derive(Clone,Copy,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="snake_case")]
pub(crate) enum InputIntent {
    HumanTerminal,AgentText,AgentSubmit,AgentControl,LifecycleLaunch,
    UserPathInsertion,TerminalProtocol,TerminalDisplayControl,
}
#[derive(Clone,Copy,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="snake_case")]
pub(crate) enum TargetKind { Agent,Shell }
