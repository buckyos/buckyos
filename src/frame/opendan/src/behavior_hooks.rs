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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderFailedOutcome {
    Default,
    /// `mode = "fallback_behavior"` with `target = "<behavior_name>"`.
    FallbackBehavior { target: String },
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

pub fn resolve_ctx_limit(
    hook: Option<&HookPoint>,
) -> Result<CtxLimitOutcome, HookPointError> {
    let Some(hook) = hook else {
        return Ok(CtxLimitOutcome::Default);
    };
    let mode = hook.ensure_mode("on_context_limit_reached", &["compress_then_continue"])?;
    match mode {
        "compress_then_continue" => Ok(CtxLimitOutcome::CompressThenContinue),
        _ => unreachable!("ensure_mode whitelist already verified"),
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
    let mode =
        hook.ensure_mode("on_interrupt_graceful", &["cancel_pending_tools_then_continue"])?;
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
