//! Process-wide state wiring: config + filedb + all volatile managers.

use crate::acl::AclTable;
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
    pub acls: AclTable,
    /// Cached `system/zone_owner_user_id` (buckyos mode).
    pub zone_owner: std::sync::RwLock<Option<String>>,
    pub handles: HandleCodec,
    /// Key for stateless watch tokens (per-process; watch state is volatile).
    pub watch_key: [u8; 32],
    pub sessions: SessionMgr,
    pub leases: LeaseMgr,
    pub revisions: RevisionMgr,
    pub bus: EventBus,
    pub uploads: UploadMgr,
    pub copies: crate::copy::CopyJournal,
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
        let copies = crate::copy::CopyJournal::open(&config.data_dir)?;
        let acls = AclTable::load(&db)?;
        if matches!(config.auth, crate::config::AuthMode::Runtime) {
            seed_default_acls(&config, &db, &acls)?;
        }
        Ok(Arc::new(AppState {
            handles: HandleCodec::new(handle_key),
            watch_key: rand::random(),
            sessions: SessionMgr::new(config.replay_window),
            leases: LeaseMgr::new(Duration::from_secs(config.lease_ttl_secs)),
            revisions: RevisionMgr::new(),
            bus: EventBus::new(),
            uploads,
            copies,
            scan_state: std::sync::Mutex::new(Default::default()),
            acls,
            zone_owner: std::sync::RwLock::new(None),
            db,
            config,
        }))
    }
}

/// Once per filedb, the zone-shared areas of doc/path_usage.md: the library is
/// shared by all zone users, publish is what `public.<zone>` already serves.
/// They are ordinary records afterwards, so the owner may change them.
fn seed_default_acls(config: &ServerConfig, db: &FileDb, acls: &AclTable) -> NfsResult<()> {
    use crate::acl::{AclEntry, AclRecord, Level, PUBLIC, ZONE_USERS};
    if db.kv_get("acl_defaults_seeded")?.is_some() {
        return Ok(());
    }
    for (path, principal, level) in [("library", ZONE_USERS, Level::Write), ("publish", PUBLIC, Level::Read)] {
        if config.root("srv").is_some_and(|r| r.path.join(path).is_dir()) && acls.get("srv", path).is_none() {
            acls.set(
                db,
                AclRecord {
                    root: "srv".into(),
                    path: path.into(),
                    inherit: true,
                    entries: vec![AclEntry { principal: principal.into(), level }],
                    updated_by: "system".into(),
                    updated_at: crate::fsutil::unix_now(),
                },
            )?;
        }
    }
    db.kv_set("acl_defaults_seeded", "1")
}
