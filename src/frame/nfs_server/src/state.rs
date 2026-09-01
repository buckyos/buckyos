//! Process-wide state wiring: config + filedb + all volatile managers.

use crate::config::ServerConfig;
use crate::error::NfsResult;
use crate::filedb::FileDb;
use crate::handle::HandleCodec;
use crate::session::{LeaseMgr, SessionMgr};
use crate::uploads::UploadMgr;
use crate::watch::{EventBus, RevisionMgr};
use std::sync::Arc;
use std::time::Duration;

pub struct AppState {
    pub config: ServerConfig,
    pub db: FileDb,
    pub handles: HandleCodec,
    /// Key for stateless watch tokens (per-process; watch state is volatile).
    pub watch_key: [u8; 32],
    pub sessions: SessionMgr,
    pub leases: LeaseMgr,
    pub revisions: RevisionMgr,
    pub bus: EventBus,
    pub uploads: UploadMgr,
    /// Reconciler's per-dir fingerprints from the previous scan (see reconciler.rs).
    pub scan_state: std::sync::Mutex<crate::reconciler::ScanState>,
}

pub type SharedState = Arc<AppState>;

impl AppState {
    pub fn new(config: ServerConfig) -> NfsResult<SharedState> {
        std::fs::create_dir_all(&config.data_dir)?;
        for root in &config.exports {
            if !root.path.is_dir() {
                return Err(crate::error::invalid(format!(
                    "export root '{}' does not exist or is not a directory: {}",
                    root.id,
                    root.path.display()
                )));
            }
        }
        let db = FileDb::open(&config.db_path())?;
        let handle_key = db.handle_key()?;
        let uploads = UploadMgr::new(config.staging_dir())?;
        Ok(Arc::new(AppState {
            handles: HandleCodec::new(handle_key),
            watch_key: rand::random(),
            sessions: SessionMgr::new(config.replay_window),
            leases: LeaseMgr::new(Duration::from_secs(config.lease_ttl_secs)),
            revisions: RevisionMgr::new(),
            bus: EventBus::new(),
            uploads,
            scan_state: std::sync::Mutex::new(Default::default()),
            db,
            config,
        }))
    }
}
