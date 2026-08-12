//! Typed outcomes for behavior `[on_xxx]` bypass switches.
//!
//! Per doc/opendan/Agent配置改进.md §5 + §7.2, the four bypass switches
//! (`on_context_limit_reached`, `on_provider_failed`,
//! `on_interrupt_graceful`, `on_interrupt_discard`) are each "write the
//! section ⇒ enable; omit ⇒ runtime default". v0 ships exactly one mode
//! per site — the resolvers below decode `Option<HookPoint>` into a small
//! typed enum the session worker switches on.
//!
//! The runtime-default branch is `*::Default` in each outcome enum and
//! corresponds to the pre-config-rewrite hardcoded behavior:
//! - context limit ⇒ "compress up to MAX_COMPRESS_ROUNDS then abort"
//!   (so omitting the hook keeps today's safety net)
//! - provider failed ⇒ surface error
//! - graceful interrupt ⇒ cancel pending tools then continue
//! - discard interrupt ⇒ truncate history & wait for next input
//!
//! Future revisions can wire a `mode = "script"` variant + an additional
//! enum variant carrying the script handle; existing call sites that
//! match on the enum will get a compile error and have to address it.

use crate::hook_point::{HookPoint, HookPointError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CtxLimitOutcome {
    /// No hook configured ⇒ keep the historical compress-then-abort policy.
    Default,
    /// `mode = "compress_then_continue"`
    CompressThenContinue,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LlmMessageCompressPolicy {
    pub trigger_ratio: f32,
    pub target_ratio: f32,
    pub hard_limit_ratio: f32,
    pub min_turns_between_compress: u32,
    pub preserve_cache_stability: bool,
    pub context_window_tokens: Option<u32>,
}

impl Default for LlmMessageCompressPolicy {
    fn default() -> Self {
        Self {
            trigger_ratio: 0.80,
            target_ratio: 0.50,
            hard_limit_ratio: 0.95,
            min_turns_between_compress: 2,
            preserve_cache_stability: true,
            context_window_tokens: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderFailedOutcome {
    Default,
    /// `mode = "fallback_behavior"` with `target = "<behavior_name>"`.
    FallbackBehavior {
        target: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InterruptOutcome {
    Default,
    /// `mode = "cancel_pending_tools_then_continue"` — only valid for the
    /// graceful interrupt site. The discard site rejects it at load time.
    CancelPendingThenContinue,
    /// `mode = "end"` — only valid for the discard interrupt site.
    End,
}

pub fn resolve_ctx_limit(hook: Option<&HookPoint>) -> Result<CtxLimitOutcome, HookPointError> {
    let Some(hook) = hook else {
        return Ok(CtxLimitOutcome::Default);
    };
    let mode = hook.ensure_mode("on_context_limit_reached", &["compress_then_continue"])?;
    match mode {
        "compress_then_continue" => Ok(CtxLimitOutcome::CompressThenContinue),
        _ => unreachable!("ensure_mode whitelist already verified"),
    }
}

pub fn resolve_llm_message_compress(
    hook: Option<&HookPoint>,
) -> Result<Option<LlmMessageCompressPolicy>, HookPointError> {
    const SITE: &str = "on_llm_message_compress";
    let Some(hook) = hook else {
        return Ok(None);
    };
    let mode = hook.ensure_mode(SITE, &["context_window_ratio"])?;
    match mode {
        "context_window_ratio" => {
            let mut policy = LlmMessageCompressPolicy::default();
            if let Some(value) = hook.optional_f64(SITE, "trigger_ratio")? {
                policy.trigger_ratio = checked_ratio(hook, SITE, "trigger_ratio", value)?;
            }
            if let Some(value) = hook.optional_f64(SITE, "target_ratio")? {
                policy.target_ratio = checked_ratio(hook, SITE, "target_ratio", value)?;
            }
            if let Some(value) = hook.optional_f64(SITE, "hard_limit_ratio")? {
                policy.hard_limit_ratio = checked_ratio(hook, SITE, "hard_limit_ratio", value)?;
            }
            if policy.target_ratio >= policy.trigger_ratio {
                return Err(HookPointError::InvalidParam {
                    site: SITE,
                    mode: hook.mode.clone(),
                    key: "target_ratio",
                    reason: "must be lower than trigger_ratio".to_string(),
                });
            }
            if policy.hard_limit_ratio < policy.trigger_ratio {
                return Err(HookPointError::InvalidParam {
                    site: SITE,
                    mode: hook.mode.clone(),
                    key: "hard_limit_ratio",
                    reason: "must be greater than or equal to trigger_ratio".to_string(),
                });
            }
            if let Some(value) = hook.optional_u64(SITE, "min_turns_between_compress")? {
                policy.min_turns_between_compress = value.min(u32::MAX as u64) as u32;
            }
            if let Some(value) = hook.optional_bool(SITE, "preserve_cache_stability")? {
                policy.preserve_cache_stability = value;
            }
            if let Some(value) = hook.optional_u64(SITE, "context_window_tokens")? {
                if value == 0 {
                    return Err(HookPointError::InvalidParam {
                        site: SITE,
                        mode: hook.mode.clone(),
                        key: "context_window_tokens",
                        reason: "must be greater than 0".to_string(),
                    });
                }
                policy.context_window_tokens = Some(value.min(u32::MAX as u64) as u32);
            }
            Ok(Some(policy))
        }
        _ => unreachable!("ensure_mode whitelist already verified"),
    }
}

fn checked_ratio(
    hook: &HookPoint,
    site: &'static str,
    key: &'static str,
    value: f64,
) -> Result<f32, HookPointError> {
    if value > 0.0 && value <= 1.0 {
        Ok(value as f32)
    } else {
        Err(HookPointError::InvalidParam {
            site,
            mode: hook.mode.clone(),
            key,
            reason: "must be in (0.0, 1.0]".to_string(),
        })
    }
}

pub fn resolve_provider_failed(
    hook: Option<&HookPoint>,
) -> Result<ProviderFailedOutcome, HookPointError> {
    let Some(hook) = hook else {
        return Ok(ProviderFailedOutcome::Default);
    };
    let mode = hook.ensure_mode("on_provider_failed", &["fallback_behavior"])?;
    match mode {
        "fallback_behavior" => {
            let target = hook
                .require_string("on_provider_failed", "target")?
                .to_string();
            Ok(ProviderFailedOutcome::FallbackBehavior { target })
        }
        _ => unreachable!("ensure_mode whitelist already verified"),
    }
}

pub fn resolve_interrupt_graceful(
    hook: Option<&HookPoint>,
) -> Result<InterruptOutcome, HookPointError> {
    let Some(hook) = hook else {
        return Ok(InterruptOutcome::Default);
    };
    let mode = hook.ensure_mode(
        "on_interrupt_graceful",
        &["cancel_pending_tools_then_continue"],
    )?;
    match mode {
        "cancel_pending_tools_then_continue" => Ok(InterruptOutcome::CancelPendingThenContinue),
        _ => unreachable!("ensure_mode whitelist already verified"),
    }
}

pub fn resolve_interrupt_discard(
    hook: Option<&HookPoint>,
) -> Result<InterruptOutcome, HookPointError> {
    let Some(hook) = hook else {
        return Ok(InterruptOutcome::Default);
    };
    let mode = hook.ensure_mode("on_interrupt_discard", &["end"])?;
    match mode {
        "end" => Ok(InterruptOutcome::End),
        _ => unreachable!("ensure_mode whitelist already verified"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctx_limit_default_when_none() {
        assert_eq!(resolve_ctx_limit(None).unwrap(), CtxLimitOutcome::Default);
    }

    #[test]
    fn ctx_limit_accepts_compress() {
        let h = HookPoint::fixed("compress_then_continue");
        assert_eq!(
            resolve_ctx_limit(Some(&h)).unwrap(),
            CtxLimitOutcome::CompressThenContinue
        );
    }

    #[test]
    fn ctx_limit_rejects_unknown() {
        let h = HookPoint::fixed("retry_with_smaller_model");
        assert!(resolve_ctx_limit(Some(&h)).is_err());
    }

    #[test]
    fn llm_message_compress_absent_disables_auto_trigger() {
        assert_eq!(resolve_llm_message_compress(None).unwrap(), None);
    }

    #[test]
    fn llm_message_compress_accepts_ratio_policy() {
        let toml_src = r#"
            mode = "context_window_ratio"
            trigger_ratio = 0.82
            target_ratio = 0.61
            hard_limit_ratio = 0.96
            min_turns_between_compress = 3
            preserve_cache_stability = false
            context_window_tokens = 128000
        "#;
        let h: HookPoint = toml::from_str(toml_src).unwrap();
        let policy = resolve_llm_message_compress(Some(&h)).unwrap().unwrap();
        assert_eq!(policy.trigger_ratio, 0.82);
        assert_eq!(policy.target_ratio, 0.61);
        assert_eq!(policy.hard_limit_ratio, 0.96);
        assert_eq!(policy.min_turns_between_compress, 3);
        assert!(!policy.preserve_cache_stability);
        assert_eq!(policy.context_window_tokens, Some(128000));
    }

    #[test]
    fn llm_message_compress_rejects_target_above_trigger() {
        let toml_src = r#"
            mode = "context_window_ratio"
            trigger_ratio = 0.80
            target_ratio = 0.90
        "#;
        let h: HookPoint = toml::from_str(toml_src).unwrap();
        assert!(resolve_llm_message_compress(Some(&h)).is_err());
    }

    #[test]
    fn provider_fallback_requires_target() {
        let h = HookPoint::fixed("fallback_behavior");
        assert!(resolve_provider_failed(Some(&h)).is_err());
    }

    #[test]
    fn provider_fallback_picks_target() {
        let toml_src = r#"
            mode = "fallback_behavior"
            target = "explorer_safe_mode"
        "#;
        let h: HookPoint = toml::from_str(toml_src).unwrap();
        assert_eq!(
            resolve_provider_failed(Some(&h)).unwrap(),
            ProviderFailedOutcome::FallbackBehavior {
                target: "explorer_safe_mode".to_string()
            }
        );
    }

    #[test]
    fn graceful_only_accepts_its_mode() {
        let h = HookPoint::fixed("end");
        assert!(resolve_interrupt_graceful(Some(&h)).is_err());
    }

    #[test]
    fn discard_only_accepts_end() {
        let h = HookPoint::fixed("cancel_pending_tools_then_continue");
        assert!(resolve_interrupt_discard(Some(&h)).is_err());
        let h = HookPoint::fixed("end");
        assert_eq!(
            resolve_interrupt_discard(Some(&h)).unwrap(),
            InterruptOutcome::End
        );
    }
}
