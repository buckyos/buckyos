//! `user.preferences.get` (notepads/user-preferences-api-todo.md): the signed-in user's settings for a page of an
//! App, or of a system service, on its own origin — the theme, colour and language the Desktop uses, and the
//! user's profile — so the page looks like the rest of the system without a Desktop around it.
//!
//! Control Panel reads them with its own identity: an App's session cannot read `users/{user}/desktop/*` or
//! `users/{user}/profile` itself (system-config gives an App only its own data). What the caller gets depends
//! on whom the session is for (`PreferenceTier`); the Desktop's own layout settings and account security are
//! never part of the answer.

use crate::user_mgr::{load_user_profile, merge_profile_values, profile_value_from_doc, public_profile_value};
use crate::{ControlPanelServer, RpcAuthPrincipal};
use ::kRPC::{RPCErrors, RPCRequest, RPCResponse, RPCResult};
use buckyos_api::{get_buckyos_api_runtime, AuthTarget, SystemConfigError, TokenPrincipalKind};
use serde_json::{json, Map, Value};

/// The account's appearance: the desktop state every device of the user shares (Desktop
/// `shared/appearance/model.ts`, session `default`, state key `appearance`).
fn appearance_key(user_id: &str) -> String {
    format!("users/{user_id}/desktop/default/appearance")
}

/// What a session may read of its user's settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PreferenceTier {
    /// Any session of the account, an App's included: the appearance and the public profile.
    Basic,
    /// A page of a system service (`system:*`: AI Workspace, the Desktop): the private profile as well.
    System,
}

impl PreferenceTier {
    pub(crate) fn of(principal: &RpcAuthPrincipal) -> Self {
        match AuthTarget::from_canonical_key(&principal.authenticated_app_id) {
            Ok(AuthTarget::System { .. }) => Self::System,
            _ => Self::Basic,
        }
    }
}

/// What `tier` sees of the stored settings. A field unset, invalid or not open to the tier is left out;
/// `accent: null` is the Zone's default colour.
pub(crate) fn preferences_view(
    tier: PreferenceTier,
    appearance: Option<&Value>,
    public_profile: Value,
    private_profile: Option<Value>,
) -> Value {
    let mut view = Map::new();
    if let Some(Value::Object(stored)) = appearance {
        if let Some(theme) = stored.get("theme").and_then(Value::as_str).filter(|theme| matches!(*theme, "light" | "dark")) {
            view.insert("theme".into(), json!(theme));
        }
        match stored.get("accent") {
            Some(Value::Null) => {
                view.insert("accent".into(), Value::Null);
            }
            Some(Value::String(accent)) if is_hex_colour(accent) => {
                view.insert("accent".into(), json!(accent.to_ascii_lowercase()));
            }
            _ => {}
        }
        if let Some(locale) = stored.get("locale").and_then(Value::as_str).filter(|locale| is_locale_tag(locale)) {
            view.insert("locale".into(), json!(locale));
        }
    }
    if public_profile.as_object().is_some_and(|profile| !profile.is_empty()) {
        view.insert("profile".into(), public_profile);
    }
    if tier == PreferenceTier::System {
        if let Some(profile) = private_profile {
            view.insert("private_profile".into(), profile);
        }
    }
    Value::Object(view)
}

fn is_hex_colour(value: &str) -> bool {
    value.len() == 7 && value.starts_with('#') && value[1..].chars().all(|c| c.is_ascii_hexdigit())
}

/// A BCP 47 tag as the Desktop stores it (`en`, `zh-CN`, `pt-BR`).
fn is_locale_tag(value: &str) -> bool {
    !value.is_empty() && value.len() <= 35 && value.split('-').all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric()))
}

impl ControlPanelServer {
    pub(crate) async fn handle_user_preferences_get(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        // An account's own settings: a device, a service or an Agent speaking for itself has none to show.
        if principal.principal_kind != TokenPrincipalKind::User || principal.session.is_external {
            return Ok(RPCResponse::new(RPCResult::Success(json!({})), req.seq));
        }
        let user_id = principal.username.as_str();
        let tier = PreferenceTier::of(principal);

        let runtime = get_buckyos_api_runtime()?;
        let client = runtime.get_system_config_client().await?;
        let appearance = match client.get(&appearance_key(user_id)).await {
            Ok(value) => serde_json::from_str::<Value>(&value.value).ok(),
            Err(SystemConfigError::KeyNotFound(_)) => None,
            Err(error) => return Err(RPCErrors::ReasonError(format!("read appearance: {error}"))),
        };
        let private_profile = load_user_profile(&client, user_id).await?;
        let did_profile = client
            .get(&format!("users/{user_id}/doc"))
            .await
            .ok()
            .and_then(|doc| serde_json::from_str::<Value>(&doc.value).ok())
            .as_ref()
            .and_then(profile_value_from_doc);
        let public_profile = merge_profile_values(private_profile.as_ref().map(public_profile_value), did_profile);
        let private_profile = private_profile.and_then(|profile| serde_json::to_value(profile).ok());

        Ok(RPCResponse::new(
            RPCResult::Success(preferences_view(tier, appearance.as_ref(), public_profile, private_profile)),
            req.seq,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stored() -> Value {
        json!({
            "theme": "dark",
            "locale": "zh-CN",
            "accent": "#0F7F8C",
            "iconSize": "large",
            "window": { "titleBarOpacity": 80, "backgroundOpacity": 90 },
            "wallpaper": { "kind": "picture", "ref": "obj://bucket/x.jpg" },
            "mobileFeed": "activity",
        })
    }

    #[test]
    fn an_app_sees_the_appearance_and_the_public_profile_only() {
        let view = preferences_view(
            PreferenceTier::Basic,
            Some(&stored()),
            json!({ "name": "Alice" }),
            Some(json!({ "name": "Alice", "birthday": "1990-01-01" })),
        );
        assert_eq!(
            view,
            json!({ "theme": "dark", "locale": "zh-CN", "accent": "#0f7f8c", "profile": { "name": "Alice" } })
        );
    }

    #[test]
    fn a_system_page_also_sees_the_private_profile() {
        let view = preferences_view(
            PreferenceTier::System,
            Some(&stored()),
            json!({ "name": "Alice" }),
            Some(json!({ "name": "Alice", "birthday": "1990-01-01" })),
        );
        assert_eq!(view["private_profile"], json!({ "name": "Alice", "birthday": "1990-01-01" }));
        // the Desktop's own layout is nobody else's business, whoever asks
        for field in ["iconSize", "window", "wallpaper", "mobileFeed"] {
            assert!(view.get(field).is_none(), "{field}");
        }
    }

    #[test]
    fn unset_or_invalid_fields_are_left_out() {
        let view = preferences_view(
            PreferenceTier::Basic,
            Some(&json!({ "theme": "sepia", "accent": "blue", "locale": "en US" })),
            json!({}),
            None,
        );
        assert_eq!(view, json!({}));
        assert_eq!(preferences_view(PreferenceTier::System, None, json!({}), None), json!({}));
        // null is a choice: the Zone's default colour
        let view = preferences_view(PreferenceTier::Basic, Some(&json!({ "accent": null })), json!({}), None);
        assert_eq!(view, json!({ "accent": null }));
    }

    #[test]
    fn the_tier_follows_whom_the_session_is_for() {
        let mut principal = RpcAuthPrincipal {
            username: "alice".into(),
            owner_user_id: "alice".into(),
            authenticated_app_id: "app:crm@alice".into(),
            user_type: buckyos_api::UserType::User,
            owner_did: String::new(),
            is_user_session: true,
            is_control_panel_session: false,
            principal_kind: TokenPrincipalKind::User,
            agent_permission_mode: None,
            session: crate::PrincipalSession::default(),
        };
        assert_eq!(PreferenceTier::of(&principal), PreferenceTier::Basic);
        principal.authenticated_app_id = "system:aiworkspace".into();
        assert_eq!(PreferenceTier::of(&principal), PreferenceTier::System);
        principal.authenticated_app_id = "system:control-panel".into();
        assert_eq!(PreferenceTier::of(&principal), PreferenceTier::System);
        principal.authenticated_app_id = "not a target".into();
        assert_eq!(PreferenceTier::of(&principal), PreferenceTier::Basic);
    }
}
