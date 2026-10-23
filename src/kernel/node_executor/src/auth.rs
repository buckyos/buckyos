//! Authorization snapshot holder and stream login (doc §5.6, §7.2).

use std::collections::BTreeSet;
use std::sync::RwLock;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use buckyos_api::{
    agent_executor_resource, node_executor_resource, AppInstanceId, ExecutorConfig, ExecutorError,
    ExecutorErrorCode, ExecutorOp, ExecutorPrincipalBinding, ExecutorProfile, TokenPrincipalKind,
    TokenUse, NODE_EXECUTOR_LEASE_MS,
};

use crate::store::Actor;

/// Claims of a verify-hub session token that passed signature, issuer and
/// expiry checks.
#[derive(Debug, Clone)]
pub struct VerifiedSession {
    pub subject: String,
    pub principal_kind: TokenPrincipalKind,
    pub token_use: TokenUse,
    pub app_instance_id: Option<AppInstanceId>,
    /// `target.authorization_key()` of the token; RBAC is checked with it.
    pub authorization_key: String,
    pub expires_at_ms: u64,
}

#[async_trait]
pub trait TokenVerifier: Send + Sync {
    async fn verify(&self, token: &str) -> Result<VerifiedSession, String>;
}

#[async_trait]
pub trait RbacChecker: Send + Sync {
    async fn allowed(&self, user_id: &str, app_key: &str, resource: &str, action: &str) -> bool;
}

struct Snapshot {
    config: ExecutorConfig,
    valid_until: Instant,
    valid_until_suspend_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyResult {
    /// A newer generation replaced the snapshot (or the first one arrived).
    Replaced,
    /// Same generation/revision, later deadline.
    Renewed,
    Ignored,
}

#[derive(Default)]
pub struct AuthState {
    snapshot: RwLock<Option<Snapshot>>,
}

impl AuthState {
    /// Remaining validity comes from the deadline as seen now, capped at one
    /// lease, and is then tracked on the monotonic clock.
    pub fn apply(&self, config: ExecutorConfig, now_wall_ms: u64, admission: crate::ExecutorAdmission) -> ApplyResult {
        let valid = match admission {
            crate::ExecutorAdmission::Node => config.validate(),
            crate::ExecutorAdmission::Wss => config.validate_execution(),
        };
        if valid.is_err() {
            return ApplyResult::Ignored;
        }
        let remaining = config
            .auth_valid_until_ms
            .saturating_sub(now_wall_ms)
            .min(NODE_EXECUTOR_LEASE_MS);
        let valid_until = Instant::now() + Duration::from_millis(remaining);
        let valid_until_suspend_ms = suspend_clock_ms().saturating_add(remaining);
        let mut guard = self.snapshot.write().unwrap();
        match guard.as_mut() {
            Some(current) if config.generation < current.config.generation => ApplyResult::Ignored,
            Some(current) if config.generation == current.config.generation => {
                if config.revision != current.config.revision
                    || config.auth_valid_until_ms <= current.config.auth_valid_until_ms
                {
                    return ApplyResult::Ignored;
                }
                current.config.auth_valid_until_ms = config.auth_valid_until_ms;
                if valid_until > current.valid_until {
                    current.valid_until = valid_until;
                    current.valid_until_suspend_ms = valid_until_suspend_ms;
                }
                ApplyResult::Renewed
            }
            _ => {
                *guard = Some(Snapshot {
                    config,
                    valid_until,
                    valid_until_suspend_ms,
                });
                ApplyResult::Replaced
            }
        }
    }

    /// The node config no longer carries an executor section.
    pub fn clear(&self) -> bool {
        self.snapshot.write().unwrap().take().is_some()
    }

    /// The snapshot, only while it is enabled and inside its lease.
    pub fn current(&self) -> Option<ExecutorConfig> {
        let guard = self.snapshot.read().unwrap();
        guard
            .as_ref()
            .filter(|s| s.config.is_open()
                && crate::now_ms() < s.config.auth_valid_until_ms
                && Instant::now() < s.valid_until
                && suspend_clock_ms() < s.valid_until_suspend_ms)
            .map(|s| s.config.clone())
    }

    pub fn has_snapshot(&self) -> bool {
        self.snapshot.read().unwrap().is_some()
    }
}

fn suspend_clock_ms() -> u64 {
    #[cfg(target_os = "linux")]
    unsafe {
        let mut value: libc::timespec = std::mem::zeroed();
        if libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut value) == 0 {
            return (value.tv_sec as u64).saturating_mul(1000)
                .saturating_add(value.tv_nsec as u64 / 1_000_000);
        }
    }
    crate::now_ms()
}

/// What a logged-in stream may do; recomputed when the snapshot changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    pub actor: Actor,
    pub profile_id: String,
    pub profile: ExecutorProfile,
    pub binding: ExecutorPrincipalBinding,
    pub caps: BTreeSet<ExecutorOp>,
}

impl Grant {
    /// The parts of the grant whose change revokes rather than narrows.
    pub fn same_target(&self, other: &Grant) -> bool {
        self.actor == other.actor
            && self.profile.run_as == other.profile.run_as
            && self.profile.workdir_roots == other.profile.workdir_roots
            && self.profile.env == other.profile.env
            && self.binding.app_instance_id == other.binding.app_instance_id
    }
}

fn denied(code: ExecutorErrorCode, msg: &str) -> ExecutorError {
    ExecutorError::new(code, msg)
}

/// Strict App → Agent mapping of §7.2 against one snapshot. RBAC is checked
/// by the caller with `rbac_ops`.
pub fn map_login(
    session: &VerifiedSession,
    config: &ExecutorConfig,
    profile_id: &str,
    host_caps: &BTreeSet<ExecutorOp>,
    identity_mode: crate::ExecutorIdentityMode,
) -> Result<Grant, ExecutorError> {
    if session.token_use != TokenUse::Session || session.principal_kind != TokenPrincipalKind::App {
        return Err(denied(
            ExecutorErrorCode::Unauthenticated,
            "only App session tokens can log in",
        ));
    }
    let app_instance = session.app_instance_id.as_ref().ok_or_else(|| {
        denied(ExecutorErrorCode::Unauthenticated, "token target is not an App instance")
    })?;
    if session.subject != app_instance.owner_user_id() {
        return Err(denied(
            ExecutorErrorCode::Unauthenticated,
            "token subject is not the App instance owner",
        ));
    }
    let mut bound = config
        .principal_bindings
        .iter()
        .filter(|(_, b)| &b.app_instance_id == app_instance);
    let (agent_did, binding) = match (bound.next(), bound.next()) {
        (Some(found), None) => found,
        (None, _) => {
            return Err(denied(
                ExecutorErrorCode::Forbidden,
                "the App instance is not bound to an authorized Agent",
            ))
        }
        _ => {
            return Err(denied(
                ExecutorErrorCode::Forbidden,
                "the App instance is shared by several Agents",
            ))
        }
    };
    if app_instance.app_id().app_did().to_string() != *agent_did {
        return Err(denied(
            ExecutorErrorCode::Forbidden,
            "the Agent DID differs from the App DID",
        ));
    }
    if !binding.enabled || !binding.app_installed {
        return Err(denied(ExecutorErrorCode::Forbidden, "the Agent is disabled"));
    }
    let profile = config
        .profiles
        .get(profile_id)
        .ok_or_else(|| denied(ExecutorErrorCode::Forbidden, "unknown executor profile"))?;
    if !profile.principals.iter().any(|p| p == agent_did) {
        return Err(denied(
            ExecutorErrorCode::Forbidden,
            "the Agent is not a principal of this profile",
        ));
    }
    let fits_host = identity_mode.honors_run_as(profile);
    let caps: BTreeSet<ExecutorOp> = config
        .granted_ops(profile_id, agent_did)
        .intersection(host_caps)
        .copied()
        .filter(|op| fits_host || crate::CONTROL_OPS.contains(op))
        .collect();
    if !caps.contains(&ExecutorOp::OpenStream) {
        return Err(denied(ExecutorErrorCode::Forbidden, "open_stream is not granted"));
    }
    Ok(Grant {
        actor: Actor {
            agent_did: agent_did.clone(),
            app_instance_id: app_instance.to_string(),
        },
        profile_id: profile_id.to_string(),
        profile: profile.clone(),
        binding: binding.clone(),
        caps,
    })
}

/// Narrow `grant.caps` by RBAC on `obj://node/<node>/executor/<profile>`.
pub async fn apply_rbac(
    rbac: &dyn RbacChecker,
    node_id: &str,
    admission: crate::ExecutorAdmission,
    session: &VerifiedSession,
    grant: &mut Grant,
) {
    let resource = match admission {
        crate::ExecutorAdmission::Node => node_executor_resource(node_id, &grant.profile_id),
        crate::ExecutorAdmission::Wss => agent_executor_resource(node_id, &grant.profile_id),
    };
    let mut allowed = BTreeSet::new();
    for op in grant.caps.iter() {
        if rbac
            .allowed(
                &session.subject,
                &session.authorization_key,
                &resource,
                &op.rbac_action(),
            )
            .await
        {
            allowed.insert(*op);
        }
    }
    grant.caps = allowed;
}

