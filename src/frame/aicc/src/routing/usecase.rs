//! The usecase table (doc/aicc/aicc_router.md §11).
//!
//! A usecase routes to one exact model. Each of its two cells — the model and
//! what to do when that model's Provider is unavailable — comes from the
//! highest layer that sets it (zone < user < App group < session group);
//! otherwise the model is the default expansion of the logical directory.
//! Health, latency observations and request size never take part, so the
//! same configuration and inventory always give the same model.

use super::policy::QuotaSource;
use super::{FallbackTraceStep, RouteDecision, Router, RoutingError, RoutingRequest, UsecaseTrace};
use crate::model::ExactModelName;
use buckyos_api::{
    AiccUsecaseRoutes, OnUnavailable, RouteFallback, RouteFallbackKind, AICC_FALLBACK_LLM,
};
use std::collections::BTreeSet;

/// The usecases of the table, in display order.
pub(crate) const USECASES: [&str; 29] = [
    "decision",
    "llm.chat",
    "llm.plan",
    "llm.code",
    "llm.swift",
    "llm.summarize",
    "llm.translate",
    "llm.vision",
    "embedding.text",
    "embedding.multimodal",
    "rerank",
    "image.txt2img",
    "image.img2img",
    "image.inpaint",
    "image.upscale",
    "image.bg_remove",
    "vision.ocr",
    "vision.caption",
    "vision.detect",
    "vision.segment",
    "audio.tts",
    "audio.asr",
    "audio.music",
    "audio.enhance",
    "video.generate",
    "video.edit",
    "video.extend",
    "video.upscale",
    "agent_runtime.computer_use",
];

/// The LLM usecases the fallback LLM serves when they have no model.
pub(crate) const LLM_USECASES: [&str; 7] = [
    "llm.chat",
    "llm.plan",
    "llm.code",
    "llm.swift",
    "llm.summarize",
    "llm.translate",
    "llm.vision",
];

/// A row of the table: one of the usecases or the fallback LLM.
pub(crate) fn is_usecase(path: &str) -> bool {
    path == AICC_FALLBACK_LLM || USECASES.contains(&path)
}

/// A layer above the defaults.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum LayerId {
    Zone,
    User,
    Group(String),
}

impl LayerId {
    pub(crate) fn name(&self) -> String {
        match self {
            Self::Zone => "zone".into(),
            Self::User => "user".into(),
            Self::Group(id) => format!("group:{id}"),
        }
    }
}

/// The layers that apply to one caller, lowest first.
#[derive(Clone, Debug, Default)]
pub(crate) struct UsecaseLayers {
    pub layers: Vec<(LayerId, AiccUsecaseRoutes)>,
    pub app_group: Option<String>,
    pub session_group: Option<String>,
    pub route_group_ignored: bool,
}

/// A model one layer chose.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ModelChoice {
    pub layer: String,
    pub model: String,
}

/// The merged cells of one usecase.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct UsecaseCells {
    /// Models to try, highest layer first. A locked zone model is the only one.
    pub models: Vec<ModelChoice>,
    pub model_locked: bool,
    pub on_unavailable: OnUnavailable,
    /// `default` or the layer that set it.
    pub on_unavailable_source: String,
    pub on_unavailable_locked: bool,
}

impl UsecaseLayers {
    pub(crate) fn fallback_llm_enabled(&self) -> bool {
        self.layers
            .iter()
            .find(|(layer, _)| *layer == LayerId::Zone)
            .and_then(|(_, routes)| routes.get(AICC_FALLBACK_LLM))
            .and_then(|route| route.enabled)
            .unwrap_or(false)
    }

    pub(crate) fn cells(&self, usecase: &str) -> UsecaseCells {
        let mut cells = UsecaseCells {
            models: Vec::new(),
            model_locked: false,
            on_unavailable: OnUnavailable::None,
            on_unavailable_source: "default".into(),
            on_unavailable_locked: false,
        };
        for (layer, routes) in &self.layers {
            let Some(route) = routes.get(usecase) else {
                continue;
            };
            // Only the administrator layer may lock a cell.
            let may_lock = *layer == LayerId::Zone;
            if let Some(model) = &route.model {
                if !cells.model_locked {
                    cells.models.insert(
                        0,
                        ModelChoice {
                            layer: layer.name(),
                            model: model.value.clone(),
                        },
                    );
                    if may_lock && model.locked {
                        cells.models.truncate(1);
                        cells.model_locked = true;
                    }
                }
            }
            if let Some(policy) = &route.on_unavailable {
                if !cells.on_unavailable_locked {
                    cells.on_unavailable = policy.value;
                    cells.on_unavailable_source = layer.name();
                    cells.on_unavailable_locked = may_lock && policy.locked;
                }
            }
        }
        cells
    }
}

/// A route decided through the usecase table.
#[derive(Clone, Debug)]
pub(crate) struct UsecaseRoute {
    pub decision: RouteDecision,
    pub on_unavailable: OnUnavailable,
    /// The fallback LLM served a usecase without a model.
    pub coverage: bool,
}

/// Route `request.model`: through the table when it is a usecase, by plain
/// static expansion when it is a specification, family or exact model.
pub(crate) fn resolve<Q: QuotaSource>(
    router: &Router<'_, Q>,
    layers: &UsecaseLayers,
    request: &RoutingRequest,
    allow_coverage: bool,
) -> Result<UsecaseRoute, RoutingError> {
    let usecase = request.model.as_str();
    if !is_usecase(usecase) {
        return router.route(request).map(|decision| UsecaseRoute {
            decision,
            on_unavailable: OnUnavailable::None,
            coverage: false,
        });
    }
    let cells = layers.cells(usecase);
    let trace = |source: String| UsecaseTrace {
        usecase: usecase.to_owned(),
        usecase_source: source,
        on_unavailable: cells.on_unavailable,
        on_unavailable_source: cells.on_unavailable_source.clone(),
        app_group: layers.app_group.clone(),
        session_group: layers.session_group.clone(),
        route_group_ignored: layers.route_group_ignored,
        skipped_models: Vec::new(),
    };
    let mut skipped = Vec::new();
    for choice in &cells.models {
        match router.route_pinned(request, &choice.model) {
            Ok(mut decision) => {
                let mut usecase_trace = trace(choice.layer.clone());
                usecase_trace.skipped_models = skipped;
                decision.trace.usecase = Some(usecase_trace);
                return Ok(UsecaseRoute {
                    decision,
                    on_unavailable: cells.on_unavailable,
                    coverage: false,
                });
            }
            Err(error) => skipped.push(FallbackTraceStep {
                from: choice.model.clone(),
                to: choice.layer.clone(),
                reason: skip_reason(&error),
            }),
        }
    }
    match router.route(request) {
        Ok(mut decision) => {
            let mut usecase_trace = trace("default".into());
            usecase_trace.skipped_models = skipped;
            decision.trace.usecase = Some(usecase_trace);
            Ok(UsecaseRoute {
                decision,
                on_unavailable: cells.on_unavailable,
                coverage: false,
            })
        }
        Err(error)
            if allow_coverage
                && LLM_USECASES.contains(&usecase)
                && layers.fallback_llm_enabled()
                && matches!(error, RoutingError::NoCandidate { .. })
                && !error.is_context_length_insufficient() =>
        {
            let mut fallback = request.clone();
            fallback.model = AICC_FALLBACK_LLM.to_owned();
            let mut route = resolve(router, layers, &fallback, false)?;
            route.coverage = true;
            route.decision.trace.requested_model = usecase.to_owned();
            route.decision.trace.fallback = Some(RouteFallback {
                kind: RouteFallbackKind::Coverage,
                from: usecase.to_owned(),
                to: AICC_FALLBACK_LLM.to_owned(),
                policy: None,
                reason: Some("usecase_has_no_model".into()),
            });
            route.decision.trace.user_summary.was_fallback = true;
            Ok(route)
        }
        Err(error) => Err(error),
    }
}

fn skip_reason(error: &RoutingError) -> String {
    match error {
        RoutingError::InvalidExactModel(_) => "model_missing".into(),
        RoutingError::ExactModelUnavailable { reasons, .. } => reasons
            .first()
            .map(|reason| reason.code.clone())
            .unwrap_or_else(|| "model_unavailable".into()),
        other => other.to_string(),
    }
}

/// What a usecase policy needs to look for a backup model.
pub(crate) struct FallbackSearch<'a> {
    /// Unit prices of an exact model, `None` when unknown.
    pub price_of: &'a dyn Fn(&str) -> Option<crate::catalog::Pricing>,
    pub to_usd: &'a dyn Fn(f64, &str) -> Option<f64>,
    /// Provider instance names in configuration order.
    pub provider_order: &'a [String],
    /// Providers an administrator marked faulty.
    pub faulty: &'a BTreeSet<String>,
}

/// The backup model `policy` gives when the Provider of `failed` is
/// unavailable (doc/aicc/aicc_router.md §9.3). The failed Provider and every
/// Provider marked faulty count as disabled; the backup must meet the same
/// request and usecase requirements. `Ok(None)`: the policy has no model.
pub(crate) fn unavailable_fallback<Q: QuotaSource>(
    router: &Router<'_, Q>,
    layers: &UsecaseLayers,
    request: &RoutingRequest,
    coverage: bool,
    failed: &super::SelectedRoute,
    policy: OnUnavailable,
    search: &FallbackSearch<'_>,
) -> Result<Option<RouteDecision>, RoutingError> {
    let mut request = request.clone();
    request
        .excluded_providers
        .insert(failed.provider_instance_name.clone());
    request
        .excluded_providers
        .extend(search.faulty.iter().cloned());
    let decision = match policy {
        OnUnavailable::None => return Ok(None),
        OnUnavailable::SameModel => same_model(router, &request, coverage, failed, search),
        OnUnavailable::NextModel => match resolve(router, layers, &request, coverage) {
            Ok(route) => Some(route.decision),
            Err(error) if no_candidate(&error) => None,
            Err(error) => return Err(error),
        },
        OnUnavailable::NotPricier => {
            let Some(original) = (search.price_of)(&failed.exact_model) else {
                return Ok(None);
            };
            let mut found = None;
            for _ in 0..64 {
                let route = match resolve(router, layers, &request, coverage) {
                    Ok(route) => route,
                    Err(error) if no_candidate(&error) => break,
                    Err(error) => return Err(error),
                };
                let selected = &route.decision.selected.exact_model;
                if (search.price_of)(selected).is_some_and(|price| {
                    crate::policy::pricing::not_pricier(&price, &original, search.to_usd)
                }) {
                    found = Some(route.decision);
                    break;
                }
                request.excluded_models.insert(selected.clone());
            }
            found
        }
    };
    Ok(decision.map(|mut decision| {
        decision.trace.fallback = Some(RouteFallback {
            kind: RouteFallbackKind::Unavailable,
            from: failed.exact_model.clone(),
            to: decision.selected.exact_model.clone(),
            policy: Some(policy),
            reason: None,
        });
        decision.trace.user_summary.was_failover = true;
        decision
    }))
}

fn no_candidate(error: &RoutingError) -> bool {
    matches!(
        error,
        RoutingError::NoCandidate { .. } | RoutingError::ExactModelUnavailable { .. }
    )
}

/// The same model on another Provider: same Model Driver, origin model and
/// variant; by Provider weight, then by configuration order.
fn same_model<Q: QuotaSource>(
    router: &Router<'_, Q>,
    request: &RoutingRequest,
    coverage: bool,
    failed: &super::SelectedRoute,
    search: &FallbackSearch<'_>,
) -> Option<RouteDecision> {
    let variant = ExactModelName::parse(&failed.exact_model)
        .ok()
        .and_then(|name| name.variant().map(str::to_owned));
    let mut peers = router
        .registry
        .model_views()
        .into_iter()
        .filter(|model| {
            model.model_driver_id == failed.model_driver_id
                && model.origin_model_id == failed.origin_model_id
                && model.variant == variant
                && !request
                    .excluded_providers
                    .contains(&model.provider_instance_name)
        })
        .collect::<Vec<_>>();
    let order = |name: &str| {
        search
            .provider_order
            .iter()
            .position(|provider| provider == name)
            .unwrap_or(usize::MAX)
    };
    peers.sort_by(|left, right| {
        router
            .registry
            .provider_weight(&right.provider_instance_name)
            .total_cmp(
                &router
                    .registry
                    .provider_weight(&left.provider_instance_name),
            )
            .then_with(|| {
                order(&left.provider_instance_name).cmp(&order(&right.provider_instance_name))
            })
            .then_with(|| left.exact_model.cmp(&right.exact_model))
    });
    let usecase = if coverage {
        AICC_FALLBACK_LLM.to_owned()
    } else {
        request.model.clone()
    };
    peers.into_iter().find_map(|peer| {
        let mut pinned = request.clone();
        pinned.model = usecase.clone();
        if is_usecase(&usecase) {
            router.route_pinned(&pinned, &peer.exact_model).ok()
        } else {
            pinned.model = peer.exact_model.clone();
            router.route(&pinned).ok()
        }
    })
}

#[cfg(test)]
mod tests;
