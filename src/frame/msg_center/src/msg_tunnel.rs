use anyhow::{bail, Result as AnyResult};
use async_trait::async_trait;
use buckyos_api::{DeliveryReportResult, MsgRecordWithObject};
use name_lib::DID;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use thiserror::Error;

#[async_trait]
pub trait MsgTunnel: Send + Sync {
    fn tunnel_did(&self) -> DID;
    fn name(&self) -> &str;
    fn platform(&self) -> &str;

    fn supports_ingress(&self) -> bool {
        true
    }

    fn supports_egress(&self) -> bool {
        true
    }

    async fn start(&self) -> AnyResult<()>;
    async fn stop(&self) -> AnyResult<()>;

    async fn send_record(&self, record: MsgRecordWithObject) -> AnyResult<DeliveryReportResult> {
        let _ = record;
        bail!(
            "tunnel {} does not implement send_record",
            self.tunnel_did().to_string()
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MsgTunnelInstanceState {
    Registered,
    Starting,
    Running,
    Stopping,
    Stopped,
    Faulted,
}

#[derive(Debug, Clone)]
pub struct MsgTunnelInstanceInfo {
    pub tunnel_did: DID,
    pub name: String,
    pub platform: String,
    pub supports_ingress: bool,
    pub supports_egress: bool,
    pub state: MsgTunnelInstanceState,
    pub registered_at_ms: u64,
    pub updated_at_ms: u64,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct MsgTunnelOpReport {
    pub tunnel_did: DID,
    pub ok: bool,
    pub state: Option<MsgTunnelInstanceState>,
    pub error: Option<String>,
}

impl MsgTunnelOpReport {
    fn success(info: MsgTunnelInstanceInfo) -> Self {
        Self {
            tunnel_did: info.tunnel_did,
            ok: true,
            state: Some(info.state),
            error: None,
        }
    }

    fn failed(tunnel_did: DID, error: MsgTunnelMgrError) -> Self {
        Self {
            tunnel_did,
            ok: false,
            state: None,
            error: Some(error.to_string()),
        }
    }
}

#[derive(Debug, Error)]
pub enum MsgTunnelMgrError {
    #[error("msg tunnel instance lock poisoned")]
    LockPoisoned,
    #[error("msg tunnel {0} already registered")]
    AlreadyRegistered(String),
    #[error("msg tunnel {0} not found")]
    NotFound(String),
    #[error("msg tunnel {0} is not running")]
    NotRunning(String),
    #[error("msg tunnel {0} does not support egress send")]
    EgressNotSupported(String),
    #[error("record {0} has no route.tunnel_did")]
    MissingRouteTunnelDid(String),
    #[error("msg tunnel {tunnel} cannot {op} from state {state:?}")]
    InvalidStateTransition {
        tunnel: String,
        op: &'static str,
        state: MsgTunnelInstanceState,
    },
    #[error("msg tunnel {tunnel} {op} failed: {error}")]
    OperationFailed {
        tunnel: String,
        op: &'static str,
        error: String,
    },
}

pub type MsgTunnelMgrResult<T> = std::result::Result<T, MsgTunnelMgrError>;

struct MsgTunnelEntry {
    tunnel: Arc<dyn MsgTunnel>,
    info: MsgTunnelInstanceInfo,
}

#[derive(Clone, Default)]
pub struct MsgTunnelInstanceMgr {
    entries: Arc<RwLock<HashMap<String, MsgTunnelEntry>>>,
}

impl MsgTunnelInstanceMgr {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &self,
        tunnel: Arc<dyn MsgTunnel>,
    ) -> MsgTunnelMgrResult<MsgTunnelInstanceInfo> {
        let tunnel_did = tunnel.tunnel_did();
        let key = tunnel_did.to_string();
        let now_ms = Self::now_ms();
        let info = MsgTunnelInstanceInfo {
            tunnel_did: tunnel_did.clone(),
            name: tunnel.name().to_string(),
            platform: tunnel.platform().to_string(),
            supports_ingress: tunnel.supports_ingress(),
            supports_egress: tunnel.supports_egress(),
            state: MsgTunnelInstanceState::Registered,
            registered_at_ms: now_ms,
            updated_at_ms: now_ms,
            last_error: None,
        };

        let mut entries = self
            .entries
            .write()
            .map_err(|_| MsgTunnelMgrError::LockPoisoned)?;
        if entries.contains_key(&key) {
            return Err(MsgTunnelMgrError::AlreadyRegistered(key));
        }

        entries.insert(
            key,
            MsgTunnelEntry {
                tunnel,
                info: info.clone(),
            },
        );
        Ok(info)
    }

    pub fn unregister(&self, tunnel_did: &DID) -> MsgTunnelMgrResult<MsgTunnelInstanceInfo> {
        let key = tunnel_did.to_string();
        let mut entries = self
            .entries
            .write()
            .map_err(|_| MsgTunnelMgrError::LockPoisoned)?;
        let state = entries
            .get(&key)
            .ok_or_else(|| MsgTunnelMgrError::NotFound(key.clone()))?
            .info
            .state;

        if !matches!(
            state,
            MsgTunnelInstanceState::Registered
                | MsgTunnelInstanceState::Stopped
                | MsgTunnelInstanceState::Faulted
        ) {
            return Err(MsgTunnelMgrError::InvalidStateTransition {
                tunnel: key,
                op: "unregister",
                state,
            });
        }

        let removed = entries
            .remove(&tunnel_did.to_string())
            .ok_or_else(|| MsgTunnelMgrError::NotFound(tunnel_did.to_string()))?;
        Ok(removed.info)
    }

    pub fn get_tunnel(&self, tunnel_did: &DID) -> MsgTunnelMgrResult<Option<Arc<dyn MsgTunnel>>> {
        let entries = self
            .entries
            .read()
            .map_err(|_| MsgTunnelMgrError::LockPoisoned)?;
        Ok(entries
            .get(&tunnel_did.to_string())
            .map(|entry| entry.tunnel.clone()))
    }

    pub fn get_instance(
        &self,
        tunnel_did: &DID,
    ) -> MsgTunnelMgrResult<Option<MsgTunnelInstanceInfo>> {
        let entries = self
            .entries
            .read()
            .map_err(|_| MsgTunnelMgrError::LockPoisoned)?;
        Ok(entries
            .get(&tunnel_did.to_string())
            .map(|entry| entry.info.clone()))
    }

    pub fn list_instances(&self) -> MsgTunnelMgrResult<Vec<MsgTunnelInstanceInfo>> {
        let entries = self
            .entries
            .read()
            .map_err(|_| MsgTunnelMgrError::LockPoisoned)?;
        let mut result: Vec<_> = entries.values().map(|entry| entry.info.clone()).collect();
        result.sort_by(|left, right| {
            left.tunnel_did
                .to_string()
                .cmp(&right.tunnel_did.to_string())
        });
        Ok(result)
    }

    pub async fn start_instance(
        &self,
        tunnel_did: &DID,
    ) -> MsgTunnelMgrResult<MsgTunnelInstanceInfo> {
        let key = tunnel_did.to_string();
        let tunnel = {
            let mut entries = self
                .entries
                .write()
                .map_err(|_| MsgTunnelMgrError::LockPoisoned)?;
            let entry = entries
                .get_mut(&key)
                .ok_or_else(|| MsgTunnelMgrError::NotFound(key.clone()))?;
            match entry.info.state {
                MsgTunnelInstanceState::Running => return Ok(entry.info.clone()),
                MsgTunnelInstanceState::Starting | MsgTunnelInstanceState::Stopping => {
                    return Err(MsgTunnelMgrError::InvalidStateTransition {
                        tunnel: key.clone(),
                        op: "start",
                        state: entry.info.state,
                    });
                }
                MsgTunnelInstanceState::Registered
                | MsgTunnelInstanceState::Stopped
                | MsgTunnelInstanceState::Faulted => {}
            }
            entry.info.state = MsgTunnelInstanceState::Starting;
            entry.info.updated_at_ms = Self::now_ms();
            entry.info.last_error = None;
            entry.tunnel.clone()
        };

        let start_result = tunnel.start().await;
        match start_result {
            Ok(()) => self.update_state(&key, MsgTunnelInstanceState::Running, None),
            Err(error) => {
                let reason = error.to_string();
                let _ =
                    self.update_state(&key, MsgTunnelInstanceState::Faulted, Some(reason.clone()));
                Err(MsgTunnelMgrError::OperationFailed {
                    tunnel: key,
                    op: "start",
                    error: reason,
                })
            }
        }
    }

    pub async fn stop_instance(
        &self,
        tunnel_did: &DID,
    ) -> MsgTunnelMgrResult<MsgTunnelInstanceInfo> {
        let key = tunnel_did.to_string();
        let tunnel = {
            let mut entries = self
                .entries
                .write()
                .map_err(|_| MsgTunnelMgrError::LockPoisoned)?;
            let entry = entries
                .get_mut(&key)
                .ok_or_else(|| MsgTunnelMgrError::NotFound(key.clone()))?;
            match entry.info.state {
                MsgTunnelInstanceState::Registered | MsgTunnelInstanceState::Stopped => {
                    return Ok(entry.info.clone());
                }
                MsgTunnelInstanceState::Starting | MsgTunnelInstanceState::Stopping => {
                    return Err(MsgTunnelMgrError::InvalidStateTransition {
                        tunnel: key.clone(),
                        op: "stop",
                        state: entry.info.state,
                    });
                }
                MsgTunnelInstanceState::Running | MsgTunnelInstanceState::Faulted => {}
            }
            entry.info.state = MsgTunnelInstanceState::Stopping;
            entry.info.updated_at_ms = Self::now_ms();
            entry.tunnel.clone()
        };

        let stop_result = tunnel.stop().await;
        match stop_result {
            Ok(()) => self.update_state(&key, MsgTunnelInstanceState::Stopped, None),
            Err(error) => {
                let reason = error.to_string();
                let _ =
                    self.update_state(&key, MsgTunnelInstanceState::Faulted, Some(reason.clone()));
                Err(MsgTunnelMgrError::OperationFailed {
                    tunnel: key,
                    op: "stop",
                    error: reason,
                })
            }
        }
    }

    pub async fn start_all(&self) -> MsgTunnelMgrResult<Vec<MsgTunnelOpReport>> {
        let instances = self.list_instances()?;
        let mut reports = Vec::with_capacity(instances.len());
        for info in instances {
            let did = info.tunnel_did.clone();
            let report = match self.start_instance(&did).await {
                Ok(updated) => MsgTunnelOpReport::success(updated),
                Err(error) => MsgTunnelOpReport::failed(did, error),
            };
            reports.push(report);
        }
        Ok(reports)
    }

    pub async fn stop_all(&self) -> MsgTunnelMgrResult<Vec<MsgTunnelOpReport>> {
        let instances = self.list_instances()?;
        let mut reports = Vec::with_capacity(instances.len());
        for info in instances {
            let did = info.tunnel_did.clone();
            let report = match self.stop_instance(&did).await {
                Ok(updated) => MsgTunnelOpReport::success(updated),
                Err(error) => MsgTunnelOpReport::failed(did, error),
            };
            reports.push(report);
        }
        Ok(reports)
    }

    pub async fn send_via(
        &self,
        tunnel_did: &DID,
        record: MsgRecordWithObject,
    ) -> MsgTunnelMgrResult<DeliveryReportResult> {
        let key = tunnel_did.to_string();
        let (tunnel, info) = {
            let entries = self
                .entries
                .read()
                .map_err(|_| MsgTunnelMgrError::LockPoisoned)?;
            let entry = entries
                .get(&key)
                .ok_or_else(|| MsgTunnelMgrError::NotFound(key.clone()))?;
            (entry.tunnel.clone(), entry.info.clone())
        };

        if info.state != MsgTunnelInstanceState::Running {
            return Err(MsgTunnelMgrError::NotRunning(key));
        }
        if !info.supports_egress {
            return Err(MsgTunnelMgrError::EgressNotSupported(
                tunnel_did.to_string(),
            ));
        }

        tunnel
            .send_record(record)
            .await
            .map_err(|error| MsgTunnelMgrError::OperationFailed {
                tunnel: tunnel_did.to_string(),
                op: "send",
                error: error.to_string(),
            })
    }

    pub async fn send_record(
        &self,
        record: MsgRecordWithObject,
    ) -> MsgTunnelMgrResult<DeliveryReportResult> {
        let tunnel_did = record
            .record
            .route
            .as_ref()
            .and_then(|route| route.tunnel_did.clone())
            .ok_or_else(|| {
                MsgTunnelMgrError::MissingRouteTunnelDid(record.record.record_id.clone())
            })?;
        self.send_via(&tunnel_did, record).await
    }

    fn update_state(
        &self,
        key: &str,
        state: MsgTunnelInstanceState,
        last_error: Option<String>,
    ) -> MsgTunnelMgrResult<MsgTunnelInstanceInfo> {
        let mut entries = self
            .entries
            .write()
            .map_err(|_| MsgTunnelMgrError::LockPoisoned)?;
        let entry = entries
            .get_mut(key)
            .ok_or_else(|| MsgTunnelMgrError::NotFound(key.to_string()))?;
        entry.info.state = state;
        entry.info.updated_at_ms = Self::now_ms();
        entry.info.last_error = last_error;
        Ok(entry.info.clone())
    }

    fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use buckyos_api::{BoxKind, MsgObject, MsgRecord, MsgState, RouteInfo};
    use serde_json::json;
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

    static TEST_SEQ: AtomicU64 = AtomicU64::new(1);

    fn next_obj_id() -> ndn_lib::ObjId {
        let seq = TEST_SEQ.fetch_add(1, Ordering::SeqCst);
        ndn_lib::ObjId::new(&format!("chunk:{:016x}", seq)).unwrap()
    }

    fn build_tunnel_record(tunnel_did: DID) -> MsgRecordWithObject {
        let msg_id = next_obj_id();
        let msg = MsgObject {
            id: msg_id.clone(),
            from: DID::new("bns", "author"),
            source: None,
            to: vec![DID::new("bns", "receiver")],
            thread_key: None,
            payload: json!({ "kind": "text", "text": "hello" }),
            meta: None,
            created_at_ms: 1,
        };
        let record = MsgRecord {
            record_id: format!("record-{}", msg_id.to_string()),
            owner: tunnel_did.clone(),
            box_kind: BoxKind::TunnelOutbox,
            msg_id,
            state: MsgState::Wait,
            created_at_ms: 1,
            updated_at_ms: 1,
            route: Some(RouteInfo {
                tunnel_did: Some(tunnel_did),
                platform: Some("telegram".to_string()),
                ..Default::default()
            }),
            delivery: None,
            thread_key: None,
            sort_key: 1,
            tags: Vec::new(),
        };

        MsgRecordWithObject { record, msg }
    }

    struct MockTunnel {
        did: DID,
        name: String,
        platform: String,
        egress_enabled: bool,
        running: AtomicBool,
        start_calls: AtomicUsize,
        stop_calls: AtomicUsize,
        send_calls: AtomicUsize,
    }

    impl MockTunnel {
        fn new(subject: &str, platform: &str, egress_enabled: bool) -> Self {
            Self {
                did: DID::new("bns", subject),
                name: format!("{}-tunnel", subject),
                platform: platform.to_string(),
                egress_enabled,
                running: AtomicBool::new(false),
                start_calls: AtomicUsize::new(0),
                stop_calls: AtomicUsize::new(0),
                send_calls: AtomicUsize::new(0),
            }
        }

        fn start_count(&self) -> usize {
            self.start_calls.load(Ordering::SeqCst)
        }

        fn stop_count(&self) -> usize {
            self.stop_calls.load(Ordering::SeqCst)
        }

        fn send_count(&self) -> usize {
            self.send_calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl MsgTunnel for MockTunnel {
        fn tunnel_did(&self) -> DID {
            self.did.clone()
        }

        fn name(&self) -> &str {
            &self.name
        }

        fn platform(&self) -> &str {
            &self.platform
        }

        fn supports_egress(&self) -> bool {
            self.egress_enabled
        }

        async fn start(&self) -> AnyResult<()> {
            self.start_calls.fetch_add(1, Ordering::SeqCst);
            self.running.store(true, Ordering::SeqCst);
            Ok(())
        }

        async fn stop(&self) -> AnyResult<()> {
            self.stop_calls.fetch_add(1, Ordering::SeqCst);
            self.running.store(false, Ordering::SeqCst);
            Ok(())
        }

        async fn send_record(
            &self,
            _record: MsgRecordWithObject,
        ) -> AnyResult<DeliveryReportResult> {
            if !self.egress_enabled {
                bail!("egress is disabled");
            }
            if !self.running.load(Ordering::SeqCst) {
                bail!("tunnel is not running");
            }

            let seq = self.send_calls.fetch_add(1, Ordering::SeqCst) + 1;
            Ok(DeliveryReportResult {
                ok: true,
                external_msg_id: Some(format!("ext-{}", seq)),
                delivered_at_ms: Some(1000 + seq as u64),
                ..Default::default()
            })
        }
    }

    #[tokio::test]
    async fn register_rejects_duplicate_tunnel_did() {
        let mgr = MsgTunnelInstanceMgr::new();
        let tunnel = Arc::new(MockTunnel::new("tg-main", "telegram", true));
        let tunnel_did = tunnel.tunnel_did();

        let first = mgr.register(tunnel.clone()).unwrap();
        assert_eq!(first.state, MsgTunnelInstanceState::Registered);

        let duplicate_err = mgr.register(tunnel).unwrap_err();
        assert!(matches!(
            duplicate_err,
            MsgTunnelMgrError::AlreadyRegistered(ref did) if did == &tunnel_did.to_string()
        ));
    }

    #[tokio::test]
    async fn lifecycle_and_send_flow_work_for_running_tunnel() {
        let mgr = MsgTunnelInstanceMgr::new();
        let tunnel = Arc::new(MockTunnel::new("tg-send", "telegram", true));
        let tunnel_did = tunnel.tunnel_did();
        mgr.register(tunnel.clone()).unwrap();

        let before_start_err = mgr
            .send_via(&tunnel_did, build_tunnel_record(tunnel_did.clone()))
            .await
            .unwrap_err();
        assert!(matches!(before_start_err, MsgTunnelMgrError::NotRunning(_)));

        let running = mgr.start_instance(&tunnel_did).await.unwrap();
        assert_eq!(running.state, MsgTunnelInstanceState::Running);

        let report = mgr
            .send_record(build_tunnel_record(tunnel_did.clone()))
            .await
            .unwrap();
        assert!(report.ok);

        let stopped = mgr.stop_instance(&tunnel_did).await.unwrap();
        assert_eq!(stopped.state, MsgTunnelInstanceState::Stopped);
        assert_eq!(tunnel.start_count(), 1);
        assert_eq!(tunnel.stop_count(), 1);
        assert_eq!(tunnel.send_count(), 1);
    }

    #[tokio::test]
    async fn start_stop_all_and_unregister_follow_state_rules() {
        let mgr = MsgTunnelInstanceMgr::new();
        let tunnel_a = Arc::new(MockTunnel::new("tg-a", "telegram", true));
        let tunnel_b = Arc::new(MockTunnel::new("slack-b", "slack", false));

        let did_a = tunnel_a.tunnel_did();
        let did_b = tunnel_b.tunnel_did();
        mgr.register(tunnel_a).unwrap();
        mgr.register(tunnel_b).unwrap();

        let start_reports = mgr.start_all().await.unwrap();
        assert_eq!(start_reports.len(), 2);
        assert!(start_reports.iter().all(|report| report.ok));

        let send_err = mgr
            .send_via(&did_b, build_tunnel_record(did_b.clone()))
            .await
            .unwrap_err();
        assert!(matches!(send_err, MsgTunnelMgrError::EgressNotSupported(_)));

        let unregister_running_err = mgr.unregister(&did_a).unwrap_err();
        assert!(matches!(
            unregister_running_err,
            MsgTunnelMgrError::InvalidStateTransition { .. }
        ));

        let stop_reports = mgr.stop_all().await.unwrap();
        assert_eq!(stop_reports.len(), 2);
        assert!(stop_reports.iter().all(|report| report.ok));

        mgr.unregister(&did_a).unwrap();
        assert!(mgr.get_instance(&did_a).unwrap().is_none());
    }
}
