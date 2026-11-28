//! The usecase table, the user layer, routing groups and App bindings
//! (doc/aicc/aicc_router.md §11–§12).

use super::*;
use crate::routing::usecase::{is_usecase, UsecaseCells, USECASES};
use buckyos_api::{
    routing_methods as methods, AiccRouteOverlay, AiccUsecaseRoute, AiccUsecaseRoutes,
    RoutingBindingRemoveRequest, RoutingBindingSetRequest, RoutingBindingsListResponse,
    RoutingGroupCreateRequest, RoutingGroupDeleteRequest, RoutingGroupResponse,
    RoutingGroupUpdateRequest, RoutingGroupsListResponse, RoutingOkResponse,
    RoutingUsecaseSetRequest, RoutingUsecaseSetResponse, RoutingUsecasesRequest,
    RoutingUsecasesResponse, UsecaseCoverage, UsecaseCoverageChange, UsecaseField,
    UsecaseLayerValue, UsecaseModelState, UsecaseRow,
};

const MAX_GROUP_NAME_CHARS: usize = 64;

fn invalid(message: impl Into<String>) -> RPCErrors {
    inference_error(AiccErrorCode::InvalidRequest, message)
}

fn storage_error(error: crate::error::StorageError) -> RPCErrors {
    log::warn!("routing storage failed: {error}");
    inference_error(
        AiccErrorCode::InternalError,
        "routing storage is unavailable",
    )
}

fn write_error(error: crate::storage::PolicyWriteError) -> RPCErrors {
    match error {
        crate::storage::PolicyWriteError::NotFound => invalid("the routing group was not found"),
        crate::storage::PolicyWriteError::VersionConflict { expected, actual } => {
            AiccError::settings_revision_conflict(expected, actual).to_krpc_error()
        }
        crate::storage::PolicyWriteError::IdempotencyConflict => invalid("conflicting write"),
        crate::storage::PolicyWriteError::Storage(error) => storage_error(error),
    }
}

fn parse<T: serde::de::DeserializeOwned>(method: &str, params: Value) -> Result<T, RPCErrors> {
    serde_json::from_value(params)
        .map_err(|error| RPCErrors::ParseRequestError(format!("{method}: {error}")))
}

fn to_value<T: serde::Serialize>(value: T) -> Result<Value, RPCErrors> {
    serde_json::to_value(value).map_err(|error| RPCErrors::ReasonError(error.to_string()))
}

fn subject(caller: &AuthorizedCaller) -> String {
    format!("user:{}", caller.user_id)
}

/// Where a write of the table goes.
pub(crate) enum UsecaseTarget {
    Zone,
    User,
    Group(String),
}

impl UsecaseTarget {
    pub(crate) fn parse(target: &str) -> Result<Self, RPCErrors> {
        match target {
            "zone" => Ok(Self::Zone),
            "user" => Ok(Self::User),
            other => other
                .strip_prefix("group:")
                .filter(|id| !id.trim().is_empty())
                .map(|id| Self::Group(id.to_owned()))
                .ok_or_else(|| invalid("target must be zone, user or group:<id>")),
        }
    }
}

/// Apply one `routing.usecase.set` to a layer's content.
pub(crate) fn apply_usecase_set(
    routes: &mut AiccUsecaseRoutes,
    request: &RoutingUsecaseSetRequest,
) {
    let route = routes.entry(request.usecase.clone()).or_default();
    for field in &request.clear {
        match field {
            UsecaseField::Model => route.model = None,
            UsecaseField::OnUnavailable => route.on_unavailable = None,
        }
    }
    if let Some(model) = &request.model {
        route.model = Some(model.clone());
    }
    if let Some(policy) = &request.on_unavailable {
        route.on_unavailable = Some(policy.clone());
    }
    if let Some(enabled) = request.enabled {
        route.enabled = Some(enabled);
    }
    if route.is_empty() {
        routes.remove(&request.usecase);
    }
}

impl RuntimeInferencePort {
    /// Check one layer entry (doc/aicc/aicc_router.md §11.1): the usecase
    /// exists; only the zone layer locks or switches the fallback LLM; the
    /// model exists, has the usecase's API type, meets its requirements and
    /// is visible to the caller.
    pub(crate) async fn validate_usecase_route(
        &self,
        caller: &AuthorizedCaller,
        zone: bool,
        usecase: &str,
        route: &AiccUsecaseRoute,
    ) -> Result<(), RPCErrors> {
        if !is_usecase(usecase) {
            return Err(invalid(format!("unknown usecase `{usecase}`")));
        }
        let locked = route.model.as_ref().is_some_and(|model| model.locked)
            || route
                .on_unavailable
                .as_ref()
                .is_some_and(|policy| policy.locked);
        if locked && !zone {
            return Err(invalid("only the administrator layer may lock a usecase"));
        }
        if route.enabled.is_some() && (!zone || usecase != AICC_FALLBACK_LLM) {
            return Err(invalid(
                "enabled only switches the fallback LLM in the administrator layer",
            ));
        }
        let snapshot = self.runtime.capture().await;
        if !zone {
            let admin = snapshot
                .settings
                .routing
                .as_ref()
                .and_then(|routing| routing.usecases.get(usecase));
            let model_locked = admin
                .and_then(|admin| admin.model.as_ref())
                .is_some_and(|model| model.locked);
            let policy_locked = admin
                .and_then(|admin| admin.on_unavailable.as_ref())
                .is_some_and(|policy| policy.locked);
            if (model_locked && route.model.is_some())
                || (policy_locked && route.on_unavailable.is_some())
            {
                return Err(inference_error(
                    AiccErrorCode::PolicyDenied,
                    format!("the administrator locked this cell of {usecase}"),
                ));
            }
        }
        let Some(model) = route.model.as_ref().map(|model| model.value.as_str()) else {
            return Ok(());
        };
        let registry = snapshot.models.as_ref();
        let registered = registry
            .exact_model(model)
            .ok_or_else(|| invalid(format!("exact model `{model}` is not in any inventory")))?;
        let api_type = registry
            .logical_model_views()
            .into_iter()
            .find(|view| view.path == usecase)
            .and_then(|view| view.api_type)
            .and_then(|name| {
                serde_json::from_value::<buckyos_api::ApiType>(Value::String(name)).ok()
            })
            .ok_or_else(|| invalid(format!("usecase `{usecase}` has no API type")))?;
        if !registered.api_types.contains(&api_type) {
            return Err(invalid(format!("`{model}` does not serve {usecase}")));
        }
        if usecase != AICC_FALLBACK_LLM {
            let missing = crate::model::missing_requirements(
                &registry.logical_requirement(usecase),
                registered,
            );
            if !missing.is_empty() {
                return Err(invalid(format!(
                    "`{model}` does not meet {usecase}: {}",
                    missing.join(", ")
                )));
            }
        }
        let access = self.access(caller, snapshot.as_ref()).await?;
        if !access.allows_model(model) {
            return Err(inference_error(
                AiccErrorCode::ResourceNotPermitted,
                "the model is not visible to you",
            ));
        }
        Ok(())
    }

    /// Change the caller's own layer or one of their groups; the zone
    /// layer is the settings document and changed by the service.
    pub(crate) async fn set_usecase(
        &self,
        caller: &AuthorizedCaller,
        target: &UsecaseTarget,
        request: &RoutingUsecaseSetRequest,
    ) -> Result<RoutingUsecaseSetResponse, RPCErrors> {
        let now = now_ms() as i64;
        match target {
            UsecaseTarget::Zone => Err(invalid("the zone layer is in the settings")),
            UsecaseTarget::User => {
                let subject = subject(caller);
                let (mut routes, version) = self
                    .storage
                    .routing_layer(&subject)
                    .await
                    .map_err(storage_error)?;
                if version != request.revision {
                    return Err(
                        AiccError::settings_revision_conflict(request.revision, version)
                            .to_krpc_error(),
                    );
                }
                apply_usecase_set(&mut routes, request);
                let revision = self
                    .storage
                    .put_routing_layer(&subject, &routes, version, now)
                    .await
                    .map_err(write_error)?;
                Ok(RoutingUsecaseSetResponse { ok: true, revision })
            }
            UsecaseTarget::Group(group_id) => {
                let (owner, group) = self
                    .storage
                    .routing_group(group_id)
                    .await
                    .map_err(storage_error)?
                    .filter(|(owner, _)| *owner == subject(caller))
                    .ok_or_else(|| invalid("the routing group was not found"))?;
                let mut routes = group.usecases;
                apply_usecase_set(&mut routes, request);
                let group = self
                    .storage
                    .update_routing_group(
                        &owner,
                        group_id,
                        request.revision,
                        None,
                        Some(&routes),
                        now,
                    )
                    .await
                    .map_err(write_error)?;
                Ok(RoutingUsecaseSetResponse {
                    ok: true,
                    revision: group.version,
                })
            }
        }
    }

    /// `routing.groups.*` and `routing.bindings.*`: always the caller's own.
    pub(crate) async fn routing_store_rpc(
        &self,
        caller: &AuthorizedCaller,
        method: &str,
        params: Value,
    ) -> Result<Value, RPCErrors> {
        let owner = subject(caller);
        let now = now_ms() as i64;
        match method {
            methods::ROUTING_GROUPS_LIST => to_value(RoutingGroupsListResponse {
                groups: self
                    .storage
                    .routing_groups(&owner)
                    .await
                    .map_err(storage_error)?,
            }),
            methods::ROUTING_GROUPS_CREATE => {
                let request: RoutingGroupCreateRequest = parse(method, params)?;
                let name = group_name(&request.name)?;
                self.validate_routes(caller, &request.usecases).await?;
                to_value(RoutingGroupResponse {
                    group: self
                        .storage
                        .create_routing_group(&owner, &name, &request.usecases, now)
                        .await
                        .map_err(storage_error)?,
                })
            }
            methods::ROUTING_GROUPS_UPDATE => {
                let request: RoutingGroupUpdateRequest = parse(method, params)?;
                let name = request.name.as_deref().map(group_name).transpose()?;
                if let Some(routes) = &request.usecases {
                    self.validate_routes(caller, routes).await?;
                }
                to_value(RoutingGroupResponse {
                    group: self
                        .storage
                        .update_routing_group(
                            &owner,
                            &request.group_id,
                            request.version,
                            name.as_deref(),
                            request.usecases.as_ref(),
                            now,
                        )
                        .await
                        .map_err(write_error)?,
                })
            }
            methods::ROUTING_GROUPS_DELETE => {
                let request: RoutingGroupDeleteRequest = parse(method, params)?;
                self.storage
                    .delete_routing_group(&owner, &request.group_id, request.version)
                    .await
                    .map_err(write_error)?;
                to_value(RoutingOkResponse { ok: true })
            }
            methods::ROUTING_BINDINGS_LIST => to_value(RoutingBindingsListResponse {
                bindings: self
                    .storage
                    .routing_bindings(&owner)
                    .await
                    .map_err(storage_error)?,
            }),
            methods::ROUTING_BINDINGS_SET => {
                let request: RoutingBindingSetRequest = parse(method, params)?;
                if request.app_id.trim().is_empty() {
                    return Err(invalid("app_id must not be empty"));
                }
                self.storage
                    .routing_group(&request.group_id)
                    .await
                    .map_err(storage_error)?
                    .filter(|(group_owner, _)| *group_owner == owner)
                    .ok_or_else(|| invalid("the routing group was not found"))?;
                self.storage
                    .set_routing_binding(&owner, &request.app_id, &request.group_id, now)
                    .await
                    .map_err(storage_error)?;
                to_value(RoutingOkResponse { ok: true })
            }
            methods::ROUTING_BINDINGS_REMOVE => {
                let request: RoutingBindingRemoveRequest = parse(method, params)?;
                let ok = self
                    .storage
                    .remove_routing_binding(&owner, &request.app_id)
                    .await
                    .map_err(storage_error)?;
                to_value(RoutingOkResponse { ok })
            }
            _ => Err(RPCErrors::UnknownMethod(method.to_string())),
        }
    }

    async fn validate_routes(
        &self,
        caller: &AuthorizedCaller,
        routes: &AiccUsecaseRoutes,
    ) -> Result<(), RPCErrors> {
        for (usecase, route) in routes {
            self.validate_usecase_route(caller, false, usecase, route)
                .await?;
        }
        Ok(())
    }

    /// The table one view sees (`routing.usecases`).
    pub(crate) async fn usecase_table(
        &self,
        caller: &AuthorizedCaller,
        request: RoutingUsecasesRequest,
    ) -> Result<RoutingUsecasesResponse, RPCErrors> {
        let snapshot = self.runtime.capture().await;
        let view = request.view.clone().unwrap_or_else(|| "self".into());
        let zone = snapshot
            .settings
            .routing
            .as_ref()
            .map(|routing| routing.usecases.clone())
            .unwrap_or_default();
        let owner = subject(caller);
        let mut layers = UsecaseLayers::default();
        layers.layers.push((LayerId::Zone, zone));
        let (revision, can_edit) = match view.as_str() {
            "zone" => {
                if !caller.is_admin() {
                    return Err(RPCErrors::NoPermission(
                        "only an administrator may view the zone layer".into(),
                    ));
                }
                (snapshot.settings_revision, true)
            }
            "self" => {
                let (user, version) = self
                    .storage
                    .routing_layer(&owner)
                    .await
                    .map_err(storage_error)?;
                layers.layers.push((LayerId::User, user));
                (version, true)
            }
            other => {
                let (user, _) = self
                    .storage
                    .routing_layer(&owner)
                    .await
                    .map_err(storage_error)?;
                layers.layers.push((LayerId::User, user));
                let (group_id, can_edit) = if let Some(group_id) = other.strip_prefix("group:") {
                    (Some(group_id.to_owned()), true)
                } else if let Some(app_id) = other.strip_prefix("app:") {
                    (
                        self.storage
                            .routing_binding(&owner, app_id)
                            .await
                            .map_err(storage_error)?,
                        false,
                    )
                } else {
                    return Err(invalid(
                        "view must be self, zone, group:<id> or app:<app_id>",
                    ));
                };
                let mut version = 0;
                if let Some(group_id) = group_id {
                    let (group_owner, group) = self
                        .storage
                        .routing_group(&group_id)
                        .await
                        .map_err(storage_error)?
                        .ok_or_else(|| invalid("the routing group was not found"))?;
                    if group_owner != owner {
                        return Err(invalid("the routing group was not found"));
                    }
                    version = group.version;
                    layers.app_group = other.starts_with("app:").then(|| group_id.clone());
                    layers
                        .layers
                        .push((LayerId::Group(group_id), group.usecases));
                }
                (version, can_edit)
            }
        };
        let mut excluded_providers = BTreeSet::new();
        if let Some(provider) = request.without_provider {
            if !caller.is_admin() {
                return Err(RPCErrors::NoPermission(
                    "only an administrator may preview removing a Provider".into(),
                ));
            }
            excluded_providers.insert(provider);
        }
        let access = self.access(caller, snapshot.as_ref()).await?;
        let paths = USECASES
            .iter()
            .map(|usecase| usecase.to_string())
            .chain([AICC_FALLBACK_LLM.to_string()])
            .collect::<Vec<_>>();
        let preview = Preview {
            snapshot: snapshot.as_ref(),
            models: snapshot.models.as_ref(),
            layers: &layers,
            assume_ready: None,
            excluded_providers,
            requirements: Default::default(),
            with_backup: true,
        };
        let previewed = self.preview(caller, &preview, &paths).await?;
        let rows = previewed
            .into_iter()
            .map(|path| usecase_row(&snapshot, &layers, &access, path))
            .collect();
        Ok(RoutingUsecasesResponse {
            view,
            revision,
            settings_revision: snapshot.settings_revision,
            fallback_llm_enabled: layers.fallback_llm_enabled(),
            can_edit,
            rows,
        })
    }

    /// Every usecase of the Zone before and after `inventory` is added
    /// (`provider.validate`, doc/aicc/aicc_router.md §11.4).
    pub(crate) async fn coverage_preview(
        &self,
        caller: &AuthorizedCaller,
        added: &crate::model::ProviderInventory,
    ) -> Result<Vec<UsecaseCoverageChange>, RPCErrors> {
        let snapshot = self.runtime.capture().await;
        let zone = snapshot.settings.routing.clone();
        let mut layers = UsecaseLayers::default();
        layers.layers.push((
            LayerId::Zone,
            zone.as_ref()
                .map(|routing| routing.usecases.clone())
                .unwrap_or_default(),
        ));
        let mut inventories = snapshot
            .providers
            .list()
            .into_iter()
            .filter(|provider| {
                provider.config.provider_instance_name != added.provider_instance_name
            })
            .map(|provider| provider.inventory.as_model_inventory())
            .collect::<Vec<_>>();
        inventories.push(added.clone());
        let after = super::super::model_defaults::assemble_registry(
            snapshot.catalog.as_ref(),
            &inventories,
            zone.as_ref(),
        )
        .map_err(|error| invalid(error.to_string()))?;
        let paths = USECASES
            .iter()
            .map(|usecase| usecase.to_string())
            .collect::<Vec<_>>();
        let before = self
            .preview(
                caller,
                &Preview {
                    snapshot: snapshot.as_ref(),
                    models: snapshot.models.as_ref(),
                    layers: &layers,
                    assume_ready: None,
                    excluded_providers: BTreeSet::new(),
                    requirements: Default::default(),
                    with_backup: false,
                },
                &paths,
            )
            .await?;
        let after = self
            .preview(
                caller,
                &Preview {
                    snapshot: snapshot.as_ref(),
                    models: &after,
                    layers: &layers,
                    assume_ready: Some(&added.provider_instance_name),
                    excluded_providers: BTreeSet::new(),
                    requirements: Default::default(),
                    with_backup: false,
                },
                &paths,
            )
            .await?;
        Ok(before
            .into_iter()
            .zip(after)
            .map(|(before, after)| {
                let (before_model, before_coverage) = coverage_of(&before);
                let (after_model, after_coverage) = coverage_of(&after);
                UsecaseCoverageChange {
                    usecase: before.path,
                    before: before_model,
                    before_coverage,
                    after: after_model,
                    after_coverage,
                }
            })
            .collect())
    }
}

fn coverage_of(path: &PreviewedPath) -> (Option<String>, UsecaseCoverage) {
    match &path.route {
        Ok(route) => (
            Some(route.decision.selected.exact_model.clone()),
            if route.coverage {
                UsecaseCoverage::FallbackLlm
            } else {
                UsecaseCoverage::Covered
            },
        ),
        Err(_) => (None, UsecaseCoverage::Uncovered),
    }
}

fn group_name(name: &str) -> Result<String, RPCErrors> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_GROUP_NAME_CHARS {
        return Err(invalid(format!(
            "a group name has 1..{MAX_GROUP_NAME_CHARS} characters"
        )));
    }
    Ok(name.to_owned())
}

fn usecase_row(
    snapshot: &crate::runtime::RuntimeSnapshot,
    layers: &UsecaseLayers,
    access: &crate::policy::Access,
    path: PreviewedPath,
) -> UsecaseRow {
    let cells: UsecaseCells = layers.cells(&path.path);
    let skipped = path
        .route
        .as_ref()
        .ok()
        .and_then(|route| route.decision.trace.usecase.as_ref())
        .map(|usecase| usecase.skipped_models.clone())
        .unwrap_or_default();
    let model_state = |model: &str| {
        if snapshot.models.exact_model(model).is_none() {
            UsecaseModelState::Missing
        } else if !access.allows_model(model) {
            UsecaseModelState::NotPermitted
        } else if skipped.iter().any(|step| step.from == model) {
            UsecaseModelState::Unsuitable
        } else {
            UsecaseModelState::Available
        }
    };
    let layer_values = layers
        .layers
        .iter()
        .filter_map(|(layer, routes)| {
            let route = routes.get(&path.path)?;
            Some(UsecaseLayerValue {
                layer: layer.name(),
                model: route.model.as_ref().map(|model| model.value.clone()),
                model_locked: route.model.as_ref().is_some_and(|model| model.locked),
                model_state: route.model.as_ref().map(|model| model_state(&model.value)),
                on_unavailable: route.on_unavailable.as_ref().map(|policy| policy.value),
                on_unavailable_locked: route
                    .on_unavailable
                    .as_ref()
                    .is_some_and(|policy| policy.locked),
                enabled: route.enabled,
            })
        })
        .collect();
    let (model, model_source, coverage, coverage_reason, detail) = match &path.route {
        Ok(route) => (
            Some(route.decision.selected.exact_model.clone()),
            Some(if route.coverage {
                "fallback_llm".to_string()
            } else {
                route
                    .decision
                    .trace
                    .usecase
                    .as_ref()
                    .map(|usecase| usecase.usecase_source.clone())
                    .unwrap_or_else(|| "default".into())
            }),
            if route.coverage {
                UsecaseCoverage::FallbackLlm
            } else {
                UsecaseCoverage::Covered
            },
            None,
            None,
        ),
        Err(error) => (
            None,
            None,
            UsecaseCoverage::Uncovered,
            Some(if path.no_model {
                "no_model_for_api_type".to_string()
            } else {
                "requirements_unmet".to_string()
            }),
            (!access.is_restricted()).then(|| error.clone()),
        ),
    };
    let (fallback_to, fallback_note) = match path.backup {
        Some(Ok(Some(decision))) => (Some(decision.selected.exact_model), None),
        Some(Ok(None)) => (
            None,
            Some(
                match cells.on_unavailable {
                    OnUnavailable::NotPricier => "no_known_cheaper_model",
                    _ => "no_other_model",
                }
                .to_string(),
            ),
        ),
        Some(Err(error)) => (None, Some(error)),
        None => (None, None),
    };
    UsecaseRow {
        api_type: path.api_type.unwrap_or_default(),
        enabled: (path.path == AICC_FALLBACK_LLM).then(|| layers.fallback_llm_enabled()),
        usecase: path.path,
        model,
        model_source,
        on_unavailable: cells.on_unavailable,
        on_unavailable_source: cells.on_unavailable_source,
        model_locked: cells.model_locked,
        on_unavailable_locked: cells.on_unavailable_locked,
        layers: layer_values,
        fallback_to,
        fallback_note,
        coverage,
        coverage_reason,
        detail,
    }
}

/// Apply a zone-layer `routing.usecase.set` to the settings' routing.
pub(crate) fn apply_zone_usecase_set(
    routing: &mut AiccRouteOverlay,
    request: &RoutingUsecaseSetRequest,
) {
    apply_usecase_set(&mut routing.usecases, request);
}

#[cfg(test)]
mod tests {
    use super::*;
    use buckyos_api::LockedValue;

    fn set(usecase: &str) -> RoutingUsecaseSetRequest {
        RoutingUsecaseSetRequest {
            target: "user".into(),
            usecase: usecase.into(),
            model: None,
            on_unavailable: None,
            enabled: None,
            clear: Vec::new(),
            revision: 0,
        }
    }

    #[test]
    fn a_set_changes_only_its_cells_and_empty_entries_disappear() {
        let mut routes = AiccUsecaseRoutes::new();
        apply_usecase_set(
            &mut routes,
            &RoutingUsecaseSetRequest {
                model: Some(LockedValue::new("a@x".into())),
                ..set("llm.code")
            },
        );
        apply_usecase_set(
            &mut routes,
            &RoutingUsecaseSetRequest {
                on_unavailable: Some(LockedValue::new(OnUnavailable::NextModel)),
                ..set("llm.code")
            },
        );
        let route = &routes["llm.code"];
        assert_eq!(route.model.as_ref().unwrap().value, "a@x");
        assert_eq!(
            route.on_unavailable.as_ref().unwrap().value,
            OnUnavailable::NextModel
        );
        apply_usecase_set(
            &mut routes,
            &RoutingUsecaseSetRequest {
                clear: vec![UsecaseField::Model, UsecaseField::OnUnavailable],
                ..set("llm.code")
            },
        );
        assert!(routes.is_empty());
    }

    #[test]
    fn targets_are_zone_user_or_a_group() {
        assert!(matches!(
            UsecaseTarget::parse("zone"),
            Ok(UsecaseTarget::Zone)
        ));
        assert!(matches!(
            UsecaseTarget::parse("user"),
            Ok(UsecaseTarget::User)
        ));
        assert!(matches!(
            UsecaseTarget::parse("group:rg_1"),
            Ok(UsecaseTarget::Group(id)) if id == "rg_1"
        ));
        for bad in ["group:", "session", ""] {
            assert!(UsecaseTarget::parse(bad).is_err(), "{bad}");
        }
    }
}
