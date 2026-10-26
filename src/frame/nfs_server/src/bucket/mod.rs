//! Loader of the application object buckets (doc/arch/应用对象桶 Bucket 设计.md
//! §14.1). The model lives in cyfs-ndn `bucket_store`; this module opens the
//! `bucket-index` instance, turns verified sessions into callers, answers the
//! identity policy (RBAC, first layer), creates declared buckets from app
//! specs, serves kRPC `bucket.*` (`rpc`) and the data plane (`http`), and runs
//! the GC host and maintenance loops (`tasks`).

pub mod http;
pub mod rpc;
pub mod s3;
pub mod spec;
pub mod tasks;

use crate::auth::Credentials;
use crate::config::AuthMode;
use crate::ndm;
use crate::state::AppState;
use bucket_store::{Action, BucketError, BucketEvent, BucketInfo, BucketResult, BucketScope, BucketStore, Caller, EventSink, IdentityPolicy, IndexDb};
use buckyos_api::TokenPrincipalKind;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const CACHE_TTL: Duration = Duration::from_secs(60);

pub struct BucketHost {
    store: tokio::sync::OnceCell<Arc<BucketStore>>,
    /// Last spec sync per `<appid>@<owner>`.
    pub(crate) synced: Mutex<HashMap<String, Instant>>,
    groups: Mutex<HashMap<String, (Instant, Vec<String>, bool)>>,
}

impl Default for BucketHost {
    fn default() -> Self {
        BucketHost { store: tokio::sync::OnceCell::new(), synced: Default::default(), groups: Default::default() }
    }
}

fn index_path_from_connection(connection: &str) -> Option<PathBuf> {
    let body = connection.strip_prefix("sqlite://").or_else(|| connection.strip_prefix("sqlite:"))?;
    let path = body.split('?').next()?;
    (!path.is_empty() && path != ":memory:").then(|| PathBuf::from(path))
}

async fn index_path(state: &AppState) -> BucketResult<PathBuf> {
    match state.config.auth {
        AuthMode::Runtime => {
            let instance = buckyos_api::get_rdb_instance(
                buckyos_api::NFS_SERVER_SERVICE_NAME,
                None,
                buckyos_api::BUCKET_INDEX_RDB_INSTANCE_ID,
            )
            .await
            .map_err(|e| BucketError::Internal(format!("bucket-index instance: {e}")))?;
            index_path_from_connection(&instance.connection)
                .ok_or_else(|| BucketError::Internal(format!("bucket-index is not a sqlite file: {}", instance.connection)))
        }
        _ => Ok(state.config.data_dir.join("bucket-index.db")),
    }
}

struct KEventSink;

impl EventSink for KEventSink {
    fn emit(&self, event: BucketEvent) {
        let id = match &event {
            BucketEvent::Object { bucket_id, .. } => buckyos_api::bucket_event_id(bucket_id),
            BucketEvent::QuotaWarning { bucket_id: Some(b), .. } => buckyos_api::bucket_event_id(b),
            BucketEvent::QuotaWarning { pool_kind, pool_id, .. } => {
                let pool = pool_id.clone().unwrap_or_default().replace('@', "/");
                format!("/bucket-quota/{}/{}", pool_kind.map(|k| k.as_str()).unwrap_or("pool"), pool)
            }
        };
        let Ok(data) = serde_json::to_value(&event) else { return };
        tokio::spawn(async move {
            let Ok(runtime) = buckyos_api::get_buckyos_api_runtime() else { return };
            match runtime.get_kevent_client().await {
                Ok(client) => {
                    if let Err(e) = client.pub_event(&id, data).await {
                        log::debug!("bucket event {id}: {e:?}");
                    }
                }
                Err(e) => log::debug!("bucket event {id}: no kevent client: {e:?}"),
            }
        });
    }
}

impl BucketHost {
    pub async fn store(&self, state: &AppState) -> BucketResult<&Arc<BucketStore>> {
        self.store
            .get_or_try_init(|| async {
                let ndm = state
                    .share
                    .store(&state.config)
                    .await
                    .map_err(|e| BucketError::Store(e.message))?
                    .clone();
                let path = index_path(state).await?;
                log::info!("bucket index at {}", path.display());
                let store = BucketStore::new(IndexDb::open(&path)?, ndm);
                if matches!(state.config.auth, AuthMode::Runtime) {
                    store.set_event_sink(Arc::new(KEventSink));
                }
                Ok(store)
            })
            .await
    }

    /// `(groups, zone user)` of a user, from `users/<u>/settings` (cached).
    pub(crate) async fn user_groups(&self, state: &AppState, user: &str) -> (Vec<String>, bool) {
        if let Some((at, groups, zone_user)) = self.groups.lock().unwrap().get(user).cloned() {
            if at.elapsed() < CACHE_TTL {
                return (groups, zone_user);
            }
        }
        let found = match &state.config.auth {
            AuthMode::Runtime => load_groups(user).await,
            _ => Some((vec!["users".to_string()], true)),
        };
        let (groups, zone_user) = found.unwrap_or_default();
        self.groups.lock().unwrap().insert(user.to_string(), (Instant::now(), groups.clone(), zone_user));
        (groups, zone_user)
    }
}

async fn load_groups(user: &str) -> Option<(Vec<String>, bool)> {
    let client = buckyos_api::get_buckyos_api_runtime().ok()?.get_system_config_client().await.ok()?;
    let raw = client.get(&format!("users/{user}/settings")).await.ok()?;
    let settings: serde_json::Value = serde_json::from_str(&raw.value).ok()?;
    let kind = settings["user_type"].as_str().unwrap_or("user").to_ascii_lowercase();
    let mut groups = vec![match kind.as_str() {
        "user" => "users".to_string(),
        other => other.to_string(),
    }];
    if let Some(extra) = settings["groups"].as_array() {
        groups.extend(extra.iter().filter_map(|g| g.as_str().map(String::from)));
    }
    Some((groups, kind != "limited"))
}

/// The verified session behind a request, before any bucket is known.
#[derive(Clone)]
pub enum Session {
    /// Standalone without authentication: full access.
    Open,
    /// A user, possibly through an app (`app_key` = `app:<id>` or `system:<service>`).
    User { user: String, app_key: String, app: String, token: String, sudo: bool, app_session: bool },
    /// An Agent (DID session) acting for its owner.
    Agent { owner: String, agent: String, subject: String, app: String, token: String, read_write: bool },
    /// A zone service or device acting as itself.
    Service { service: String, subject: String, app: String, token: String },
    Anonymous,
}

fn app_name(app_key: &str) -> String {
    app_key.split_once(':').map(|(_, v)| v).unwrap_or(app_key).to_string()
}

async fn agent_read_write(user: &str, agent: &str) -> bool {
    let Ok(runtime) = buckyos_api::get_buckyos_api_runtime() else { return false };
    let Ok(client) = runtime.get_system_config_client().await else { return false };
    let Ok(agent_id) = buckyos_api::AgentId::parse(agent) else { return false };
    let Ok(value) = client.get(&buckyos_api::agent_spec_key(user, &agent_id)).await else { return false };
    serde_json::from_str::<buckyos_api::AgentSpec>(&value.value)
        .map(|spec| spec.permission_mode == buckyos_api::AgentPermissionMode::ReadWrite)
        .unwrap_or(false)
}

/// Verifies a verify-hub session token; `None` is an anonymous request.
pub async fn session_of(state: &AppState, token: Option<&str>) -> BucketResult<Session> {
    let test_key = match &state.config.auth {
        AuthMode::Disabled => return Ok(Session::Open),
        AuthMode::TestKey { public_key, .. } => Some(public_key.clone()),
        AuthMode::Runtime => None,
    };
    let Some(token) = token else { return Ok(Session::Anonymous) };
    let claims = buckyos_api::verify_nfs_session_token(token, test_key.as_deref())
        .await
        .map_err(|e| BucketError::PermissionDenied(format!("invalid session token: {e}")))?;
    Ok(match claims.principal_kind {
        TokenPrincipalKind::User | TokenPrincipalKind::App => Session::User {
            user: claims.subject,
            app: claims.app_id,
            app_key: claims.authorization_key,
            token: token.to_string(),
            sudo: claims.sudo,
            app_session: claims.principal_kind == TokenPrincipalKind::App,
        },
        TokenPrincipalKind::Agent => {
            let (owner, agent) = crate::auth::agent_owner(&claims.subject)
                .await
                .ok_or_else(|| BucketError::PermissionDenied(format!("{} is no agent of this zone", claims.subject)))?;
            let read_write = agent_read_write(&owner, &agent).await;
            Session::Agent { owner, agent, subject: claims.subject, app: claims.app_id, token: token.to_string(), read_write }
        }
        TokenPrincipalKind::System | TokenPrincipalKind::Device => Session::Service {
            service: app_name(&claims.authorization_key),
            subject: claims.subject,
            app: claims.app_id,
            token: token.to_string(),
        },
    })
}

/// Session from HTTP credentials: an explicit Bearer must verify; stale cookies fall back to anonymous.
pub async fn session_from_headers(state: &AppState, creds: &Credentials) -> BucketResult<Session> {
    if let Some(bearer) = creds.bearer().filter(|b| !b.starts_with("bkt_")) {
        return session_of(state, Some(bearer)).await;
    }
    for token in creds.cookie_tokens() {
        if let Ok(s) = session_of(state, Some(token)).await {
            if !matches!(s, Session::Anonymous) {
                return Ok(s);
            }
        }
    }
    session_of(state, None).await
}

/// RBAC (first layer) for one request, cached per resource and action.
struct RbacPolicy {
    state: Arc<AppState>,
    user: String,
    app_key: String,
    app: String,
    token: String,
    sudo: bool,
    cache: Mutex<HashMap<(String, Action), bool>>,
}

impl RbacPolicy {
    async fn enforce(&self, resource: &str, action: &str) -> bool {
        match &self.state.config.auth {
            AuthMode::Disabled => true,
            AuthMode::TestKey { .. } => {
                let sudo = self.sudo.then(|| rbac::SudoMode::Sudo(format!("su_{}", self.user)));
                rbac::enforce(&self.user, &self.app_key, resource, action, sudo).await
            }
            AuthMode::Runtime => {
                let Ok(runtime) = buckyos_api::get_buckyos_api_runtime() else { return false };
                let mut req = kRPC::RPCRequest::new("bucket.authorize", serde_json::Value::Null);
                req.token = Some(self.token.clone());
                runtime.enforce(&req, action, resource).await.is_ok()
            }
        }
    }

    fn ndm_principal(&self) -> ndm::Principal {
        ndm::Principal::new(self.user.clone(), self.app_key.clone(), self.app.clone(), Some(self.token.clone()))
    }
}

#[async_trait::async_trait]
impl IdentityPolicy for RbacPolicy {
    async fn allowed(&self, bucket: &BucketInfo, key: &str, action: Action) -> bool {
        let resource = bucket.bucket_id.resource(key);
        let cache_key = (resource.clone(), action);
        if let Some(v) = self.cache.lock().unwrap().get(&cache_key) {
            return *v;
        }
        let allowed = self.enforce(&resource, action.as_str()).await;
        self.cache.lock().unwrap().insert(cache_key, allowed);
        allowed
    }

    async fn holds(&self, obj_id: &str) -> bool {
        let p = self.ndm_principal();
        ndm::holds(&self.state, &p, obj_id) || ndm::allowed(&self.state, &p, ndm::RES_OBJECTS, "read").await
    }
}

/// An Agent works in its owner's user buckets (§4.4.3): list/read, plus create/write in read-write mode.
struct AgentPolicy {
    state: Arc<AppState>,
    owner: String,
    read_write: bool,
    principal: ndm::Principal,
}

#[async_trait::async_trait]
impl IdentityPolicy for AgentPolicy {
    async fn allowed(&self, bucket: &BucketInfo, _key: &str, action: Action) -> bool {
        bucket.scope == BucketScope::User
            && bucket.owner == self.owner
            && match action {
                Action::List | Action::Read => true,
                Action::Create | Action::Write => self.read_write,
                Action::Delete | Action::Manage => false,
            }
    }

    async fn holds(&self, obj_id: &str) -> bool {
        ndm::holds(&self.state, &self.principal, obj_id) || ndm::allowed(&self.state, &self.principal, ndm::RES_OBJECTS, "read").await
    }
}

/// A service has every action on the buckets of its own app id (zone and per-user).
struct ServicePolicy {
    state: Arc<AppState>,
    service: String,
    principal: ndm::Principal,
}

#[async_trait::async_trait]
impl IdentityPolicy for ServicePolicy {
    async fn allowed(&self, bucket: &BucketInfo, _key: &str, _action: Action) -> bool {
        bucket.appid == self.service
    }

    async fn holds(&self, obj_id: &str) -> bool {
        ndm::holds(&self.state, &self.principal, obj_id) || ndm::allowed(&self.state, &self.principal, ndm::RES_OBJECTS, "read").await
    }
}

/// The caller of one request: identities for grants, the identity policy, a capability token.
pub async fn caller_of(state: &Arc<AppState>, session: &Session, bucket_token: Option<String>) -> Caller {
    let mut caller = Caller::anonymous(bucket_token);
    match session {
        Session::Open => {
            caller.user = Some("local".into());
            caller.app = Some("local".into());
            caller.zone_user = true;
            caller.user_direct = true;
            caller.sudo = true;
            caller.identity = Arc::new(bucket_store::AllowAll);
        }
        Session::User { user, app_key, app, token, sudo, app_session } => {
            let (groups, zone_user) = state.buckets.user_groups(state, user).await;
            caller.user = Some(user.clone());
            caller.app = Some(app_name(app_key));
            caller.groups = groups;
            caller.zone_user = zone_user;
            caller.user_direct = !app_session && app_key.starts_with("system:");
            caller.sudo = *sudo;
            caller.identity = Arc::new(RbacPolicy {
                state: state.clone(),
                user: user.clone(),
                app_key: app_key.clone(),
                app: app.clone(),
                token: token.clone(),
                sudo: *sudo,
                cache: Default::default(),
            });
        }
        Session::Agent { owner, agent, subject, app, token, read_write } => {
            caller.user = Some(owner.clone());
            caller.agent = Some(agent.clone());
            caller.identity = Arc::new(AgentPolicy {
                state: state.clone(),
                owner: owner.clone(),
                read_write: *read_write,
                principal: ndm::Principal::new(subject.clone(), app.clone(), app.clone(), Some(token.clone())),
            });
        }
        Session::Service { service, subject, app, token } => {
            caller.user = Some(service.clone());
            caller.app = Some(service.clone());
            caller.identity = Arc::new(ServicePolicy {
                state: state.clone(),
                service: service.clone(),
                principal: ndm::Principal::new(subject.clone(), app.clone(), app.clone(), Some(token.clone())),
            });
        }
        Session::Anonymous => {}
    }
    caller
}

/// Zone owner, or a user whose admin policy manages other users' buckets (D16).
pub async fn is_admin(state: &Arc<AppState>, session: &Session) -> bool {
    match session {
        Session::Open => true,
        Session::User { user, app_key, app, token, sudo, .. } => {
            if crate::auth::is_zone_owner(state, user).await {
                return true;
            }
            let policy = RbacPolicy {
                state: state.clone(),
                user: user.clone(),
                app_key: app_key.clone(),
                app: app.clone(),
                token: token.clone(),
                sudo: *sudo,
                cache: Default::default(),
            };
            policy.enforce("obj://bucket/user/_/_/_/", "manage").await
        }
        _ => false,
    }
}

pub fn user_of(session: &Session) -> Option<&str> {
    match session {
        Session::Open => Some("local"),
        Session::User { user, .. } => Some(user),
        Session::Agent { owner, .. } => Some(owner),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqlite_connection_paths() {
        assert_eq!(index_path_from_connection("sqlite:/opt/buckyos/data/nfs-server/bucket-index.db?mode=rwc"), Some(PathBuf::from("/opt/buckyos/data/nfs-server/bucket-index.db")));
        assert_eq!(index_path_from_connection("sqlite:///x/y.db"), Some(PathBuf::from("/x/y.db")));
        assert_eq!(index_path_from_connection("sqlite::memory:"), None);
        assert_eq!(index_path_from_connection("postgres://x"), None);
    }
}
