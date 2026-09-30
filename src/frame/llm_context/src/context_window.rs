//! Context window accounting behind `Outcome::ContextLimitReached`.
//!
//! The waist compares an estimate of the request it is about to send with
//! the limits the caller put in `BudgetSpec`. It never looks up model
//! capabilities and never rewrites history: exceeding a limit only yields.
//!
//! The estimate goes through the injected [`Tokenizer`] for every text part
//! (message text, tool call arguments, tool results, thinking text, tool
//! specs, output schema) plus [`MESSAGE_OVERHEAD_TOKENS`] per message. Images,
//! documents and other non-text parts count [`MEDIA_PART_TOKENS`] each, so a
//! prompt dominated by large documents is underestimated; such callers should
//! leave headroom in the threshold. The estimate is compared with the
//! request size only; Provider-reported usage is never used for the check.

use buckyos_api::{AiContent, AiMessage, AiToolResultContent};

use crate::deps::{LlmInferenceRequest, Tokenizer};
use crate::outcome::ContextLimitKind;
use crate::request::{ContextThreshold, LLMContextRequest};

/// Role / framing overhead charged per message.
pub const MESSAGE_OVERHEAD_TOKENS: u64 = 4;
/// Flat charge for one image / document / other non-text part.
pub const MEDIA_PART_TOKENS: u64 = 1024;

/// Validated context limits of a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextLimits {
    /// Early-yield threshold in tokens (`>=` yields).
    pub threshold: Option<u64>,
    /// Model window in tokens.
    pub window: Option<u64>,
    /// Completion tokens reserved inside the window.
    pub reserve: u64,
}

impl ContextLimits {
    pub fn of(request: &LLMContextRequest) -> Result<Self, String> {
        let budget = &request.budget;
        let window = match budget.context_window_tokens {
            Some(0) => return Err("context_window_tokens must be > 0".to_string()),
            w => w.map(u64::from),
        };
        let reserve = u64::from(request.model_policy.max_completion_tokens.unwrap_or(0));
        if let Some(w) = window {
            if reserve >= w {
                return Err(format!(
                    "max_completion_tokens {reserve} leaves no room in context_window_tokens {w}"
                ));
            }
        }
        let threshold = match budget.context_yield_threshold {
            None => None,
            Some(ContextThreshold::AbsoluteTokens { value: 0 }) => {
                return Err("AbsoluteTokens threshold must be > 0".to_string())
            }
            Some(ContextThreshold::AbsoluteTokens { value }) => Some(u64::from(value)),
            Some(ContextThreshold::Ratio { value }) => {
                if !value.is_finite() || value <= 0.0 || value > 1.0 {
                    return Err(format!("Ratio threshold {value} is outside (0, 1]"));
                }
                let w = window
                    .ok_or_else(|| "Ratio threshold requires context_window_tokens".to_string())?;
                Some(((w as f64 * f64::from(value)).floor() as u64).max(1))
            }
        };
        Ok(Self {
            threshold,
            window,
            reserve,
        })
    }

    /// Whether any limit is configured.
    pub fn is_active(&self) -> bool {
        self.threshold.is_some() || self.window.is_some()
    }

    /// The limit an `estimated`-token request hits, if any. A request that
    /// cannot fit the window is reported before the early threshold.
    pub fn check(&self, estimated: u64) -> Option<ContextLimitKind> {
        if let Some(w) = self.window {
            if estimated.saturating_add(self.reserve) > w {
                return Some(ContextLimitKind::HardLimit);
            }
        }
        match self.threshold {
            Some(t) if estimated >= t => Some(ContextLimitKind::ApproachingWindow),
            _ => None,
        }
    }
}

fn text(tokenizer: &dyn Tokenizer, s: &str) -> u64 {
    u64::from(tokenizer.count_tokens(s))
}

fn part_tokens(tokenizer: &dyn Tokenizer, part: &AiContent) -> u64 {
    match part {
        AiContent::Text { text: t } => text(tokenizer, t),
        AiContent::Image { .. } | AiContent::Document { .. } => MEDIA_PART_TOKENS,
        AiContent::ToolUse { name, args, .. } => {
            text(tokenizer, name)
                + text(tokenizer, &serde_json::to_string(args).unwrap_or_default())
        }
        AiContent::ToolResult { content, .. } => content
            .iter()
            .map(|c| match c {
                AiToolResultContent::Text { text: t } => text(tokenizer, t),
                _ => MEDIA_PART_TOKENS,
            })
            .sum(),
        AiContent::Thinking {
            summary, text: t, ..
        } => {
            summary.as_deref().map_or(0, |s| text(tokenizer, s))
                + t.as_deref().map_or(0, |s| text(tokenizer, s))
        }
        AiContent::ProviderState { value, .. } => text(tokenizer, &value.to_string()),
    }
}

/// Estimated input tokens of `messages`.
pub fn estimate_messages(tokenizer: &dyn Tokenizer, messages: &[AiMessage]) -> u64 {
    messages
        .iter()
        .map(|m| {
            MESSAGE_OVERHEAD_TOKENS
                + m.content
                    .iter()
                    .map(|p| part_tokens(tokenizer, p))
                    .sum::<u64>()
        })
        .sum()
}

/// Estimated input tokens of one inference request: messages, the tool
/// catalogue when tool calls are allowed, and the output schema.
pub fn estimate_request(tokenizer: &dyn Tokenizer, req: &LlmInferenceRequest) -> u64 {
    let mut total = estimate_messages(tokenizer, &req.messages);
    if req.allow_tool_calls {
        for spec in &req.tool_specs {
            total += text(tokenizer, &spec.name)
                + text(tokenizer, &spec.description)
                + text(tokenizer, &spec.args_schema.to_string());
        }
    }
    if req.force_json {
        if let Some(schema) = &req.json_schema {
            total += text(tokenizer, &schema.to_string());
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::{BudgetSpec, ContextOwnerRef};

    fn request(budget: BudgetSpec, max_completion: Option<u32>) -> LLMContextRequest {
        let mut r = LLMContextRequest {
            owner: ContextOwnerRef::OneShot { id: "t".into() },
            trace: None,
            objective: String::new(),
            behavior_name: String::new(),
            input: Vec::new(),
            model_policy: Default::default(),
            tool_policy: Default::default(),
            output: Default::default(),
            budget,
            human_policy: Default::default(),
            error_policy: Default::default(),
            forbid_next_behavior: false,
        };
        r.model_policy.max_completion_tokens = max_completion;
        r
    }

    fn limits(
        threshold: Option<ContextThreshold>,
        window: Option<u32>,
        reserve: Option<u32>,
    ) -> Result<ContextLimits, String> {
        ContextLimits::of(&request(
            BudgetSpec {
                context_yield_threshold: threshold,
                context_window_tokens: window,
                ..Default::default()
            },
            reserve,
        ))
    }

    #[test]
    fn absolute_threshold_yields_at_and_above_the_boundary() {
        let l = limits(
            Some(ContextThreshold::AbsoluteTokens { value: 100 }),
            None,
            None,
        )
        .unwrap();
        assert_eq!(l.check(99), None);
        assert_eq!(l.check(100), Some(ContextLimitKind::ApproachingWindow));
        assert_eq!(l.check(101), Some(ContextLimitKind::ApproachingWindow));
    }

    #[test]
    fn ratio_uses_the_window_and_hard_limit_counts_the_reserve() {
        let l = limits(
            Some(ContextThreshold::Ratio { value: 0.5 }),
            Some(1000),
            Some(200),
        )
        .unwrap();
        assert_eq!(l.threshold, Some(500));
        assert_eq!(l.check(499), None);
        assert_eq!(l.check(500), Some(ContextLimitKind::ApproachingWindow));
        assert_eq!(l.check(800), Some(ContextLimitKind::ApproachingWindow));
        assert_eq!(l.check(801), Some(ContextLimitKind::HardLimit));
    }

    #[test]
    fn window_without_threshold_only_checks_the_hard_limit() {
        let l = limits(None, Some(1000), None).unwrap();
        assert_eq!(l.check(1000), None);
        assert_eq!(l.check(1001), Some(ContextLimitKind::HardLimit));
        let none = limits(None, None, Some(4096)).unwrap();
        assert_eq!(none.check(u64::MAX), None);
    }

    #[test]
    fn invalid_limits_are_rejected() {
        assert!(limits(
            Some(ContextThreshold::AbsoluteTokens { value: 0 }),
            None,
            None
        )
        .is_err());
        assert!(limits(Some(ContextThreshold::Ratio { value: 0.5 }), None, None).is_err());
        for bad in [0.0, -0.1, 1.5, f32::NAN, f32::INFINITY] {
            assert!(
                limits(
                    Some(ContextThreshold::Ratio { value: bad }),
                    Some(1000),
                    None
                )
                .is_err(),
                "ratio {bad} must be rejected"
            );
        }
        assert!(limits(None, Some(0), None).is_err());
        assert!(limits(None, Some(1000), Some(1000)).is_err());
        assert!(limits(
            Some(ContextThreshold::Ratio { value: 1.0 }),
            Some(1000),
            None
        )
        .is_ok());
    }
}
