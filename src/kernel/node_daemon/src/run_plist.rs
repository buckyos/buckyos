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
    static ref RUN_PLIST_LOCK: Mutex<()> = Mutex::new(());
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

    #[cfg(not(target_os = "windows"))]
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
    let _guard = RUN_PLIST_LOCK.lock().unwrap_or_else(|err| err.into_inner());
    let path = run_plist_path();
    let dir = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("run plist path {} has no parent", path.display()),
        )
    })?;
    fs::create_dir_all(dir)?;

    let now = buckyos_get_unix_timestamp();
    let mut run_plist = read_run_plist(path.as_path()).unwrap_or_default();
    run_plist.version = RUN_PLIST_VERSION;
    run_plist.updated_at = now;
    run_plist.items.insert(
        item_name.to_string(),
        RunPlistItem {
            item_name: item_name.to_string(),
            item_kind: item_kind.to_string(),
            target_state: target_state.clone(),
            observed_state: observed_state.cloned(),
            run_state,
            last_error,
            updated_at: now,
        },
    );

    write_run_plist_atomic(path.as_path(), dir, &run_plist)
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

    #[test]
    fn run_plist_path_uses_tmp_buckyos_on_unix() {
        #[cfg(not(target_os = "windows"))]
        assert_eq!(run_plist_path(), PathBuf::from("/tmp/buckyos/run.plist"));
    }
}
