//! `browser.*` of the Agent State service (Agent Browser Protocol §2).

use std::sync::Arc;

use serde_json::{json, Value};

use super::cdp::CdpError;
use super::groups::AgentState;
use super::sessions::Handed;
use super::BrowserModule;

/// An error of the browser module: `code` is what a caller acts on
/// (Agent Browser Protocol §2.3).
#[derive(Debug, Clone, PartialEq)]
pub struct BrowserError {
    pub code: &'static str,
    pub message: String,
    /// Of a `browser_unavailable`: the lasting reason this agent has no
    /// browser (`disabled`, `no_package`, `not_downloaded`); `None` when it
    /// is none of them (the browser did not start, or went away).
    pub unavailable_code: Option<&'static str>,
}

impl BrowserError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), unavailable_code: None }
    }

    pub fn unavailable(why: impl Into<String>) -> Self {
        Self::new("browser_unavailable", why)
    }

    /// `browser_unavailable` for a reason a caller can act on.
    pub fn no_browser(unavailable_code: &'static str, why: impl Into<String>) -> Self {
        Self { unavailable_code: Some(unavailable_code), ..Self::unavailable(why) }
    }

    pub fn invalid(why: impl Into<String>) -> Self {
        Self::new("invalid_argument", why)
    }

    pub fn user_in_control(group_id: &str) -> Self {
        Self::new(
            "user_in_control",
            format!("the user is operating the browser (group {group_id}); wait until they hand it back"),
        )
    }

    pub fn lease_lost(lease_id: &str) -> Self {
        Self::new("lease_lost", format!("lease {lease_id} ran out or was ended (the user took over, or it was not renewed)"))
    }

    pub fn no_such_group(group_id: &str) -> Self {
        Self::new("no_such_group", format!("no group `{group_id}`"))
    }

    pub fn no_such_tab(tab_id: &str) -> Self {
        Self::new("no_such_tab", format!("no tab `{tab_id}`"))
    }

    pub fn too_many_tabs(max: usize) -> Self {
        Self::new("too_many_tabs", format!("the browser already has {max} tabs; close one first"))
    }

    /// `kind` of the service's error form (`OpenDanError::to_json`).
    fn kind(&self) -> &'static str {
        match self.code {
            "invalid_argument" => "invalid_argument",
            "lease_lost" => "lease_lost",
            code if code.starts_with("no_such_") => "not_found",
            _ => "other",
        }
    }

    /// `{code, message}`, and `unavailable_code` when it has one: what the
    /// screen and the upload entry answer with.
    pub fn to_json(&self) -> Value {
        let mut error = json!({ "code": self.code, "message": self.message });
        if let Some(unavailable_code) = self.unavailable_code {
            error["unavailable_code"] = json!(unavailable_code);
        }
        error
    }

    /// The service's wire form of an error, with the `code`.
    pub fn to_wire(&self) -> String {
        let mut error = self.to_json();
        error["kind"] = json!(self.kind());
        error.to_string()
    }
}

impl std::fmt::Display for BrowserError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl From<CdpError> for BrowserError {
    fn from(e: CdpError) -> Self {
        match e {
            CdpError::Remote { .. } => Self::invalid(format!("the browser refused: {e}")),
            other => Self::unavailable(other.to_string()),
        }
    }
}

fn param<T: serde::de::DeserializeOwned>(params: &Value, name: &str) -> Result<T, BrowserError> {
    serde_json::from_value(params.get(name).cloned().unwrap_or(Value::Null))
        .map_err(|e| BrowserError::invalid(format!("param `{name}`: {e}")))
}

fn text(params: &Value, name: &str) -> Result<String, BrowserError> {
    let value: String = param(params, name)?;
    if value.trim().is_empty() {
        return Err(BrowserError::invalid(format!("param `{name}` is empty")));
    }
    Ok(value)
}

fn group_param(params: &Value) -> Result<String, BrowserError> {
    let group_id = text(params, "group_id")?;
    if super::groups::is_internal(&group_id) || group_id.len() > 256 {
        return Err(BrowserError::no_such_group(&group_id));
    }
    Ok(group_id)
}

fn encode<T: serde::Serialize>(value: &T) -> Result<Value, BrowserError> {
    serde_json::to_value(value).map_err(|e| BrowserError::new("other", format!("encode: {e}")))
}

/// Serve one `browser.*` call of `who`. `token` is the session token the
/// caller presented (`None`: it presented none): what it reads and writes in
/// the Zone on the caller's behalf is read and written with it.
pub async fn call(browser: &Arc<BrowserModule>, who: &str, token: Option<&str>, method: &str, params: &Value) -> Result<Value, BrowserError> {
    match method {
        "browser.status" => {
            let group_id = param::<Option<String>>(params, "group_id")?;
            let session_id = param::<Option<String>>(params, "session_id")?;
            Ok(browser.status(group_id.as_deref(), session_id.as_deref()).await)
        }
        "browser.ensure" => {
            browser.ensure().await?;
            Ok(browser.status(None, None).await)
        }
        "browser.acquire" => {
            let session_id = text(params, "session_id")?;
            encode(&browser.acquire(&session_id).await?)
        }
        "browser.renew" => {
            let expires_at_ms = browser.renew(&text(params, "lease_id")?)?;
            Ok(json!({ "expires_at_ms": expires_at_ms }))
        }
        "browser.release" => {
            let state = match param::<Option<String>>(params, "state")?.as_deref() {
                None => None,
                Some("done") => Some(AgentState::Done),
                Some("stuck") => Some(AgentState::Stuck),
                Some(other) => return Err(BrowserError::invalid(format!("state is done | stuck, not `{other}`"))),
            };
            let note = param::<Option<String>>(params, "note")?;
            browser.release(&text(params, "lease_id")?, state, note)?;
            Ok(json!({}))
        }
        "browser.request_handover" => {
            let session_id = text(params, "session_id")?;
            let reason = text(params, "reason")?;
            if !matches!(reason.as_str(), "password" | "captcha" | "other") {
                return Err(BrowserError::invalid(format!("reason is password | captcha | other, not `{reason}`")));
            }
            let note = param::<Option<String>>(params, "note")?;
            let group_id = browser.request_handover(&session_id, &reason, note).await?;
            Ok(json!({ "group_id": group_id }))
        }
        "browser.take_over" => encode(&browser.hand(who, &group_param(params)?, Handed::TakeOver).await?),
        "browser.hand_back" => encode(&browser.hand(who, &group_param(params)?, Handed::HandBack).await?),
        "browser.hand_to_agent" => encode(&browser.hand(who, &group_param(params)?, Handed::HandToAgent).await?),
        "browser.bind_default" => {
            let session_id = text(params, "session_id")?;
            if session_id == super::groups::DEFAULT_GROUP || super::groups::is_internal(&session_id) || session_id.len() > 256 {
                return Err(BrowserError::invalid(format!("`{session_id}` is not the id of a conversation")));
            }
            Ok(json!({ "default_session_id": browser.bind_default(&session_id)? }))
        }
        "browser.clear_profile" => {
            browser.clear_profile().await?;
            Ok(json!({}))
        }
        "browser.upload" => {
            browser.usable()?;
            let request_id = text(params, "request_id")?;
            let done = param::<Option<bool>>(params, "done")?.unwrap_or(false);
            super::transfer::upload(browser, token, &request_id, &text(params, "zone_path")?, done).await?;
            Ok(json!({}))
        }
        "browser.download_export" => {
            browser.usable()?;
            super::transfer::export(browser, token, &text(params, "download_id")?).await
        }
        "browser.reflow_state" => browser.reflow_state(token, &text(params, "url")?).await,
        "browser.reflow_refresh" => browser.reflow_refresh(token, &text(params, "url")?, None).await,
        "browser.reflow_action" => {
            let selector = text(params, "selector")?;
            browser.reflow_refresh(token, &text(params, "url")?, Some(selector)).await
        }
        other => Err(BrowserError::invalid(format!("unknown method `{other}`"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_carry_a_code_in_the_services_wire_form() {
        for (error, kind) in [
            (BrowserError::user_in_control("default"), "other"),
            (BrowserError::lease_lost("lease-1"), "lease_lost"),
            (BrowserError::no_such_tab("t"), "not_found"),
            (BrowserError::no_such_group("g"), "not_found"),
            (BrowserError::too_many_tabs(12), "other"),
            (BrowserError::unavailable("no chromium"), "other"),
            (BrowserError::invalid("param"), "invalid_argument"),
        ] {
            let wire: Value = serde_json::from_str(&error.to_wire()).unwrap();
            assert_eq!(wire["code"], error.code);
            assert_eq!(wire["kind"], kind, "{}", error.code);
            assert_eq!(wire["message"], error.message);
            assert_eq!(wire.get("unavailable_code"), None, "{}", error.code);
        }
        // Why the agent has no browser, when it is something a caller acts on.
        let waiting = BrowserError::no_browser("not_downloaded", "the browser is being downloaded to this node (42% of 200 MiB)");
        let wire: Value = serde_json::from_str(&waiting.to_wire()).unwrap();
        assert_eq!(
            wire,
            json!({ "kind": "other", "code": "browser_unavailable", "unavailable_code": "not_downloaded", "message": waiting.message })
        );
        assert!(libopendan::state::krpc::error_from_wire(&waiting.to_wire()).to_string().contains("42% of 200 MiB"));
        // What a caller of the facade sees keeps the message.
        let seen = libopendan::state::krpc::error_from_wire(&BrowserError::user_in_control("default").to_wire());
        assert!(seen.to_string().contains("the user is operating the browser"), "{seen}");
        assert_eq!(BrowserError::from(CdpError::Closed).code, "browser_unavailable");
        assert_eq!(BrowserError::from(CdpError::Remote { code: -32000, message: "bad url".into() }).code, "invalid_argument");
    }
}
