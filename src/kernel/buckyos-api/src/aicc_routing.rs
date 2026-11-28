//! The usecase table, its layers and routing groups
//! (doc/aicc/aicc_router.md §11–§12).
//!
//! A usecase (`llm.code`, `image.txt2img`, ...) routes to one exact model.
//! Each cell — the model and what happens when its Provider is unavailable —
//! is decided by the highest layer that sets it:
//! default < administrator < user < App group < session group.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::LockedValue;

pub mod routing_methods {
    pub const ROUTING_USECASES: &str = "routing.usecases";
    pub const ROUTING_USECASE_SET: &str = "routing.usecase.set";
    pub const ROUTING_GROUPS_LIST: &str = "routing.groups.list";
    pub const ROUTING_GROUPS_CREATE: &str = "routing.groups.create";
    pub const ROUTING_GROUPS_UPDATE: &str = "routing.groups.update";
    pub const ROUTING_GROUPS_DELETE: &str = "routing.groups.delete";
    pub const ROUTING_BINDINGS_LIST: &str = "routing.bindings.list";
    pub const ROUTING_BINDINGS_SET: &str = "routing.bindings.set";
    pub const ROUTING_BINDINGS_REMOVE: &str = "routing.bindings.remove";

    pub fn is_usecase_method(method: &str) -> bool {
        matches!(
            method,
            ROUTING_USECASES
                | ROUTING_USECASE_SET
                | ROUTING_GROUPS_LIST
                | ROUTING_GROUPS_CREATE
                | ROUTING_GROUPS_UPDATE
                | ROUTING_GROUPS_DELETE
                | ROUTING_BINDINGS_LIST
                | ROUTING_BINDINGS_SET
                | ROUTING_BINDINGS_REMOVE
        )
    }
}

/// The fallback LLM: off by default; an administrator turns it on so that an
/// LLM usecase without any model is still served.
pub const AICC_FALLBACK_LLM: &str = "llm.fallback";

/// What a usecase does when the Provider of its model is unavailable.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnUnavailable {
    /// Return the real error.
    #[default]
    None,
    /// The same model on another Provider.
    SameModel,
    /// Route again without that Provider; take the result only when it is
    /// known not to cost more.
    NotPricier,
    /// Route again without that Provider.
    NextModel,
}

/// One usecase in a layer or a routing group. Absent fields follow the
/// layer below.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiccUsecaseRoute {
    /// An exact model `model@provider`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<LockedValue<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_unavailable: Option<LockedValue<OnUnavailable>>,
    /// Only for `llm.fallback`, only in the administrator layer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

impl AiccUsecaseRoute {
    pub fn is_empty(&self) -> bool {
        self.model.is_none() && self.on_unavailable.is_none() && self.enabled.is_none()
    }
}

/// The content of a layer or a routing group: usecase → route.
pub type AiccUsecaseRoutes = BTreeMap<String, AiccUsecaseRoute>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteFallbackKind {
    /// The usecase has no model; the fallback LLM served it.
    Coverage,
    /// The usecase's model failed; its "when unavailable" policy served it.
    Unavailable,
}

/// Why an answer did not come from the usecase's own model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteFallback {
    pub kind: RouteFallbackKind,
    /// The usecase (coverage) or the exact model that failed (unavailable).
    pub from: String,
    /// `llm.fallback` (coverage) or the exact model used instead.
    pub to: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<OnUnavailable>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsecaseModelState {
    Available,
    /// Not in any enabled Provider's inventory.
    Missing,
    /// Hidden from the viewer by policy.
    NotPermitted,
    /// In the inventory but does not meet the usecase's requirements.
    Unsuitable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsecaseCoverage {
    Covered,
    /// Served by the fallback LLM.
    FallbackLlm,
    Uncovered,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsecaseLayerValue {
    /// `zone`, `user`, `group:<id>`.
    pub layer: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub model_locked: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_state: Option<UsecaseModelState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_unavailable: Option<OnUnavailable>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub on_unavailable_locked: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsecaseRow {
    pub usecase: String,
    pub api_type: String,
    /// The exact model this view routes the usecase to now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// `default` or the layer that chose `model`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_source: Option<String>,
    pub on_unavailable: OnUnavailable,
    /// `default` or the layer that set `on_unavailable`.
    pub on_unavailable_source: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub model_locked: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub on_unavailable_locked: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<UsecaseLayerValue>,
    /// What the policy would use if the Provider of `model` were unavailable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_to: Option<String>,
    /// Why `fallback_to` is empty under a policy other than `none`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_note: Option<String>,
    pub coverage: UsecaseCoverage,
    /// `no_model_for_api_type` or `requirements_unmet` when uncovered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coverage_reason: Option<String>,
    /// The routing error, for an administrator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// The row of `llm.fallback`: whether the fallback LLM is on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingUsecasesRequest {
    /// `self` (default), `zone` (administrators), `group:<id>`, `app:<app_id>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<String>,
    /// Administrators: the table as if this Provider were deleted or
    /// disabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub without_provider: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutingUsecasesResponse {
    pub view: String,
    /// The revision a `routing.usecase.set` of this view's layer passes:
    /// the settings revision (`zone`), the user layer version (`self`) or
    /// the group version (`group:<id>`).
    pub revision: u64,
    pub settings_revision: u64,
    pub fallback_llm_enabled: bool,
    /// The viewer may change this view's layer.
    pub can_edit: bool,
    pub rows: Vec<UsecaseRow>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsecaseField {
    Model,
    OnUnavailable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingUsecaseSetRequest {
    /// `zone`, `user` or `group:<id>`.
    pub target: String,
    pub usecase: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<LockedValue<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_unavailable: Option<LockedValue<OnUnavailable>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clear: Vec<UsecaseField>,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutingUsecaseSetResponse {
    pub ok: bool,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiccRoutingGroup {
    pub group_id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub usecases: AiccUsecaseRoutes,
    pub version: u64,
    pub updated_at_ms: i64,
    /// Apps (or Agents) the group is bound to.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub app_ids: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingGroupsListRequest {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoutingGroupsListResponse {
    pub groups: Vec<AiccRoutingGroup>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingGroupCreateRequest {
    pub name: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub usecases: AiccUsecaseRoutes,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingGroupUpdateRequest {
    pub group_id: String,
    pub version: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usecases: Option<AiccUsecaseRoutes>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingGroupDeleteRequest {
    pub group_id: String,
    pub version: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoutingGroupResponse {
    pub group: AiccRoutingGroup,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutingOkResponse {
    pub ok: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiccRoutingBinding {
    pub app_id: String,
    pub group_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingBindingsListRequest {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutingBindingsListResponse {
    pub bindings: Vec<AiccRoutingBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingBindingSetRequest {
    pub app_id: String,
    pub group_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingBindingRemoveRequest {
    pub app_id: String,
}

/// One usecase before and after a Provider is added
/// (`provider.validate`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsecaseCoverageChange {
    pub usecase: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
    pub before_coverage: UsecaseCoverage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
    pub after_coverage: UsecaseCoverage,
}
