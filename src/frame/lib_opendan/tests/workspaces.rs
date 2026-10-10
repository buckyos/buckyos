use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use libopendan::protocol::*;
use libopendan::state::{
    AgentStateClient, FsAgentStateClient, KrpcAgentStateClient, StateTransport,
};
use libopendan::{OpenDanError, Result};
use serde_json::Value;

fn state(path: &Path) -> FsAgentStateClient {
    FsAgentStateClient::open(path.join("agent"), "did:test:agent", None, None).unwrap()
}

fn create(path: &Path, operation: &str) -> WorkspaceCreate {
    WorkspaceCreate {
        operation_id: operation.into(),
        name: "Project".into(),
        description: "Long term project".into(),
        location: WorkspaceLocation {
            runtime_id: LOCAL_WORKSPACE_RUNTIME.into(),
            directory: path.into(),
        },
        usage: WorkspaceUsage::Collaborative,
        source_session: None,
        policy_ref: None,
    }
}

fn discover(record: &WorkspaceRecord, path: &Path) -> WorkspaceDiscover {
    WorkspaceDiscover {
        expected_workspace_id: Some(record.workspace_id.clone()),
        location: WorkspaceLocation {
            runtime_id: LOCAL_WORKSPACE_RUNTIME.into(),
            directory: path.into(),
        },
        expected_revision: Some(record.revision),
    }
}

#[tokio::test]
async fn registration_is_independent_from_storage_and_lifecycle_never_deletes_content() {
    let temp = tempfile::tempdir().unwrap();
    let state = state(temp.path());
    let path = temp.path().join("human-project");
    let request = create(&path, "first");
    let record = state.workspaces().create(&request, "user").await.unwrap();
    assert_eq!(
        record,
        state.workspaces().create(&request, "user").await.unwrap()
    );
    assert_eq!(record.location.directory, path.canonicalize().unwrap());
    assert!(!path.starts_with(state.agent_root().unwrap()));
    std::fs::write(path.join("result.txt"), "keep me").unwrap();
    std::fs::create_dir_all(temp.path().join("agent/workspace/unregistered")).unwrap();
    let archived = state
        .workspaces()
        .archive(&record.workspace_id, record.revision, "user")
        .await
        .unwrap();
    assert_eq!(archived.lifecycle, WorkspaceLifecycle::Archived);
    assert_eq!(
        state
            .workspaces()
            .query(&WorkspaceQuery::default())
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(state
        .workspaces()
        .archive(&record.workspace_id, record.revision, "user")
        .await
        .unwrap_err()
        .to_string()
        .contains("revision_conflict"));
    state
        .workspaces()
        .unregister(&record.workspace_id, archived.revision, "user")
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(path.join("result.txt")).unwrap(),
        "keep me"
    );
    let restored = state
        .workspaces()
        .discover(&discover(&record, &path), "user")
        .await
        .unwrap();
    assert_eq!(restored.workspace_id, record.workspace_id);
}

#[tokio::test]
async fn discover_moves_identity_only_after_old_directory_disappears() {
    let temp = tempfile::tempdir().unwrap();
    let state = state(temp.path());
    let old = temp.path().join("old");
    let new = temp.path().join("new");
    let record = state
        .workspaces()
        .create(&create(&old, "moving"), "user")
        .await
        .unwrap();
    std::fs::create_dir(&new).unwrap();
    std::fs::copy(
        old.join(WORKSPACE_METADATA_FILE),
        new.join(WORKSPACE_METADATA_FILE),
    )
    .unwrap();
    assert!(state
        .workspaces()
        .discover(&discover(&record, &new), "user")
        .await
        .unwrap_err()
        .to_string()
        .contains("duplicate_identity"));
    let conflict = state
        .workspaces()
        .lookup(&record.workspace_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(conflict.location, record.location);
    assert_eq!(conflict.availability, WorkspaceAvailability::Conflict);
    assert!(conflict.conflict.is_some());
    std::fs::remove_file(new.join(WORKSPACE_METADATA_FILE)).unwrap();
    std::fs::remove_dir(&new).unwrap();
    std::fs::rename(&old, &new).unwrap();
    assert!(state
        .workspaces()
        .discover(&discover(&record, &new), "user")
        .await
        .unwrap_err()
        .to_string()
        .contains("revision_conflict"));
    let moved = state
        .workspaces()
        .discover(&discover(&conflict, &new), "user")
        .await
        .unwrap();
    assert_eq!(moved.workspace_id, record.workspace_id);
    assert_eq!(moved.location_revision, record.location_revision + 1);
    assert_eq!(moved.location.directory, new);
    assert_eq!(moved.availability, WorkspaceAvailability::Available);
}

#[tokio::test]
async fn import_preserves_content_and_recovers_after_registration_failure() {
    let temp = tempfile::tempdir().unwrap();
    let state = state(temp.path());
    let path = temp.path().join("existing");
    std::fs::create_dir(&path).unwrap();
    std::fs::write(path.join("user.txt"), "existing data").unwrap();
    assert!(state
        .workspaces()
        .create(&create(&path, "create-existing"), "user")
        .await
        .unwrap_err()
        .to_string()
        .contains("directory_not_empty"));
    let request = WorkspaceImport {
        operation_id: "import-existing".into(),
        location: create(&path, "").location,
        name: Some("Imported".into()),
        description: String::new(),
        usage: WorkspaceUsage::Collaborative,
        expected_revision: None,
        source_session: None,
        policy_ref: None,
    };
    let record = state.workspaces().import(&request, "user").await.unwrap();
    std::fs::remove_file(state.layout().workspace_entry(&record.workspace_id)).unwrap();
    let retry = state.workspaces().import(&request, "user").await.unwrap();
    assert_eq!(record.workspace_id, retry.workspace_id);
    assert_eq!(
        std::fs::read_to_string(path.join("user.txt")).unwrap(),
        "existing data"
    );
    assert_eq!(
        state.workspaces().import(&request, "user").await.unwrap(),
        retry
    );
}

#[tokio::test]
async fn metadata_errors_never_initialize_or_overwrite_identity() {
    let temp = tempfile::tempdir().unwrap();
    let state = state(temp.path());
    for (name, content, expected) in [
        ("corrupt", "invalid json", "invalid_metadata"),
        (
            "future",
            r#"{"version":999,"workspace_id":"ws-12345678-1234-4234-8234-123456789abc","name":"future","created_at_ms":0}"#,
            "unknown_metadata_version",
        ),
    ] {
        let path = temp.path().join(name);
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join(WORKSPACE_METADATA_FILE), content).unwrap();
        let error = state
            .workspaces()
            .create(&create(&path, name), "user")
            .await
            .unwrap_err();
        assert!(error.to_string().contains(expected), "{error}");
        assert_eq!(
            std::fs::read_to_string(path.join(WORKSPACE_METADATA_FILE)).unwrap(),
            content
        );
    }
    assert!(state
        .workspaces()
        .query(&WorkspaceQuery::default())
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn runtime_removal_and_missing_directories_keep_registrations() {
    let temp = tempfile::tempdir().unwrap();
    let state = state(temp.path());
    let path = temp.path().join("project");
    let record = state
        .workspaces()
        .create(&create(&path, "runtime"), "user")
        .await
        .unwrap();
    assert_eq!(
        state
            .workspaces()
            .runtime_impact("local")
            .await
            .unwrap()
            .len(),
        1
    );
    let unavailable = state
        .workspaces()
        .set_runtime_available("local", false, "user")
        .await
        .unwrap();
    assert_eq!(
        unavailable[0].availability,
        WorkspaceAvailability::RuntimeUnavailable
    );
    assert!(path.exists());
    assert!(state
        .workspaces()
        .discover(&discover(&record, &path), "user")
        .await
        .unwrap_err()
        .to_string()
        .contains("runtime_unavailable"));
    let restored = state
        .workspaces()
        .set_runtime_available("local", true, "user")
        .await
        .unwrap();
    assert_eq!(restored[0].availability, WorkspaceAvailability::Available);
    std::fs::rename(&path, temp.path().join("elsewhere")).unwrap();
    assert_eq!(
        state
            .workspaces()
            .check(&record.workspace_id)
            .await
            .unwrap()
            .availability,
        WorkspaceAvailability::Missing
    );
    assert_eq!(
        state
            .workspaces()
            .query(&WorkspaceQuery::default())
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(!path.exists());
}

#[cfg(unix)]
#[tokio::test]
async fn canonical_paths_and_metadata_symlinks_do_not_expand_identity() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let state = state(temp.path());
    let path = temp.path().join("real");
    let alias = temp.path().join("alias");
    std::fs::create_dir(&path).unwrap();
    symlink(&path, &alias).unwrap();
    let record = state
        .workspaces()
        .create(&create(&alias, "alias"), "user")
        .await
        .unwrap();
    assert_eq!(record.location.directory, path);
    let external = temp.path().join("identity.json");
    std::fs::rename(path.join(WORKSPACE_METADATA_FILE), &external).unwrap();
    let original = std::fs::read(&external).unwrap();
    symlink(&external, path.join(WORKSPACE_METADATA_FILE)).unwrap();
    assert!(state
        .workspaces()
        .discover(&discover(&record, &alias), "user")
        .await
        .unwrap_err()
        .to_string()
        .contains("invalid_metadata"));
    assert_eq!(std::fs::read(&external).unwrap(), original);
    assert_eq!(
        state
            .workspaces()
            .check(&record.workspace_id)
            .await
            .unwrap()
            .availability,
        WorkspaceAvailability::InvalidMetadata
    );
}

#[tokio::test]
async fn mismatched_identity_and_operation_reuse_do_not_create_side_effects() {
    let temp = tempfile::tempdir().unwrap();
    let state = state(temp.path());
    let path = temp.path().join("project");
    let record = state
        .workspaces()
        .create(&create(&path, "once"), "user")
        .await
        .unwrap();
    let other = temp.path().join("other");
    assert!(state
        .workspaces()
        .create(&create(&other, "once"), "user")
        .await
        .unwrap_err()
        .to_string()
        .contains("operation_reused"));
    assert!(!other.exists());
    let mut request = discover(&record, &path);
    request.expected_workspace_id = Some("ws-12345678-1234-4234-8234-123456789abc".into());
    assert!(state
        .workspaces()
        .discover(&request, "user")
        .await
        .unwrap_err()
        .to_string()
        .contains("identity_mismatch"));
    assert_eq!(
        state
            .workspaces()
            .lookup(&record.workspace_id)
            .await
            .unwrap()
            .unwrap(),
        record
    );
}

struct Loopback(Arc<FsAgentStateClient>);
#[async_trait]
impl StateTransport for Loopback {
    async fn call(&self, method: &str, params: Value) -> Result<Value> {
        libopendan::state::krpc::serve_call(self.0.as_ref(), "caller", method, &params)
            .await
            .unwrap_or_else(|| Err(OpenDanError::NotFound(method.into())))
    }
}

#[tokio::test]
async fn rpc_uses_server_identity_and_classifies_all_writes() {
    let temp = tempfile::tempdir().unwrap();
    let state = Arc::new(state(temp.path()));
    let client = KrpcAgentStateClient::new("did:test:agent", Arc::new(Loopback(state)));
    let record = client
        .workspaces()
        .create(&create(&temp.path().join("rpc"), "rpc"), "forged")
        .await
        .unwrap();
    assert_eq!(record.registered_by, "caller");
    assert_eq!(
        client
            .workspaces()
            .query(&WorkspaceQuery::default())
            .await
            .unwrap(),
        vec![record.clone()]
    );
    assert_eq!(
        client
            .workspaces()
            .lookup(&record.workspace_id)
            .await
            .unwrap(),
        Some(record)
    );
    for method in [
        "create",
        "import",
        "discover",
        "check",
        "update",
        "archive",
        "unregister",
        "set_runtime_available",
    ] {
        assert!(libopendan::state::krpc::is_write(&format!(
            "workspaces.{method}"
        )));
    }
    assert!(!libopendan::state::krpc::is_write("workspaces.query"));
    assert!(!libopendan::state::krpc::is_write(
        "workspaces.runtime_impact"
    ));
}

#[tokio::test]
async fn active_sessions_block_archive_unregister_and_relocation() {
    let temp = tempfile::tempdir().unwrap();
    let state = state(temp.path());
    let path = temp.path().join("project");
    let moved = temp.path().join("moved");
    let record = state
        .workspaces()
        .create(&create(&path, "active"), "user")
        .await
        .unwrap();
    let session: RegistryEntry = serde_json::from_value(serde_json::json!({
        "session_id": "s-active", "kind": "work", "created_by": {"principal": "user"},
        "driver": {"principal": "user"}, "location": temp.path().join("session"),
        "workspace": {"workspace_id": record.workspace_id, "access": "read_write"},
        "status": SessionStatus::created(libopendan::now_ms())
    }))
    .unwrap();
    state.sessions().register(session, "user").await.unwrap();
    assert!(state
        .workspaces()
        .archive(&record.workspace_id, record.revision, "user")
        .await
        .unwrap_err()
        .to_string()
        .contains("active_sessions"));
    assert!(state
        .workspaces()
        .unregister(&record.workspace_id, record.revision, "user")
        .await
        .unwrap_err()
        .to_string()
        .contains("active_sessions"));
    std::fs::rename(&path, &moved).unwrap();
    assert!(state
        .workspaces()
        .discover(&discover(&record, &moved), "user")
        .await
        .unwrap_err()
        .to_string()
        .contains("active_sessions"));
    let retained = state
        .workspaces()
        .lookup(&record.workspace_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retained.location, record.location);
    assert_eq!(retained.location_revision, record.location_revision);
    assert_eq!(retained.conflict.unwrap().code, "active_sessions");
}

#[tokio::test]
async fn independent_clients_enforce_compare_and_swap_and_overlap_boundaries() {
    let temp = tempfile::tempdir().unwrap();
    let first = state(temp.path());
    let second = state(temp.path());
    let path = temp.path().join("project");
    let record = first
        .workspaces()
        .create(&create(&path, "cas"), "user")
        .await
        .unwrap();
    let updated = second
        .workspaces()
        .update(
            &record.workspace_id,
            &WorkspaceUpdate {
                expected_revision: record.revision,
                lifecycle: None,
                private_notes: Some("agent private notes".into()),
            },
            "user",
        )
        .await
        .unwrap();
    assert!(first
        .workspaces()
        .archive(&record.workspace_id, record.revision, "user")
        .await
        .unwrap_err()
        .to_string()
        .contains("revision_conflict"));
    assert_eq!(
        first
            .workspaces()
            .lookup(&record.workspace_id)
            .await
            .unwrap()
            .unwrap(),
        updated
    );
    let nested = path.join("nested");
    assert!(first
        .workspaces()
        .create(&create(&nested, "nested"), "user")
        .await
        .unwrap_err()
        .to_string()
        .contains("overlapping_directory"));
    assert!(!nested.exists());
    let metadata = std::fs::read_to_string(path.join(WORKSPACE_METADATA_FILE)).unwrap();
    assert!(!metadata.contains("agent private notes"));
}

#[tokio::test]
async fn unknown_registration_version_is_not_silently_rewritten() {
    let temp = tempfile::tempdir().unwrap();
    let state = state(temp.path());
    let record = state
        .workspaces()
        .create(&create(&temp.path().join("project"), "version"), "user")
        .await
        .unwrap();
    let path = state.layout().workspace_entry(&record.workspace_id);
    let mut value = serde_json::to_value(&record).unwrap();
    value["version"] = serde_json::json!(99);
    std::fs::write(&path, value.to_string()).unwrap();
    assert!(state
        .workspaces()
        .lookup(&record.workspace_id)
        .await
        .unwrap_err()
        .to_string()
        .contains("invalid_registration"));
    assert!(state
        .workspaces()
        .check(&record.workspace_id)
        .await
        .is_err());
    let unchanged: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(unchanged["version"], 99);
}

#[tokio::test]
async fn checking_resolves_removed_duplicate_without_changing_identity_or_notes() {
    let temp = tempfile::tempdir().unwrap();
    let state = state(temp.path());
    let original = temp.path().join("original");
    let duplicate = temp.path().join("duplicate");
    let record = state
        .workspaces()
        .create(&create(&original, "duplicate-clear"), "user")
        .await
        .unwrap();
    let record = state
        .workspaces()
        .update(
            &record.workspace_id,
            &WorkspaceUpdate {
                expected_revision: record.revision,
                lifecycle: None,
                private_notes: Some("preserve notes".into()),
            },
            "user",
        )
        .await
        .unwrap();
    std::fs::create_dir(&duplicate).unwrap();
    std::fs::copy(
        original.join(WORKSPACE_METADATA_FILE),
        duplicate.join(WORKSPACE_METADATA_FILE),
    )
    .unwrap();
    assert!(state
        .workspaces()
        .discover(&discover(&record, &duplicate), "user")
        .await
        .is_err());
    let conflict = state
        .workspaces()
        .check(&record.workspace_id)
        .await
        .unwrap();
    assert_eq!(conflict.availability, WorkspaceAvailability::Conflict);
    std::fs::write(duplicate.join(WORKSPACE_METADATA_FILE), "corrupt").unwrap();
    assert_eq!(
        state
            .workspaces()
            .check(&record.workspace_id)
            .await
            .unwrap()
            .availability,
        WorkspaceAvailability::Conflict
    );
    std::fs::remove_file(duplicate.join(WORKSPACE_METADATA_FILE)).unwrap();
    std::fs::remove_dir(&duplicate).unwrap();
    let cleared = state
        .workspaces()
        .check(&record.workspace_id)
        .await
        .unwrap();
    assert_eq!(cleared.availability, WorkspaceAvailability::Available);
    assert_eq!(cleared.location, record.location);
    assert_eq!(cleared.location_revision, record.location_revision);
    assert_eq!(cleared.private_notes, "preserve notes");
    assert!(cleared.conflict.is_none());
}

#[tokio::test]
async fn local_runtime_on_another_host_cannot_reuse_same_path() {
    let temp = tempfile::tempdir().unwrap();
    let state = state(temp.path());
    let path = temp.path().join("project");
    let record = state
        .workspaces()
        .create(&create(&path, "host"), "user")
        .await
        .unwrap();
    let mut foreign = record.clone();
    foreign.runtime_host = format!("{}-other", record.runtime_host);
    libopendan::fsutil::atomic_replace_json(
        &state.layout().workspace_entry(&record.workspace_id),
        &foreign,
    )
    .unwrap();
    let checked = state
        .workspaces()
        .check(&record.workspace_id)
        .await
        .unwrap();
    assert_eq!(
        checked.availability,
        WorkspaceAvailability::RuntimeUnavailable
    );
    assert!(state
        .workspaces()
        .discover(&discover(&checked, &path), "user")
        .await
        .unwrap_err()
        .to_string()
        .contains("old_runtime_unavailable"));
    assert_eq!(
        state
            .workspaces()
            .lookup(&record.workspace_id)
            .await
            .unwrap()
            .unwrap()
            .runtime_host,
        foreign.runtime_host
    );
}

#[tokio::test]
async fn retrying_create_never_recreates_a_lost_registered_directory_or_identity() {
    let temp = tempfile::tempdir().unwrap();
    let state = state(temp.path());
    let path = temp.path().join("project");
    let request = create(&path, "never-recreate");
    let record = state.workspaces().create(&request, "user").await.unwrap();
    let moved = temp.path().join("moved");
    std::fs::rename(&path, &moved).unwrap();
    assert!(state.workspaces().create(&request, "user").await.is_err());
    assert!(!path.exists());
    std::fs::create_dir(&path).unwrap();
    assert!(state
        .workspaces()
        .create(&request, "user")
        .await
        .unwrap_err()
        .to_string()
        .contains("missing_metadata"));
    assert!(!path.join(WORKSPACE_METADATA_FILE).exists());
    assert_eq!(
        state
            .workspaces()
            .lookup(&record.workspace_id)
            .await
            .unwrap()
            .unwrap()
            .location_revision,
        record.location_revision
    );
}
