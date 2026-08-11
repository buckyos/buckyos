#![allow(dead_code)]

//! Delivery executor registry.
//!
//! `DeliveryExecutor` is the shared interface both transport families
//! implement (see `doc/message_hub/Message Tunnel Design.md` §2):
//!
//! - MessageHub — native zone↔zone transport for shareable DIDs;
//! - MessageTunnel — external platform adapters (Telegram/Email/Lark…)
//!   addressed by local shadow endpoint DIDs.
//!
//! Executors consume complete `DeliveryRecord`s from their own
//! `DELIVERY_QUEUE` (owner = `transport_did`) and report results back through
//! `report_delivery`; they never see mailbox records or session projections.

use anyhow::{bail, Result as AnyResult};
use async_trait::async_trait;
use buckyos_api::{DeliveryRecordWithObject, DeliveryReportResult};
use name_lib::DID;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use thiserror::Error;

#[async_trait]
pub trait DeliveryExecutor: Send + Sync {
    /// The executor's DID: owner key of its DELIVERY_QUEUE.
    fn transport_did(&self) -> DID;
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

    /// Execute one delivery task. The envelope is a complete delivery
    /// instruction; executors must fail (not guess) when it is incomplete.
    async fn execute_delivery(
        &self,
        record: DeliveryRecordWithObject,
    ) -> AnyResult<DeliveryReportResult> {
        let _ = record;
        bail!(
            "executor {} does not implement execute_delivery",
            self.transport_did().to_string()
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutorInstanceState {
    Registered,
    Starting,
    Running,
    Stopping,
    Stopped,
    Faulted,
}

#[derive(Debug, Clone)]
pub struct ExecutorInstanceInfo {
    pub transport_did: DID,
    pub name: String,
    pub platform: String,
    pub supports_ingress: bool,
    pub supports_egress: bool,
    pub state: ExecutorInstanceState,
    pub registered_at_ms: u64,
    pub updated_at_ms: u64,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ExecutorOpReport {
    pub transport_did: DID,
    pub ok: bool,
    pub state: Option<ExecutorInstanceState>,
    pub error: Option<String>,
}

impl ExecutorOpReport {
    fn success(info: ExecutorInstanceInfo) -> Self {
        Self {
            transport_did: info.transport_did,
            ok: true,
            state: Some(info.state),
            error: None,
        }
    }

    fn failed(transport_did: DID, error: ExecutorMgrError) -> Self {
        Self {
            transport_did,
            ok: false,
            state: None,
            error: Some(error.to_string()),
        }
    }
}

#[derive(Debug, Error)]
pub enum ExecutorMgrError {
    #[error("delivery executor instance lock poisoned")]
    LockPoisoned,
    #[error("delivery executor {0} already registered")]
    AlreadyRegistered(String),
    #[error("delivery executor {0} not found")]
    NotFound(String),
    #[error("delivery executor {0} is not running")]
    NotRunning(String),
    #[error("delivery executor {0} does not support egress send")]
    EgressNotSupported(String),
    #[error("delivery executor {executor} cannot {op} from state {state:?}")]
    InvalidStateTransition {
        executor: String,
        op: &'static str,
        state: ExecutorInstanceState,
    },
    #[error("delivery executor {executor} {op} failed: {error}")]
    OperationFailed {
        executor: String,
        op: &'static str,
        error: String,
    },
}

pub type ExecutorMgrResult<T> = std::result::Result<T, ExecutorMgrError>;

struct ExecutorEntry {
    executor: Arc<dyn DeliveryExecutor>,
    info: ExecutorInstanceInfo,
}

#[derive(Clone, Default)]
pub struct DeliveryExecutorMgr {
    entries: Arc<RwLock<HashMap<String, ExecutorEntry>>>,
}

impl DeliveryExecutorMgr {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &self,
        executor: Arc<dyn DeliveryExecutor>,
    ) -> ExecutorMgrResult<ExecutorInstanceInfo> {
        let transport_did = executor.transport_did();
        let key = transport_did.to_string();
        let now_ms = Self::now_ms();
        let info = ExecutorInstanceInfo {
            transport_did: transport_did.clone(),
            name: executor.name().to_string(),
            platform: executor.platform().to_string(),
            supports_ingress: executor.supports_ingress(),
            supports_egress: executor.supports_egress(),
            state: ExecutorInstanceState::Registered,
            registered_at_ms: now_ms,
            updated_at_ms: now_ms,
            last_error: None,
        };

        let mut entries = self
            .entries
            .write()
            .map_err(|_| ExecutorMgrError::LockPoisoned)?;
        if entries.contains_key(&key) {
            return Err(ExecutorMgrError::AlreadyRegistered(key));
        }

        entries.insert(
            key,
            ExecutorEntry {
                executor,
                info: info.clone(),
            },
        );
        Ok(info)
    }

    pub fn unregister(&self, transport_did: &DID) -> ExecutorMgrResult<ExecutorInstanceInfo> {
        let key = transport_did.to_string();
        let mut entries = self
            .entries
            .write()
            .map_err(|_| ExecutorMgrError::LockPoisoned)?;
        let state = entries
            .get(&key)
            .ok_or_else(|| ExecutorMgrError::NotFound(key.clone()))?
            .info
            .state;

        if !matches!(
            state,
            ExecutorInstanceState::Registered
                | ExecutorInstanceState::Stopped
                | ExecutorInstanceState::Faulted
        ) {
            return Err(ExecutorMgrError::InvalidStateTransition {
                executor: key,
                op: "unregister",
                state,
            });
        }

        let removed = entries
            .remove(&transport_did.to_string())
            .ok_or_else(|| ExecutorMgrError::NotFound(transport_did.to_string()))?;
        Ok(removed.info)
    }

    pub fn get_executor(
        &self,
        transport_did: &DID,
    ) -> ExecutorMgrResult<Option<Arc<dyn DeliveryExecutor>>> {
        let entries = self
            .entries
            .read()
            .map_err(|_| ExecutorMgrError::LockPoisoned)?;
        Ok(entries
            .get(&transport_did.to_string())
            .map(|entry| entry.executor.clone()))
    }

    pub fn get_instance(
        &self,
        transport_did: &DID,
    ) -> ExecutorMgrResult<Option<ExecutorInstanceInfo>> {
        let entries = self
            .entries
            .read()
            .map_err(|_| ExecutorMgrError::LockPoisoned)?;
        Ok(entries
            .get(&transport_did.to_string())
            .map(|entry| entry.info.clone()))
    }

    pub fn list_instances(&self) -> ExecutorMgrResult<Vec<ExecutorInstanceInfo>> {
        let entries = self
            .entries
            .read()
            .map_err(|_| ExecutorMgrError::LockPoisoned)?;
        let mut result: Vec<_> = entries.values().map(|entry| entry.info.clone()).collect();
        result.sort_by(|left, right| {
            left.transport_did
                .to_string()
                .cmp(&right.transport_did.to_string())
        });
        Ok(result)
    }

    pub async fn start_instance(
        &self,
        transport_did: &DID,
    ) -> ExecutorMgrResult<ExecutorInstanceInfo> {
        let key = transport_did.to_string();
        let executor = {
            let mut entries = self
                .entries
                .write()
                .map_err(|_| ExecutorMgrError::LockPoisoned)?;
            let entry = entries
                .get_mut(&key)
                .ok_or_else(|| ExecutorMgrError::NotFound(key.clone()))?;
            match entry.info.state {
                ExecutorInstanceState::Running => return Ok(entry.info.clone()),
                ExecutorInstanceState::Starting | ExecutorInstanceState::Stopping => {
                    return Err(ExecutorMgrError::InvalidStateTransition {
                        executor: key.clone(),
                        op: "start",
                        state: entry.info.state,
                    });
                }
                ExecutorInstanceState::Registered
                | ExecutorInstanceState::Stopped
                | ExecutorInstanceState::Faulted => {}
            }
            entry.info.state = ExecutorInstanceState::Starting;
            entry.info.updated_at_ms = Self::now_ms();
            entry.info.last_error = None;
            entry.executor.clone()
        };

        let start_result = executor.start().await;
        match start_result {
            Ok(()) => self.update_state(&key, ExecutorInstanceState::Running, None),
            Err(error) => {
                let reason = error.to_string();
                let _ =
                    self.update_state(&key, ExecutorInstanceState::Faulted, Some(reason.clone()));
                Err(ExecutorMgrError::OperationFailed {
                    executor: key,
                    op: "start",
                    error: reason,
                })
            }
        }
    }

    pub async fn stop_instance(
        &self,
        transport_did: &DID,
    ) -> ExecutorMgrResult<ExecutorInstanceInfo> {
        let key = transport_did.to_string();
        let executor = {
            let mut entries = self
                .entries
                .write()
                .map_err(|_| ExecutorMgrError::LockPoisoned)?;
            let entry = entries
                .get_mut(&key)
                .ok_or_else(|| ExecutorMgrError::NotFound(key.clone()))?;
            match entry.info.state {
                ExecutorInstanceState::Registered | ExecutorInstanceState::Stopped => {
                    return Ok(entry.info.clone());
                }
                ExecutorInstanceState::Starting | ExecutorInstanceState::Stopping => {
                    return Err(ExecutorMgrError::InvalidStateTransition {
                        executor: key.clone(),
                        op: "stop",
                        state: entry.info.state,
                    });
                }
                ExecutorInstanceState::Running | ExecutorInstanceState::Faulted => {}
            }
            entry.info.state = ExecutorInstanceState::Stopping;
            entry.info.updated_at_ms = Self::now_ms();
            entry.executor.clone()
        };

        let stop_result = executor.stop().await;
        match stop_result {
            Ok(()) => self.update_state(&key, ExecutorInstanceState::Stopped, None),
            Err(error) => {
                let reason = error.to_string();
                let _ =
                    self.update_state(&key, ExecutorInstanceState::Faulted, Some(reason.clone()));
                Err(ExecutorMgrError::OperationFailed {
                    executor: key,
                    op: "stop",
                    error: reason,
                })
            }
        }
    }

    pub async fn start_all(&self) -> ExecutorMgrResult<Vec<ExecutorOpReport>> {
        let instances = self.list_instances()?;
        let mut reports = Vec::with_capacity(instances.len());
        for info in instances {
            let did = info.transport_did.clone();
            let report = match self.start_instance(&did).await {
                Ok(updated) => ExecutorOpReport::success(updated),
                Err(error) => ExecutorOpReport::failed(did, error),
            };
            reports.push(report);
        }
        Ok(reports)
    }

    pub async fn stop_all(&self) -> ExecutorMgrResult<Vec<ExecutorOpReport>> {
        let instances = self.list_instances()?;
        let mut reports = Vec::with_capacity(instances.len());
        for info in instances {
            let did = info.transport_did.clone();
            let report = match self.stop_instance(&did).await {
                Ok(updated) => ExecutorOpReport::success(updated),
                Err(error) => ExecutorOpReport::failed(did, error),
            };
            reports.push(report);
        }
        Ok(reports)
    }

    pub async fn execute_via(
        &self,
        transport_did: &DID,
        record: DeliveryRecordWithObject,
    ) -> ExecutorMgrResult<DeliveryReportResult> {
        let key = transport_did.to_string();
        let (executor, info) = {
            let entries = self
                .entries
                .read()
                .map_err(|_| ExecutorMgrError::LockPoisoned)?;
            let entry = entries
                .get(&key)
                .ok_or_else(|| ExecutorMgrError::NotFound(key.clone()))?;
            (entry.executor.clone(), entry.info.clone())
        };

        if info.state != ExecutorInstanceState::Running {
            return Err(ExecutorMgrError::NotRunning(key));
        }
        if !info.supports_egress {
            return Err(ExecutorMgrError::EgressNotSupported(
                transport_did.to_string(),
            ));
        }

        executor
            .execute_delivery(record)
            .await
            .map_err(|error| ExecutorMgrError::OperationFailed {
                executor: transport_did.to_string(),
                op: "execute_delivery",
                error: error.to_string(),
            })
    }

    /// Execute a delivery on the executor named by its envelope.
    pub async fn execute_delivery(
        &self,
        record: DeliveryRecordWithObject,
    ) -> ExecutorMgrResult<DeliveryReportResult> {
        let transport_did = record.record.envelope.transport_did.clone();
        self.execute_via(&transport_did, record).await
    }

    fn update_state(
        &self,
        key: &str,
        state: ExecutorInstanceState,
        last_error: Option<String>,
    ) -> ExecutorMgrResult<ExecutorInstanceInfo> {
        let mut entries = self
            .entries
            .write()
            .map_err(|_| ExecutorMgrError::LockPoisoned)?;
        let entry = entries
            .get_mut(key)
            .ok_or_else(|| ExecutorMgrError::NotFound(key.to_string()))?;
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
    use buckyos_api::{DeliveryEnvelope, DeliveryRecord, DeliveryState, TransportKind};
    use ndn_lib::{MsgContent, MsgContentFormat, MsgObjKind, MsgObject, NamedObject};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    fn build_delivery_record(transport_did: DID) -> DeliveryRecordWithObject {
        let msg = MsgObject {
            from: DID::new("bns", "author"),
            to: vec![DID::new("bns", "receiver")],
            kind: MsgObjKind::Chat,
            content: MsgContent {
                format: Some(MsgContentFormat::TextPlain),
                content: "hello".to_string(),
                ..Default::default()
            },
            created_at_ms: 1,
            ..Default::default()
        };
        let msg_id = msg.gen_obj_id().0;
        let record = DeliveryRecord {
            delivery_id: format!("dlv-{}", msg_id.to_string()),
            envelope: DeliveryEnvelope {
                msg_id,
                target_did: msg.to.first().cloned().unwrap(),
                transport_did,
                transport: TransportKind::Tunnel {
                    platform: "telegram".to_string(),
                    tunnel_instance_id: "tg-main-tunnel".to_string(),
                },
                address: None,
            },
            state: DeliveryState::Wait,
            attempts: 0,
            next_retry_at_ms: None,
            external_msg_id: None,
            delivered_at_ms: None,
            last_error: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        };

        DeliveryRecordWithObject {
            record,
            msg: Some(msg),
        }
    }

    struct MockExecutor {
        did: DID,
        name: String,
        platform: String,
        egress_enabled: bool,
        running: AtomicBool,
        start_calls: AtomicUsize,
        stop_calls: AtomicUsize,
        send_calls: AtomicUsize,
    }

    impl MockExecutor {
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
    impl DeliveryExecutor for MockExecutor {
        fn transport_did(&self) -> DID {
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

        async fn execute_delivery(
            &self,
            _record: DeliveryRecordWithObject,
        ) -> AnyResult<DeliveryReportResult> {
            if !self.egress_enabled {
                bail!("egress is disabled");
            }
            if !self.running.load(Ordering::SeqCst) {
                bail!("executor is not running");
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
    async fn register_rejects_duplicate_transport_did() {
        let mgr = DeliveryExecutorMgr::new();
        let executor = Arc::new(MockExecutor::new("tg-main", "telegram", true));
        let transport_did = executor.transport_did();

        let first = mgr.register(executor.clone()).unwrap();
        assert_eq!(first.state, ExecutorInstanceState::Registered);

        let duplicate_err = mgr.register(executor).unwrap_err();
        assert!(matches!(
            duplicate_err,
            ExecutorMgrError::AlreadyRegistered(ref did) if did == &transport_did.to_string()
        ));
    }

    #[tokio::test]
    async fn lifecycle_and_send_flow_work_for_running_executor() {
        let mgr = DeliveryExecutorMgr::new();
        let executor = Arc::new(MockExecutor::new("tg-send", "telegram", true));
        let transport_did = executor.transport_did();
        mgr.register(executor.clone()).unwrap();

        let before_start_err = mgr
            .execute_via(&transport_did, build_delivery_record(transport_did.clone()))
            .await
            .unwrap_err();
        assert!(matches!(before_start_err, ExecutorMgrError::NotRunning(_)));

        let running = mgr.start_instance(&transport_did).await.unwrap();
        assert_eq!(running.state, ExecutorInstanceState::Running);

        let report = mgr
            .execute_delivery(build_delivery_record(transport_did.clone()))
            .await
            .unwrap();
        assert!(report.ok);

        let stopped = mgr.stop_instance(&transport_did).await.unwrap();
        assert_eq!(stopped.state, ExecutorInstanceState::Stopped);
        assert_eq!(executor.start_count(), 1);
        assert_eq!(executor.stop_count(), 1);
        assert_eq!(executor.send_count(), 1);
    }

    #[tokio::test]
    async fn start_stop_all_and_unregister_follow_state_rules() {
        let mgr = DeliveryExecutorMgr::new();
        let executor_a = Arc::new(MockExecutor::new("tg-a", "telegram", true));
        let executor_b = Arc::new(MockExecutor::new("slack-b", "slack", false));

        let did_a = executor_a.transport_did();
        let did_b = executor_b.transport_did();
        mgr.register(executor_a).unwrap();
        mgr.register(executor_b).unwrap();

        let start_reports = mgr.start_all().await.unwrap();
        assert_eq!(start_reports.len(), 2);
        assert!(start_reports.iter().all(|report| report.ok));

        let send_err = mgr
            .execute_via(&did_b, build_delivery_record(did_b.clone()))
            .await
            .unwrap_err();
        assert!(matches!(send_err, ExecutorMgrError::EgressNotSupported(_)));

        let unregister_running_err = mgr.unregister(&did_a).unwrap_err();
        assert!(matches!(
            unregister_running_err,
            ExecutorMgrError::InvalidStateTransition { .. }
        ));

        let stop_reports = mgr.stop_all().await.unwrap();
        assert_eq!(stop_reports.len(), 2);
        assert!(stop_reports.iter().all(|report| report.ok));

        mgr.unregister(&did_a).unwrap();
        assert!(mgr.get_instance(&did_a).unwrap().is_none());
    }
}
