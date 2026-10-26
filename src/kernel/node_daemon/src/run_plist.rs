use crate::run_item::RunItemTargetState;
use buckyos_api::ServiceInstanceState;
use buckyos_kit::buckyos_get_unix_timestamp;
use lazy_static::lazy_static;
use log::warn;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const RUN_PLIST_VERSION: u32 = 1;
const RUN_PLIST_FILE_NAME: &str = "run.plist";

lazy_static! {
    static ref RUN_PLIST_STORE: Mutex<RunPlistStore> = Mutex::new(RunPlistStore::default());
}

#[derive(Default)]
struct RunPlistStore {
    plist: Option<RunPlist>,
    batch_depth: usize,
}

impl RunPlistStore {
    fn load(&mut self, path: &Path) -> &mut RunPlist {
        self.plist
            .get_or_insert_with(|| read_run_plist(path).unwrap_or_default())
    }

    fn update(&mut self, path: &Path, item: RunPlistItem) -> io::Result<()> {
        let immediate = matches!(
            item.run_state,
            RunPlistItemState::Deploying
                | RunPlistItemState::Deployed
                | RunPlistItemState::Starting
                | RunPlistItemState::Stopping
                | RunPlistItemState::StartFailed
                | RunPlistItemState::DeployFailed
                | RunPlistItemState::StopFailed
                | RunPlistItemState::ObserveFailed
        ) || self
            .load(path)
            .items
            .get(&item.item_name)
            .is_some_and(|previous| {
                matches!(
                    (&previous.run_state, &item.run_state),
                    (RunPlistItemState::Stopping, RunPlistItemState::Stopped)
                        | (RunPlistItemState::Starting, RunPlistItemState::Started)
                )
            });
        let plist = self.load(path);
        plist.updated_at = item.updated_at;
        plist.items.insert(item.item_name.clone(), item);
        if immediate || self.batch_depth == 0 {
            self.flush(path)?;
        }
        Ok(())
    }

    fn flush(&mut self, path: &Path) -> io::Result<()> {
        let dir = path.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "run plist path has no parent")
        })?;
        fs::create_dir_all(dir)?;
        let plist = self.load(path);
        plist.version = RUN_PLIST_VERSION;
        plist.updated_at = buckyos_get_unix_timestamp();
        write_run_plist_atomic(path, dir, plist)
    }

    fn finish_batch(&mut self, path: &Path) -> io::Result<()> {
        self.batch_depth -= 1;
        if self.batch_depth == 0 {
            self.flush(path)?;
        }
        Ok(())
    }
}

pub struct RunPlistBatch {
    _private: (),
}

pub fn begin_run_plist_batch() -> RunPlistBatch {
    let mut store = RUN_PLIST_STORE
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    store.load(&run_plist_path());
    store.batch_depth += 1;
    RunPlistBatch { _private: () }
}

impl Drop for RunPlistBatch {
    fn drop(&mut self) {
        let mut store = RUN_PLIST_STORE
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        if let Err(err) = store.finish_batch(&run_plist_path()) {
            warn!("flush run plist batch failed: {}", err);
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub enum RunPlistItemState {
    PlannedStart,
    AlreadyRunning,
    Deploying,
    Deployed,
    Starting,
    Started,
    WaitingDeploy,
    StartFailed,
    DeployFailed,
    PlannedStop,
    Stopping,
    StopFailed,
    Stopped,
    Exited,
    NotExist,
    ObserveFailed,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RunPlistItem {
    pub item_name: String,
    pub item_kind: String,
    pub target_state: RunItemTargetState,
    pub observed_state: Option<ServiceInstanceState>,
    pub run_state: RunPlistItemState,
    pub last_error: Option<String>,
    pub updated_at: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RunPlist {
    pub version: u32,
    pub updated_at: u64,
    pub items: BTreeMap<String, RunPlistItem>,
}

impl Default for RunPlist {
    fn default() -> Self {
        Self {
            version: RUN_PLIST_VERSION,
            updated_at: buckyos_get_unix_timestamp(),
            items: BTreeMap::new(),
        }
    }
}

pub fn run_plist_path() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        return std::env::temp_dir()
            .join("buckyos")
            .join(RUN_PLIST_FILE_NAME);
    }

    #[cfg(target_os = "macos")]
    {
        return buckyos_kit::get_buckyos_service_local_data_dir("node_daemon")
            .join(RUN_PLIST_FILE_NAME);
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        PathBuf::from("/tmp/buckyos").join(RUN_PLIST_FILE_NAME)
    }
}

pub fn update_run_item(
    item_name: &str,
    item_kind: &str,
    target_state: &RunItemTargetState,
    observed_state: Option<&ServiceInstanceState>,
    run_state: RunPlistItemState,
    last_error: Option<String>,
) {
    if let Err(err) = update_run_item_impl(
        item_name,
        item_kind,
        target_state,
        observed_state,
        run_state,
        last_error,
    ) {
        warn!("update run plist failed: {}", err);
    }
}

fn update_run_item_impl(
    item_name: &str,
    item_kind: &str,
    target_state: &RunItemTargetState,
    observed_state: Option<&ServiceInstanceState>,
    run_state: RunPlistItemState,
    last_error: Option<String>,
) -> io::Result<()> {
    let now = buckyos_get_unix_timestamp();
    let mut store = RUN_PLIST_STORE
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    store.update(
        &run_plist_path(),
        RunPlistItem {
            item_name: item_name.to_string(),
            item_kind: item_kind.to_string(),
            target_state: target_state.clone(),
            observed_state: observed_state.cloned(),
            run_state,
            last_error,
            updated_at: now,
        },
    )
}

fn read_run_plist(path: &Path) -> io::Result<RunPlist> {
    let content = fs::read_to_string(path)?;
    serde_json::from_str(content.as_str()).map_err(|err| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("parse run plist {} failed: {}", path.display(), err),
        )
    })
}

fn write_run_plist_atomic(path: &Path, dir: &Path, run_plist: &RunPlist) -> io::Result<()> {
    let tmp_path = dir.join(format!("{}.tmp", RUN_PLIST_FILE_NAME));
    let content = serde_json::to_vec_pretty(run_plist).map_err(|err| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("serialize run plist failed: {}", err),
        )
    })?;
    fs::write(tmp_path.as_path(), content)?;
    fs::rename(tmp_path, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(name: &str, state: RunPlistItemState) -> RunPlistItem {
        RunPlistItem {
            item_name: name.to_string(),
            item_kind: "kernel".to_string(),
            target_state: RunItemTargetState::Running,
            observed_state: Some(ServiceInstanceState::Started),
            run_state: state,
            last_error: None,
            updated_at: buckyos_get_unix_timestamp(),
        }
    }

    #[test]
    fn batch_persists_complete_first_round_and_retains_existing_items() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(RUN_PLIST_FILE_NAME);
        let mut store = RunPlistStore::default();
        store.batch_depth = 1;
        for name in ["kernel-a", "app-b", "local-c"] {
            store
                .update(&path, item(name, RunPlistItemState::AlreadyRunning))
                .unwrap();
        }
        assert!(!path.exists());
        store.finish_batch(&path).unwrap();
        assert_eq!(read_run_plist(&path).unwrap().items.len(), 3);

        let mut reloaded = RunPlistStore::default();
        reloaded.batch_depth = 1;
        reloaded
            .update(&path, item("kernel-a", RunPlistItemState::AlreadyRunning))
            .unwrap();
        reloaded.finish_batch(&path).unwrap();
        let plist = read_run_plist(&path).unwrap();
        assert_eq!(plist.items.len(), 3);
        assert!(matches!(
            plist.items["app-b"].run_state,
            RunPlistItemState::AlreadyRunning
        ));
    }

    #[test]
    fn transitions_and_failures_flush_pending_observations_immediately() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(RUN_PLIST_FILE_NAME);
        let mut store = RunPlistStore::default();
        store.batch_depth = 1;
        store
            .update(&path, item("existing", RunPlistItemState::AlreadyRunning))
            .unwrap();
        for state in [
            RunPlistItemState::Starting,
            RunPlistItemState::Started,
            RunPlistItemState::Stopping,
            RunPlistItemState::Stopped,
            RunPlistItemState::ObserveFailed,
        ] {
            let expected = serde_json::to_value(&state).unwrap();
            store.update(&path, item("changing", state)).unwrap();
            let plist = read_run_plist(&path).unwrap();
            assert_eq!(plist.items.len(), 2);
            assert_eq!(
                serde_json::to_value(&plist.items["changing"].run_state).unwrap(),
                expected
            );
        }
        store.finish_batch(&path).unwrap();
    }

    #[test]
    fn nested_batch_flushes_once_at_outer_boundary_even_without_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(RUN_PLIST_FILE_NAME);
        let mut store = RunPlistStore::default();
        store.batch_depth = 2;
        store.finish_batch(&path).unwrap();
        assert!(!path.exists());
        store.finish_batch(&path).unwrap();
        assert!(read_run_plist(&path).unwrap().items.is_empty());
    }

    #[test]
    fn failed_flush_can_be_retried_without_losing_items() {
        let dir = tempfile::tempdir().unwrap();
        let parent = dir.path().join("blocked");
        fs::write(&parent, "not a directory").unwrap();
        let path = parent.join(RUN_PLIST_FILE_NAME);
        let mut store = RunPlistStore::default();
        assert!(store
            .update(&path, item("service", RunPlistItemState::Starting))
            .is_err());
        fs::remove_file(&parent).unwrap();
        store.flush(&path).unwrap();
        assert!(read_run_plist(&path).unwrap().items.contains_key("service"));
    }

    #[test]
    fn run_plist_path_matches_node_control() {
        #[cfg(not(target_os = "windows"))]
        assert_eq!(
            run_plist_path(),
            buckyos_api::node_control::run_plist_path(&buckyos_kit::get_buckyos_root_dir())
        );
    }
}
