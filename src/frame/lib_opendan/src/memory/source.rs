//! Original source events, as the host knows them. The component asks the
//! host instead of trusting the writer: whether an event exists, who
//! produced it, whether it is still readable (provenance), and whether it is
//! a Runtime-attached Memory block (never a new fact source, E-15).

use serde::{Deserialize, Serialize};

use super::ActorKind;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceEventInfo {
    pub event_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_kind: Option<ActorKind>,
    /// The original can still be read (history not purged, access allowed).
    pub readable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub excerpt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<String>,
    /// A Memory block the Runtime attached to a model input.
    #[serde(default)]
    pub runtime_attachment: bool,
}

pub trait SourceEvents: Send + Sync {
    fn lookup(&self, event_ref: &str) -> Option<SourceEventInfo>;
}
