//! Which Apps may notify a user (`doc/message_hub/System Notify.md` §4): an
//! App declares [`APP_PERMISSION_NOTIFY`] in its AppDoc, and the user turns
//! Apps off one by one. Without a choice of the user's the App is as the
//! installation approved it.
//!
//! The choices are owner-scoped UI state under a name of its own, next to
//! the default conversations.

use crate::msg_center::MessageCenter;
use crate::notify::NotifyCaller;
use buckyos_api::*;
use kRPC::{RPCErrorDetail, RPCErrors};
use name_lib::DID;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;

const NOTIFY_APPS_STATE: &str = "notify-apps";

type Result<T> = std::result::Result<T, RPCErrors>;

/// What an installation says about notifications.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledApp {
    pub app_instance_id: AppInstanceId,
    pub show_name: String,
    pub app_icon_url: Option<String>,
    /// The AppDoc declares the permission.
    pub declared: bool,
    /// The installation approved it.
    pub approved: bool,
}

impl InstalledApp {
    pub fn from_spec(spec: &AppServiceSpec) -> Self {
        let now = buckyos_kit::buckyos_get_unix_timestamp();
        Self {
            app_instance_id: spec.app_instance_id.clone(),
            show_name: spec.app_doc.show_name.clone(),
            app_icon_url: spec.app_doc.app_icon_url().map(str::to_string),
            declared: has_permission(&spec.app_doc.permissions, APP_PERMISSION_NOTIFY, now),
            approved: has_permission(&spec.permission, APP_PERMISSION_NOTIFY, now),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Switch {
    enabled: bool,
}

fn detailed(code: &str, message: String, app: &AppInstanceId) -> RPCErrors {
    RPCErrors::Detailed(RPCErrorDetail::new(
        code,
        message,
        Some(json!({ "app_instance_id": app })),
    ))
}

impl MessageCenter {
    async fn notify_switch(&self, owner: &DID, app: &AppInstanceId) -> Result<Option<bool>> {
        let entry = self
            .msg_box_db
            .get_owner_ui_session_state(owner, NOTIFY_APPS_STATE, &app.to_string())
            .await?;
        Ok(entry
            .and_then(|e| serde_json::from_value::<Switch>(e.value).ok())
            .map(|s| s.enabled))
    }

    async fn notify_app_enabled(&self, owner: &DID, app: &InstalledApp) -> Result<bool> {
        if !app.declared {
            return Ok(false);
        }
        Ok(self
            .notify_switch(owner, &app.app_instance_id)
            .await?
            .unwrap_or(app.approved))
    }

    /// Only Apps are checked: system services, the pages of the system and
    /// Agents are not (`NotifyCaller::app_instance` is an App that is no
    /// Agent).
    pub(crate) async fn check_app_may_notify(&self, caller: &NotifyCaller) -> Result<()> {
        let (Some(id), Some(owner)) = (caller.app_instance.as_ref(), caller.user_did.as_ref())
        else {
            return Ok(());
        };
        let app = self.token_verifier.get().installed_app(id).await?;
        let Some(app) = app.filter(|app| app.declared) else {
            return Err(detailed(
                NOTIFY_ERR_NOT_DECLARED,
                format!("{id} does not declare the permission `{APP_PERMISSION_NOTIFY}`"),
                id,
            ));
        };
        if !self.notify_app_enabled(owner, &app).await? {
            return Err(detailed(
                NOTIFY_ERR_DISABLED,
                format!("the user turned off the notifications of {id}"),
                id,
            ));
        }
        Ok(())
    }

    /// Whose settings a call reads or changes: the user's own in the pages
    /// of the system, anyone's for a system service.
    pub(crate) async fn notify_settings_user(
        &self,
        caller: &NotifyCaller,
        to_user: Option<&str>,
    ) -> Result<(String, DID)> {
        let to_user = to_user.map(str::trim).filter(|u| !u.is_empty());
        if caller.system {
            let user = to_user
                .ok_or_else(|| RPCErrors::ParseRequestError("to_user is required".into()))?;
            let did = self
                .token_verifier
                .get()
                .resolve_user_did(user)
                .await
                .map_err(|e| RPCErrors::ParseRequestError(format!("unknown user {user}: {e}")))?;
            return Ok((user.to_string(), did));
        }
        match (caller.user_id.as_ref(), caller.user_did.as_ref()) {
            (Some(user), Some(did))
                if caller.system_page && to_user.is_none_or(|u| u == user.as_str()) =>
            {
                Ok((user.clone(), did.clone()))
            }
            _ => Err(RPCErrors::NoPermission(
                "notification settings are the user's own, in the pages of the system".into(),
            )),
        }
    }

    async fn notify_app_view(
        &self,
        owner: &DID,
        id: &AppInstanceId,
        installed: Option<&InstalledApp>,
        last_sent_at_ms: Option<u64>,
    ) -> Result<NotifyApp> {
        Ok(NotifyApp {
            app_instance_id: id.clone(),
            app_id: id.app_id().clone(),
            owner_user_id: id.owner_user_id().to_string(),
            show_name: installed.map(|app| app.show_name.clone()),
            app_icon_url: installed.and_then(|app| app.app_icon_url.clone()),
            installed: installed.is_some(),
            declared: installed.is_some_and(|app| app.declared),
            enabled: match installed {
                Some(app) => self.notify_app_enabled(owner, app).await?,
                None => false,
            },
            last_sent_at_ms,
        })
    }

    /// The Apps that sent `owner` something, with when they last did.
    async fn notify_senders(&self, owner: &DID) -> Result<BTreeMap<AppInstanceId, u64>> {
        let mut out: BTreeMap<AppInstanceId, u64> = BTreeMap::new();
        for (requester, at) in self.notifies.requesters(owner).await? {
            let Some((_, target)) = requester.split_once('|') else {
                continue;
            };
            let Ok(AuthTarget::App { app_instance_id }) = AuthTarget::from_canonical_key(target)
            else {
                continue;
            };
            if self
                .agent_of_app(app_instance_id.app_id().as_str(), None)
                .await
                .is_some()
            {
                continue;
            }
            let last = out.entry(app_instance_id).or_default();
            *last = (*last).max(at);
        }
        Ok(out)
    }

    pub(crate) async fn notify_list_apps(
        &self,
        caller: &NotifyCaller,
        req: NotifyListAppsReq,
    ) -> Result<NotifyListAppsResult> {
        let (user, owner) = self
            .notify_settings_user(caller, req.to_user.as_deref())
            .await?;
        let verifier = self.token_verifier.get();
        let mut senders = self.notify_senders(&owner).await?;
        let mut apps = Vec::new();
        for app in verifier.installed_apps(&user).await? {
            let last = senders.remove(&app.app_instance_id);
            if !app.declared && last.is_none() {
                continue;
            }
            let id = app.app_instance_id.clone();
            apps.push(self.notify_app_view(&owner, &id, Some(&app), last).await?);
        }
        for (id, last) in senders {
            let installed = verifier.installed_app(&id).await?;
            apps.push(
                self.notify_app_view(&owner, &id, installed.as_ref(), Some(last))
                    .await?,
            );
        }
        apps.sort_by_key(|app| {
            (
                app.show_name
                    .clone()
                    .unwrap_or_else(|| app.app_id.to_string())
                    .to_lowercase(),
                app.app_instance_id.to_string(),
            )
        });
        Ok(NotifyListAppsResult { apps })
    }

    pub(crate) async fn notify_set_app(
        &self,
        caller: &NotifyCaller,
        req: NotifySetAppReq,
    ) -> Result<NotifyApp> {
        let (_, owner) = self
            .notify_settings_user(caller, req.to_user.as_deref())
            .await?;
        let id = req.app_instance_id;
        let app = self
            .token_verifier
            .get()
            .installed_app(&id)
            .await?
            .filter(|app| app.declared)
            .ok_or_else(|| {
                detailed(
                    NOTIFY_ERR_NOT_DECLARED,
                    format!("{id} does not declare the permission `{APP_PERMISSION_NOTIFY}`"),
                    &id,
                )
            })?;
        self.msg_box_db
            .upsert_owner_ui_session_state(
                &owner,
                NOTIFY_APPS_STATE,
                &id.to_string(),
                &json!(Switch {
                    enabled: req.enabled
                }),
                Self::now_ms(),
            )
            .await?;
        let last = self.notify_senders(&owner).await?.get(&id).copied();
        self.notify_app_view(&owner, &id, Some(&app), last).await
    }
}
