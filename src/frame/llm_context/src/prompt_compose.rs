//! Render-then-budget orchestrator. See `notepads/prompt_render_engine.md` §6.4.
//!
//! Translates a list of `SectionSpec` into a `Vec<AiMessage>` by running
//! `prompt_engine::render` on each template, then handing the resulting
//! strings to `prompt_budget::PromptBudgeter::fit`. No agent / workflow
//! vocabulary — both layers consume the same pipeline.

use std::collections::HashMap;

use buckyos_api::{AiMessage, AiRole};

use crate::deps::Tokenizer;
use crate::prompt_budget::{BudgetedSection, PromptBudgeter, TruncFrom};
use crate::prompt_engine::{PromptRenderEngine, RenderError, RenderStats, RenderVars, ValueLoader};

#[derive(Debug, Clone)]
pub struct SectionSpec {
    pub key: String,
    pub role: AiRole,
    pub template: String,
    pub priority: u8,
    pub min_tokens: u32,
    pub trunc: TruncFrom,
    /// Section-local vars layered on top of the request's global vars. Local
    /// keys override global keys with the same name.
    pub local_vars: Option<RenderVars>,
}

pub struct CompositionRequest<'a> {
    pub sections: Vec<SectionSpec>,
    pub total_budget_tokens: u32,
    pub vars: &'a RenderVars,
    pub engine: &'a PromptRenderEngine,
    pub tokenizer: &'a dyn Tokenizer,
}

#[derive(Debug)]
pub struct CompositionOutcome {
    pub messages: Vec<AiMessage>,
    pub dropped: Vec<String>,
    pub render_stats: HashMap<String, RenderStats>,
    pub tokens_used: u32,
    pub tokens_remaining: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum CompositionError {
    #[error("render section `{key}`: {source}")]
    Render {
        key: String,
        #[source]
        source: RenderError,
    },
}

pub async fn compose<L>(
    request: CompositionRequest<'_>,
    loader: &L,
) -> Result<CompositionOutcome, CompositionError>
where
    L: ValueLoader + ?Sized,
{
    // Phase 1: render each section's template against merged vars.
    let mut rendered: Vec<(SectionSpec, String)> = Vec::with_capacity(request.sections.len());
    let mut render_stats: HashMap<String, RenderStats> = HashMap::new();

    for spec in request.sections.into_iter() {
        let merged: RenderVars = match spec.local_vars.as_ref() {
            Some(local) => request.vars.clone().merged(local),
            None => request.vars.clone(),
        };
        let result = request
            .engine
            .render(spec.template.as_str(), &merged, loader)
            .await
            .map_err(|source| CompositionError::Render {
                key: spec.key.clone(),
                source,
            })?;
        render_stats.insert(spec.key.clone(), result.stats);
        rendered.push((spec, result.rendered));
    }

    // Phase 2: feed renderings to the budgeter.
    let budgeted: Vec<BudgetedSection> = rendered
        .iter()
        .map(|(spec, content)| BudgetedSection {
            key: spec.key.clone(),
            content: content.clone(),
            priority: spec.priority,
            min_tokens: spec.min_tokens,
            trunc: spec.trunc,
        })
        .collect();
    let budgeter = PromptBudgeter::new(request.tokenizer, request.total_budget_tokens);
    let outcome = budgeter.fit(budgeted);

    // Phase 3: build AiMessages in input order, looking up role from the
    // matching SectionSpec.
    let mut role_by_key: HashMap<String, AiRole> = HashMap::new();
    for (spec, _) in &rendered {
        role_by_key.insert(spec.key.clone(), spec.role);
    }

    let mut messages: Vec<AiMessage> = Vec::new();
    for fitted in &outcome.kept {
        if fitted.content.is_empty() {
            continue;
        }
        let role = role_by_key
            .get(&fitted.key)
            .copied()
            .unwrap_or(AiRole::User);
        messages.push(AiMessage::text(role, fitted.content.clone()));
    }

    Ok(CompositionOutcome {
        messages,
        dropped: outcome.dropped,
        render_stats,
        tokens_used: outcome.tokens_used,
        tokens_remaining: outcome.tokens_remaining,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deps::Tokenizer;
    use crate::prompt_engine::{NullValueLoader, PromptRenderEngine};
    use async_trait::async_trait;
    use buckyos_api::AiContent;
    use serde_json::Value as Json;
    use std::collections::HashMap;

    struct CharTok;
    impl Tokenizer for CharTok {
        fn count_tokens(&self, text: &str) -> u32 {
            text.chars().count() as u32
        }
    }

    struct MapLoader {
        values: HashMap<String, Json>,
    }
    #[async_trait]
    impl ValueLoader for MapLoader {
        async fn load(&self, expr: &str) -> Result<Option<Json>, RenderError> {
            Ok(self.values.get(expr).cloned())
        }
    }

    struct ErrLoader;
    #[async_trait]
    impl ValueLoader for ErrLoader {
        async fn load(&self, _expr: &str) -> Result<Option<Json>, RenderError> {
            Err(RenderError::Loader("synthetic".into()))
        }
    }

    fn spec(key: &str, role: AiRole, template: &str, priority: u8, min_tokens: u32) -> SectionSpec {
        SectionSpec {
            key: key.into(),
            role,
            template: template.into(),
            priority,
            min_tokens,
            trunc: TruncFrom::Tail,
            local_vars: None,
        }
    }

    fn first_text(msg: &AiMessage) -> &str {
        msg.content
            .iter()
            .find_map(|b| match b {
                AiContent::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .unwrap()
    }

    #[tokio::test]
    async fn single_section_fits() {
        let engine = PromptRenderEngine::with_defaults();
        let tok = CharTok;
        let vars = RenderVars::new();
        let req = CompositionRequest {
            sections: vec![spec("sys", AiRole::System, "hello", 1, 0)],
            total_budget_tokens: 100,
            vars: &vars,
            engine: &engine,
            tokenizer: &tok,
        };
        let out = compose(req, &NullValueLoader).await.unwrap();
        assert_eq!(out.messages.len(), 1);
        assert_eq!(out.messages[0].role, AiRole::System);
        assert_eq!(first_text(&out.messages[0]), "hello");
    }

    #[tokio::test]
    async fn multi_section_preserves_input_order() {
        let engine = PromptRenderEngine::with_defaults();
        let tok = CharTok;
        let vars = RenderVars::new();
        let req = CompositionRequest {
            sections: vec![
                spec("sys", AiRole::System, "S", 5, 0),
                spec("usr", AiRole::User, "U", 5, 0),
                spec("asst", AiRole::Assistant, "A", 5, 0),
            ],
            total_budget_tokens: 100,
            vars: &vars,
            engine: &engine,
            tokenizer: &tok,
        };
        let out = compose(req, &NullValueLoader).await.unwrap();
        let roles: Vec<AiRole> = out.messages.iter().map(|m| m.role).collect();
        assert_eq!(roles, vec![AiRole::System, AiRole::User, AiRole::Assistant]);
    }

    #[tokio::test]
    async fn low_priority_gets_truncated_under_pressure() {
        let engine = PromptRenderEngine::with_defaults();
        let tok = CharTok;
        let vars = RenderVars::new();
        let mut hi = spec("hi", AiRole::System, "AAAAAA", 9, 0);
        let mut lo = spec("lo", AiRole::User, "BBBBBB", 1, 1);
        hi.trunc = TruncFrom::Tail;
        lo.trunc = TruncFrom::Tail;
        let req = CompositionRequest {
            sections: vec![hi, lo],
            total_budget_tokens: 7,
            vars: &vars,
            engine: &engine,
            tokenizer: &tok,
        };
        let out = compose(req, &NullValueLoader).await.unwrap();
        // High priority kept full; low priority left with just its min=1.
        let kept_hi = &out.messages[0];
        let kept_lo = &out.messages[1];
        assert_eq!(first_text(kept_hi), "AAAAAA");
        assert_eq!(first_text(kept_lo), "B");
    }

    #[tokio::test]
    async fn section_dropped_when_min_exceeds_budget() {
        let engine = PromptRenderEngine::with_defaults();
        let tok = CharTok;
        let vars = RenderVars::new();
        let req = CompositionRequest {
            sections: vec![
                spec("big", AiRole::System, "AAAAA", 1, 100),
                spec("ok", AiRole::User, "B", 1, 0),
            ],
            total_budget_tokens: 4,
            vars: &vars,
            engine: &engine,
            tokenizer: &tok,
        };
        let out = compose(req, &NullValueLoader).await.unwrap();
        assert_eq!(out.dropped, vec!["big".to_string()]);
        assert_eq!(out.messages.len(), 1);
        assert_eq!(first_text(&out.messages[0]), "B");
    }

    #[tokio::test]
    async fn local_vars_override_global_vars() {
        let engine = PromptRenderEngine::with_defaults();
        let tok = CharTok;
        let mut vars = RenderVars::new();
        vars.vars
            .insert("name".to_string(), Json::String("global".into()));
        let mut local = RenderVars::new();
        local
            .vars
            .insert("name".to_string(), Json::String("local".into()));

        let s = SectionSpec {
            key: "x".into(),
            role: AiRole::User,
            template: "hi {{ name }}".into(),
            priority: 1,
            min_tokens: 0,
            trunc: TruncFrom::Tail,
            local_vars: Some(local),
        };
        let req = CompositionRequest {
            sections: vec![s],
            total_budget_tokens: 100,
            vars: &vars,
            engine: &engine,
            tokenizer: &tok,
        };
        let out = compose(req, &NullValueLoader).await.unwrap();
        assert_eq!(first_text(&out.messages[0]), "hi local");
    }

    #[tokio::test]
    async fn loader_error_propagates_with_key() {
        let engine = PromptRenderEngine::with_defaults();
        let tok = CharTok;
        let vars = RenderVars::new();
        let req = CompositionRequest {
            sections: vec![spec("bad", AiRole::User, "x {{ y }}", 1, 0)],
            total_budget_tokens: 100,
            vars: &vars,
            engine: &engine,
            tokenizer: &tok,
        };
        let err = compose(req, &ErrLoader).await.unwrap_err();
        match err {
            CompositionError::Render { key, .. } => assert_eq!(key, "bad"),
        }
    }

    #[tokio::test]
    async fn same_role_sections_are_not_merged() {
        let engine = PromptRenderEngine::with_defaults();
        let tok = CharTok;
        let vars = RenderVars::new();
        let req = CompositionRequest {
            sections: vec![
                spec("a", AiRole::System, "alpha", 1, 0),
                spec("b", AiRole::System, "beta", 1, 0),
            ],
            total_budget_tokens: 100,
            vars: &vars,
            engine: &engine,
            tokenizer: &tok,
        };
        let out = compose(req, &NullValueLoader).await.unwrap();
        assert_eq!(out.messages.len(), 2);
        assert_eq!(first_text(&out.messages[0]), "alpha");
        assert_eq!(first_text(&out.messages[1]), "beta");
    }

    #[tokio::test]
    async fn render_stats_recorded_per_section() {
        let engine = PromptRenderEngine::with_defaults();
        let tok = CharTok;
        let vars = RenderVars::new();
        let loader = MapLoader {
            values: HashMap::from([("user".to_string(), Json::String("alice".to_string()))]),
        };
        let req = CompositionRequest {
            sections: vec![spec("s", AiRole::User, "hi {{ user }}", 1, 0)],
            total_budget_tokens: 100,
            vars: &vars,
            engine: &engine,
            tokenizer: &tok,
        };
        let out = compose(req, &loader).await.unwrap();
        assert_eq!(first_text(&out.messages[0]), "hi alice");
        let stats = out.render_stats.get("s").unwrap();
        assert_eq!(stats.var_registered, 1);
    }
}
