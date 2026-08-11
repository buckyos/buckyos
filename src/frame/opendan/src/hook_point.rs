//! Reusable `HookPoint` shape for config-driven decisions in the runtime.
//!
//! A `HookPoint` is a `{ mode, ...params }` blob that v0 parses into a fixed
//! enum at the consumption site. The shape is deliberately uniform across
//! [behavior `[on_xxx]` bypass switches][1], dispatcher rule strategies, and
//! `session_id_strategy` evaluators so a future revision can introduce a
//! `mode = "script"` variant + a single script-engine evaluator without
//! reshaping any on-disk config that already uses HookPoint.
//!
//! `mode` is a free-form string; each consumer maintains a small whitelist of
//! accepted values and reads the strategy-specific params from `params`.
//! v0 rejects unknown modes with a clear error — see
//! [`HookPoint::ensure_mode`].
//!
//! [1]: doc/opendan/Agent配置改进.md §5.2 + §7.2.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct HookPoint {
    pub mode: String,
    /// Strategy-specific extra params. `#[serde(flatten)]` so TOML
    /// `mode = "fallback_behavior"\ntarget = "..."` round-trips into
    /// `params = {"target": ...}`.
    #[serde(flatten, default)]
    pub params: BTreeMap<String, toml::Value>,
}

#[derive(Debug, thiserror::Error)]
pub enum HookPointError {
    #[error("hook `{site}`: unknown mode `{mode}` (supported: {supported})")]
    UnknownMode {
        site: &'static str,
        mode: String,
        supported: String,
    },
    #[error("hook `{site}` mode=`{mode}`: missing required param `{key}`")]
    MissingParam {
        site: &'static str,
        mode: String,
        key: &'static str,
    },
    #[error("hook `{site}` mode=`{mode}`: param `{key}` must be a string")]
    NotAString {
        site: &'static str,
        mode: String,
        key: &'static str,
    },
    #[error("hook `{site}` mode=`{mode}`: invalid param `{key}`: {reason}")]
    InvalidParam {
        site: &'static str,
        mode: String,
        key: &'static str,
        reason: String,
    },
}

impl HookPoint {
    /// Construct a fixed-mode hook with no extra params. Used by built-in
    /// defaults so call sites don't need to special-case "no hook
    /// configured" vs "default mode wired" in their decision tree.
    pub fn fixed(mode: impl Into<String>) -> Self {
        Self {
            mode: mode.into(),
            params: BTreeMap::new(),
        }
    }

    /// Validate that this hook's mode is in the supported set for the
    /// given call site. Returns the trimmed mode on success.
    pub fn ensure_mode<'a>(
        &'a self,
        site: &'static str,
        supported: &[&str],
    ) -> Result<&'a str, HookPointError> {
        let mode = self.mode.trim();
        if supported.iter().any(|s| *s == mode) {
            Ok(mode)
        } else {
            Err(HookPointError::UnknownMode {
                site,
                mode: self.mode.clone(),
                supported: supported.join(", "),
            })
        }
    }

    /// Read a required string param. Used by hooks like
    /// `fallback_behavior` which need `target = "..."`.
    pub fn require_string(
        &self,
        site: &'static str,
        key: &'static str,
    ) -> Result<&str, HookPointError> {
        let value = self.params.get(key).ok_or(HookPointError::MissingParam {
            site,
            mode: self.mode.clone(),
            key,
        })?;
        value.as_str().ok_or(HookPointError::NotAString {
            site,
            mode: self.mode.clone(),
            key,
        })
    }

    /// Read an optional string param. `None` when missing or not a string.
    pub fn optional_string(&self, key: &str) -> Option<&str> {
        self.params.get(key).and_then(|v| v.as_str())
    }

    pub fn optional_f64(
        &self,
        site: &'static str,
        key: &'static str,
    ) -> Result<Option<f64>, HookPointError> {
        let Some(value) = self.params.get(key) else {
            return Ok(None);
        };
        match value {
            toml::Value::Float(v) => Ok(Some(*v)),
            toml::Value::Integer(v) => Ok(Some(*v as f64)),
            _ => Err(HookPointError::InvalidParam {
                site,
                mode: self.mode.clone(),
                key,
                reason: "expected number".to_string(),
            }),
        }
    }

    pub fn optional_u64(
        &self,
        site: &'static str,
        key: &'static str,
    ) -> Result<Option<u64>, HookPointError> {
        let Some(value) = self.params.get(key) else {
            return Ok(None);
        };
        match value {
            toml::Value::Integer(v) if *v >= 0 => Ok(Some(*v as u64)),
            _ => Err(HookPointError::InvalidParam {
                site,
                mode: self.mode.clone(),
                key,
                reason: "expected non-negative integer".to_string(),
            }),
        }
    }

    pub fn optional_bool(
        &self,
        site: &'static str,
        key: &'static str,
    ) -> Result<Option<bool>, HookPointError> {
        let Some(value) = self.params.get(key) else {
            return Ok(None);
        };
        match value {
            toml::Value::Boolean(v) => Ok(Some(*v)),
            _ => Err(HookPointError::InvalidParam {
                site,
                mode: self.mode.clone(),
                key,
                reason: "expected boolean".to_string(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flatten_round_trip() {
        let toml_src = r#"
            mode = "fallback_behavior"
            target = "safe_mode"
        "#;
        let h: HookPoint = toml::from_str(toml_src).unwrap();
        assert_eq!(h.mode, "fallback_behavior");
        assert_eq!(h.require_string("site", "target").unwrap(), "safe_mode");
    }

    #[test]
    fn ensure_mode_rejects_unknown() {
        let h = HookPoint::fixed("never_heard_of_it");
        let err = h.ensure_mode("site", &["a", "b"]).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("never_heard_of_it"));
        assert!(msg.contains("a, b"));
    }

    #[test]
    fn missing_required_param() {
        let h = HookPoint::fixed("fallback_behavior");
        assert!(h.require_string("site", "target").is_err());
    }
}
