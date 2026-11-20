use std::{path::Path, path::PathBuf, sync::Arc, time::Duration};

use buckyos_api::{completed_install, BuckyOSInfo, BUCKYOS_INFO_KEY};
use buckyos_kit::{get_channel, get_target, get_version};
use system_config::kv_provider::KVStoreProvider;
use tokio::sync::Mutex;

// The caller holds the same store lock used by RPC writers. A changed value and
// its revision are committed and flushed by KVStoreProvider::set together.
pub(super) async fn refresh(root: &Path, store: &dyn KVStoreProvider) -> Result<(), String> {
    let Some(receipt) = completed_install::read(root).map_err(|e| e.to_string())? else {
        return Ok(());
    };
    if receipt.target_build != completed_install::target_build()
        || receipt.runtime_build != completed_install::runtime_build()
    {
        // The new service can start before the installer publishes its success.
        return Ok(());
    }
    let Some(value) = store
        .get(BUCKYOS_INFO_KEY.into())
        .await
        .map_err(|e| e.to_string())?
    else {
        // Scheduler owns the initial record and the original installation time.
        return Ok(());
    };
    let current: BuckyOSInfo = serde_json::from_str(&value).map_err(|e| e.to_string())?;
    current.validate()?;
    let mut installed = BuckyOSInfo::from_runtime(
        get_version(),
        &get_channel().to_string(),
        get_target(),
        current.installed_at,
    );
    if current.version == installed.version
        && current.build_version == installed.build_version
        && current.target == installed.target
        && receipt.completed_at <= current.updated_at
    {
        // Replaying an already applied receipt, including after a service restart
        // with a different channel environment, is not a new installation.
        return Ok(());
    }
    installed.updated_at = receipt.completed_at.max(current.updated_at);
    let value = serde_json::to_string(&installed).map_err(|e| e.to_string())?;
    store
        .set(BUCKYOS_INFO_KEY.into(), value)
        .await
        .map_err(|e| e.to_string())
}

// Apply receipts even when Settings is closed. Only report a recurring error once.
pub(super) async fn monitor(
    root: PathBuf,
    store: Arc<Mutex<dyn KVStoreProvider>>,
    interval: Duration,
) {
    let mut previous_error = None;
    loop {
        let error = refresh(&root, &*store.lock().await).await.err();
        if previous_error != error {
            if let Some(reason) = &error {
                log::error!(
                    "cannot refresh {BUCKYOS_INFO_KEY} from completed installation: {reason}"
                );
            }
            previous_error = error;
        }
        tokio::time::sleep(interval).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use completed_install::CompletedInstall;
    use std::fs;
    use system_config::sled_provider::SledStore;

    fn write_receipt(root: &Path, receipt: CompletedInstall) {
        let path = root.join(completed_install::COMPLETED_INSTALL_FILE);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, serde_json::to_vec(&receipt).unwrap()).unwrap();
    }

    async fn record(store: &dyn KVStoreProvider) -> (BuckyOSInfo, u64) {
        let (value, revision) = store
            .get_with_revision(BUCKYOS_INFO_KEY.into())
            .await
            .unwrap()
            .unwrap();
        (serde_json::from_str(&value).unwrap(), revision)
    }

    #[tokio::test]
    async fn only_success_of_the_running_release_changes_the_existing_version() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path();
        let database = path.join("local/system_config");
        let store = SledStore::open(&database).unwrap();
        let old = BuckyOSInfo::from_runtime("0.2.0+build260101", "stable", "old-platform", 50);
        store
            .create(BUCKYOS_INFO_KEY, &serde_json::to_string(&old).unwrap())
            .await
            .unwrap();
        let original = record(&store).await;

        refresh(path, &store).await.unwrap();
        assert_eq!(record(&store).await, original);
        let mut previous = CompletedInstall::new(100);
        previous.target_build = "0.2.0+build260101".to_owned();
        previous.runtime_build = previous.target_build.clone();
        write_receipt(path, previous);
        refresh(path, &store).await.unwrap();
        assert_eq!(record(&store).await, original);

        let mut other_commit = CompletedInstall::new(110);
        other_commit.runtime_build = format!("{}.other.branch", other_commit.target_build);
        write_receipt(path, other_commit);
        refresh(path, &store).await.unwrap();
        assert_eq!(record(&store).await, original);

        write_receipt(path, CompletedInstall::new(120));
        refresh(path, &store).await.unwrap();
        let (installed, revision) = record(&store).await;
        let mut expected =
            BuckyOSInfo::from_runtime(get_version(), &get_channel().to_string(), get_target(), 50);
        expected.updated_at = 120;
        assert_eq!(installed, expected);
        assert_eq!(revision, original.1 + 1);

        drop(store);
        let reopened = SledStore::open_existing(&database).unwrap();
        refresh(path, &reopened).await.unwrap();
        assert_eq!(record(&reopened).await, (expected.clone(), revision));

        write_receipt(path, CompletedInstall::new(125));
        refresh(path, &reopened).await.unwrap();
        expected.updated_at = 125;
        assert_eq!(record(&reopened).await, (expected.clone(), revision + 1));
        write_receipt(path, CompletedInstall::new(100));
        refresh(path, &reopened).await.unwrap();
        assert_eq!(record(&reopened).await, (expected.clone(), revision + 1));

        // A restart under a different channel environment may only update the channel
        // for a newly completed installation.
        expected.release_channel = if get_channel().to_string() == "stable" {
            "nightly"
        } else {
            "stable"
        }
        .to_owned();
        reopened
            .set(
                BUCKYOS_INFO_KEY.into(),
                serde_json::to_string(&expected).unwrap(),
            )
            .await
            .unwrap();
        write_receipt(path, CompletedInstall::new(125));
        refresh(path, &reopened).await.unwrap();
        assert_eq!(record(&reopened).await, (expected, revision + 2));
    }

    #[tokio::test]
    async fn scheduler_owns_first_install_and_invalid_existing_data_is_preserved() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path();
        let store = SledStore::open(&path.join("local/system_config")).unwrap();
        write_receipt(path, CompletedInstall::new(100));
        refresh(path, &store).await.unwrap();
        assert_eq!(store.get(BUCKYOS_INFO_KEY.into()).await.unwrap(), None);

        store
            .create(BUCKYOS_INFO_KEY, r#"{"schema_version":99}"#)
            .await
            .unwrap();
        let original = store
            .get_with_revision(BUCKYOS_INFO_KEY.into())
            .await
            .unwrap();
        assert!(refresh(path, &store).await.is_err());
        assert_eq!(
            store
                .get_with_revision(BUCKYOS_INFO_KEY.into())
                .await
                .unwrap(),
            original
        );
    }

    #[tokio::test]
    async fn running_service_persists_late_completion_without_a_settings_request() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path();
        let store: Arc<Mutex<dyn KVStoreProvider>> = Arc::new(Mutex::new(
            SledStore::open(&path.join("local/system_config")).unwrap(),
        ));
        let current =
            BuckyOSInfo::from_runtime(get_version(), &get_channel().to_string(), get_target(), 50);
        store
            .lock()
            .await
            .create(BUCKYOS_INFO_KEY, &serde_json::to_string(&current).unwrap())
            .await
            .unwrap();
        let worker = tokio::spawn(monitor(
            path.to_owned(),
            store.clone(),
            Duration::from_millis(20),
        ));
        tokio::task::yield_now().await;
        write_receipt(path, CompletedInstall::new(120));
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if record(&*store.lock().await).await.0.updated_at == 120 {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("service did not record the completed installation");
        worker.abort();
    }
}
